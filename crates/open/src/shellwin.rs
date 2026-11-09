//! Lets `SHOpenFolderAndSelectItems` ("Show in folder") tell Daleris which file to select.
//!
//! That call opens the folder, then looks in `IShellWindows` for a window showing it, and asks that window's shell view
//! to select the file. So for a few seconds this process stands in as such a window: registered under the folder, with
//! just enough of a browser and view to be asked, and no visible window. None of this is documented; it's the same
//! arrangement Files App and Explorer++ use. If any step fails the folder simply opens with nothing selected.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use windows::core::{BOOL, implement, w, IUnknown, IUnknownImpl, Interface, BSTR, GUID, HRESULT, PCWSTR};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CLOAK};
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::*;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::System::Variant::*;
use windows::Win32::UI::Controls::LPFNSVADDPROPSHEETPAGE;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// How long a folder waits for its selection before opening without one. The call normally asks within milliseconds.
const WAIT_BEFORE_OPENING: Duration = Duration::from_millis(250);
/// How long this stays registered after that, for a slow caller. Anything later would surprise the person.
const WAIT_AFTER_OPENING: Duration = Duration::from_secs(4);

/// Opens a folder through `open`, with the file a "Show in folder" asks for if it does.
/// A selection that arrives after the folder opened goes to `open` a second time.
/// Everything that could hang (Explorer may not be responding) is on another thread, so the folder always opens.
pub fn open_watching_for_selection(folder: &Path, open: impl Fn(Option<PathBuf>) -> bool) -> bool {
    let (tx, rx) = channel();
    let path = folder.to_path_buf();
    let _ = std::thread::Builder::new().name("shell window".into()).spawn(move || { // i18n-ignore: thread name
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| watch(&path, tx)));
    });
    let opened = match rx.recv_timeout(WAIT_BEFORE_OPENING) {
        // stays a moment longer, for the caller to see that it was heard
        Ok(item) => {
            let opened = open(Some(item));
            std::thread::sleep(Duration::from_millis(100));
            return opened;
        }
        Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => open(None),
    };
    if opened {
        if let Ok(item) = rx.recv_timeout(WAIT_AFTER_OPENING) {
            // A selection after the folder already opened. Without the file it would just be the same folder again.
            if open(Some(item)) {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    opened
}

fn watch(folder: &Path, tx: Sender<PathBuf>) -> Option<()> {
    let wide: Vec<u16> = folder.as_os_str().to_os_string().encode_wide_null();
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }.ok().ok()?;
    let mut pidl = std::ptr::null_mut();
    unsafe { SHParseDisplayName(PCWSTR(wide.as_ptr()), None, &mut pidl, 0, None) }.ok()?;
    let hwnd = hidden_window()?;
    let shell_windows: IShellWindows = unsafe { CoCreateInstance(&ShellWindows, None, CLSCTX_LOCAL_SERVER) }.ok()?;
    let view: IWebBrowserApp = ShellWin { hwnd, folder: pidl as usize, tx }.into();
    let dispatch: IDispatch = view.cast().ok()?;
    let size = unsafe { ILGetSize(Some(pidl)) };
    // what `IShellWindows` wants a folder's address in: a byte array of the item ID list
    let location = unsafe { InitVariantFromBuffer(pidl as *const _, size) }.ok()?;
    let empty = VARIANT::default();

    let pending = unsafe { shell_windows.RegisterPending(GetCurrentThreadId() as i32, &location, &empty, SWC_BROWSER) }.ok()?;
    let cookie = unsafe { shell_windows.Register(&dispatch, hwnd.0 as isize as i32, SWC_BROWSER) };
    let cookie = match cookie {
        Ok(c) => c,
        Err(_) => {
            let _ = unsafe { shell_windows.Revoke(pending) };
            return None;
        }
    };
    let _ = unsafe { shell_windows.OnNavigate(cookie, &location) };
    let deadline = Instant::now() + WAIT_BEFORE_OPENING + WAIT_AFTER_OPENING + Duration::from_millis(250);
    let mut msg = MSG::default();
    while Instant::now() < deadline {
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let _ = MsgWaitForMultipleObjectsEx(None, 50, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
        }
    }
    let _ = unsafe { shell_windows.Revoke(cookie) };
    if pending != cookie {
        let _ = unsafe { shell_windows.Revoke(pending) };
    }
    Some(())
}

fn hidden_window() -> Option<HWND> {
    let module = unsafe { GetModuleHandleW(None) }.ok()?;
    let class = WNDCLASSW { lpfnWndProc: Some(window_proc), hInstance: module.into(), lpszClassName: w!("DalerisShellWindow"), ..Default::default() };
    unsafe { RegisterClassW(&class) };
    let hwnd = unsafe { CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, w!("DalerisShellWindow"), w!("Daleris"), WINDOW_STYLE(0), 0, 0, 0, 0, None, None, Some(module.into()), None) }.ok()?; // i18n-ignore: class and title of a window nobody sees
    let cloak: BOOL = true.into();
    let _ = unsafe { DwmSetWindowAttribute(hwnd, DWMWA_CLOAK, &cloak as *const BOOL as *const _, 4) };
    Some(hwnd)
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT { unsafe {
    DefWindowProcW(hwnd, msg, wparam, lparam)
}}

trait WideNull {
    fn encode_wide_null(&self) -> Vec<u16>;
}

impl WideNull for OsString {
    fn encode_wide_null(&self) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        self.encode_wide().chain(std::iter::once(0)).collect()
    }
}

