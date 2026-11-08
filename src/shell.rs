//! Windows shell integration: real file icons & thumbnails, the native
//! context menu, open / open-with / properties, recycle bin, file copy/move,
//! the system clipboard (so copy/paste works with Explorer), network drives.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};

use windows::core::{w, Interface, PCSTR, PCWSTR, PSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::NetworkManagement::WNet::*;
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Ole::CF_HDROP;
use windows::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::util::{from_wide, wide};

/// Windows has its own Properties dialog that can be opened instead of ours.
pub const HAS_NATIVE_PROPERTIES: bool = true;

pub fn init_com() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
    }
}

/// Whether this Windows has Mica for apps: Windows 11 from build 22621.
pub fn has_mica() -> bool {
    use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
    crate::fsx::reg_text(HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion", "CurrentBuildNumber").and_then(|b| b.parse::<u32>().ok()).is_some_and(|b| b >= 22621)
}

/// Asks Windows for Mica behind the window, or none, tinted for a dark or light theme. The window must have been made
/// see-through at start for it to show.
pub fn set_backdrop(hwnd: isize, mica: bool, dark: bool) {
    use windows::Win32::Graphics::Dwm::*;
    use windows::Win32::UI::Controls::MARGINS;
    let hwnd = HWND(hwnd as *mut _);
    let dark = windows::core::BOOL::from(dark);
    // Mica Alt, as Explorer uses behind its tabs: more of the wallpaper's colour comes through than plain Mica's
    let kind = if mica { DWMSBT_TABBEDWINDOW } else { DWMSBT_NONE };
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as *const _, 4);
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &kind as *const _ as *const _, 4);
        // Mica only shows through where the window is frame to Windows. Extended from the top, Windows draws its own
        // caption buttons there over Daleris's, so the frame comes up from the bottom instead, far enough to cover it all.
        let up = if mica { 32_000 } else { 0 };
        let _ = DwmExtendFrameIntoClientArea(hwnd, &MARGINS { cxLeftWidth: 0, cxRightWidth: 0, cyTopHeight: 0, cyBottomHeight: up });
    }
}

pub fn main_hwnd() -> HWND {
    unsafe { GetActiveWindow() }
}

/// Test hook: this thread's windows can't take focus or show on the taskbar.
pub fn test_never_activate() {
    use windows::Win32::Foundation::LPARAM;
    use windows::core::BOOL;
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{EnumThreadWindows, GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW};
    unsafe extern "system" fn each(hwnd: HWND, _: LPARAM) -> BOOL { unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0) as isize);
        true.into()
    }}
    unsafe {
        let _ = EnumThreadWindows(GetCurrentThreadId(), Some(each), LPARAM(0));
    }
}

// ---------------------------------------------------------------- images

pub struct RawImg {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
    pub premultiplied: bool,
}

#[derive(Clone)]
pub enum ImgReq {
    /// Small icon + type name, by extension (cheap, shared) or by full path (exe/lnk/ico, drives).
    Icon { key: String, path: String, by_attrs: bool, is_dir: bool, px: u32 },
    /// Shell thumbnail (images, videos, PDFs…, falls back to a big icon).
    Thumb { key: String, path: PathBuf, size: i32, icon_only: bool, preview_only: bool },
    /// The icon Explorer shows for a known folder (Desktop, Downloads…), read from its
    /// shell identity rather than its disk folder (which can carry a custom desktop.ini icon).
    KnownFolder { key: String, id: windows::core::GUID, px: u32 },
}

impl ImgReq {
    fn key(&self) -> &str {
        match self {
            ImgReq::Icon { key, .. } | ImgReq::Thumb { key, .. } | ImgReq::KnownFolder { key, .. } => key,
        }
    }
}

pub struct ImgResult {
    pub key: String,
    pub img: Option<RawImg>,
    pub type_name: Option<String>,
}

type Queue = Arc<(Mutex<Vec<ImgReq>>, Condvar)>;

/// Background image loader. LIFO so whatever just scrolled into view loads first.
pub struct Loader {
    icons: Queue,
    thumbs: Queue,
    /// Anything on a network path. Those can take a dead server's time, which local files shouldn't have to wait for.
    net: Queue,
    pub rx: Receiver<ImgResult>,
}

impl Loader {
    pub fn new(ctx: eframe::egui::Context) -> Self {
        let (tx, rx) = channel();
        let icons: Queue = Arc::new((Mutex::new(Vec::new()), Condvar::new()));
        let thumbs: Queue = Arc::new((Mutex::new(Vec::new()), Condvar::new()));
        spawn_workers(&icons, 2, &tx, &ctx);
        spawn_workers(&thumbs, 3, &tx, &ctx);
        let net: Queue = Arc::new((Mutex::new(Vec::new()), Condvar::new()));
        spawn_workers(&net, 2, &tx, &ctx);
        Loader { icons, thumbs, net, rx }
    }

    pub fn request(&self, r: ImgReq) {
        let q = match &r {
            ImgReq::Icon { path, by_attrs: false, .. } if crate::fsx::is_network_path(Path::new(path)) => &self.net,
            ImgReq::Thumb { path, .. } if crate::fsx::is_network_path(path) => &self.net,
            ImgReq::Icon { .. } | ImgReq::KnownFolder { .. } => &self.icons,
            ImgReq::Thumb { .. } => &self.thumbs,
        };
        let mut v = q.0.lock().unwrap();
        v.push(r);
        q.1.notify_one();
    }

