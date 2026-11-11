//! Windows's shell namespace, for places with no file system path: phones and cameras connected by USB (portable
//! devices, over MTP). They're read through shell items, and copied to and from with `IFileOperation`, which shows
//! Windows's own progress. How such a place is written as a path is in `shellpath.rs`.
//!
//! Everything here can take as long as the device takes (MTP is slow), so none of it runs on the window's thread.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use windows::core::{w, Interface, GUID, PCWSTR, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::SystemServices::{SFGAO_FLAGS, SFGAO_FOLDER, SFGAO_HIDDEN, SFGAO_STREAM};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::fscommon::{Entry, FailKind, FolderError, Watcher, A_DIR, A_HIDDEN};
use crate::i18n::tr;
use crate::shellpath::{self, Device};
use crate::util::{ext_of, wide};

// The storage property set: name, size and dates of an item, whatever kind of place it's in.
const FMTID_STORAGE: GUID = GUID::from_u128(0xb725f130_47ef_101a_a5f1_02608c9eebac);
const PKEY_ITEM_NAME_DISPLAY: PROPERTYKEY = PROPERTYKEY { fmtid: FMTID_STORAGE, pid: 10 };
const PKEY_SIZE: PROPERTYKEY = PROPERTYKEY { fmtid: FMTID_STORAGE, pid: 12 };
const PKEY_DATE_MODIFIED: PROPERTYKEY = PROPERTYKEY { fmtid: FMTID_STORAGE, pid: 14 };
const PKEY_DATE_CREATED: PROPERTYKEY = PROPERTYKEY { fmtid: FMTID_STORAGE, pid: 15 };
/// Portable devices announce themselves with this device interface.
const WPD_INTERFACE: GUID = GUID::from_u128(0x6ac27878_a6fa_4155_ba85_f98f491d4f33);

/// An item ID list, freed when dropped.
pub struct Idl(pub *mut ITEMIDLIST);

impl Drop for Idl {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { ILFree(Some(self.0 as *const _)) };
        }
    }
}

impl Idl {
    fn bytes(&self) -> Vec<u8> {
        unsafe {
            let n = ILGetSize(Some(self.0 as *const _)) as usize;
            std::slice::from_raw_parts(self.0 as *const u8, n).to_vec()
        }
    }

    fn from_bytes(b: &[u8]) -> Option<Idl> {
        if b.len() < 2 {
            return None;
        }
        unsafe {
            let p = CoTaskMemAlloc(b.len()) as *mut u8;
            if p.is_null() {
                return None;
            }
            std::ptr::copy_nonoverlapping(b.as_ptr(), p, b.len());
            Some(Idl(p as *mut ITEMIDLIST))
        }
    }

    /// Hands the list over; the taker frees it.
    pub fn into_raw(self) -> *mut ITEMIDLIST {
        let p = self.0;
        std::mem::forget(self);
        p
    }
}

/// What was seen of each place: its item ID list, so it can be found again without walking down from the device, and
/// what it looked like in its folder's list.
struct Known {
    idl: Vec<u8>,
    entry: Option<Entry>,
}

static KNOWN: Mutex<Option<HashMap<String, Known>>> = Mutex::new(None);
static DEVICES: Mutex<Vec<Device>> = Mutex::new(Vec::new());

fn remember(p: &Path, idl: &Idl, entry: Option<Entry>) {
    let mut all = KNOWN.lock().unwrap_or_else(PoisonError::into_inner);
    let all = all.get_or_insert_with(HashMap::new);
    // a list nobody goes through by hand: it only needs to not grow forever
    if all.len() > 200_000 {
        all.clear();
    }
    all.insert(shellpath::key(p), Known { idl: idl.bytes(), entry });
}

fn known_idl(p: &Path) -> Option<Idl> {
    let all = KNOWN.lock().unwrap_or_else(PoisonError::into_inner);
    Idl::from_bytes(&all.as_ref()?.get(&shellpath::key(p))?.idl)
}

fn forget(p: &Path) {
    if let Some(all) = KNOWN.lock().unwrap_or_else(PoisonError::into_inner).as_mut() {
        all.remove(&shellpath::key(p));
    }
}

fn fail(e: &windows::core::Error) -> FolderError {
    let code = e.code().0 as u32;
    let kind = match code {
        // not there, or the device went away
        0x8007_0002 | 0x8007_0003 | 0x8007_048F | 0x8002_802B | 0x8007_0490 => FailKind::NotFound,
        0x8007_0005 => FailKind::AccessDenied,
        0x8007_04C7 => FailKind::Cancelled,
        _ => FailKind::Other,
    };
    let hex = hex(code);
    FolderError::new(kind, trf!("{message} (error {code})", message = e.message().trim(), code = hex))
}

