//! Linux & macOS back-end for file-system access: directory listing, mounts
//! (local disks, removable media, SMB/CIFS, NFS, SSHFS, rclone and other FUSE
//! mounts, GNOME GVFS shares), well-known folders, cloud folders and change watching.

#[allow(unused_imports)] // CStr is used for macOS mount names
use std::ffi::{CStr, CString};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub use crate::fscommon::*;
use crate::i18n::{tr, tx};
use crate::util::ext_of;

#[cfg(target_os = "macos")]
const UF_HIDDEN: u32 = 0x8000;
/// macOS File Provider / iCloud placeholder whose data isn't on this Mac.
#[cfg(target_os = "macos")]
const SF_DATALESS: u32 = 0x4000_0000;

fn sys_ft(t: std::io::Result<std::time::SystemTime>) -> Option<u64> {
    let d = t.ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(unix_secs_to_filetime(d.as_secs() as i64, d.subsec_nanos()))
}

/// Map Unix metadata onto the shared attribute bits.
fn attrs_for(parent: &Path, name: &str, md: &std::fs::Metadata, link: bool, is_dir: bool) -> u32 {
    let mut a = 0;
    if is_dir {
        a |= A_DIR;
    }
    if name.starts_with('.') {
        a |= A_HIDDEN;
    }
    if link {
        a |= A_REPARSE;
    }
    if md.permissions().readonly() {
        a |= A_READONLY;
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt as Mac;
        let fl = md.st_flags();
        if fl & UF_HIDDEN != 0 {
            a |= A_HIDDEN;
        }
        if fl & SF_DATALESS != 0 {
            a |= A_RECALL_ON_DATA;
        }
    }
    // never wander into kernel/virtual trees when crawling the whole disk
    if is_dir && is_virtual_tree(parent, name) {
        a |= A_REPARSE | A_SYSTEM;
    }
    a
}

fn is_virtual_tree(parent: &Path, name: &str) -> bool {
    let p = parent.as_os_str().as_bytes();
    (p == b"/" && matches!(name, "proc" | "sys" | "dev" | "run" | "snap" | "lost+found" | "System" | "private" | "cores" | "Volumes")) // i18n-ignore: folder names on disk
        || (p == b"/System" && name == "Volumes") // i18n-ignore: a folder name on disk
}

fn entry_of(parent: &Path, name: String, lmd: std::fs::Metadata) -> Entry {
    let link = lmd.file_type().is_symlink();
    let md = if link { std::fs::metadata(parent.join(&name)).unwrap_or(lmd) } else { lmd };
    let is_dir = md.is_dir();
    let mtime = unix_secs_to_filetime(md.mtime(), md.mtime_nsec() as u32);
    Entry {
        lower: name.to_lowercase(),
        ext: ext_of(&name, is_dir),
        attrs: attrs_for(parent, &name, &md, link, is_dir),
        is_dir,
        size: if is_dir { 0 } else { md.len() },
        mtime,
        ctime: sys_ft(md.created()).unwrap_or(mtime),
        atime: unix_secs_to_filetime(md.atime(), md.atime_nsec() as u32),
        name,
    }
}

/// Stream a directory's entries in batches so the first rows paint immediately.
/// What a system error means for someone looking at a folder.
fn classify(e: &std::io::Error) -> FolderError {
    use std::io::ErrorKind as K;
    let kind = match e.kind() {
        K::NotFound | K::NotADirectory => FailKind::NotFound,
        K::PermissionDenied => FailKind::AccessDenied,
        K::TimedOut | K::HostUnreachable | K::NetworkUnreachable | K::ConnectionRefused | K::ConnectionReset | K::NotConnected | K::StaleNetworkFileHandle => FailKind::Unreachable,
        _ => FailKind::Other,
    };
    FolderError::new(kind, e.to_string())
}

/// A look at the folder, enough to know whether it's still there.
pub fn check_folder(path: &Path) -> Result<(), FolderError> {
    std::fs::read_dir(path).map(|_| ()).map_err(|e| classify(&e))
}