    /// Drop queued thumbnails that are no longer on screen (e.g. after navigating).
    pub fn clear_thumbs(&self) -> Vec<String> {
        let mut v = self.thumbs.0.lock().unwrap();
        let mut gone: Vec<String> = v.drain(..).map(|r| r.key().to_string()).collect();
        // and whatever is waiting for a server: the person has moved on
        gone.extend(self.net.0.lock().unwrap().drain(..).map(|r| r.key().to_string()));
        gone
    }
}

fn spawn_workers(q: &Queue, n: usize, tx: &Sender<ImgResult>, ctx: &eframe::egui::Context) {
    for i in 0..n {
        let q = q.clone();
        let tx = tx.clone();
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name(format!("img{i}"))
            .spawn(move || {
                init_com();
                loop {
                    let req = {
                        let mut v = q.0.lock().unwrap();
                        while v.is_empty() {
                            v = q.1.wait(v).unwrap();
                        }
                        v.pop().unwrap()
                    };
                    let res = match &req {
                        ImgReq::Icon { key, path, by_attrs, is_dir, px } => {
                            let (img, tn) = unsafe { file_icon(path, *by_attrs, *is_dir, *px) };
                            ImgResult { key: key.clone(), img, type_name: tn }
                        }
                        ImgReq::Thumb { key, path, size, icon_only, preview_only } => {
                            let img = unsafe { thumbnail(path, *size, *icon_only, *preview_only) };
                            ImgResult { key: key.clone(), img, type_name: None }
                        }
                        ImgReq::KnownFolder { key, id, px } => {
                            let img = unsafe { known_folder_icon(id, *px) };
                            ImgResult { key: key.clone(), img, type_name: None }
                        }
                    };
                    if tx.send(res).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            })
            .ok();
    }
}

/// The icon for a file or folder at `px` pixels, from the smallest of Windows's icon sets at least that big, and its type
/// name. Shrinking a bigger icon by averaging keeps it crisp where stretching the standard one made it soft.
unsafe fn file_icon(path: &str, by_attrs: bool, is_dir: bool, px: u32) -> (Option<RawImg>, Option<String>) { unsafe {
    let w = wide(path);
    let mut sfi = SHFILEINFOW::default();
    let mut flags = SHGFI_SYSICONINDEX | SHGFI_ICON | SHGFI_LARGEICON | SHGFI_TYPENAME;
    if by_attrs {
        flags |= SHGFI_USEFILEATTRIBUTES;
    }
    let attrs = if is_dir { FILE_ATTRIBUTE_DIRECTORY } else { FILE_ATTRIBUTE_NORMAL };
    let r = SHGetFileInfoW(PCWSTR(w.as_ptr()), attrs, Some(&mut sfi), std::mem::size_of::<SHFILEINFOW>() as u32, flags);
    if r == 0 {
        return (None, None);
    }
    let tn = from_wide(&sfi.szTypeName);
    let standard = if !sfi.hIcon.is_invalid() {
        let i = icon_to_rgba(sfi.hIcon);
        let _ = DestroyIcon(sfi.hIcon);
        i
    } else {
        None
    };
    let sized = sized_icon(sfi.iIcon, px);
    (sized.or(standard).map(|i| shrink(i, px)), (!tn.is_empty()).then_some(tn))
}}

unsafe fn known_folder_icon(id: &windows::core::GUID, px: u32) -> Option<RawImg> { unsafe {
    let pidl = SHGetKnownFolderIDList(id, 0, None).ok()?;
    // the shell image service returns the same modern icon Explorer's navigation pane shows
    let via_factory = (|| -> Option<RawImg> {
        let f: IShellItemImageFactory = SHCreateItemFromIDList(pidl).ok()?;
        let side = px.max(16) as i32;
        let hbm = f.GetImage(SIZE { cx: side, cy: side }, SIIGBF(SIIGBF_RESIZETOFIT.0 | SIIGBF_ICONONLY.0)).ok()?;
        let r = bitmap_bgra(hbm);
        let _ = DeleteObject(hbm.into());
        let (w, h, mut px) = r?;
        for p in px.chunks_exact_mut(4) {
            p.swap(0, 2);
        }
        Some(RawImg { w, h, rgba: px, premultiplied: true })
    })();
    if via_factory.is_some() {
        ILFree(Some(pidl as *const _));
        return via_factory;
    }
    let mut sfi = SHFILEINFOW::default();
    let r = SHGetFileInfoW(
        PCWSTR(pidl as *const u16),
        FILE_FLAGS_AND_ATTRIBUTES(0),
        Some(&mut sfi),
        std::mem::size_of::<SHFILEINFOW>() as u32,
        SHGFI_PIDL | SHGFI_ICON | SHGFI_LARGEICON,
    );
    ILFree(Some(pidl as *const _));
    if r == 0 || sfi.hIcon.is_invalid() {
        return None;
    }
    let img = icon_to_rgba(sfi.hIcon);
    let _ = DestroyIcon(sfi.hIcon);
    img
}}

/// The standard Windows "choose a folder" dialog.
pub fn pick_folder() -> Option<PathBuf> {
    unsafe {
        let d: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let opts = d.GetOptions().ok()?;
        d.SetOptions(FILEOPENDIALOGOPTIONS(opts.0 | FOS_PICKFOLDERS.0 | FOS_FORCEFILESYSTEM.0)).ok()?;
        d.Show(Some(main_hwnd())).ok()?;
        let item = d.GetResult().ok()?;
        let p = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from)
    }
}

