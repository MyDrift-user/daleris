//! Raw, fast file system access: directory listing via FindFirstFileExW with
//! large fetch (no per-file stat calls), drives, network shares, cloud sync roots.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::NetworkManagement::WNet::WNetGetConnectionW;
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::WaitForSingleObject;
use windows::Win32::UI::Shell::*;

use crate::util::{ext_of, from_wide, wide};

pub use crate::fscommon::*;


/// Path form that survives >260 chars.
pub fn long_path(p: &Path) -> Vec<u16> {
    let s = p.as_os_str().to_string_lossy();
    if s.len() < 240 || s.starts_with(r"\\?\") {
        return wide(p.as_os_str());
    }
    if let Some(rest) = s.strip_prefix(r"\\") {
        wide(format!(r"\\?\UNC\{rest}"))
    } else {
        wide(format!(r"\\?\{s}"))
    }
}

pub fn ft(f: FILETIME) -> u64 {
    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
}

/// Is this `\\server` with no share part?
pub fn is_server_root(p: &Path) -> Option<String> {
    let s = p.to_string_lossy();
    let rest = s.strip_prefix(r"\\")?;
    let rest = rest.trim_end_matches('\\');
    if !rest.is_empty() && !rest.contains('\\') && !rest.starts_with('?') {
        Some(rest.to_string())
    } else {
        None
    }
}

/// Stream a directory's entries in batches so the first rows paint immediately.
pub fn list_dir(path: &Path, cancel: &AtomicBool, mut emit: impl FnMut(Vec<Entry>)) -> Result<(), FolderError> {
    if let Some(server) = is_server_root(path) {
        emit(list_shares(&server)?);
        return Ok(());
    }
    let mut pattern = path.as_os_str().to_os_string();
    if !pattern.to_string_lossy().ends_with('\\') {
        pattern.push("\\");
    }
    pattern.push("*");
    let pat = long_path(Path::new(&pattern));
    let mut data = WIN32_FIND_DATAW::default();
    let handle = unsafe {
        FindFirstFileExW(
            PCWSTR(pat.as_ptr()),
            FindExInfoBasic,
            &mut data as *mut _ as *mut _,
            FindExSearchNameMatch,
            None,
            FIND_FIRST_EX_LARGE_FETCH,
        )
    };
    let handle = match handle {
        Ok(h) => h,
        Err(e) => {
            if e.code() == ERROR_FILE_NOT_FOUND.to_hresult() {
                return Ok(()); // empty folder (e.g. a bare drive root with nothing in it)
            }
            return Err(classify(path, (e.code().0 as u32) & 0xFFFF, e.message()));
        }
    };
    let mut batch = Vec::with_capacity(512);
    let mut first = true;
    loop {
        if let Some(e) = entry_from(&data) {
            batch.push(e);
            if (first && batch.len() >= 256) || batch.len() >= 4096 {
                emit(std::mem::replace(&mut batch, Vec::with_capacity(4096)));
                first = false;
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
            }
        }
        if unsafe { FindNextFileW(handle, &mut data) }.is_err() {
            break;
        }
    }
    unsafe {
        let _ = FindClose(handle);
    }
    if !batch.is_empty() {
        emit(batch);
    }
    Ok(())
}

/// What a system error means for someone looking at this path.
fn classify(path: &Path, code: u32, message: String) -> FolderError {
    let kind = classify_code(code, is_network_path(path), drive_letter(path).is_some_and(|l| !drive_exists(l)));
    FolderError::new(kind, format!("{} (error {code})", message.trim()))
}

/// The kind of failure a system error code is. `letter_gone`: the path is on a drive letter that doesn't exist now.
fn classify_code(code: u32, network: bool, letter_gone: bool) -> FailKind {
    match code {
        // the drive went away, which for a mapped one means it's not connected
        2 | 3 | 15 | 53 | 67 | 1200 if letter_gone => FailKind::Disconnected,
        // the network isn't there, or the server doesn't answer
        53 | 51 | 54 | 59 | 64 | 121 | 1231 | 1232 | 1222 | 1311 => FailKind::Unreachable,
        67 => FailKind::ShareMissing,
        5 | 86 | 1219 | 1326 | 1327 | 1328 | 1330 | 1331 | 1909 => FailKind::AccessDenied,
        1201 | 2250 | 1200 => FailKind::Disconnected,
        2 | 3 | 15 | 161 | 267 => FailKind::NotFound,
        21 | 1005 | 1006 | 1392 => FailKind::NotReady,
        _ if network => FailKind::Unreachable,
        _ => FailKind::Other,
    }
}