pub fn list_dir(path: &Path, cancel: &AtomicBool, mut emit: impl FnMut(Vec<Entry>)) -> Result<(), FolderError> {
    let rd = std::fs::read_dir(path).map_err(|e| classify(&e))?;
    let mut batch = Vec::with_capacity(256);
    let mut first = true;
    for de in rd.flatten() {
        let name = de.file_name().to_string_lossy().to_string();
        let Ok(lmd) = de.metadata() else { continue };
        batch.push(entry_of(path, name, lmd));
        if (first && batch.len() >= 256) || batch.len() >= 4096 {
            emit(std::mem::replace(&mut batch, Vec::with_capacity(4096)));
            first = false;
            if cancel.load(Ordering::Relaxed) {
                break;
            }
        }
    }
    if !batch.is_empty() {
        emit(batch);
    }
    Ok(())
}

/// Fast directory scan for the search crawler and size counter (does not follow links).
pub fn for_each_raw(path: &Path, mut f: impl FnMut(RawEntry)) -> bool {
    let Ok(rd) = std::fs::read_dir(path) else { return false };
    for de in rd.flatten() {
        let Ok(md) = de.metadata() else { continue };
        let name = de.file_name();
        let name = name.to_string_lossy();
        let link = md.file_type().is_symlink();
        let is_dir = md.is_dir();
        f(RawEntry {
            attrs: attrs_for(path, &name, &md, link, is_dir),
            size: if is_dir { 0 } else { md.len() },
            mtime: unix_secs_to_filetime(md.mtime(), md.mtime_nsec() as u32),
            name: name.to_string().into(),
        });
    }
    true
}

pub fn stat(path: &Path) -> Option<Entry> {
    let lmd = std::fs::symlink_metadata(path).ok()?;
    let parent = path.parent().unwrap_or(Path::new("/"));
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string_lossy().to_string());
    let mut e = entry_of(parent, name, lmd);
    e.attrs &= !A_SYSTEM;
    Some(e)
}

/// Windows `\\server` listing; on Linux/macOS servers are reached by mounting (smb://…).
pub fn is_server_root(_p: &Path) -> Option<String> {
    None
}

// ---------------------------------------------------------------- drives & mounts

fn stand_in_id(i: usize) -> char {
    char::from_u32(0xE000 + i as u32).unwrap_or('?')
}

fn kind_for(fs: &str, mount: &str, source: &str) -> Option<DriveKind> {
    let f = fs.to_ascii_lowercase();
    const VIRTUAL: &[&str] = &[
        "proc", "sysfs", "devtmpfs", "devpts", "tmpfs", "securityfs", "cgroup", "cgroup2", "pstore", "bpf", "autofs", "mqueue", "hugetlbfs",
        "debugfs", "tracefs", "configfs", "fusectl", "binfmt_misc", "ramfs", "nsfs", "efivarfs", "squashfs", "overlay", "rpc_pipefs",
        "selinuxfs", "fuse.portal", "fuse.gvfsd-fuse", "fuse.snapfuse", "devfs", "nullfs", "fdesc", "lxcfs", "fuse.lxcfs", "fuse.doc",
    ];
    if VIRTUAL.contains(&f.as_str()) {
        return None;
    }
    let hidden_prefix = ["/proc", "/sys", "/dev", "/snap", "/boot", "/var/lib/docker", "/var/snap", "/System/Volumes", "/private/var"];
    if mount != "/" && hidden_prefix.iter().any(|p| mount == *p || mount.starts_with(&format!("{p}/"))) {
        return None;
    }
    if mount.starts_with("/run/") && !mount.starts_with("/run/media/") {
        return None;
    }
    let network = matches!(f.as_str(), "cifs" | "smb3" | "smbfs" | "nfs" | "nfs4" | "9p" | "afs" | "afpfs" | "webdav" | "davfs" | "ftp" | "ceph" | "glusterfs")
        || matches!(f.as_str(), "fuse.sshfs" | "fuse.rclone" | "fuse.s3fs" | "fuse.davfs" | "fuse.curlftpfs" | "fuse.gcsfuse" | "fuse.goofys" | "fuse.juicefs")
        || (f.starts_with("fuse") && (source.contains('@') || source.contains(":/") || source.ends_with(':')));
    if network {
        return Some(DriveKind::Network);
    }
    if matches!(f.as_str(), "iso9660" | "udf" | "cd9660") {
        return Some(DriveKind::Optical);
    }
    if mount.starts_with("/media/") || mount.starts_with("/run/media/") || mount.starts_with("/Volumes/") || mount.starts_with("/mnt/") {
        return Some(DriveKind::Removable);
    }
    Some(DriveKind::Fixed)
}

