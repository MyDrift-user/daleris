//! The in-process COM server behind "Open and Save dialogs use Daleris".
//!
//! Why COM activation and not a hook: a per-user registration of `CLSID_FileOpenDialog` and `CLSID_FileSaveDialog` means
//! Windows loads this only into a process that is creating one of those dialogs, right then. Nothing is loaded into
//! every process and nothing patches any code, and turning the switch off is deleting a registry key. The price is that
//! apps that don't use these classes (old `GetOpenFileName` apps, packaged apps) keep Windows' dialog, which is safe.
//!
//! What it does in the other app's process is deliberately little:
//! - `DllGetClassObject` decides first. Anything on the list in `deny.rs`, or anything elevated, packaged, sandboxed or
//!   unsure, gets `comdlg32`'s own class factory back and this DLL takes no further part.
//! - Otherwise each dialog object wraps a real one. Everything the app sets also goes to the real one, so that it's
//!   ready to take over at any moment, and holds what the app reads back. `Show` is what's different: it starts Daleris
//!   as a separate process for the window, which also shows the controls the app added and reports back what's done
//!   with them as it happens.
//! - No wait here is open-ended. If Daleris can't start, doesn't answer, hangs or dies, the real dialog is shown instead.
//! - Every entry point catches panics, as unwinding into someone else's app would end it.

#![cfg(windows)]

mod log;
mod modal;
mod proxy;

use std::ffi::{c_void, OsString};
use std::os::windows::ffi::OsStringExt;
use std::sync::atomic::{AtomicUsize, Ordering};

use daleris_integration::deny;
use daleris_integration::reg::{policy_fixed, Hive};
use daleris_integration::Feature;
use windows::core::{BOOL, implement, IUnknown, Interface, GUID, HRESULT};
use windows::Win32::Foundation::*;
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TokenIsAppContainer, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
use windows::Win32::System::LibraryLoader::{GetModuleFileNameW, GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};
use windows::Win32::System::ProcessStatus::{EnumProcessModules, GetModuleBaseNameW};
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId, OpenProcessToken};
use windows::Win32::UI::Shell::{IFileOpenDialog, IFileSaveDialog};

const CLSID_OPEN: GUID = GUID::from_u128(0xDC1C5A9C_E88A_4DDE_A5A1_60F82A20AEF7);
const CLSID_SAVE: GUID = GUID::from_u128(0xC0B4E2F3_BA21_4773_8DBA_335EC946EB8B);

/// Objects of ours alive, and server locks: while either is above 0 the DLL must stay loaded.
static LIVE: AtomicUsize = AtomicUsize::new(0);

pub(crate) struct Alive;

impl Alive {
    pub(crate) fn new() -> Alive {
        LIVE.fetch_add(1, Ordering::SeqCst);
        Alive
    }
}

impl Drop for Alive {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Runs code from another app's call, with a panic turned into a failure code.
pub(crate) fn guard<T>(f: impl FnOnce() -> windows::core::Result<T>) -> windows::core::Result<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| Err(E_FAIL.into()))
}

type GetClassObject = unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut c_void) -> HRESULT;

/// `comdlg32`'s own class factory, from the system folder only.
fn real_factory(clsid: &GUID) -> windows::core::Result<IClassFactory> {
    let module = unsafe { LoadLibraryExW(windows::core::w!("comdlg32.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }?;
    let entry = unsafe { GetProcAddress(module, windows::core::s!("DllGetClassObject")) }.ok_or_else(windows::core::Error::from_thread)?;
    // SAFETY: that export has this signature in every COM server.
    let get: GetClassObject = unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, GetClassObject>(entry) };
    let mut out = std::ptr::null_mut();
    unsafe { get(clsid, &IClassFactory::IID, &mut out) }.ok()?;
    // SAFETY: on success it handed over an IClassFactory reference.
    Ok(unsafe { IClassFactory::from_raw(out) })
}

/// This program's path.
pub(crate) fn host_path() -> String {
    let mut buf = vec![0u16; 2048];
    let len = unsafe { GetModuleFileNameW(None, &mut buf) } as usize;
    OsString::from_wide(&buf[..len.min(buf.len())]).to_string_lossy().into_owned()
}

fn token_says(class: windows::Win32::Security::TOKEN_INFORMATION_CLASS) -> bool {
    let mut token = HANDLE::default();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.is_err() {
        return true;
    }
    let mut value = TOKEN_ELEVATION::default();
    let mut len = 0;
    let got = unsafe { GetTokenInformation(token, class, Some(&mut value as *mut _ as *mut c_void), std::mem::size_of::<TOKEN_ELEVATION>() as u32, &mut len) };
    let _ = unsafe { CloseHandle(token) };
    got.is_err() || value.TokenIsElevated != 0
}

fn any_watchful_module() -> bool {
    let process = unsafe { GetCurrentProcess() };
    let mut modules = vec![HMODULE::default(); 1024];
    let mut needed = 0;
    let size = std::mem::size_of_val(modules.as_slice()) as u32;
    if unsafe { EnumProcessModules(process, modules.as_mut_ptr(), size, &mut needed) }.is_err() {
        return true;
    }
    let count = (needed as usize / std::mem::size_of::<HMODULE>()).min(modules.len());
    modules[..count].iter().any(|&m| {
        let mut name = [0u16; 260];
        let len = unsafe { GetModuleBaseNameW(process, Some(m), &mut name) } as usize;
        deny::is_watchful_module(&String::from_utf16_lossy(&name[..len]))
    })
}

/// Why this process gets Windows' own dialog, or None if Daleris may show it. Anything unsure means Windows' own.
fn leave_alone() -> Option<&'static str> {
    let hive = Hive::real();
    if policy_fixed(Feature::Dialogs) == Some(false) || !hive.flag(Feature::Dialogs) {
        return Some("switched off");
    }
    if let Some(reason) = deny::reason_to_leave_alone(&host_path(), &hive.exclusions()) {
        return Some(reason);
    }
    if !hive.program_path().is_some_and(|p| p.is_file()) {
        return Some("Daleris isn't there");
    }
    if token_says(TokenElevation) {
        return Some("elevated");
    }
    if token_says(TokenIsAppContainer) {
        return Some("sandboxed");
    }
    let mut len = 0;
    if unsafe { GetCurrentPackageFullName(&mut len, Some(windows::core::PWSTR::null())) } != APPMODEL_ERROR_NO_PACKAGE {
        return Some("packaged");
    }
    let mut session = 0;
    if unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) }.is_err() || session == 0 {
        return Some("no desktop");
    }
    if any_watchful_module() {
        return Some("game or anti-cheat module loaded");
    }
    None
}

