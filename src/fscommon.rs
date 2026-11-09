//! File-system types shared by every platform. The Windows (`fsx.rs`) and Linux/macOS
//! (`fsx_unix.rs`) back-ends both fill these in, so the rest of the app never needs to
//! know which system it runs on.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use crate::i18n::tr;

// Attribute bits (Windows' values; the Unix back-end maps its own flags onto them).
pub const A_READONLY: u32 = 0x1;
pub const A_HIDDEN: u32 = 0x2;
pub const A_SYSTEM: u32 = 0x4;
pub const A_DIR: u32 = 0x10;
pub const A_REPARSE: u32 = 0x400;
pub const A_COMPRESSED: u32 = 0x800;
pub const A_OFFLINE: u32 = 0x1000;
pub const A_ENCRYPTED: u32 = 0x4000;
pub const A_RECALL_ON_OPEN: u32 = 0x40000;
pub const A_PINNED: u32 = 0x80000;
pub const A_UNPINNED: u32 = 0x100000;
pub const A_RECALL_ON_DATA: u32 = 0x400000;

/// Times are kept as Windows FILETIME ticks (100 ns since 1601) everywhere.
#[cfg_attr(windows, allow(dead_code))]
pub const UNIX_EPOCH_AS_FILETIME_SECS: u64 = 11_644_473_600;

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub lower: String,
    pub ext: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: u64,
    pub ctime: u64,
    pub atime: u64,
    pub attrs: u32,
}

impl Entry {
    pub fn hidden(&self) -> bool {
        self.attrs & (A_HIDDEN | A_SYSTEM) != 0
    }

    /// A name that starts with a dot.
    pub fn is_dotfile(&self) -> bool {
        is_dot_name(&self.name)
    }
}

pub fn is_dot_name(name: &str) -> bool {
    name.starts_with('.')
}

/// Why nothing can be added to a folder right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoWrite {
    NoAccess,
    ReadOnly,
    /// a drive without a disk, or one that's locked
    NotReady,
    Unreachable,
    /// the server wants a sign-in that isn't there or isn't right (Windows tells; Linux and macOS don't)
    #[cfg_attr(not(windows), allow(dead_code))]
    SignIn,
}

/// Why a folder can't be shown, in terms a person can do something about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailKind {
    /// the server doesn't answer, or the network isn't there
    Unreachable,
    /// the server answers, and has no such share
    ShareMissing,
    AccessDenied,
    /// the server turned down the user name or password Windows tried. Windows only, as are the next three
    #[cfg_attr(not(windows), allow(dead_code))]
    SignIn,
    /// the password has expired, or has to be changed before it can be used
    #[cfg_attr(not(windows), allow(dead_code))]
    PasswordExpired,
    /// the account is locked, after too many wrong passwords, or switched off
    #[cfg_attr(not(windows), allow(dead_code))]
    AccountLocked,
    /// this PC is connected to the server with another account already, and Windows allows one per server
    #[cfg_attr(not(windows), allow(dead_code))]
    OtherUser,
    /// a mapped drive that isn't connected
    Disconnected,
    NotFound,
    /// a drive without a disk, or one that's locked
    NotReady,
    /// a drive BitLocker keeps locked (Windows)
    #[cfg_attr(not(windows), allow(dead_code))]
    Locked,
    /// the person stopped waiting
    Cancelled,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderError {
    pub kind: FailKind,
    /// what the system said, for the small print
    pub detail: String,
}

impl FolderError {
    pub fn new(kind: FailKind, detail: impl Into<String>) -> FolderError {
        FolderError { kind, detail: detail.into() }
    }

    /// Is this worth trying again without being asked? It may well sort itself out.
    pub fn may_recover(&self) -> bool {
        matches!(self.kind, FailKind::Unreachable | FailKind::ShareMissing | FailKind::Disconnected | FailKind::NotReady | FailKind::Locked)
    }
}

/// The roots of the network drives this PC has, for telling network paths apart without asking the disk.
static NETWORK_ROOTS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// What the drive list says is on the network.
pub fn set_network_roots(roots: Vec<PathBuf>) {
    *NETWORK_ROOTS.lock().unwrap_or_else(PoisonError::into_inner) = roots;
}

/// The network drive `p` is on.
pub fn network_root_of(p: &Path) -> Option<PathBuf> {
    NETWORK_ROOTS.lock().unwrap_or_else(PoisonError::into_inner).iter().find(|r| is_under(p, r)).cloned()
}

/// Could a look at this path take as long as a server takes to not answer? Doesn't touch the disk.
pub fn is_network_path(p: &Path) -> bool {
    let s = p.to_string_lossy();
    (cfg!(windows) && s.starts_with(r"\\") && !s.starts_with(r"\\?\") && !s.starts_with(r"\\.\")) || network_root_of(p).is_some()
}

/// One directory entry as the search crawler and size counter see it.
pub struct RawEntry<'a> {
    pub name: Cow<'a, str>,
    pub attrs: u32,
    pub size: u64,
    pub mtime: u64,
}

/// Status of a file in a cloud sync folder (OneDrive / Dropbox / Google Drive / iCloud).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cloud {
    None,
    OnlineOnly,
    Local,
    AlwaysLocal,
}