fn mount_label(mount: &str) -> String {
    if mount == "/" {
        // "Macintosh HD" is what Apple calls it in every language
        return if cfg!(target_os = "macos") { "Macintosh HD".into() } else { tr("File System").into() }; // i18n-ignore: Apple's name
    }
    Path::new(mount).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| mount.to_string())
}

/// `\040` style escapes in /proc/self/mountinfo.
#[cfg(target_os = "linux")]
fn unescape_mount(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
            out.push((b[i + 1] - b'0') * 64 + (b[i + 2] - b'0') * 8 + (b[i + 3] - b'0'));
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

/// GNOME/GVFS network shares (smb://, sftp://, ftp://, dav://) mounted through its FUSE bridge.
#[cfg(target_os = "linux")]
fn gvfs_drives(out: &mut Vec<Drive>) {
    let uid = unsafe { libc::getuid() };
    let dir = PathBuf::from(format!("/run/user/{uid}/gvfs"));
    let Ok(rd) = std::fs::read_dir(&dir) else { return };
    for de in rd.flatten() {
        let name = de.file_name().to_string_lossy().to_string();
        let (scheme, rest) = name.split_once(':').unwrap_or((name.as_str(), ""));
        let kv: std::collections::HashMap<&str, &str> = rest.split(',').filter_map(|p| p.split_once('=')).collect();
        let server = kv.get("server").or(kv.get("host")).copied().unwrap_or("");
        let (label, url) = match scheme {
            "smb-share" => {
                let share = kv.get("share").copied().unwrap_or("");
                (trf!("{share} on {server}", share = share, server = server), format!("smb://{server}/{share}"))
            }
            "sftp" => (format!("{server} (SFTP)"), format!("sftp://{server}")),
            "ftp" => (format!("{server} (FTP)"), format!("ftp://{server}")),
            "dav" | "davs" => (format!("{server} (WebDAV)"), format!("{scheme}://{server}")), // i18n-ignore: a protocol's name
            "nfs" => (format!("{server} (NFS)"), format!("nfs://{server}")),
            _ => (name.clone(), name.clone()),
        };
        out.push(Drive {
            letter: stand_in_id(out.len()),
            root: de.path(),
            kind: DriveKind::Network,
            label,
            remote: Some(url),
            fs: "gvfs (FUSE)".into(),
            total: 0,
            free: 0,
            ready: false,
            checked: false,
            disconnected: false,
            locked: false,
        });
    }
}

/// Changes when something is mounted or unmounted. Cheap enough to ask every second: the list is read without
/// asking any of the drives.
pub fn drive_signature() -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for d in drives_quick() {
        d.root.hash(&mut h);
    }
    h.finish()
}

/// Mounted file systems, quickly (no capacity queries, as those can hang on dead network mounts).
#[cfg(target_os = "linux")]
pub fn drives_quick() -> Vec<Drive> {
    let mut out: Vec<Drive> = Vec::new();
    let text = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    for line in text.lines() {
        // id parent major:minor root mountpoint options [optional…], then a lone "-" separator field, then fstype source superoptions
        let Some((left, right)) = line.split_once(" - ") else { continue };
        let l: Vec<&str> = left.split(' ').collect();
        let r: Vec<&str> = right.split(' ').collect();
        if l.len() < 5 || r.len() < 2 {
            continue;
        }
        let mount = unescape_mount(l[4]);
        let (fs, source) = (r[0].to_string(), unescape_mount(r[1]));
        let Some(kind) = kind_for(&fs, &mount, &source) else { continue };
        if out.iter().any(|d| d.root == Path::new(&mount)) {
            continue; // bind mounts / stacked mounts
        }
        let remote = (kind == DriveKind::Network || fs.starts_with("fuse")).then(|| source.clone());
        out.push(Drive {
            letter: stand_in_id(out.len()),
            root: PathBuf::from(&mount),
            kind,
            label: mount_label(&mount),
            remote,
            fs,
            total: 0,
            free: 0,
            ready: false,
            checked: false,
            disconnected: false,
            locked: false,
        });
    }
    gvfs_drives(&mut out);
    out
}