unsafe fn icon_to_rgba(h: HICON) -> Option<RawImg> { unsafe {
    let mut ii = ICONINFO::default();
    GetIconInfo(h, &mut ii).ok()?;
    let color = bitmap_bgra(ii.hbmColor);
    let mask = bitmap_bgra(ii.hbmMask);
    let _ = DeleteObject(ii.hbmColor.into());
    let _ = DeleteObject(ii.hbmMask.into());
    let (w, hh, mut px) = color?;
    let has_alpha = px.chunks_exact(4).any(|p| p[3] != 0);
    if !has_alpha {
        if let Some((mw, mh, m)) = mask {
            if mw == w && mh >= hh {
                for i in 0..w * hh {
                    px[i * 4 + 3] = if m[i * 4] == 0 { 255 } else { 0 };
                }
            }
        }
    }
    for p in px.chunks_exact_mut(4) {
        p.swap(0, 2);
    }
    Some(RawImg { w, h: hh, rgba: px, premultiplied: false })
}}

unsafe fn bitmap_bgra(hbm: HBITMAP) -> Option<(usize, usize, Vec<u8>)> { unsafe {
    if hbm.is_invalid() {
        return None;
    }
    let mut bm = BITMAP::default();
    if GetObjectW(hbm.into(), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut _)) == 0 {
        return None;
    }
    let (w, h) = (bm.bmWidth, bm.bmHeight.abs());
    if w <= 0 || h <= 0 || w > 4096 || h > 4096 {
        return None;
    }
    let mut bi = BITMAPINFO::default();
    bi.bmiHeader = BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: w,
        biHeight: -h,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        ..Default::default()
    };
    let mut buf = vec![0u8; (w * h * 4) as usize];
    let hdc = CreateCompatibleDC(None);
    let n = GetDIBits(hdc, hbm, 0, h as u32, Some(buf.as_mut_ptr() as *mut _), &mut bi, DIB_RGB_COLORS);
    let _ = DeleteDC(hdc);
    (n != 0).then_some((w as usize, h as usize, buf))
}}

/// A picture file, read by the system at about `size` pixels, such as a person's account picture.
pub fn picture(path: &Path, size: u32) -> Option<RawImg> {
    init_com();
    unsafe { thumbnail(path, size as i32, false, true) }
}

unsafe fn thumbnail(path: &Path, size: i32, icon_only: bool, preview_only: bool) -> Option<RawImg> { unsafe {
    let w = wide(path.as_os_str());
    let f: IShellItemImageFactory = SHCreateItemFromParsingName(PCWSTR(w.as_ptr()), None).ok()?;
    let flags = if icon_only {
        SIIGBF(SIIGBF_RESIZETOFIT.0 | SIIGBF_ICONONLY.0)
    } else if preview_only {
        SIIGBF(SIIGBF_RESIZETOFIT.0 | SIIGBF_THUMBNAILONLY.0)
    } else {
        SIIGBF_RESIZETOFIT
    };
    let hbm = f.GetImage(SIZE { cx: size, cy: size }, flags).ok()?;
    let r = bitmap_bgra(hbm);
    let _ = DeleteObject(hbm.into());
    let (w, h, mut px) = r?;
    let has_alpha = px.chunks_exact(4).any(|p| p[3] != 0);
    for p in px.chunks_exact_mut(4) {
        p.swap(0, 2);
        if !has_alpha {
            p[3] = 255;
        }
    }
    Some(RawImg { w, h, rgba: px, premultiplied: true })
}}

// ---------------------------------------------------------------- simple verbs