fn drive_letter(path: &Path) -> Option<char> {
    let s = path.to_string_lossy();
    let mut c = s.chars();
    let l = c.next()?;
    (l.is_ascii_alphabetic() && c.next() == Some(':')).then(|| l.to_ascii_uppercase())
}

fn drive_exists(letter: char) -> bool {
    let bit = letter as u32 - 'A' as u32;
    let mask = unsafe { GetLogicalDrives() };
    mask & (1 << bit) != 0
}

/// Not in the bindings, so found by name. 0 is success, as with the rest of this API.
unsafe fn restore_connection(owner: HWND, device: PCWSTR, ask: bool) -> u32 { unsafe {
    use windows::core::{s, w};
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    type Restore = unsafe extern "system" fn(HWND, PCWSTR, i32) -> u32;
    let Ok(lib) = LoadLibraryW(w!("mpr.dll")) else { return ERROR_NOT_SUPPORTED.0 };
    match GetProcAddress(lib, s!("WNetRestoreSingleConnectionW")) {
        Some(f) => std::mem::transmute::<_, Restore>(f)(owner, device, ask as i32),
        None => ERROR_NOT_SUPPORTED.0,
    }
}}

/// Connects a mapped drive again. With `ask`, Windows may ask for a password, in a window owned by `owner`. Can take
/// as long as a server takes to not answer.
pub fn reconnect_drive(letter: char, owner: HWND, ask: bool) -> Result<(), FolderError> {
    let local = wide(format!("{letter}:"));
    let rc = unsafe { restore_connection(owner, PCWSTR(local.as_ptr()), ask) };
    if rc == 0 {
        return Ok(());
    }
    let message = windows::core::Error::from_hresult(WIN32_ERROR(rc).to_hresult()).message();
    // asked for the password and got nothing
    if rc == 1223 {
        return Err(FolderError::new(FailKind::Cancelled, message));
    }
    Err(classify(Path::new(&format!("{letter}:\\")), rc, message))
}

/// A look at the folder, enough to know whether it's still there. Can take as long as a server takes to not answer.
pub fn check_folder(path: &Path) -> Result<(), FolderError> {
    let mut pattern = path.as_os_str().to_os_string();
    if !pattern.to_string_lossy().ends_with('\\') {
        pattern.push("\\");
    }
    pattern.push("*");
    let pat = long_path(Path::new(&pattern));
    let mut data = WIN32_FIND_DATAW::default();
    match unsafe { FindFirstFileExW(PCWSTR(pat.as_ptr()), FindExInfoBasic, &mut data as *mut _ as *mut _, FindExSearchNameMatch, None, FIND_FIRST_EX_FLAGS(0)) } {
        Ok(h) => {
            unsafe {
                let _ = FindClose(h);
            }
            Ok(())
        }
        Err(e) if e.code() == ERROR_FILE_NOT_FOUND.to_hresult() => Ok(()),
        Err(e) => Err(classify(path, (e.code().0 as u32) & 0xFFFF, e.message())),
    }
}

fn entry_from(d: &WIN32_FIND_DATAW) -> Option<Entry> {
    let name = from_wide(&d.cFileName);
    if name == "." || name == ".." {
        return None;
    }
    let attrs = d.dwFileAttributes;
    let is_dir = attrs & A_DIR != 0;
    Some(Entry {
        lower: name.to_lowercase(),
        ext: ext_of(&name, is_dir),
        name,
        is_dir,
        size: ((d.nFileSizeHigh as u64) << 32) | d.nFileSizeLow as u64,
        mtime: ft(d.ftLastWriteTime),
        ctime: ft(d.ftCreationTime),
        atime: ft(d.ftLastAccessTime),
        attrs,
    })
}