#[cfg(target_os = "macos")]
pub fn drives_quick() -> Vec<Drive> {
    const MNT_NOWAIT: libc::c_int = 2;
    const MNT_DONTBROWSE: u32 = 0x0010_0000;
    let mut out: Vec<Drive> = Vec::new();
    let mut buf: *mut libc::statfs = std::ptr::null_mut();
    let n = unsafe { libc::getmntinfo(&mut buf, MNT_NOWAIT) };
    if n <= 0 || buf.is_null() {
        return out;
    }
    let mounts = unsafe { std::slice::from_raw_parts(buf, n as usize) };
    for m in mounts {
        let cs = |a: &[libc::c_char]| unsafe { CStr::from_ptr(a.as_ptr()) }.to_string_lossy().to_string();
        let (mount, fs, source) = (cs(&m.f_mntonname), cs(&m.f_fstypename), cs(&m.f_mntfromname));
        if mount != "/" && m.f_flags & MNT_DONTBROWSE != 0 {
            continue;
        }
        let Some(kind) = kind_for(&fs, &mount, &source) else { continue };
        let remote = (kind == DriveKind::Network || fs.contains("fuse")).then(|| source.clone());
        let bs = m.f_bsize as u64;
        out.push(Drive {
            letter: stand_in_id(out.len()),
            root: PathBuf::from(&mount),
            kind,
            label: mount_label(&mount),
            remote,
            fs,
            total: m.f_blocks * bs,
            free: m.f_bavail * bs,
            ready: m.f_blocks > 0,
            checked: true,
            disconnected: false,
            locked: false,
        });
    }
    out
}

/// Capacity for one mount.
pub fn drive_details(d: &mut Drive) {
    let Ok(c) = CString::new(d.root.as_os_str().as_bytes()) else { return };
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } == 0 {
        let unit = if s.f_frsize > 0 { s.f_frsize as u64 } else { s.f_bsize as u64 };
        d.total = s.f_blocks as u64 * unit;
        d.free = s.f_bavail as u64 * unit;
        d.ready = d.total > 0;
    }
}

// ---------------------------------------------------------------- well-known folders

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// Folder names from ~/.config/user-dirs.dirs (localised on many Linux desktops).
#[cfg(target_os = "linux")]
fn xdg_user_dir(key: &str, default: &str) -> PathBuf {
    let h = home();
    let cfg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| h.join(".config"));
    if let Ok(text) = std::fs::read_to_string(cfg.join("user-dirs.dirs")) {
        for line in text.lines() {
            if let Some(v) = line.strip_prefix(&format!("XDG_{key}_DIR=")) {
                let v = v.trim().trim_matches('"').replace("$HOME", &h.to_string_lossy());
                return PathBuf::from(v);
            }
        }
    }
    h.join(default)
}

pub fn known_folders() -> Vec<Place> {
    let h = home();
    // the names stay English inside the program, which finds places by them; they're translated where drawn. The
    // second name on a line is the folder's name on disk.
    #[cfg(target_os = "linux")]
    let list: Vec<(&str, PathBuf)> = vec![
        (tx("Home"), h.clone()),
        (tx("Desktop"), xdg_user_dir("DESKTOP", "Desktop")), // i18n-ignore: a folder name on disk
        (tx("Downloads"), xdg_user_dir("DOWNLOAD", "Downloads")), // i18n-ignore: a folder name on disk
        (tx("Documents"), xdg_user_dir("DOCUMENTS", "Documents")), // i18n-ignore: a folder name on disk
        (tx("Pictures"), xdg_user_dir("PICTURES", "Pictures")), // i18n-ignore: a folder name on disk
        (tx("Music"), xdg_user_dir("MUSIC", "Music")), // i18n-ignore: a folder name on disk
        (tx("Videos"), xdg_user_dir("VIDEOS", "Videos")), // i18n-ignore: a folder name on disk
    ];
    #[cfg(not(target_os = "linux"))]
    let list: Vec<(&str, PathBuf)> = vec![
        (tx("Home"), h.clone()),
        (tx("Desktop"), h.join("Desktop")), // i18n-ignore: a folder name on disk
        (tx("Downloads"), h.join("Downloads")), // i18n-ignore: a folder name on disk
        (tx("Documents"), h.join("Documents")), // i18n-ignore: a folder name on disk
        (tx("Pictures"), h.join("Pictures")), // i18n-ignore: a folder name on disk
        (tx("Music"), h.join("Music")), // i18n-ignore: a folder name on disk
        (tx("Videos"), h.join("Movies")), // i18n-ignore: a folder name on disk
    ];
    list.into_iter()
        .filter(|(n, p)| p.is_dir() && (*n == "Home" || p != &h)) // i18n-ignore: the place's key
        .map(|(n, p)| Place { name: n.into(), path: p, kf: () })
        .collect()
}