pub fn cloud_state(attrs: u32, in_sync_root: bool) -> Cloud {
    if attrs & A_PINNED != 0 {
        Cloud::AlwaysLocal
    } else if attrs & (A_RECALL_ON_DATA | A_RECALL_ON_OPEN | A_OFFLINE) != 0 {
        Cloud::OnlineOnly
    } else if in_sync_root || attrs & A_UNPINNED != 0 {
        Cloud::Local
    } else {
        Cloud::None
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)] // not every system reports every kind
pub enum DriveKind {
    Fixed,
    Removable,
    Network,
    Optical,
    Ram,
    Unknown,
}

#[derive(Clone, Debug)]
pub struct Drive {
    /// Windows drive letter; on Linux/macOS a unique stand-in id for the mount
    pub letter: char,
    pub root: PathBuf,
    pub kind: DriveKind,
    pub label: String,
    /// network source (\\server\share, //server/share, user@host:/path…)
    pub remote: Option<String>,
    pub fs: String,
    pub total: u64,
    pub free: u64,
    pub ready: bool,
    /// the details came back, whatever they said. Until then `ready` means nothing.
    pub checked: bool,
    /// a remembered mapping that isn't connected right now (Windows)
    pub disconnected: bool,
    /// locked by BitLocker until someone unlocks it (Windows)
    pub locked: bool,
}

impl Drive {
    pub fn title(&self) -> String {
        let l = if !self.label.is_empty() {
            self.label.clone()
        } else if let Some(r) = &self.remote {
            r.trim_start_matches('\\').replace('\\', "/").trim_end_matches('/').rsplit('/').next().unwrap_or(tr("Network")).to_string()
        } else {
            // only ever shown, never compared: the sidebar and tabs key drives by their root
            match self.kind {
                DriveKind::Fixed => tr("Local Disk"),
                DriveKind::Removable => tr("USB Drive"),
                DriveKind::Network => tr("Network Drive"),
                DriveKind::Optical => tr("DVD Drive"),
                DriveKind::Ram => tr("RAM Disk"),
                DriveKind::Unknown => tr("Drive"),
            }
            .to_string()
        };
        let l = undash_name(&l);
        if cfg!(windows) {
            format!("{l} ({}:)", self.letter)
        } else {
            l
        }
    }

    /// FUSE mounts (sshfs, rclone, gvfs, ntfs-3g…) on Linux and macOS.
    pub fn is_fuse(&self) -> bool {
        let f = self.fs.to_ascii_lowercase();
        f.starts_with("fuse") || f.contains("macfuse") || f.contains("osxfuse")
    }
}

/// Opaque identity of a well-known folder (a Windows KNOWNFOLDERID on Windows).
#[cfg(windows)]
pub type KfId = windows::core::GUID;
#[cfg(not(windows))]
pub type KfId = ();

#[derive(Clone, Debug)]
pub struct Place {
    pub name: String,
    pub path: PathBuf,
    /// known-folder id, used to fetch the folder's proper icon (Windows)
    pub kf: KfId,
}

#[derive(Clone, Debug)]
pub struct CloudRoot {
    pub name: String,
    #[allow(dead_code)] // kept for per-provider icons/actions later
    pub provider: String,
    pub path: PathBuf,
}

const SEP: char = std::path::MAIN_SEPARATOR;

fn trimmed(p: &Path) -> String {
    let s = p.to_string_lossy();
    let t = s.trim_end_matches(SEP);
    if t.is_empty() { s.to_string() } else { t.to_string() }
}

/// Same file-system path (case-insensitive on Windows and macOS).
pub fn same_path(a: &Path, b: &Path) -> bool {
    let (a, b) = (trimmed(a), trimmed(b));
    if cfg!(any(windows, target_os = "macos")) {
        a.to_lowercase() == b.to_lowercase()
    } else {
        a == b
    }
}

/// `p` is `root` or somewhere inside it.
pub fn is_under(p: &Path, root: &Path) -> bool {
    let fold = |s: String| if cfg!(any(windows, target_os = "macos")) { s.to_lowercase() } else { s };
    let p = fold(p.to_string_lossy().to_string());
    let r = fold(trimmed(root));
    if r == SEP.to_string() {
        return p.starts_with(SEP);
    }
    p == r || (p.starts_with(&r) && p[r.len()..].starts_with(SEP))
}

#[cfg_attr(windows, allow(dead_code))]
pub fn unix_secs_to_filetime(secs: i64, nanos: u32) -> u64 {
    let s = secs + UNIX_EPOCH_AS_FILETIME_SECS as i64;
    if s < 0 {
        return 0;
    }
    s as u64 * 10_000_000 + nanos as u64 / 100
}

/// Dash characters the app never shows: hyphen-minus is fine, these are not.
const DASHES: [char; 9] = ['\u{2012}', '\u{2013}', '\u{2014}', '\u{2015}', '\u{2212}', '\u{2E3A}', '\u{2E3B}', '\u{FE58}', '\u{FE63}'];

/// A system name with an account part after a spaced hyphen, shown as "OneDrive (Personal)".
pub fn undash_name(s: &str) -> String {
    for sep in [" - ", " \u{2013} ", " \u{2014} "] {
        if let Some((a, b)) = s.split_once(sep) {
            return format!("{} ({})", undash_name(a), undash_name(b));
        }
    }
    s.replace(DASHES, "-")
}

/// Text from the system (menu items) shown without dashes: an aside dash becomes a colon.
pub fn undash_text(s: &str) -> String {
    let mut t = s.to_string();
    for sep in [" - ", " \u{2013} ", " \u{2014} "] {
        t = t.replace(sep, ": ");
    }
    t.replace(DASHES, "-")
}

/// Stops its background watcher thread when dropped.
pub struct Watcher {
    pub(crate) stop: Arc<AtomicBool>,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