#[implement(IClassFactory)]
struct Factory {
    clsid: GUID,
    real: IClassFactory,
    _alive: Alive,
}

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(&self, outer: windows::core::Ref<'_, IUnknown>, iid: *const GUID, out: *mut *mut c_void) -> windows::core::Result<()> {
        let outer = outer.as_ref();
        let real = self.real.clone();
        let clsid = self.clsid;
        let made = guard(|| {
            if outer.is_some() {
                // Aggregated: the real class says what that means.
                return Err(CLASS_E_NOAGGREGATION.into());
            }
            // SAFETY: COM's rules for this call: the pointers are valid.
            unsafe { *out = std::ptr::null_mut() };
            let object: IUnknown = if clsid == CLSID_OPEN {
                let real_dialog: IFileOpenDialog = unsafe { real.CreateInstance(None) }?;
                proxy::open(real_dialog)?
            } else {
                let real_dialog: IFileSaveDialog = unsafe { real.CreateInstance(None) }?;
                proxy::save(real_dialog)?
            };
            unsafe { object.query(iid, out) }.ok()
        });
        match made {
            Ok(()) => Ok(()),
            // never leave an app without a dialog: the real one, without us
            Err(e) => {
                log::note(&format!("Windows' own dialog: couldn't make ours: {:?}", e.code()));
                if outer.is_some() {
                    return Err(e);
                }
                let mut object: *mut c_void = std::ptr::null_mut();
                // SAFETY: COM's rules; `iid` and `out` came from the caller.
                unsafe { (Interface::vtable(&self.real).CreateInstance)(Interface::as_raw(&self.real), std::ptr::null_mut(), iid, &mut object) }.ok()?;
                unsafe { *out = object };
                Ok(())
            }
        }
    }

    fn LockServer(&self, lock: BOOL) -> windows::core::Result<()> {
        if lock.as_bool() {
            LIVE.fetch_add(1, Ordering::SeqCst);
        } else {
            LIVE.fetch_sub(1, Ordering::SeqCst);
        }
        Ok(())
    }
}

/// COM's way in.
///
/// # Safety
/// Called by COM with valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(clsid: *const GUID, iid: *const GUID, out: *mut *mut c_void) -> HRESULT { unsafe {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> HRESULT {
        if out.is_null() || clsid.is_null() || iid.is_null() {
            return E_POINTER;
        }
        *out = std::ptr::null_mut();
        let clsid = *clsid;
        log::refresh();
        if clsid != CLSID_OPEN && clsid != CLSID_SAVE {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let what = if clsid == CLSID_OPEN { "Open" } else { "Save" };
        log::note(&format!("an app asked for a {what} dialog, from {}", log::this_dll()));
        let real = match real_factory(&clsid) {
            Ok(f) => f,
            Err(e) => {
                log::note(&format!("couldn't reach Windows' own class: {:?}", e.code()));
                return e.code();
            }
        };
        // An unexpected panic in these checks counts as a reason too.
        let alone = std::panic::catch_unwind(leave_alone).unwrap_or(Some("something went wrong"));
        if let Some(reason) = alone {
            log::note(&format!("Windows' own dialog: {reason}"));
            return real.query(iid, out);
        }
        log::note("Daleris may show this one");
        let factory: IClassFactory = Factory { clsid, real, _alive: Alive::new() }.into();
        factory.query(iid, out)
    }));
    result.unwrap_or(E_FAIL)
}}

/// COM asks this before unloading the DLL.
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if LIVE.load(Ordering::SeqCst) == 0 {
        S_OK
    } else {
        S_FALSE
    }
}