// ---------------------------------------------------------------- cloud folders

/// Cloud folders: macOS File Provider (~/Library/CloudStorage) and iCloud Drive;
/// on Linux the usual sync-client folders.
pub fn cloud_roots() -> Vec<CloudRoot> {
    let h = home();
    let mut out: Vec<CloudRoot> = Vec::new();
    if cfg!(target_os = "macos") {
        if let Ok(rd) = std::fs::read_dir(h.join("Library/CloudStorage")) {
            for de in rd.flatten().filter(|d| d.path().is_dir()) {
                let n = de.file_name().to_string_lossy().to_string();
                let (provider, account) = n.split_once('-').unwrap_or((n.as_str(), ""));
                let name = if account.is_empty() { provider.to_string() } else { format!("{provider} ({account})") };
                out.push(CloudRoot { name, provider: provider.into(), path: de.path() });
            }
        }
        let icloud = h.join("Library/Mobile Documents/com~apple~CloudDocs"); // i18n-ignore: a path
        if icloud.is_dir() {
            out.push(CloudRoot { name: "iCloud Drive".into(), provider: "iCloud".into(), path: icloud }); // i18n-ignore: Apple's product name
        }
    }
    // product names, and the folders their apps make
    for (prov, sub) in [
        ("Dropbox", "Dropbox"), // i18n-ignore: product and folder names
        ("OneDrive", "OneDrive"),
        ("GoogleDrive", "Google Drive"), // i18n-ignore: product and folder names
        ("GoogleDrive", "GoogleDrive"),
        ("Nextcloud", "Nextcloud"), // i18n-ignore: product and folder names
        ("ownCloud", "ownCloud"),
        ("pCloud", "pCloudDrive"),
        ("MEGA", "MEGA"),
        ("Insync", "Insync"), // i18n-ignore: product and folder names
    ] {
        let p = h.join(sub);
        if p.is_dir() && !out.iter().any(|r| same_path(&r.path, &p)) {
            out.push(CloudRoot { name: sub.into(), provider: prov.into(), path: p });
        }
    }
    for r in &mut out {
        r.name = undash_name(&r.name); // account names are shown in brackets: "OneDrive (Personal)"
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// "Always keep on this device" / "Free up space" (macOS iCloud and File Provider clouds).
pub fn set_pin(path: &Path, keep_local: bool) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err(tr("Keeping files offline is managed by your sync app on Linux").into());
    }
    let icloud = path.to_string_lossy().contains("/Library/Mobile Documents/"); // i18n-ignore: a path
    let (tool, verb) = if icloud { ("brctl", if keep_local { "download" } else { "evict" }) } else { ("fileproviderctl", if keep_local { "materialize" } else { "evict" }) };
    let st = std::process::Command::new(tool).arg(verb).arg(path).status().map_err(|e| e.to_string())?;
    if st.success() {
        Ok(())
    } else {
        Err(trf!("{tool} {verb} failed", tool = tool, verb = verb))
    }
}