/// Fast directory scan for the search crawler and size counter.
pub fn for_each_raw(path: &Path, mut f: impl FnMut(RawEntry)) -> bool {
    let mut pattern = path.as_os_str().to_os_string();
    if !pattern.to_string_lossy().ends_with('\\') {
        pattern.push("\\");
    }
    pattern.push("*");
    let pat = long_path(Path::new(&pattern));
    let mut data = WIN32_FIND_DATAW::default();
    let Ok(handle) = (unsafe {
        FindFirstFileExW(
            PCWSTR(pat.as_ptr()),
            FindExInfoBasic,
            &mut data as *mut _ as *mut _,
            FindExSearchNameMatch,
            None,
            FIND_FIRST_EX_LARGE_FETCH,
        )
    }) else {
        return false;
    };
    loop {
        let n = &data.cFileName;
        let dot = n[0] == b'.' as u16 && (n[1] == 0 || (n[1] == b'.' as u16 && n[2] == 0));
        if !dot {
            let end = n.iter().position(|&c| c == 0).unwrap_or(n.len());
            f(RawEntry {
                name: String::from_utf16_lossy(&n[..end]).into(),
                attrs: data.dwFileAttributes,
                size: ((data.nFileSizeHigh as u64) << 32) | data.nFileSizeLow as u64,
                mtime: ft(data.ftLastWriteTime),
            });
        }
        if unsafe { FindNextFileW(handle, &mut data) }.is_err() {
            break;
        }
    }
    unsafe {
        let _ = FindClose(handle);
    }
    true
}

pub fn stat(path: &Path) -> Option<Entry> {
    let w = long_path(path);
    let mut data = WIN32_FILE_ATTRIBUTE_DATA::default();
    unsafe { GetFileAttributesExW(PCWSTR(w.as_ptr()), GetFileExInfoStandard, &mut data as *mut _ as *mut _) }.ok()?;
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string_lossy().to_string());
    let is_dir = data.dwFileAttributes & A_DIR != 0;
    Some(Entry {
        lower: name.to_lowercase(),
        ext: ext_of(&name, is_dir),
        name,
        is_dir,
        size: ((data.nFileSizeHigh as u64) << 32) | data.nFileSizeLow as u64,
        mtime: ft(data.ftLastWriteTime),
        ctime: ft(data.ftCreationTime),
        atime: ft(data.ftLastAccessTime),
        attrs: data.dwFileAttributes,
    })
}

// ---------------------------------------------------------------- network shares

fn list_shares(server: &str) -> Result<Vec<Entry>, FolderError> {
    let srv = wide(format!(r"\\{server}"));
    let mut buf: *mut u8 = std::ptr::null_mut();
    let (mut read, mut total) = (0u32, 0u32);
    let rc = unsafe { NetShareEnum(PCWSTR(srv.as_ptr()), 1, &mut buf, u32::MAX, &mut read, &mut total, None) };
    if rc != 0 || buf.is_null() {
        let kind = if rc == 0 { FailKind::Other } else { classify_code(rc, true, false) };
        return Err(FolderError::new(kind, format!("Could not list shares on \\\\{server} (error {rc})")));
    }
    let mut out = Vec::new();
    unsafe {
        let items = std::slice::from_raw_parts(buf as *const SHARE_INFO_1, read as usize);
        for it in items {
            let name = it.shi1_netname.to_string().unwrap_or_default();
            // disk shares only; skip admin `$` shares
            if it.shi1_type.0 & 0xFF != 0 || name.ends_with('$') {
                continue;
            }
            out.push(Entry {
                lower: name.to_lowercase(),
                ext: String::new(),
                name,
                is_dir: true,
                size: 0,
                mtime: 0,
                ctime: 0,
                atime: 0,
                attrs: A_DIR,
            });
        }
        windows::Win32::NetworkManagement::NetManagement::NetApiBufferFree(Some(buf as *const _));
    }
    Ok(out)
}

// ---------------------------------------------------------------- drives