/// Opens a web page in the default browser.
pub fn open_url(url: &str) {
    let u = wide(url);
    unsafe {
        ShellExecuteW(Some(main_hwnd()), PCWSTR::null(), PCWSTR(u.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

pub fn open(path: &Path) {
    let owner = main_hwnd().0 as isize;
    let path = path.to_path_buf();
    // A file on a server that's gone keeps this waiting for as long as the server takes to not answer.
    crate::bg::submit(&path.clone(), "open", move || {
        init_com();
        let p = wide(path.as_os_str());
        let dir = path.parent().map(|d| wide(d.as_os_str()));
        unsafe {
            ShellExecuteW(
                Some(HWND(owner as *mut _)),
                PCWSTR::null(), // the file type's default action (open, play, run, edit…)
                PCWSTR(p.as_ptr()),
                PCWSTR::null(),
                dir.as_ref().map(|d| PCWSTR(d.as_ptr())).unwrap_or(PCWSTR::null()),
                SW_SHOWNORMAL,
            );
        }
    });
}

/// Credential Manager, where a server's saved account can be corrected or removed.
pub fn credential_manager() {
    run_program("control.exe", "/name Microsoft.CredentialManager", &std::env::temp_dir());
}

/// Starts a program with these arguments in `dir`, as Explorer runs a New menu command.
pub fn run_program(program: &str, args: &str, dir: &Path) {
    let (p, a, d) = (wide(program), wide(args), wide(dir.as_os_str()));
    unsafe {
        ShellExecuteW(Some(main_hwnd()), PCWSTR::null(), PCWSTR(p.as_ptr()), PCWSTR(a.as_ptr()), PCWSTR(d.as_ptr()), SW_SHOWNORMAL);
    }
}

pub fn run_as_admin(path: &Path) {
    let p = wide(path.as_os_str());
    unsafe {
        ShellExecuteW(Some(main_hwnd()), w!("runas"), PCWSTR(p.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

pub fn open_with(path: &Path) {
    let p = wide(path.as_os_str());
    let info = OPENASINFO {
        pcszFile: PCWSTR(p.as_ptr()),
        pcszClass: PCWSTR::null(),
        oaifInFlags: OPEN_AS_INFO_FLAGS(OAIF_EXEC.0 | OAIF_ALLOW_REGISTRATION.0 | OAIF_REGISTER_EXT.0),
    };
    unsafe {
        let _ = SHOpenWithDialog(Some(main_hwnd()), &info);
    }
}

pub fn properties(path: &Path) {
    let p = wide(path.as_os_str());
    unsafe {
        let _ = SHObjectProperties(Some(main_hwnd()), SHOP_FILEPATH, PCWSTR(p.as_ptr()), PCWSTR::null());
    }
}

/// The friendly name of the app that opens this file type ("Photos", "Notepad"…).
pub fn default_app(ext: &str) -> Option<String> {
    if ext.is_empty() {
        return None;
    }
    let e = wide(format!(".{ext}"));
    let mut buf = [0u16; 260];
    let mut len = buf.len() as u32;
    let hr = unsafe { AssocQueryStringW(ASSOCF_INIT_IGNOREUNKNOWN, ASSOCSTR_FRIENDLYAPPNAME, PCWSTR(e.as_ptr()), PCWSTR::null(), Some(windows::core::PWSTR(buf.as_mut_ptr())), &mut len) };
    let s = from_wide(&buf);
    (hr.is_ok() && !s.is_empty()).then_some(s)
}

pub fn open_terminal(dir: &Path) {
    use std::process::Command;
    let ok = Command::new("wt.exe").arg("-d").arg(dir).spawn().is_ok();
    if !ok {
        let _ = Command::new("powershell.exe").arg("-NoExit").current_dir(dir).spawn();
    }
}

pub fn map_network_drive() {
    unsafe {
        let _ = WNetConnectionDialog(main_hwnd(), RESOURCETYPE_DISK.0);
    }
}

pub fn disconnect_drive(letter: char) -> Result<(), String> {
    let w = wide(format!("{letter}:"));
    let rc = unsafe { WNetCancelConnection2W(PCWSTR(w.as_ptr()), CONNECT_UPDATE_PROFILE, false) };
    if rc == NO_ERROR {
        Ok(())
    } else {
        Err(format!("Could not disconnect {letter}: (error {})", rc.0))
    }
}

// ---------------------------------------------------------------- file operations

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    Copy,
    Move,
    Recycle,
}

fn multi_sz(paths: &[PathBuf]) -> Vec<u16> {
    let mut v = Vec::new();
    for p in paths {
        v.extend(p.as_os_str().to_string_lossy().encode_utf16());
        v.push(0);
    }
    v.push(0);
    v
}

/// Runs on a worker thread. Daleris uses it for the Recycle Bin, and for what's left of a copy or move when the person
/// lets Windows finish it, so Windows can ask for administrator rights.
pub fn file_op(op: Op, src: Vec<PathBuf>, dest: Option<PathBuf>, done: impl FnOnce(Result<(), String>) + Send + 'static) {
    let owner = main_hwnd().0 as isize;
    crate::bg::spawn("shell-op", move || {
        init_com();
        let from = multi_sz(&src);
        let to = dest.as_ref().map(|d| multi_sz(std::slice::from_ref(d)));
        let same_dir = dest.as_ref().is_some_and(|d| src.iter().all(|s| s.parent() == Some(d.as_path())));
        let mut flags = FOF_ALLOWUNDO.0;
        if op == Op::Copy && same_dir {
            flags |= FOF_RENAMEONCOLLISION.0;
        }
        // Daleris says itself what went wrong; Windows still warns before something too big for the bin is deleted for good
        if op == Op::Recycle {
            flags |= FOF_SILENT.0 | FOF_NOCONFIRMATION.0 | FOF_NOERRORUI.0 | FOF_WANTNUKEWARNING.0;
        }
        let mut s = SHFILEOPSTRUCTW {
            hwnd: HWND(owner as *mut _),
            wFunc: match op {
                Op::Copy => FO_COPY,
                Op::Move => FO_MOVE,
                Op::Recycle => FO_DELETE,
            },
            pFrom: PCWSTR(from.as_ptr()),
            pTo: to.as_ref().map(|t| PCWSTR(t.as_ptr())).unwrap_or(PCWSTR::null()),
            fFlags: flags as u16,
            ..Default::default()
        };
        let rc = unsafe { SHFileOperationW(&mut s) };
        done(if rc == 0 { Ok(()) } else if s.fAnyOperationsAborted.as_bool() { Ok(()) } else { Err(format!("Operation failed (code {rc:#x})")) });
    });
}

// ---------------------------------------------------------------- clipboard

/// Put files on the Windows clipboard as CF_HDROP (+ "Preferred DropEffect" for cut),
/// exactly what Explorer does, so pasting into Explorer or any app works.
pub fn clipboard_set_files(paths: &[PathBuf], cut: bool) -> bool {
    unsafe {
        if OpenClipboard(Some(main_hwnd())).is_err() {
            return false;
        }
        let _ = EmptyClipboard();
        let list = multi_sz(paths);
        let header = std::mem::size_of::<DROPFILES>();
        let bytes = header + list.len() * 2;
        let ok = (|| -> Option<()> {
            let hg = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes).ok()?;
            let p = GlobalLock(hg) as *mut u8;
            if p.is_null() {
                return None;
            }
            let df = p as *mut DROPFILES;
            (*df).pFiles = header as u32;
            (*df).fWide = TRUE;
            std::ptr::copy_nonoverlapping(list.as_ptr() as *const u8, p.add(header), list.len() * 2);
            let _ = GlobalUnlock(hg);
            SetClipboardData(CF_HDROP.0 as u32, Some(HANDLE(hg.0))).ok()?;

            let fmt = RegisterClipboardFormatW(w!("Preferred DropEffect"));
            let he = GlobalAlloc(GMEM_MOVEABLE, 4).ok()?;
            let pe = GlobalLock(he) as *mut u32;
            if !pe.is_null() {
                *pe = if cut { 2 } else { 5 };
                let _ = GlobalUnlock(he);
                let _ = SetClipboardData(fmt, Some(HANDLE(he.0)));
            }
            Some(())
        })()
        .is_some();
        let _ = CloseClipboard();
        ok
    }
}

pub fn clipboard_set_text(text: &str) {
    unsafe {
        if OpenClipboard(Some(main_hwnd())).is_err() {
            return;
        }
        let _ = EmptyClipboard();
        let w = wide(text);
        if let Ok(hg) = GlobalAlloc(GMEM_MOVEABLE, w.len() * 2) {
            let p = GlobalLock(hg) as *mut u16;
            if !p.is_null() {
                std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
                let _ = GlobalUnlock(hg);
                let _ = SetClipboardData(13 /* CF_UNICODETEXT */, Some(HANDLE(hg.0)));
            }
        }
        let _ = CloseClipboard();
    }
}

/// Files on the clipboard and whether they were cut.
pub fn clipboard_get_files() -> Option<(Vec<PathBuf>, bool)> {
    unsafe {
        if IsClipboardFormatAvailable(CF_HDROP.0 as u32).is_err() {
            return None;
        }
        OpenClipboard(Some(main_hwnd())).ok()?;
        let r = (|| {
            let h = GetClipboardData(CF_HDROP.0 as u32).ok()?;
            let hdrop = HDROP(h.0);
            let n = DragQueryFileW(hdrop, 0xFFFF_FFFF, None);
            let mut out = Vec::new();
            for i in 0..n {
                let len = DragQueryFileW(hdrop, i, None) as usize;
                let mut buf = vec![0u16; len + 1];
                DragQueryFileW(hdrop, i, Some(&mut buf));
                out.push(PathBuf::from(from_wide(&buf)));
            }
            let fmt = RegisterClipboardFormatW(w!("Preferred DropEffect"));
            let mut cut = false;
            if let Ok(he) = GetClipboardData(fmt) {
                let p = GlobalLock(HGLOBAL(he.0)) as *const u32;
                if !p.is_null() {
                    cut = *p & 2 != 0 && *p & 1 == 0;
                    let _ = GlobalUnlock(HGLOBAL(he.0));
                }
            }
            Some((out, cut))
        })();
        let _ = CloseClipboard();
        r
    }
}

pub fn clipboard_has_files() -> bool {
    unsafe { IsClipboardFormatAvailable(CF_HDROP.0 as u32).is_ok() }
}

// ---------------------------------------------------------------- native context menu

thread_local! {
    static ACTIVE_MENU: RefCell<Option<IContextMenu>> = const { RefCell::new(None) };
    static HELPER: RefCell<Option<HWND>> = const { RefCell::new(None) };
}

unsafe extern "system" fn helper_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT { unsafe {
    // Submenus like "Send to", "Open with" and "New" are filled lazily through these messages.
    if matches!(msg, WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR) {
        let res = ACTIVE_MENU.with(|m| {
            let m = m.borrow();
            let cm = m.as_ref()?;
            match cm.cast::<IContextMenu3>() { Ok(cm3) => {
                let mut r = LRESULT(0);
                if cm3.HandleMenuMsg2(msg, wp, lp, Some(&mut r)).is_ok() {
                    return Some(r);
                }
            } _ => { match cm.cast::<IContextMenu2>() { Ok(cm2) => {
                if cm2.HandleMenuMsg(msg, wp, lp).is_ok() {
                    return Some(LRESULT(0));
                }
            } _ => {}}}}
            None
        });
        if let Some(r) = res {
            return r;
        }
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}}

unsafe fn helper_window() -> Option<HWND> { unsafe {
    if let Some(h) = HELPER.with(|h| *h.borrow()) {
        return Some(h);
    }
    let hinst = GetModuleHandleW(None).ok()?;
    let cls = w!("DalerisMenuHelper");
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(helper_proc),
        hInstance: hinst.into(),
        lpszClassName: cls,
        ..Default::default()
    };
    RegisterClassExW(&wc);
    let h = CreateWindowExW(WINDOW_EX_STYLE(0), cls, w!(""), WS_POPUP, 0, 0, 0, 0, Some(main_hwnd()), None, Some(hinst.into()), None).ok()?;
    HELPER.with(|x| *x.borrow_mut() = Some(h));
    Some(h)
}}

pub enum MenuOutcome {
    Nothing,
    Invoked,
    Rename,
}

/// The Windows context menu handler for some items (or a folder's background),
/// with every shell extension loaded: 7-Zip, Git, VS Code, OneDrive, Defender…
struct ShellCm {
    cm: IContextMenu,
    _pidls: Pidls, // dropped after `cm`
}

struct Pidls(Vec<*mut ITEMIDLIST>);

impl Drop for Pidls {
    fn drop(&mut self) {
        for p in &self.0 {
            unsafe { ILFree(Some(*p as *const _)) };
        }
    }
}

unsafe fn context_menu_for(items: &[PathBuf], dir: &Path, owner: HWND) -> windows::core::Result<ShellCm> { unsafe {
    let mut pidls = Pidls(Vec::new());
    let cm: IContextMenu = if items.is_empty() {
        let w = wide(dir.as_os_str());
        let mut pidl = std::ptr::null_mut();
        SHParseDisplayName(PCWSTR(w.as_ptr()), None, &mut pidl, 0, None)?;
        pidls.0.push(pidl);
        let desktop = SHGetDesktopFolder()?;
        let folder: IShellFolder = desktop.BindToObject(pidl, None)?;
        folder.CreateViewObject(owner)?
    } else {
        let mut folder: Option<IShellFolder> = None;
        let mut children: Vec<*const ITEMIDLIST> = Vec::new();
        let parent0 = items[0].parent().map(|p| p.to_path_buf());
        for p in items {
            if p.parent().map(|x| x.to_path_buf()) != parent0 {
                continue; // shell menus need one shared parent folder
            }
            let w = wide(p.as_os_str());
            let mut pidl = std::ptr::null_mut();
            if SHParseDisplayName(PCWSTR(w.as_ptr()), None, &mut pidl, 0, None).is_err() {
                continue;
            }
            pidls.0.push(pidl);
            let mut last = std::ptr::null_mut();
            let f: IShellFolder = SHBindToParent(pidl, Some(&mut last))?;
            if folder.is_none() {
                folder = Some(f);
            }
            children.push(last as *const _);
        }
        let Some(folder) = folder else {
            return Err(windows::core::Error::from_thread());
        };
        folder.GetUIObjectOf(owner, &children, None)?
    };
    Ok(ShellCm { cm, _pidls: pidls })
}}

unsafe fn invoke_id(cm: &IContextMenu, id: usize, dir: &Path) -> windows::core::Result<MenuOutcome> { unsafe {
    let mut verb = [0u8; 64];
    if cm.GetCommandString(id, GCS_VERBA, None, PSTR(verb.as_mut_ptr()), verb.len() as u32).is_ok() {
        let end = verb.iter().position(|&c| c == 0).unwrap_or(verb.len());
        if verb[..end].eq_ignore_ascii_case(b"rename") {
            return Ok(MenuOutcome::Rename);
        }
    }
    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    let wdir = wide(dir.as_os_str());
    let info = CMINVOKECOMMANDINFOEX {
        cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
        fMask: 0x4000 /* CMIC_MASK_UNICODE */ | CMIC_MASK_PTINVOKE,
        hwnd: main_hwnd(),
        lpVerb: PCSTR(id as *const u8),
        lpVerbW: PCWSTR(id as *const u16),
        lpDirectoryW: PCWSTR(wdir.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ptInvoke: pt,
        ..Default::default()
    };
    cm.InvokeCommand(&info as *const _ as *const CMINVOKECOMMANDINFO)?;
    Ok(MenuOutcome::Invoked)
}}

/// Show the full Windows context menu as a native popup ("Show more options").
pub fn native_menu(items: &[PathBuf], dir: &Path) -> MenuOutcome {
    unsafe { native_menu_inner(items, dir) }.unwrap_or(MenuOutcome::Nothing)
}

unsafe fn native_menu_inner(items: &[PathBuf], dir: &Path) -> windows::core::Result<MenuOutcome> { unsafe {
    let owner = helper_window().ok_or_else(windows::core::Error::from_thread)?;
    let scm = context_menu_for(items, dir, owner)?;
    let menu = CreatePopupMenu()?;
    let r = (|| -> windows::core::Result<MenuOutcome> {
        scm.cm.QueryContextMenu(menu, 0, 1, 0x7FFF, CMF_NORMAL | CMF_EXPLORE).ok()?;
        ACTIVE_MENU.with(|m| *m.borrow_mut() = Some(scm.cm.clone()));
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(owner);
        let cmd = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, pt.x, pt.y, owner, None);
        let _ = PostMessageW(Some(owner), WM_NULL, WPARAM(0), LPARAM(0));
        ACTIVE_MENU.with(|m| *m.borrow_mut() = None);
        if cmd.0 <= 0 {
            return Ok(MenuOutcome::Nothing);
        }
        invoke_id(&scm.cm, (cmd.0 - 1) as usize, dir)
    })();
    let _ = DestroyMenu(menu);
    r
}}

/// One item of the Windows menu, read out so we can draw it in our own menu.
pub struct ShellEntry {
    /// command id (offset + 1); 0 for separators and submenus
    pub id: u32,
    pub label: String,
    /// stable key used for customisation ("v:verb" or "l:label")
    pub key: String,
    pub icon: Option<RawImg>,
    pub children: Vec<ShellEntry>,
    pub separator: bool,
    pub disabled: bool,
    pub default: bool,
}

/// A live Windows context menu whose items are shown inside our menu. It stays
/// alive while our menu is open so a click can invoke the original command.
pub struct ShellMenu {
    scm: ShellCm,
    menu: HMENU,
    dir: PathBuf,
    pub entries: Vec<ShellEntry>,
    pub built: std::time::Instant,
}

impl Drop for ShellMenu {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyMenu(self.menu);
        }
    }
}

pub fn menu_key(verb: &str, label: &str) -> String {
    if !verb.is_empty() {
        return format!("v:{}", verb.to_lowercase());
    }
    format!("l:{}", clean_label(label).to_lowercase().trim_end_matches("...").trim_end_matches('…').trim())
}

/// "Open &with" → "Open with", "A && B" → "A & B", drop "\tCtrl+X" accelerators.
pub fn clean_label(s: &str) -> String {
    let s = s.split('\t').next().unwrap_or("");
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            if chars.peek() == Some(&'&') {
                out.push('&');
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    // Windows/app menu text is shown without dashes
    crate::fscommon::undash_text(out.trim())
}

impl ShellMenu {
    /// `extended` = Shift was held: include the extended verbs, as Explorer does.
    pub fn build(items: &[PathBuf], dir: &Path, extended: bool) -> Option<ShellMenu> {
        unsafe {
            let owner = helper_window()?;
            let scm = context_menu_for(items, dir, owner).ok()?;
            let menu = CreatePopupMenu().ok()?;
            let mut flags = CMF_NORMAL | CMF_EXPLORE;
            if extended {
                flags |= CMF_EXTENDEDVERBS;
            }
            if scm.cm.QueryContextMenu(menu, 0, 1, 0x7FFF, flags).is_err() {
                let _ = DestroyMenu(menu);
                return None;
            }
            let cm3 = scm.cm.cast::<IContextMenu3>().ok();
            let cm2 = scm.cm.cast::<IContextMenu2>().ok();
            let entries = read_menu(&scm.cm, cm3.as_ref(), cm2.as_ref(), menu, 0);
            Some(ShellMenu { scm, menu, dir: dir.to_path_buf(), entries, built: std::time::Instant::now() })
        }
    }

    /// Windows attaches some item icons a moment after the menu is built; read
    /// bitmaps for items that had none yet. Returns (command id, image) pairs.
    pub fn late_icons(&self) -> Vec<(u32, RawImg)> {
        fn ids(es: &[ShellEntry], out: &mut Vec<u32>) {
            for e in es {
                if e.id > 0 && e.id < 0x4000_0000 && e.icon.is_none() {
                    out.push(e.id);
                }
                ids(&e.children, out);
            }
        }
        let mut want = Vec::new();
        ids(&self.entries, &mut want);
        let mut out = Vec::new();
        for id in want {
            unsafe {
                let mut mii = MENUITEMINFOW { cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32, fMask: MIIM_BITMAP, ..Default::default() };
                if GetMenuItemInfoW(self.menu, id, false, &mut mii).is_ok() && !mii.hbmpItem.is_invalid() && (mii.hbmpItem.0 as isize) > 16 {
                    if let Some(img) = menu_bitmap(mii.hbmpItem) {
                        out.push((id, img));
                    }
                }
            }
        }
        out
    }

    pub fn invoke(&self, id: u32) -> MenuOutcome {
        if id == 0 {
            return MenuOutcome::Nothing;
        }
        unsafe { invoke_id(&self.scm.cm, (id - 1) as usize, &self.dir) }.unwrap_or(MenuOutcome::Nothing)
    }
}

unsafe fn read_menu(cm: &IContextMenu, cm3: Option<&IContextMenu3>, cm2: Option<&IContextMenu2>, menu: HMENU, depth: u32) -> Vec<ShellEntry> { unsafe {
    let mut out = Vec::new();
    let n = GetMenuItemCount(Some(menu));
    for i in 0..n.max(0) {
        let mut text = [0u16; 260];
        let mut mii = MENUITEMINFOW {
            cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_FTYPE | MIIM_STATE | MIIM_ID | MIIM_SUBMENU | MIIM_STRING | MIIM_BITMAP,
            dwTypeData: windows::core::PWSTR(text.as_mut_ptr()),
            cch: (text.len() - 1) as u32,
            ..Default::default()
        };
        if GetMenuItemInfoW(menu, i as u32, true, &mut mii).is_err() {
            continue;
        }
        if mii.fType.0 & MFT_SEPARATOR.0 != 0 {
            out.push(ShellEntry { id: 0, label: String::new(), key: String::new(), icon: None, children: Vec::new(), separator: true, disabled: false, default: false });
            continue;
        }
        let label = clean_label(&from_wide(&text));
        if label.is_empty() {
            continue; // owner-drawn items carry no text we could show
        }
        let mut children = Vec::new();
        let id;
        let mut verb = String::new();
        if !mii.hSubMenu.is_invalid() {
            // submenus have no command id; give each its own (never invoked) so icons don't collide
            static NEXT_SUB: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x4000_0000);
            id = NEXT_SUB.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if depth < 3 {
                // lazily filled submenus ("Send to", "Open with", "New") fill themselves on this message
                let (wp, lp) = (WPARAM(mii.hSubMenu.0 as usize), LPARAM(i as isize));
                if let Some(c3) = cm3 {
                    let mut r = LRESULT(0);
                    let _ = c3.HandleMenuMsg2(WM_INITMENUPOPUP, wp, lp, Some(&mut r));
                } else if let Some(c2) = cm2 {
                    let _ = c2.HandleMenuMsg(WM_INITMENUPOPUP, wp, lp);
                }
                children = read_menu(cm, cm3, cm2, mii.hSubMenu, depth + 1);
            }
            if children.iter().all(|c| c.separator) {
                continue;
            }
        } else {
            id = mii.wID;
            if (1..=0x7FFF).contains(&id) {
                let mut buf = [0u16; 128];
                if cm.GetCommandString((id - 1) as usize, GCS_VERBW, None, PSTR(buf.as_mut_ptr() as *mut u8), buf.len() as u32).is_ok() {
                    verb = from_wide(&buf);
                }
            } else {
                continue;
            }
        }
        let bmp = mii.hbmpItem;
        let icon = if !bmp.is_invalid() && (bmp.0 as isize) > 16 { menu_bitmap(bmp) } else { None };
        out.push(ShellEntry {
            key: menu_key(&verb, &label),
            id,
            label,
            icon,
            children,
            separator: false,
            disabled: mii.fState.0 & MFS_DISABLED.0 != 0,
            default: mii.fState.0 & MFS_DEFAULT.0 != 0,
        });
    }
    out
}}

unsafe fn menu_bitmap(bmp: HBITMAP) -> Option<RawImg> { unsafe {
    bitmap_bgra(bmp).map(|(w, h, mut px)| {
        let has_alpha = px.chunks_exact(4).any(|p| p[3] != 0);
        for p in px.chunks_exact_mut(4) {
            p.swap(0, 2);
            if !has_alpha {
                p[3] = 255;
            }
        }
        RawImg { w, h, rgba: px, premultiplied: true }
    })
}}

/// Where a .lnk shortcut points.
pub fn shortcut_target(lnk: &Path) -> Option<PathBuf> {
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let pf: IPersistFile = link.cast().ok()?;
        let w = wide(lnk.as_os_str());
        pf.Load(PCWSTR(w.as_ptr()), STGM_READ).ok()?;
        let mut buf = [0u16; 1024];
        link.GetPath(&mut buf, std::ptr::null_mut(), 0).ok()?;
        let s = from_wide(&buf);
        (!s.is_empty()).then(|| PathBuf::from(s))
    }
}

/// The icon at `index` of the system image list, from the smallest of Windows's sets (small, large, extra large, jumbo)
/// at least `px` big; the biggest when none is.
unsafe fn sized_icon(index: i32, px: u32) -> Option<RawImg> { unsafe {
    use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
    let mut best: Option<(i32, IImageList)> = None;
    for list in [SHIL_SMALL, SHIL_LARGE, SHIL_EXTRALARGE, SHIL_JUMBO] {
        let Ok(il) = SHGetImageList::<IImageList>(list as i32) else { continue };
        let (mut cx, mut cy) = (0, 0);
        if il.GetIconSize(&mut cx, &mut cy).is_err() {
            continue;
        }
        let fits = cx >= px as i32;
        let better = match &best {
            None => true,
            Some((have, _)) => (fits && (*have < px as i32 || cx < *have)) || (!fits && *have < px as i32 && cx > *have),
        };
        if better {
            best = Some((cx, il));
        }
    }
    let (_, il) = best?;
    let h = il.GetIcon(index, ILD_TRANSPARENT.0).ok()?;
    let img = icon_to_rgba(h);
    let _ = DestroyIcon(h);
    img
}}

/// Fits a picture into `px` by averaging each square of pixels it covers, which keeps edges sharp where a GPU stretch
/// blurs them. Smaller pictures are left as they are.
fn shrink(img: RawImg, px: u32) -> RawImg {
    let px = px.max(1) as usize;
    if img.w <= px && img.h <= px {
        return img;
    }
    let scale = img.w.max(img.h) as f32 / px as f32;
    let (w, h) = (((img.w as f32 / scale).round() as usize).max(1), ((img.h as f32 / scale).round() as usize).max(1));
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        let (y0, y1) = ((y as f32 * scale) as usize, (((y + 1) as f32 * scale).ceil() as usize).min(img.h));
        for x in 0..w {
            let (x0, x1) = ((x as f32 * scale) as usize, (((x + 1) as f32 * scale).ceil() as usize).min(img.w));
            let mut sum = [0u32; 4];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let i = (sy * img.w + sx) * 4;
                    for c in 0..4 {
                        sum[c] += img.rgba[i + c] as u32;
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)).max(1) as u32;
            for c in 0..4 {
                out[(y * w + x) * 4 + c] = (sum[c] / n) as u8;
            }
        }
    }
    RawImg { w, h, rgba: out, premultiplied: img.premultiplied }
}