/// An error code as Windows writes it, 0x80070005.
fn hex(code: u32) -> String {
    format!("{code:#x}")
}

fn not_found(p: &Path) -> FolderError {
    FolderError::new(FailKind::NotFound, p.display().to_string())
}

fn take_wide(p: PWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    let s = unsafe { p.to_string() }.unwrap_or_default();
    unsafe { CoTaskMemFree(Some(p.0 as *const _)) };
    s
}

/// The name a person knows an item by, with its extension whatever Explorer's settings hide, fit to be part of a path.
fn name_of(it: &IShellItem2) -> String {
    let n = unsafe { it.GetString(&PKEY_ITEM_NAME_DISPLAY) }.map(take_wide).unwrap_or_default();
    let n = if n.is_empty() { unsafe { it.GetDisplayName(SIGDN_PARENTRELATIVEEDITING) }.map(take_wide).unwrap_or_default() } else { n };
    let n = if n.is_empty() { unsafe { it.GetDisplayName(SIGDN_NORMALDISPLAY) }.map(take_wide).unwrap_or_default() } else { n };
    shellpath::clean_segment(&n)
}

fn filetime(it: &IShellItem2, key: &PROPERTYKEY) -> u64 {
    unsafe { it.GetFileTime(key) }.map(|f| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64).unwrap_or(0)
}

fn entry_of(it: &IShellItem2) -> Entry {
    let name = name_of(it);
    let a = unsafe { it.GetAttributes(SFGAO_FOLDER | SFGAO_STREAM | SFGAO_HIDDEN) }.unwrap_or(SFGAO_FLAGS(0));
    // a zip file on a phone is a folder to the shell, and a file to the person
    let is_dir = a.0 & SFGAO_FOLDER.0 != 0 && a.0 & SFGAO_STREAM.0 == 0;
    let mut attrs = if is_dir { A_DIR } else { 0 };
    if a.0 & SFGAO_HIDDEN.0 != 0 {
        attrs |= A_HIDDEN;
    }
    Entry {
        lower: name.to_lowercase(),
        ext: ext_of(&name, is_dir),
        is_dir,
        size: if is_dir { 0 } else { unsafe { it.GetUInt64(&PKEY_SIZE) }.unwrap_or(0) },
        mtime: filetime(it, &PKEY_DATE_MODIFIED),
        ctime: filetime(it, &PKEY_DATE_CREATED),
        atime: 0,
        attrs,
        name,
    }
}

/// Calls `f` with each item in `folder` and its item ID list, until `f` says stop or `cancel` is set.
fn each_child(folder: &IShellItem, cancel: &AtomicBool, mut f: impl FnMut(&IShellItem2, &Idl) -> bool) -> Result<(), FolderError> {
    unsafe {
        let parent = Idl(SHGetIDListFromObject(folder).map_err(|e| fail(&e))?);
        let sf: IShellFolder = folder.BindToHandler(None, &BHID_SFObject).map_err(|e| fail(&e))?;
        let mut en: Option<IEnumIDList> = None;
        let flags = (SHCONTF_FOLDERS.0 | SHCONTF_NONFOLDERS.0 | SHCONTF_INCLUDEHIDDEN.0) as u32;
        sf.EnumObjects(HWND::default(), flags, &mut en).ok().map_err(|e| fail(&e))?;
        // nothing in it
        let Some(en) = en else { return Ok(()) };
        loop {
            let mut buf = [std::ptr::null_mut(); 32];
            let mut got = 0u32;
            let hr = en.Next(&mut buf, Some(&mut got));
            for &child in &buf[..got as usize] {
                let child = Idl(child);
                let abs = Idl(ILCombine(Some(parent.0 as *const _), Some(child.0 as *const _)));
                if abs.0.is_null() {
                    continue;
                }
                if let Ok(it) = SHCreateItemFromIDList::<IShellItem2>(abs.0) {
                    if !f(&it, &abs) {
                        return Ok(());
                    }
                }
            }
            hr.ok().map_err(|e| fail(&e))?;
            if hr != S_OK || got == 0 || cancel.load(Ordering::Relaxed) {
                return Ok(());
            }
        }
    }
}