/// Fast pass: letters, types and network mappings. No volume queries (those can hang on dead shares).
pub fn drives_quick() -> Vec<Drive> {
    let mask = unsafe { GetLogicalDrives() };
    let mut out = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let root = format!("{letter}:\\");
        let w = wide(&root);
        let kind = match unsafe { GetDriveTypeW(PCWSTR(w.as_ptr())) } {
            2 => DriveKind::Removable,
            3 => DriveKind::Fixed,
            4 => DriveKind::Network,
            5 => DriveKind::Optical,
            6 => DriveKind::Ram,
            _ => DriveKind::Unknown,
        };
        let mut disconnected = false;
        let remote = if kind == DriveKind::Network {
            let local = wide(format!("{letter}:"));
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let rc = unsafe { WNetGetConnectionW(PCWSTR(local.as_ptr()), Some(PWSTR(buf.as_mut_ptr())), &mut len) };
            // 1201: remembered, not connected. The name still comes back.
            disconnected = rc == WIN32_ERROR(1201);
            if rc == NO_ERROR || disconnected {
                Some(from_wide(&buf))
            } else {
                None
            }
        } else {
            None
        };
        out.push(Drive {
            letter,
            root: PathBuf::from(root),
            kind,
            label: String::new(),
            remote,
            fs: String::new(),
            total: 0,
            free: 0,
            ready: false,
            checked: false,
            disconnected,
        });
    }
    // Windows remembers mappings it couldn't connect at logon, and those have no drive letter until they do.
    for i in (0..26u32).filter(|i| mask & (1 << i) == 0) {
        let letter = (b'A' + i as u8) as char;
        if let Some(remote) = remembered_drive(letter) {
            out.push(Drive {
                letter,
                root: PathBuf::from(format!("{letter}:\\")),
                kind: DriveKind::Network,
                label: String::new(),
                remote: Some(remote),
                fs: String::new(),
                total: 0,
                free: 0,
                ready: false,
                checked: true,
                disconnected: true,
            });
        }
    }
    out.sort_by_key(|d| d.letter);
    out
}

/// What a drive letter that's not in use is remembered as, if anything. Doesn't touch the network.
fn remembered_drive(letter: char) -> Option<String> {
    let local = wide(format!("{letter}:"));
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    let rc = unsafe { WNetGetConnectionW(PCWSTR(local.as_ptr()), Some(PWSTR(buf.as_mut_ptr())), &mut len) };
    (rc == WIN32_ERROR(1201)).then(|| from_wide(&buf)).filter(|r| !r.is_empty())
}

/// Slow pass for one drive: label, file system and capacity.
pub fn drive_details(d: &mut Drive) {
    d.checked = true;
    d.ready = false;
    let w = wide(d.root.as_os_str());
    let mut label = [0u16; 261];
    let mut fs = [0u16; 64];
    if unsafe { GetVolumeInformationW(PCWSTR(w.as_ptr()), Some(&mut label), None, None, None, Some(&mut fs)) }.is_ok() {
        d.label = from_wide(&label);
        d.fs = from_wide(&fs);
    }
    let (mut free, mut total) = (0u64, 0u64);
    if unsafe { GetDiskFreeSpaceExW(PCWSTR(w.as_ptr()), Some(&mut free), Some(&mut total), None) }.is_ok() {
        d.free = free;
        d.total = total;
        d.ready = true;
    }
}

// ---------------------------------------------------------------- known folders


pub fn known_folders() -> Vec<Place> {
    let ids: [(&str, windows::core::GUID); 7] = [
        ("Home", FOLDERID_Profile),
        ("Desktop", FOLDERID_Desktop),
        ("Downloads", FOLDERID_Downloads),
        ("Documents", FOLDERID_Documents),
        ("Pictures", FOLDERID_Pictures),
        ("Music", FOLDERID_Music),
        ("Videos", FOLDERID_Videos),
    ];
    let mut out = Vec::new();
    for (name, id) in ids {
        if let Ok(p) = unsafe { SHGetKnownFolderPath(&id, KF_FLAG_DEFAULT, None) } {
            let s = unsafe { p.to_string() }.unwrap_or_default();
            unsafe { CoTaskMemFree(Some(p.0 as *const _)) };
            if !s.is_empty() {
                out.push(Place { name: name.into(), path: PathBuf::from(s), kf: id });
            }
        }
    }
    out
}

// ---------------------------------------------------------------- cloud sync roots