#[implement(IWebBrowserApp, IServiceProvider, IShellView)]
struct ShellWin {
    hwnd: HWND,
    /// the folder's item ID list, kept as a number since this is only ever used on one thread
    folder: usize,
    tx: Sender<PathBuf>,
}

impl ShellWin {
    fn selected(&self, child: *const ITEMIDLIST, flags: u32) -> windows::core::Result<()> {
        const WANTS_IT: u32 = SVSI_SELECT.0 as u32 | SVSI_FOCUSED.0 as u32 | SVSI_ENSUREVISIBLE.0 as u32 | SVSI_EDIT.0 as u32;
        if child.is_null() || flags & WANTS_IT == 0 {
            return Ok(());
        }
        let path = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
            let full = ILCombine(Some(self.folder as *const ITEMIDLIST), Some(child));
            if full.is_null() {
                return None;
            }
            let mut buf = [0u16; 2048];
            let ok = SHGetPathFromIDListEx(full, &mut buf, GPFIDL_DEFAULT).as_bool();
            CoTaskMemFree(Some(full as *const _));
            let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            (ok && end > 0).then(|| PathBuf::from(OsString::from_wide(&buf[..end])))
        }));
        if let Ok(Some(path)) = path {
            let _ = self.tx.send(path);
        }
        Ok(())
    }
}

impl IServiceProvider_Impl for ShellWin_Impl {
    fn QueryService(&self, _service: *const GUID, riid: *const GUID, out: *mut *mut core::ffi::c_void) -> windows::core::Result<()> {
        // SAFETY: COM's rules for this call: both pointers are valid.
        unsafe {
            *out = std::ptr::null_mut();
            let me: IUnknown = self.to_interface();
            me.query(riid, out).ok()
        }
    }
}

impl IOleWindow_Impl for ShellWin_Impl {
    fn GetWindow(&self) -> windows::core::Result<HWND> {
        Ok(self.hwnd)
    }
    fn ContextSensitiveHelp(&self, _: BOOL) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
}

