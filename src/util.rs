use std::cmp::Ordering;
#[cfg(windows)]
use std::ffi::OsStr;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;

/// A folder from the environment, ignored when empty or relative as the XDG rules ask.
#[cfg(not(windows))]
fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute())
}

/// Where the settings file and icon packs live:
/// `%APPDATA%\Daleris`, `~/Library/Application Support/Daleris` or `~/.config/daleris`.
pub fn config_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    return std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("Daleris"));
    #[cfg(target_os = "macos")]
    return env_dir("HOME").map(|h| h.join("Library/Application Support/Daleris"));
    #[cfg(not(any(windows, target_os = "macos")))]
    return env_dir("XDG_CONFIG_HOME").or_else(|| env_dir("HOME").map(|h| h.join(".config"))).map(|d| d.join("daleris"));
}

/// Is this the top of the drive the system runs from?
pub fn is_system_root(path: &std::path::Path) -> bool {
    #[cfg(windows)]
    {
        let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        path.to_str().is_some_and(|p| p.trim_end_matches('\\').eq_ignore_ascii_case(&drive))
    }
    #[cfg(not(windows))]
    {
        path == std::path::Path::new("/")
    }
}

/// Brings along the settings from when the app was called Bolt Explorer. Only Windows saved any then.
pub fn adopt_old_settings() {
    #[cfg(windows)]
    if let Some(new) = config_dir().filter(|d| !d.exists()) {
        if let Some(old) = new.parent().map(|p| p.join("BoltExplorer")).filter(|d| d.is_dir()) {
            let _ = std::fs::rename(old, new);
        }
    }
}

/// Where the log goes:
/// `%LOCALAPPDATA%\Daleris\logs`, `~/Library/Logs/Daleris` or `~/.local/state/daleris`.
pub fn log_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    return std::env::var_os("LOCALAPPDATA").map(|a| PathBuf::from(a).join("Daleris").join("logs"));
    #[cfg(target_os = "macos")]
    return env_dir("HOME").map(|h| h.join("Library/Logs/Daleris"));
    #[cfg(not(any(windows, target_os = "macos")))]
    return env_dir("XDG_STATE_HOME").or_else(|| env_dir("HOME").map(|h| h.join(".local/state"))).map(|d| d.join("daleris"));
}

/// Null-terminated UTF-16 for Win32 calls.
#[cfg(windows)]
pub fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// "file10" sorts after "file9", like Explorer.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        let (ca, cb) = (a[i], b[j]);
        if ca.is_ascii_digit() && cb.is_ascii_digit() {
            let si = i;
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            let sj = j;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let na = trim_zeros(&a[si..i]);
            let nb = trim_zeros(&b[sj..j]);
            let ord = na.len().cmp(&nb.len()).then_with(|| na.cmp(nb));
            if ord != Ordering::Equal {
                return ord;
            }
        } else {
            if ca != cb {
                return ca.cmp(&cb);
            }
            i += 1;
            j += 1;
        }
    }
    (a.len() - i).cmp(&(b.len() - j))
}

fn trim_zeros(s: &[u8]) -> &[u8] {
    let n = s.iter().take_while(|&&c| c == b'0').count();
    &s[n.min(s.len().saturating_sub(1))..]
}

pub fn fmt_size(n: u64) -> String {
    const U: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];
    if n < 1024 {
        return format!("{n} B");
    }
    let mut v = n as f64 / 1024.0;
    let mut u = 0;
    while v >= 1024.0 && u < U.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if v >= 100.0 {
        format!("{:.0} {}", v, U[u])
    } else {
        format!("{:.1} {}", v, U[u])
    }
}

pub fn fmt_count(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn fmt_duration(d: std::time::Duration) -> String {
    let ms = d.as_secs_f64() * 1000.0;
    if ms < 10.0 {
        format!("{ms:.2} ms")
    } else if ms < 1000.0 {
        format!("{ms:.0} ms")
    } else {
        format!("{:.1} s", ms / 1000.0)
    }
}

/// FILETIME (100ns ticks since 1601) to local "2026-11-02 09:12".
pub fn fmt_filetime(ft: u64) -> String {
    if ft == 0 {
        return String::new();
    }
    let st = filetime_local(ft);
    format!("{:04}-{:02}-{:02} {:02}:{:02}", st.year, st.month, st.day, st.hour, st.minute)
}

/// A calendar date and time in the local time zone.
pub struct LocalTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
}

#[cfg(windows)]
pub fn filetime_local(ft: u64) -> LocalTime {
    use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows::Win32::Storage::FileSystem::FileTimeToLocalFileTime;
    use windows::Win32::System::Time::FileTimeToSystemTime;
    let f = FILETIME { dwLowDateTime: ft as u32, dwHighDateTime: (ft >> 32) as u32 };
    let mut local = FILETIME::default();
    let mut st = SYSTEMTIME::default();
    unsafe {
        let _ = FileTimeToLocalFileTime(&f, &mut local);
        let _ = FileTimeToSystemTime(&local, &mut st);
    }
    LocalTime { year: st.wYear as i32, month: st.wMonth as u32, day: st.wDay as u32, hour: st.wHour as u32, minute: st.wMinute as u32 }
}