/// Cloud drives registered with Windows' Cloud Files API (the same list Explorer's
/// navigation pane uses), plus OneDrive's environment variables as a fallback.
pub fn cloud_roots() -> Vec<CloudRoot> {
    let mut out: Vec<CloudRoot> = Vec::new();
    let base = wide(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\SyncRootManager");
    unsafe {
        let mut hk = HKEY::default();
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, PCWSTR(base.as_ptr()), Some(0), KEY_READ, &mut hk) == ERROR_SUCCESS {
            let mut i = 0;
            loop {
                let mut name = [0u16; 512];
                let mut len = name.len() as u32;
                if RegEnumKeyExW(hk, i, Some(PWSTR(name.as_mut_ptr())), &mut len, None, Some(PWSTR::null()), None, None) != ERROR_SUCCESS {
                    break;
                }
                i += 1;
                let key_name = from_wide(&name);
                let display = reg_string(hk, &key_name, "DisplayNameResource")
                    .map(|s| resolve_indirect(&s))
                    .filter(|s| !s.is_empty());
                let sub = wide(format!(r"{key_name}\UserSyncRoots"));
                let mut uk = HKEY::default();
                if RegOpenKeyExW(hk, PCWSTR(sub.as_ptr()), Some(0), KEY_READ, &mut uk) != ERROR_SUCCESS {
                    continue;
                }
                let mut j = 0;
                loop {
                    let mut vname = [0u16; 256];
                    let mut vlen = vname.len() as u32;
                    let mut data = [0u8; 2048];
                    let mut dlen = data.len() as u32;
                    if RegEnumValueW(uk, j, Some(PWSTR(vname.as_mut_ptr())), &mut vlen, None, None, Some(data.as_mut_ptr()), Some(&mut dlen))
                        != ERROR_SUCCESS
                    {
                        break;
                    }
                    j += 1;
                    let wd: Vec<u16> = data[..dlen as usize].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                    let path = PathBuf::from(from_wide(&wd));
                    if path.as_os_str().is_empty() || !path.exists() {
                        continue;
                    }
                    let provider = key_name.split('!').next().unwrap_or("Cloud").to_string();
                    let account = key_name.rsplit('!').next().unwrap_or("").split('|').next().unwrap_or("").to_string();
                    let name = display.clone().unwrap_or_else(|| {
                        if account.is_empty() || account.starts_with("S-1-") {
                            provider.clone()
                        } else {
                            format!("{provider} ({account})")
                        }
                    });
                    out.push(CloudRoot { name, provider: provider.clone(), path });
                }
                let _ = RegCloseKey(uk);
            }
            let _ = RegCloseKey(hk);
        }
    }
    for var in ["OneDriveConsumer", "OneDriveCommercial", "OneDrive"] {
        if let Some(p) = std::env::var_os(var) {
            let path = PathBuf::from(p);
            if path.exists() && !out.iter().any(|r| same_path(&r.path, &path)) {
                let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or("OneDrive".into());
                out.push(CloudRoot { name, provider: "OneDrive".into(), path });
            }
        }
    }
    // Dropbox / Google Drive default folders that aren't registered as sync roots
    if let Some(home) = std::env::var_os("USERPROFILE") {
        for (prov, sub) in [("Dropbox", "Dropbox"), ("GoogleDrive", "Google Drive"), ("iCloudDrive", "iCloudDrive")] {
            let p = PathBuf::from(&home).join(sub);
            if p.is_dir() && !out.iter().any(|r| same_path(&r.path, &p)) {
                out.push(CloudRoot { name: sub.into(), provider: prov.into(), path: p });
            }
        }
    }
    for r in &mut out {
        r.name = undash_name(&r.name); // account names are shown in brackets: "OneDrive (Personal)"
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out.dedup_by(|a, b| same_path(&a.path, &b.path));
    out
}


unsafe fn reg_string(hk: HKEY, sub: &str, value: &str) -> Option<String> { unsafe {
    let s = wide(sub);
    let v = wide(value);
    let mut buf = [0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    let rc = RegGetValueW(hk, PCWSTR(s.as_ptr()), PCWSTR(v.as_ptr()), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut len));
    (rc == ERROR_SUCCESS).then(|| from_wide(&buf))
}}

pub fn resolve_indirect(s: &str) -> String {
    if !s.starts_with('@') {
        return s.to_string();
    }
    let w = wide(s);
    let mut out = [0u16; 512];
    match unsafe { SHLoadIndirectString(PCWSTR(w.as_ptr()), &mut out, None) } {
        Ok(()) => from_wide(&out),
        Err(_) => String::new(),
    }
}

/// Turn the read-only / hidden attributes on or off (None = leave as is).
pub fn set_attrs(path: &Path, readonly: Option<bool>, hidden: Option<bool>) -> Result<(), String> {
    let w = long_path(path);
    let cur = unsafe { GetFileAttributesW(PCWSTR(w.as_ptr())) };
    if cur == INVALID_FILE_ATTRIBUTES {
        return Err("File not found".into());
    }
    // only the attributes SetFileAttributes accepts
    let mut a = cur & (A_READONLY | A_HIDDEN | A_SYSTEM | 0x20 /* archive */ | 0x2000 /* not indexed */ | A_OFFLINE | A_PINNED | A_UNPINNED);
    if let Some(r) = readonly {
        a = if r { a | A_READONLY } else { a & !A_READONLY };
    }
    if let Some(h) = hidden {
        a = if h { a | A_HIDDEN } else { a & !A_HIDDEN };
    }
    unsafe { SetFileAttributesW(PCWSTR(w.as_ptr()), FILE_FLAGS_AND_ATTRIBUTES(a)) }.map_err(|e| e.message().to_string())
}

/// Pin ("Always keep on this device") or unpin ("Free up space"). The sync engine
/// reacts to these attributes exactly as it does when Explorer sets them.
pub fn set_pin(path: &Path, keep_local: bool) -> Result<(), String> {
    let w = long_path(path);
    let cur = unsafe { GetFileAttributesW(PCWSTR(w.as_ptr())) };
    if cur == INVALID_FILE_ATTRIBUTES {
        return Err("File not found".into());
    }
    let mut a = cur & !(A_DIR | A_REPARSE | A_COMPRESSED | A_ENCRYPTED | A_OFFLINE | A_RECALL_ON_DATA | A_RECALL_ON_OPEN);
    if keep_local {
        a = (a | A_PINNED) & !A_UNPINNED;
    } else {
        a = (a | A_UNPINNED) & !A_PINNED;
    }
    unsafe { SetFileAttributesW(PCWSTR(w.as_ptr()), FILE_FLAGS_AND_ATTRIBUTES(a)) }.map_err(|e| e.message().to_string())
}

// ---------------------------------------------------------------- change watching


/// Calls `on_change` whenever something in `path` (non-recursive) changes.
pub fn watch(path: &Path, on_change: impl Fn() + Send + 'static) -> Watcher {
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    let w = long_path(path);
    std::thread::Builder::new()
        .name("watch".into())
        .spawn(move || unsafe {
            let filter = FILE_NOTIFY_CHANGE_FILE_NAME
                | FILE_NOTIFY_CHANGE_DIR_NAME
                | FILE_NOTIFY_CHANGE_SIZE
                | FILE_NOTIFY_CHANGE_LAST_WRITE
                | FILE_NOTIFY_CHANGE_ATTRIBUTES;
            let Ok(h) = FindFirstChangeNotificationW(PCWSTR(w.as_ptr()), false, filter) else {
                return;
            };
            while !s2.load(Ordering::Relaxed) {
                if WaitForSingleObject(h, 250) == WAIT_OBJECT_0 {
                    if s2.load(Ordering::Relaxed) {
                        break;
                    }
                    on_change();
                    if FindNextChangeNotification(h).is_err() {
                        break;
                    }
                }
            }
            let _ = FindCloseChangeNotification(h);
        })
        .ok();
    Watcher { stop }
}

/// Connects the mapped drive `path` is on again, if it's on one. `owner` is the window a password prompt belongs to.
pub fn reconnect_for(path: &Path, owner: isize, ask: bool) -> Result<(), FolderError> {
    match drive_letter(path) {
        Some(l) => reconnect_drive(l, HWND(owner as *mut _), ask),
        None => Err(FolderError::new(FailKind::Other, "This isn't a mapped drive")),
    }
}

#[cfg(test)]
mod classify_tests {
    use super::*;

    #[test]
    fn system_errors_become_what_a_person_can_act_on() {
        assert_eq!(classify_code(53, true, false), FailKind::Unreachable);
        assert_eq!(classify_code(1232, true, false), FailKind::Unreachable);
        assert_eq!(classify_code(64, true, false), FailKind::Unreachable);
        assert_eq!(classify_code(67, true, false), FailKind::ShareMissing);
        assert_eq!(classify_code(5, true, false), FailKind::AccessDenied);
        assert_eq!(classify_code(1326, true, false), FailKind::AccessDenied);
        assert_eq!(classify_code(1201, true, false), FailKind::Disconnected);
        assert_eq!(classify_code(3, false, false), FailKind::NotFound);
        assert_eq!(classify_code(3, true, true), FailKind::Disconnected, "the drive letter itself is gone");
        assert_eq!(classify_code(21, false, false), FailKind::NotReady);
        assert_eq!(classify_code(9999, false, false), FailKind::Other);
        assert_eq!(classify_code(9999, true, false), FailKind::Unreachable);
    }
}