impl IShellView_Impl for ShellWin_Impl {
    fn TranslateAccelerator(&self, _: *const MSG) -> HRESULT {
        S_FALSE
    }
    fn EnableModeless(&self, _: BOOL) -> windows::core::Result<()> {
        Ok(())
    }
    fn UIActivate(&self, _: u32) -> windows::core::Result<()> {
        Ok(())
    }
    fn Refresh(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn CreateViewWindow(&self, _: windows::core::Ref<'_, IShellView>, _: *const FOLDERSETTINGS, _: windows::core::Ref<'_, IShellBrowser>, _: *const RECT) -> windows::core::Result<HWND> {
        Err(E_NOTIMPL.into())
    }
    fn DestroyViewWindow(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn GetCurrentInfo(&self) -> windows::core::Result<FOLDERSETTINGS> {
        Ok(FOLDERSETTINGS::default())
    }
    fn AddPropertySheetPages(&self, _: u32, _: LPFNSVADDPROPSHEETPAGE, _: LPARAM) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn SaveViewState(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn SelectItem(&self, item: *const ITEMIDLIST, flags: u32) -> windows::core::Result<()> {
        self.selected(item, flags)
    }
    fn GetItemObject(&self, _: &_SVGIO, _: *const GUID, out: *mut *mut core::ffi::c_void) -> windows::core::Result<()> {
        if !out.is_null() {
            // SAFETY: COM's rules for this call: the pointer is valid when it isn't null.
            unsafe { *out = std::ptr::null_mut() };
        }
        Err(E_NOINTERFACE.into())
    }
}

impl IDispatch_Impl for ShellWin_Impl {
    fn GetTypeInfoCount(&self) -> windows::core::Result<u32> {
        Ok(0)
    }
    fn GetTypeInfo(&self, _: u32, _: u32) -> windows::core::Result<ITypeInfo> {
        Err(E_NOTIMPL.into())
    }
    fn GetIDsOfNames(&self, _: *const GUID, _: *const PCWSTR, _: u32, _: u32, _: *mut i32) -> windows::core::Result<()> {
        Err(DISP_E_UNKNOWNNAME.into())
    }
    fn Invoke(&self, _: i32, _: *const GUID, _: u32, _: DISPATCH_FLAGS, _: *const DISPPARAMS, _: *mut VARIANT, _: *mut EXCEPINFO, _: *mut u32) -> windows::core::Result<()> {
        Err(DISP_E_MEMBERNOTFOUND.into())
    }
}

impl IWebBrowser_Impl for ShellWin_Impl {
    fn GoBack(&self) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn GoForward(&self) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn GoHome(&self) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn GoSearch(&self) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn Navigate(&self, _: &BSTR, _: *const VARIANT, _: *const VARIANT, _: *const VARIANT, _: *const VARIANT) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn Refresh(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn Refresh2(&self, _: *const VARIANT) -> windows::core::Result<()> {
        Ok(())
    }
    fn Stop(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn Application(&self) -> windows::core::Result<IDispatch> {
        self.to_interface::<IUnknown>().cast()
    }
    fn Parent(&self) -> windows::core::Result<IDispatch> {
        self.to_interface::<IUnknown>().cast()
    }
    fn Container(&self) -> windows::core::Result<IDispatch> {
        self.to_interface::<IUnknown>().cast()
    }
    fn Document(&self) -> windows::core::Result<IDispatch> {
        self.to_interface::<IUnknown>().cast()
    }
    fn TopLevelContainer(&self) -> windows::core::Result<VARIANT_BOOL> {
        Ok(VARIANT_TRUE)
    }
    fn Type(&self) -> windows::core::Result<BSTR> {
        Ok(BSTR::from("Daleris")) // i18n-ignore: a name Windows asks for, never shown
    }
    fn Left(&self) -> windows::core::Result<i32> {
        Ok(0)
    }
    fn SetLeft(&self, _: i32) -> windows::core::Result<()> {
        Ok(())
    }
    fn Top(&self) -> windows::core::Result<i32> {
        Ok(0)
    }
    fn SetTop(&self, _: i32) -> windows::core::Result<()> {
        Ok(())
    }
    fn Width(&self) -> windows::core::Result<i32> {
        Ok(0)
    }
    fn SetWidth(&self, _: i32) -> windows::core::Result<()> {
        Ok(())
    }
    fn Height(&self) -> windows::core::Result<i32> {
        Ok(0)
    }
    fn SetHeight(&self, _: i32) -> windows::core::Result<()> {
        Ok(())
    }
    fn LocationName(&self) -> windows::core::Result<BSTR> {
        Ok(BSTR::new())
    }
    fn LocationURL(&self) -> windows::core::Result<BSTR> {
        Ok(BSTR::new())
    }
    fn Busy(&self) -> windows::core::Result<VARIANT_BOOL> {
        Ok(VARIANT_FALSE)
    }
}

impl IWebBrowserApp_Impl for ShellWin_Impl {
    fn Quit(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn ClientToWindow(&self, _: *mut i32, _: *mut i32) -> windows::core::Result<()> {
        Ok(())
    }
    fn PutProperty(&self, _: &BSTR, _: &VARIANT) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn GetProperty(&self, _: &BSTR) -> windows::core::Result<VARIANT> {
        Err(E_NOTIMPL.into())
    }
    fn Name(&self) -> windows::core::Result<BSTR> {
        Ok(BSTR::from("Daleris")) // i18n-ignore: a name Windows asks for, never shown
    }
    fn HWND(&self) -> windows::core::Result<SHANDLE_PTR> {
        Ok(SHANDLE_PTR(self.hwnd.0 as isize))
    }
    fn FullName(&self) -> windows::core::Result<BSTR> {
        let path = std::env::current_exe().map_err(|_| windows::core::Error::from(E_FAIL))?;
        Ok(BSTR::from(path.display().to_string()))
    }
    fn Path(&self) -> windows::core::Result<BSTR> {
        self.FullName()
    }
    fn Visible(&self) -> windows::core::Result<VARIANT_BOOL> {
        Ok(VARIANT_FALSE)
    }
    fn SetVisible(&self, _: VARIANT_BOOL) -> windows::core::Result<()> {
        Ok(())
    }
    fn StatusBar(&self) -> windows::core::Result<VARIANT_BOOL> {
        Ok(VARIANT_FALSE)
    }
    fn SetStatusBar(&self, _: VARIANT_BOOL) -> windows::core::Result<()> {
        Ok(())
    }
    fn StatusText(&self) -> windows::core::Result<BSTR> {
        Ok(BSTR::new())
    }
    fn SetStatusText(&self, _: &BSTR) -> windows::core::Result<()> {
        Ok(())
    }
    fn ToolBar(&self) -> windows::core::Result<i32> {
        Ok(0)
    }
    fn SetToolBar(&self, _: i32) -> windows::core::Result<()> {
        Ok(())
    }
    fn MenuBar(&self) -> windows::core::Result<VARIANT_BOOL> {
        Ok(VARIANT_FALSE)
    }
    fn SetMenuBar(&self, _: VARIANT_BOOL) -> windows::core::Result<()> {
        Ok(())
    }
    fn FullScreen(&self) -> windows::core::Result<VARIANT_BOOL> {
        Ok(VARIANT_FALSE)
    }
    fn SetFullScreen(&self, _: VARIANT_BOOL) -> windows::core::Result<()> {
        Ok(())
    }
}