/// The shell item at a shell place: from what was seen of it before, or found from the device down, by name.
pub fn item(p: &Path) -> Result<IShellItem, FolderError> {
    crate::shell::init_com();
    let s = p.to_string_lossy().to_string();
    let (root, segs) = shellpath::split(&s).ok_or_else(|| not_found(p))?;
    // the deepest part already known
    let mut start = None;
    for n in (0..=segs.len()).rev() {
        let at = shellpath::build(root, &segs[..n]);
        if let Some(idl) = known_idl(&at) {
            match unsafe { SHCreateItemFromIDList::<IShellItem>(idl.0) } {
                Ok(it) => {
                    start = Some((n, it));
                    break;
                }
                Err(_) => forget(&at),
            }
        }
    }
    let (mut n, mut it) = match start {
        Some(s) => s,
        None => {
            let w = wide(root);
            let it: IShellItem = unsafe { SHCreateItemFromParsingName(PCWSTR(w.as_ptr()), None) }.map_err(|e| fail(&e))?;
            if let Ok(idl) = unsafe { SHGetIDListFromObject(&it) } {
                remember(Path::new(root), &Idl(idl), None);
            }
            // its name, for the tab and the address bar, when the device list hasn't been read yet
            if let Ok(name) = unsafe { it.GetDisplayName(SIGDN_NORMALDISPLAY) }.map(take_wide) {
                shellpath::remember_name(Path::new(root), &name);
            }
            (0, it)
        }
    };
    let never = AtomicBool::new(false);
    while n < segs.len() {
        let here = shellpath::build(root, &segs[..n]);
        let want = segs[n].to_lowercase();
        let mut found = None;
        // what's in the folder is remembered on the way, so the next look needn't walk it again
        each_child(&it, &never, |child, idl| {
            let e = entry_of(child);
            let hit = e.lower == want;
            remember(&here.join(&e.name), idl, Some(e));
            if hit {
                found = child.cast::<IShellItem>().ok();
            }
            !hit
        })?;
        it = found.ok_or_else(|| not_found(p))?;
        n += 1;
    }
    Ok(it)
}

/// The item ID list of a shell place, for the shell calls that take one.
pub fn idl(p: &Path) -> Option<Idl> {
    known_idl(p).or_else(|| item(p).ok().and_then(|it| unsafe { SHGetIDListFromObject(&it) }.ok().map(Idl)))
}

/// Streams what's in a shell folder, a few at a time: a phone sends its list slowly.
pub fn list(p: &Path, cancel: &AtomicBool, mut emit: impl FnMut(Vec<Entry>)) -> Result<(), FolderError> {
    let folder = item(p)?;
    let mut batch = Vec::new();
    let mut last = Instant::now();
    each_child(&folder, cancel, |child, idl| {
        let e = entry_of(child);
        remember(&p.join(&e.name), idl, Some(e.clone()));
        batch.push(e);
        if batch.len() >= 512 || last.elapsed() > Duration::from_millis(250) {
            emit(std::mem::take(&mut batch));
            last = Instant::now();
        }
        true
    })?;
    if !batch.is_empty() {
        emit(batch);
    }
    Ok(())
}

/// What a shell place looked like when its folder was last listed. Doesn't ask the device.
pub fn stat(p: &Path) -> Option<Entry> {
    let all = KNOWN.lock().unwrap_or_else(PoisonError::into_inner);
    all.as_ref()?.get(&shellpath::key(p))?.entry.clone()
}

/// Whether the shell place is still there.
pub fn check(p: &Path) -> Result<(), FolderError> {
    forget(p);
    item(p).map(|_| ())
}

/// The shell place an item is, if it's on a device: its names walked up to the device.
fn path_of(it: &IShellItem) -> Option<PathBuf> {
    let mut names = Vec::new();
    let mut cur = it.clone();
    for _ in 0..128 {
        let parsing = unsafe { cur.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING) }.map(take_wide).ok()?;
        if let Some((root, segs)) = shellpath::split(&parsing) {
            if segs.is_empty() && root.len() == parsing.len() {
                names.reverse();
                let p = shellpath::build(root, &names.iter().map(String::as_str).collect::<Vec<_>>());
                if let (Ok(idl), Ok(it2)) = (unsafe { SHGetIDListFromObject(it) }, it.cast::<IShellItem2>()) {
                    remember(&p, &Idl(idl), Some(entry_of(&it2)));
                }
                return Some(p);
            }
        }
        names.push(name_of(&cur.cast::<IShellItem2>().ok()?));
        cur = unsafe { cur.GetParent() }.ok()?;
    }
    None
}