/// Read-only via permissions; "hidden" via the macOS hidden flag (Linux hides dot-names).
pub fn set_attrs(path: &Path, readonly: Option<bool>, hidden: Option<bool>) -> Result<(), String> {
    if let Some(r) = readonly {
        let md = std::fs::metadata(path).map_err(|e| e.to_string())?;
        let mut perm = md.permissions();
        use std::os::unix::fs::PermissionsExt;
        let mode = perm.mode();
        perm.set_mode(if r { mode & !0o222 } else { mode | 0o200 });
        std::fs::set_permissions(path, perm).map_err(|e| e.to_string())?;
    }
    if let Some(h) = hidden {
        #[cfg(target_os = "macos")]
        {
            use std::os::macos::fs::MetadataExt as Mac;
            let md = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
            let flags = if h { md.st_flags() | UF_HIDDEN } else { md.st_flags() & !UF_HIDDEN };
            let c = CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
            if unsafe { libc::chflags(c.as_ptr(), flags as _) } != 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let is_dot = path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'));
            if h != is_dot {
                return Err(tr("On Linux a file is hidden when its name starts with a dot. Rename it to change that").into());
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- change watching

/// Calls `on_change` whenever something in `path` (non-recursive) changes.
#[cfg(target_os = "linux")]
pub fn watch(path: &Path, on_change: impl Fn() + Send + 'static) -> Watcher {
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    let cpath = CString::new(path.as_os_str().as_bytes()).ok();
    std::thread::Builder::new()
        .name("watch".into())
        .spawn(move || unsafe {
            let Some(cpath) = cpath else { return };
            let fd = libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC);
            if fd < 0 {
                return;
            }
            let mask = libc::IN_CREATE | libc::IN_DELETE | libc::IN_MOVED_FROM | libc::IN_MOVED_TO | libc::IN_MODIFY | libc::IN_ATTRIB | libc::IN_CLOSE_WRITE;
            if libc::inotify_add_watch(fd, cpath.as_ptr(), mask) < 0 {
                libc::close(fd);
                return;
            }
            let mut buf = [0u8; 4096];
            while !s2.load(Ordering::Relaxed) {
                let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
                if libc::poll(&mut pfd, 1, 250) > 0 {
                    while libc::read(fd, buf.as_mut_ptr() as *mut _, buf.len()) > 0 {}
                    if !s2.load(Ordering::Relaxed) {
                        on_change();
                    }
                }
            }
            libc::close(fd);
        })
        .ok();
    Watcher { stop }
}

/// macOS: compare the folder's modification time (changes when entries are added, removed or renamed).
#[cfg(not(target_os = "linux"))]
pub fn watch(path: &Path, on_change: impl Fn() + Send + 'static) -> Watcher {
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    let path = path.to_path_buf();
    std::thread::Builder::new()
        .name("watch".into())
        .spawn(move || {
            let stamp = |p: &Path| std::fs::metadata(p).ok().map(|m| (m.mtime(), m.mtime_nsec(), m.nlink()));
            let mut last = stamp(&path);
            while !s2.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(700));
                let now = stamp(&path);
                if now != last {
                    last = now;
                    on_change();
                }
            }
        })
        .ok();
    Watcher { stop }
}

/// Only Windows has mapped drives to connect again.
pub fn reconnect_for(_path: &Path, _owner: isize, _ask: bool) -> Result<(), FolderError> {
    Err(FolderError::new(FailKind::Other, tr("This isn't a mapped drive")))
}

/// Whether files can be added to `dir`. Can take as long as a server takes to not answer, so not on the window's thread.
pub fn can_add_to(dir: &Path) -> Result<(), NoWrite> {
    let Ok(c) = CString::new(dir.as_os_str().as_bytes()) else { return Err(NoWrite::Unreachable) };
    if unsafe { libc::access(c.as_ptr(), libc::W_OK | libc::X_OK) } == 0 {
        return Ok(());
    }
    Err(match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::EROFS) => NoWrite::ReadOnly,
        Some(libc::EACCES) | Some(libc::EPERM) => NoWrite::NoAccess,
        _ => NoWrite::Unreachable,
    })
}

/// BitLocker is Windows's: never locked here.
pub fn bitlocker_locked(_root: &Path) -> bool {
    false
}

pub fn unlock_bitlocker(_root: &Path) {}