#[cfg(not(windows))]
pub fn filetime_local(ft: u64) -> LocalTime {
    let secs = (ft / 10_000_000) as i64 - crate::fscommon::UNIX_EPOCH_AS_FILETIME_SECS as i64;
    let t: libc::time_t = secs as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::localtime_r(&t, &mut tm);
    }
    LocalTime { year: tm.tm_year + 1900, month: (tm.tm_mon + 1) as u32, day: tm.tm_mday as u32, hour: tm.tm_hour as u32, minute: tm.tm_min as u32 }
}

pub fn now_filetime() -> u64 {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    (d.as_secs() + 11_644_473_600) * 10_000_000 + d.subsec_nanos() as u64 / 100
}

pub const FT_DAY: u64 = 24 * 3600 * 10_000_000;

/// FILETIME for a local calendar date (approximate: UTC midnight).
pub fn filetime_from_ymd(y: i64, m: i64, d: i64) -> Option<u64> {
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || y < 1601 {
        return None;
    }
    // days from civil (Howard Hinnant)
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days_unix = era * 146097 + doe - 719468;
    let secs = days_unix * 86400 + 11_644_473_600;
    if secs < 0 {
        return None;
    }
    Some(secs as u64 * 10_000_000)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Folder,
    Image,
    Video,
    Audio,
    Document,
    Code,
    Archive,
    App,
    Other,
}

pub fn kind_of(ext: &str, is_dir: bool) -> Kind {
    if is_dir {
        return Kind::Folder;
    }
    match ext {
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "ico" | "tif" | "tiff" | "heic" | "svg" | "avif" | "raw" | "psd" => Kind::Image,
        "mp4" | "mkv" | "mov" | "avi" | "wmv" | "webm" | "m4v" | "flv" | "mpg" | "mpeg" => Kind::Video,
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "wma" | "opus" | "mid" => Kind::Audio,
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "odt" | "ods" | "rtf" | "txt" | "md" | "csv" | "epub" => Kind::Document,
        "rs" | "c" | "h" | "cpp" | "hpp" | "cs" | "js" | "ts" | "tsx" | "jsx" | "py" | "go" | "java" | "kt" | "lua" | "luau" | "json" | "toml"
        | "yaml" | "yml" | "xml" | "html" | "htm" | "css" | "scss" | "sh" | "ps1" | "bat" | "cmd" | "sql" | "ini" | "cfg" | "log" | "swift"
        | "rb" | "php" => Kind::Code,
        "zip" | "7z" | "rar" | "tar" | "gz" | "bz2" | "xz" | "zst" | "cab" | "iso" => Kind::Archive,
        "exe" | "msi" | "dll" | "lnk" | "appx" | "msix" | "com" | "scr" => Kind::App,
        _ => Kind::Other,
    }
}

pub fn kind_from_word(w: &str) -> Option<Kind> {
    Some(match w {
        "folder" | "dir" | "folders" => Kind::Folder,
        "image" | "picture" | "photo" | "pic" | "images" => Kind::Image,
        "video" | "movie" | "videos" => Kind::Video,
        "audio" | "music" | "sound" => Kind::Audio,
        "doc" | "document" | "docs" => Kind::Document,
        "code" | "source" => Kind::Code,
        "archive" | "zip" | "compressed" => Kind::Archive,
        "app" | "program" | "exe" => Kind::App,
        _ => return None,
    })
}

/// Text-ish files we can preview by reading their bytes.
pub fn is_text_ext(ext: &str) -> bool {
    matches!(kind_of(ext, false), Kind::Code)
        || matches!(
            ext,
            "txt" | "md" | "csv" | "tsv" | "gitignore" | "env" | "lock" | "conf" | "properties" | "nfo" | "srt" | "vtt" | "svg" | "rst" | "tex"
        )
}

/// Simple `*` / `?` wildcard match on already-lowercased input.
pub fn glob_match(pat: &[u8], s: &[u8]) -> bool {
    let (mut p, mut i) = (0usize, 0usize);
    let (mut star, mut mark) = (usize::MAX, 0usize);
    while i < s.len() {
        if p < pat.len() && (pat[p] == b'?' || pat[p] == s[i]) {
            p += 1;
            i += 1;
        } else if p < pat.len() && pat[p] == b'*' {
            star = p;
            p += 1;
            mark = i;
        } else if star != usize::MAX {
            p = star + 1;
            mark += 1;
            i = mark;
        } else {
            return false;
        }
    }
    while p < pat.len() && pat[p] == b'*' {
        p += 1;
    }
    p == pat.len()
}

pub fn ext_of(name: &str, is_dir: bool) -> String {
    if is_dir {
        return String::new();
    }
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => name[i + 1..].to_ascii_lowercase(),
        _ => String::new(),
    }
}