/// A shell item for any path, on a device or not.
fn item_any(p: &Path) -> Result<IShellItem, String> {
    if shellpath::is_shell(p) {
        return item(p).map_err(|e| e.detail);
    }
    let w = wide(p.as_os_str());
    unsafe { SHCreateItemFromParsingName(PCWSTR(w.as_ptr()), None) }.map_err(|e| e.message().to_string())
}

// ---------------------------------------------------------------- devices

/// The phones, cameras and other devices in This PC that have no drive letter, as last seen.
pub fn devices() -> Vec<Device> {
    DEVICES.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// The device interfaces present, which change as a phone is connected or taken away. Cheap: no shell, no device.
fn wpd_signature() -> Vec<u16> {
    use windows::Win32::Devices::DeviceAndDriverInstallation::*;
    unsafe {
        let mut len = 0u32;
        if CM_Get_Device_Interface_List_SizeW(&mut len, &WPD_INTERFACE, PCWSTR::null(), CM_GET_DEVICE_INTERFACE_LIST_PRESENT) != CR_SUCCESS {
            return Vec::new();
        }
        let mut buf = vec![0u16; len as usize + 1];
        if CM_Get_Device_Interface_ListW(&WPD_INTERFACE, PCWSTR::null(), &mut buf, CM_GET_DEVICE_INTERFACE_LIST_PRESENT) != CR_SUCCESS {
            return Vec::new();
        }
        buf
    }
}

/// This PC's children that are devices without a drive letter, read from the shell.
fn read_devices() -> Vec<Device> {
    let w = wide(shellpath::THIS_PC);
    let Ok(pc) = (unsafe { SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(w.as_ptr()), None) }) else { return Vec::new() };
    let prefix = format!("{}\\\\\\?\\", shellpath::THIS_PC).to_lowercase();
    let mut out = Vec::new();
    let _ = each_child(&pc, &AtomicBool::new(false), |child, idl| {
        let parsing = unsafe { child.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING) }.map(take_wide).unwrap_or_default();
        if parsing.to_lowercase().starts_with(&prefix) {
            let name = unsafe { child.GetDisplayName(SIGDN_NORMALDISPLAY) }.map(take_wide).unwrap_or_default();
            let root = PathBuf::from(&parsing);
            remember(&root, idl, None);
            shellpath::remember_name(&root, &name);
            out.push(Device { root, name });
        }
        true
    });
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// Keeps the device list up to date, telling `changed` when a device comes or goes, as drives are watched.
pub fn watch_devices(changed: impl Fn() + Send + 'static) {
    crate::bg::spawn("device-watch", move || {
        crate::shell::init_com();
        let mut last: Option<Vec<u16>> = None;
        loop {
            let now = wpd_signature();
            if last.as_ref() != Some(&now) {
                last = Some(now);
                // a device's shell folder turns up a moment after its interface
                for wait in [0, 1500, 4000] {
                    std::thread::sleep(Duration::from_millis(wait));
                    let found = read_devices();
                    let mut cur = DEVICES.lock().unwrap_or_else(PoisonError::into_inner);
                    if *cur != found {
                        log::info!("portable devices: {:?}", found.iter().map(|d| d.name.as_str()).collect::<Vec<_>>());
                        *cur = found;
                        drop(cur);
                        changed();
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}

// ---------------------------------------------------------------- opening

/// Opens an item on a device with its default action, as Explorer does: Windows copies it over first where it has to.
pub fn open(p: &Path, owner: isize) {
    let p = p.to_path_buf();
    crate::bg::submit(&p.clone(), "open", move || {
        crate::shell::init_com();
        let Some(idl) = idl(&p) else {
            log::warn!("can't open {}: it isn't there", p.display());
            return;
        };
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_IDLIST | SEE_MASK_NOASYNC,
            hwnd: HWND(owner as *mut _),
            lpIDList: idl.0 as *mut _,
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        if let Err(e) = unsafe { ShellExecuteExW(&mut info) } {
            log::warn!("can't open {}: {e}", p.display());
        }
    });
}

// ---------------------------------------------------------------- copying, moving, deleting

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShellOp {
    Copy,
    Move,
    /// for good: Daleris asked first
    Delete,
    Recycle,
}

/// Copies, moves or deletes with Windows's own file operation and progress, on a thread of its own. For anything on a
/// device, which Daleris's own copy can't reach.
pub fn file_op(op: ShellOp, srcs: Vec<PathBuf>, dest: Option<PathBuf>, done: impl FnOnce(Result<(), String>) + Send + 'static) {
    let owner = crate::shell::main_hwnd().0 as isize;
    crate::bg::spawn("shell-op", move || {
        crate::shell::init_com();
        done(run(op, &srcs, dest.as_deref(), Some(HWND(owner as *mut _)), false));
    });
}

fn operation(owner: Option<HWND>, quiet: bool, flags: u32) -> Result<IFileOperation, String> {
    let fo: IFileOperation = unsafe { CoCreateInstance(&FileOperation, None, CLSCTX_ALL) }.map_err(|e| e.message().to_string())?;
    let mut flags = flags;
    if quiet {
        flags |= FOF_SILENT.0 | FOF_NOCONFIRMATION.0 | FOF_NOERRORUI.0 | FOF_NOCONFIRMMKDIR.0;
    }
    unsafe {
        fo.SetOperationFlags(FILEOPERATION_FLAGS(flags)).map_err(|e| e.message().to_string())?;
        if let Some(h) = owner {
            let _ = fo.SetOwnerWindow(h);
        }
    }
    Ok(fo)
}

fn perform(fo: &IFileOperation) -> Result<(), String> {
    let r = unsafe { fo.PerformOperations() };
    log::debug!("file operation: {r:?}, aborted {:?}", unsafe { fo.GetAnyOperationsAborted() });
    // called off by the person, in Windows's own window
    if unsafe { fo.GetAnyOperationsAborted() }.is_ok_and(|a| a.as_bool()) {
        return Ok(());
    }
    r.map_err(|e| {
        let code = hex(e.code().0 as u32);
        trf!("Operation failed (code {code})", code = code)
    })
}

/// Does `op` and waits for it. `owner` is the window Windows's progress and questions belong to; `quiet` asks
/// nothing and shows nothing.
pub fn run(op: ShellOp, srcs: &[PathBuf], dest: Option<&Path>, owner: Option<HWND>, quiet: bool) -> Result<(), String> {
    let flags = match op {
        ShellOp::Copy | ShellOp::Move => FOF_ALLOWUNDO.0 | FOF_NOCONFIRMMKDIR.0,
        // Daleris asked already
        ShellOp::Delete => FOF_NOCONFIRMATION.0,
        // a device has no Recycle Bin: Windows says so, and asks whether to delete for good
        ShellOp::Recycle => FOF_ALLOWUNDO.0 | FOFX_RECYCLEONDELETE.0,
    };
    let fo = operation(owner, quiet, flags)?;
    let to = match (op, dest) {
        (ShellOp::Copy | ShellOp::Move, Some(d)) => Some(item_any(d)?),
        (ShellOp::Copy | ShellOp::Move, None) => return Err(tr("File not found").into()),
        _ => None,
    };
    for s in srcs {
        let it = item_any(s)?;
        unsafe {
            match (op, &to) {
                (ShellOp::Copy, Some(to)) => fo.CopyItem(&it, to, PCWSTR::null(), None),
                (ShellOp::Move, Some(to)) => fo.MoveItem(&it, to, PCWSTR::null(), None),
                _ => fo.DeleteItem(&it, None),
            }
        }
        .map_err(|e| e.message().to_string())?;
    }
    let r = perform(&fo);
    for s in srcs.iter().filter(|_| op != ShellOp::Copy) {
        forget(s);
    }
    r
}

/// Makes a folder, or an empty file, called `name` in the shell folder `dir`, without asking anything.
pub fn new_item(dir: &Path, name: &str, folder: bool) -> Result<(), String> {
    crate::shell::init_com();
    let fo = operation(Some(crate::shell::main_hwnd()), true, 0)?;
    let d = item_any(dir)?;
    let n = wide(name);
    let attrs = if folder { windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY.0 } else { windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL.0 };
    unsafe { fo.NewItem(&d, attrs, PCWSTR(n.as_ptr()), PCWSTR::null(), None) }.map_err(|e| e.message().to_string())?;
    perform(&fo)
}

/// Gives the item at `p` a new name.
pub fn rename(p: &Path, new: &str) -> Result<(), String> {
    crate::shell::init_com();
    let fo = operation(Some(crate::shell::main_hwnd()), false, FOF_ALLOWUNDO.0)?;
    let it = item_any(p)?;
    let n = wide(new);
    unsafe { fo.RenameItem(&it, PCWSTR(n.as_ptr()), None) }.map_err(|e| e.message().to_string())?;
    let r = perform(&fo);
    forget(p);
    r
}

// ---------------------------------------------------------------- clipboard

fn shell_idlist_format() -> u32 {
    unsafe { windows::Win32::System::DataExchange::RegisterClipboardFormatW(w!("Shell IDList Array")) } // i18n-ignore: Windows's clipboard format name
}

/// Puts items, some on a device, on the clipboard as the shell does, so Explorer and Daleris can paste them.
pub fn clipboard_set(paths: &[PathBuf], cut: bool) -> bool {
    use windows::Win32::System::Memory::*;
    use windows::Win32::System::Ole::{OleInitialize, OleSetClipboard};
    crate::shell::init_com();
    unsafe {
        let _ = OleInitialize(None);
        let idls: Vec<Idl> = paths.iter().filter_map(|p| if shellpath::is_shell(p) { idl(p) } else { path_idl(p) }).collect();
        if idls.is_empty() {
            return false;
        }
        let raw: Vec<*const ITEMIDLIST> = idls.iter().map(|i| i.0 as *const _).collect();
        let Ok(array) = SHCreateShellItemArrayFromIDLists(&raw) else { return false };
        let Ok(data) = array.BindToHandler::<_, IDataObject>(None, &BHID_DataObject) else { return false };
        let fmt = windows::Win32::System::DataExchange::RegisterClipboardFormatW(w!("Preferred DropEffect")); // i18n-ignore: Windows's clipboard format name
        if let Ok(h) = GlobalAlloc(GMEM_MOVEABLE, 4) {
            let p = GlobalLock(h) as *mut u32;
            if !p.is_null() {
                *p = if cut { 2 } else { 5 };
                let _ = GlobalUnlock(h);
                let fe = FORMATETC { cfFormat: fmt as u16, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 };
                let medium = STGMEDIUM { tymed: TYMED_HGLOBAL.0 as u32, u: STGMEDIUM_0 { hGlobal: h }, pUnkForRelease: std::mem::ManuallyDrop::new(None) };
                let _ = data.SetData(&fe, &medium, true);
            }
        }
        OleSetClipboard(&data).is_ok()
    }
}

fn path_idl(p: &Path) -> Option<Idl> {
    let w = wide(p.as_os_str());
    let mut pidl = std::ptr::null_mut();
    unsafe { SHParseDisplayName(PCWSTR(w.as_ptr()), None, &mut pidl, 0, None) }.ok()?;
    Some(Idl(pidl))
}

/// Whether the clipboard holds shell items without file paths, such as files copied from a phone in Explorer.
pub fn clipboard_has_items() -> bool {
    unsafe { windows::Win32::System::DataExchange::IsClipboardFormatAvailable(shell_idlist_format()).is_ok() }
}

/// The items on the clipboard as paths, those on a device as shell places, and whether they were cut.
pub fn clipboard_get() -> Option<(Vec<PathBuf>, bool)> {
    use windows::Win32::System::Memory::*;
    use windows::Win32::System::Ole::{OleGetClipboard, OleInitialize};
    if !clipboard_has_items() {
        return None;
    }
    crate::shell::init_com();
    unsafe {
        let _ = OleInitialize(None);
        let data = OleGetClipboard().ok()?;
        let array: IShellItemArray = SHCreateShellItemArrayFromDataObject(&data).ok()?;
        let mut out = Vec::new();
        for i in 0..array.GetCount().ok()? {
            let Ok(it) = array.GetItemAt(i) else { continue };
            match it.GetDisplayName(SIGDN_FILESYSPATH).map(take_wide) {
                Ok(p) if !p.is_empty() => out.push(PathBuf::from(p)),
                _ => out.extend(path_of(&it)),
            }
        }
        let fmt = windows::Win32::System::DataExchange::RegisterClipboardFormatW(w!("Preferred DropEffect")); // i18n-ignore: Windows's clipboard format name
        let fe = FORMATETC { cfFormat: fmt as u16, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 };
        let mut cut = false;
        if let Ok(mut m) = data.GetData(&fe) {
            let h = m.u.hGlobal;
            let p = GlobalLock(h) as *const u32;
            if !p.is_null() {
                cut = *p & 2 != 0 && *p & 1 == 0;
                let _ = GlobalUnlock(h);
            }
            windows::Win32::System::Ole::ReleaseStgMedium(&mut m);
        }
        Some((out, cut))
    }
}

// ---------------------------------------------------------------- watching

const WM_SHELL_CHANGE: u32 = WM_APP + 0x51;

thread_local! {
    static CHANGED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

unsafe extern "system" fn watch_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_SHELL_CHANGE {
            // the notice is kept for us until it's been looked at
            let mut idls: *mut *mut ITEMIDLIST = std::ptr::null_mut();
            let mut event = 0i32;
            let lock = SHChangeNotification_Lock(HANDLE(wp.0 as *mut _), lp.0 as u32, Some(&mut idls), Some(&mut event));
            if !lock.is_invalid() {
                let _ = SHChangeNotification_Unlock(lock);
            }
            CHANGED.with(|c| c.set(true));
            return LRESULT(0);
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }
}

/// Calls `on_change` when Windows says something in the shell folder `p` changed: copied there, deleted, renamed, in
/// Daleris or anywhere else. At most once a second, as each change makes the whole folder be read again.
pub fn watch(p: &Path, on_change: impl Fn() + Send + 'static) -> Watcher {
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    let p = p.to_path_buf();
    let _ = std::thread::Builder::new().name("shell-watch".into()).spawn(move || unsafe {
        crate::shell::init_com();
        let Some(folder) = idl(&p) else { return };
        let Ok(hinst) = windows::Win32::System::LibraryLoader::GetModuleHandleW(None) else { return };
        let cls = w!("DalerisShellWatch");
        let wc = WNDCLASSEXW { cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32, lpfnWndProc: Some(watch_proc), hInstance: hinst.into(), lpszClassName: cls, ..Default::default() };
        RegisterClassExW(&wc);
        let Ok(hwnd) = CreateWindowExW(WINDOW_EX_STYLE(0), cls, w!(""), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, Some(hinst.into()), None) else { return };
        let entry = SHChangeNotifyEntry { pidl: folder.0, fRecursive: false.into() };
        let id = SHChangeNotifyRegister(hwnd, SHCNRF_ShellLevel | SHCNRF_InterruptLevel | SHCNRF_NewDelivery, SHCNE_ALLEVENTS.0 as i32, WM_SHELL_CHANGE, 1, &entry);
        let mut pending: Option<Instant> = None;
        let mut last_told = Instant::now() - Duration::from_secs(5);
        while !s2.load(Ordering::Relaxed) {
            MsgWaitForMultipleObjects(None, false, 200, QS_ALLINPUT);
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            if CHANGED.with(|c| c.replace(false)) {
                pending.get_or_insert_with(Instant::now);
            }
            if pending.is_some() && last_told.elapsed() >= Duration::from_secs(1) {
                pending = None;
                last_told = Instant::now();
                on_change();
            }
        }
        if id != 0 {
            let _ = SHChangeNotifyDeregister(id);
        }
        let _ = DestroyWindow(hwnd);
    });
    Watcher { stop }
}

#[cfg(test)]
mod tests {
    //! No phone here: a .zip file is a shell folder that behaves the same way, with no file system path inside it.
    //! The tests reach it through the same code, with the zip's parsing name as the root.
    use super::*;

    /// A zip with a folder and a file in it, made by Windows's own zip folder, and a shell path into it.
    fn zip_with_files(dir: &Path) -> PathBuf {
        let src = dir.join("src");
        std::fs::create_dir_all(src.join("Pictures")).unwrap();
        std::fs::write(src.join("Pictures").join("photo.txt"), b"hello from the phone").unwrap();
        std::fs::write(src.join("notes.txt"), b"12345").unwrap();
        let zip = dir.join("phone.zip");
        let status = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", &format!("Compress-Archive -Path '{}\\*' -DestinationPath '{}'", src.display(), zip.display())])
            .status()
            .unwrap();
        assert!(status.success());
        zip
    }

    /// The zip as a shell root: the tests put it where a device's parsing name would be.
    fn rooted(zip: &Path, rest: &[&str]) -> PathBuf {
        // the cache is keyed by path; a zip's parsing name is its file path, so start from its item
        let it = item_any(zip).unwrap();
        let tag = zip.parent().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let root = PathBuf::from(format!("::{{ZIP-{tag}}}"));
        remember(&root, &Idl(unsafe { SHGetIDListFromObject(&it) }.unwrap()), None);
        let mut p = root;
        for r in rest {
            p = p.join(r);
        }
        p
    }

    fn names(p: &Path) -> Vec<(String, bool, u64)> {
        let mut got = Vec::new();
        list(p, &AtomicBool::new(false), |b| got.extend(b.into_iter().map(|e| (e.name, e.is_dir, e.size)))).unwrap();
        got.sort();
        got
    }

    /// Waits with this thread's messages handled, as a window's thread would: the zip folder finishes its work through
    /// them.
    fn pump(d: Duration) {
        let end = Instant::now() + d;
        while Instant::now() < end {
            unsafe {
                MsgWaitForMultipleObjects(None, false, 20, QS_ALLINPUT);
                let mut msg = MSG::default();
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("daleris-shellns-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_shell_folder_lists_with_names_sizes_and_dates() {
        crate::shell::init_com();
        let dir = scratch("list");
        let zip = zip_with_files(&dir);
        let root = rooted(&zip, &[]);
        assert_eq!(names(&root), [("Pictures".to_string(), true, 0), ("notes.txt".to_string(), false, 5)]);
        // down by name, as the list goes into a folder
        let pics = root.join("Pictures");
        assert_eq!(names(&pics), [("photo.txt".to_string(), false, 20)]);
        let photo = pics.join("photo.txt");
        let e = stat(&photo).expect("seen in the listing");
        assert!(e.mtime > 0, "a date");
        assert_eq!(e.ext, "txt");
        // found again from nothing but the root, as after a restart
        forget(&pics);
        forget(&photo);
        assert!(item(&photo).is_ok());
        assert!(idl(&photo).is_some());
        assert_eq!(item(&root.join("missing")).err().map(|e| e.kind), Some(FailKind::NotFound));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn windows_copies_out_of_a_zip_and_deletes_in_it() {
        crate::shell::init_com();
        let dir = scratch("copy");
        let zip = zip_with_files(&dir);
        let root = rooted(&zip, &[]);
        names(&root);
        // out of it, into a normal folder
        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        run(ShellOp::Copy, &[root.join("notes.txt")], Some(&out), None, true).unwrap();
        assert_eq!(std::fs::read(out.join("notes.txt")).unwrap(), b"12345");
        // and deleted in it
        run(ShellOp::Delete, &[root.join("notes.txt")], None, None, true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while names(&root).iter().any(|n| n.0 == "notes.txt") && Instant::now() < deadline {
            pump(Duration::from_millis(200));
        }
        assert!(!names(&root).iter().any(|n| n.0 == "notes.txt"), "{:?}", names(&root));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A zip doesn't take files from another program on this Windows (it says "item not found"), so writing is tried
    /// in a scratch folder reached through This PC by display names, as a device's folders are: the same resolving,
    /// shell items and file operations, in a place that takes files.
    #[test]
    fn windows_copies_into_a_folder_reached_by_display_names() {
        crate::shell::init_com();
        // outside the profile, whose folders hang off the desktop rather than This PC
        let dir = PathBuf::from(format!(r"C:\Users\Public\daleris-shellns-write-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let it = item_any(&dir).unwrap();
        let shelled = path_of(&it).expect("a path through This PC");
        assert!(shelled.to_string_lossy().starts_with(shellpath::THIS_PC), "{}", shelled.display());
        assert_ne!(shelled, dir);
        // found again from This PC down, name by name
        let known: Vec<PathBuf> = shelled.ancestors().map(Path::to_path_buf).collect();
        for k in &known {
            if k.as_os_str().len() > shellpath::THIS_PC.len() {
                forget(k);
            }
        }
        assert!(item(&shelled).is_ok(), "{}", shelled.display());
        // copy a normal file in
        let local = std::env::temp_dir().join(format!("daleris-shellns-upload-{}.txt", std::process::id()));
        std::fs::write(&local, b"up").unwrap();
        run(ShellOp::Copy, std::slice::from_ref(&local), Some(&shelled), None, true).unwrap();
        assert_eq!(std::fs::read(dir.join(local.file_name().unwrap())).unwrap(), b"up");
        let _ = std::fs::remove_file(&local);
        // a new folder and a rename there
        new_item(&shelled, "New folder", true).unwrap();
        assert!(dir.join("New folder").is_dir());
        names(&shelled);
        rename(&shelled.join("New folder"), "Renamed").unwrap();
        assert!(dir.join("Renamed").is_dir());
        // copied out again, and deleted
        let name = local.file_name().unwrap().to_string_lossy().to_string();
        let out = scratch("write-out");
        names(&shelled);
        run(ShellOp::Copy, &[shelled.join(&name)], Some(&out), None, true).unwrap();
        assert!(out.join(&name).exists());
        run(ShellOp::Delete, &[shelled.join(&name)], None, None, true).unwrap();
        assert!(!dir.join(&name).exists());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&out);
    }
}
