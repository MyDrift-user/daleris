//! Who has a file open: apps on this PC, by asking Windows's Restart Manager, the lock files Office and LibreOffice put
//! next to a document they have open, which name the person, and what other people's Daleris says in its notes.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use crate::i18n::{tr, tx};

/// A small square picture of a person, as RGBA pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub size: usize,
    pub rgba: Vec<u8>,
}

/// Someone with a file open.
#[derive(Clone, Debug, PartialEq)]
pub struct Opener {
    /// "You" in the language shown, or the person, by the name their PC knows them by
    pub who: String,
    /// the program, when known
    pub app: Option<String>,
    /// since when, in seconds since 1970, when known
    pub since: Option<u64>,
    pub face: Option<Arc<Face>>,
    pub me: bool,
    /// where it's known from, for the tooltip's "known from {source}": "this PC", "their Daleris", "its lock file",
    /// marked with `tx` and shown with `tr`
    pub from: &'static str,
}

// ---------------------------------------------------------------- lock files

/// Office's owner file (`~$name.docx`) or LibreOffice's (`.~lock.name.odt#`).
pub fn is_lock(name: &str) -> bool {
    name.starts_with("~$") || (name.starts_with(".~lock.") && name.ends_with('#'))
}

/// The document a lock file stands for, among `names`. Word drops the first two letters of a longer name to make room for
/// its `~$`, so a name that ends with the rest counts too, when it's the only one.
pub fn lock_target<'a>(lock: &str, names: &[&'a str]) -> Option<&'a str> {
    if let Some(rest) = lock.strip_prefix(".~lock.").and_then(|r| r.strip_suffix('#')) {
        return names.iter().find(|n| n.eq_ignore_ascii_case(rest)).copied();
    }
    let rest = lock.strip_prefix("~$")?;
    if let Some(n) = names.iter().find(|n| n.eq_ignore_ascii_case(rest)) {
        return Some(n);
    }
    let lower = rest.to_lowercase();
    let mut fits = names.iter().filter(|n| !is_lock(n) && n.chars().count() > rest.chars().count() && n.to_lowercase().ends_with(&lower));
    match (fits.next(), fits.next()) {
        (Some(one), None) => Some(one),
        _ => None,
    }
}

/// The person a lock file names. Office writes a length and the name at the start; LibreOffice writes
/// `name,host,home,date,...`.
pub fn lock_owner(lock: &str, bytes: &[u8]) -> Option<String> {
    let name = if lock.starts_with(".~lock.") {
        String::from_utf8_lossy(bytes).split(',').next()?.trim().to_string()
    } else {
        let n = *bytes.first()? as usize;
        // the name in the code page Office used, which for most names is Latin-1
        bytes.get(1..1 + n)?.iter().map(|&b| b as char).collect::<String>().trim().to_string()
    };
    (!name.is_empty()).then_some(name)
}

// ---------------------------------------------------------------- this PC

/// The programs on this PC that have the file open, by their name, leaving out services, Explorer and Daleris itself.
#[cfg(windows)]
pub fn local_apps(path: &Path) -> Vec<String> {
    use windows::Win32::System::RestartManager::*;
    use windows::core::{PCWSTR, PWSTR};
    let mut session = 0u32;
    let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
    unsafe {
        if RmStartSession(&mut session, None, PWSTR(key.as_mut_ptr())).0 != 0 {
            return Vec::new();
        }
        let w = crate::util::wide(path.as_os_str());
        let mut out = Vec::new();
        if RmRegisterResources(session, Some(&[PCWSTR(w.as_ptr())]), None, None).0 == 0 {
            let mut list = vec![RM_PROCESS_INFO::default(); 16];
            let (mut needed, mut count, mut reasons) = (0u32, list.len() as u32, 0u32);
            if RmGetList(session, &mut needed, &mut count, Some(list.as_mut_ptr()), &mut reasons).0 == 0 {
                let me = std::process::id();
                for p in &list[..count as usize] {
                    let shown = [RmMainWindow, RmOtherWindow, RmConsole].contains(&p.ApplicationType);
                    if shown && p.Process.dwProcessId != me {
                        let name = crate::util::from_wide(&p.strAppName);
                        if !name.is_empty() && !out.contains(&name) {
                            out.push(name);
                        }
                    }
                }
            }
        }
        let _ = RmEndSession(session);
        out
    }
}

/// Not known here.
#[cfg(not(windows))]
pub fn local_apps(_path: &Path) -> Vec<String> {
    Vec::new()
}

/// This person's own picture: their Windows account picture, or `~/.face` on Linux. Read once.
pub fn my_face() -> Option<Arc<Face>> {
    static FACE: OnceLock<Option<Arc<Face>>> = OnceLock::new();
    FACE.get_or_init(|| {
        let file = account_picture()?;
        let img = crate::shell::picture(&file, 32)?;
        (img.w > 0 && img.h > 0).then(|| Arc::new(square(&img.rgba, img.w, img.h, 32)))
    })
    .clone()
}

/// Fits a picture to a square of `size`, by its middle.
fn square(rgba: &[u8], w: usize, h: usize, size: usize) -> Face {
    let side = w.min(h);
    let (x0, y0) = ((w - side) / 2, (h - side) / 2);
    let mut out = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let (sx, sy) = (x0 + x * side / size, y0 + y * side / size);
            let i = (sy * w + sx) * 4;
            out[(y * size + x) * 4..][..4].copy_from_slice(&rgba[i..i + 4]);
        }
    }
    Face { size, rgba: out }
}

#[cfg(windows)]
fn account_picture() -> Option<std::path::PathBuf> {
    use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    let sid = unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;
        let mut buf = vec![0u8; 256];
        let mut len = 0u32;
        let got = GetTokenInformation(token, TokenUser, Some(buf.as_mut_ptr() as *mut _), buf.len() as u32, &mut len).is_ok();
        let _ = CloseHandle(token);
        if !got {
            return None;
        }
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut text = windows::core::PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut text).ok()?;
        let s = text.to_string().ok();
        let _ = LocalFree(Some(HLOCAL(text.0 as _)));
        s?
    };
    let key = format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\AccountPicture\Users\{sid}");
    ["Image96", "Image64", "Image192", "Image240", "Image448"].iter().find_map(|v| { // i18n-ignore: registry value names
        let p = std::path::PathBuf::from(crate::fsx::reg_text(windows::Win32::System::Registry::HKEY_LOCAL_MACHINE, &key, v)?);
        p.is_file().then_some(p)
    })
}

#[cfg(not(windows))]
fn account_picture() -> Option<std::path::PathBuf> {
    let home = std::path::PathBuf::from(std::env::var_os("HOME")?);
    [".face", ".face.icon"].iter().map(|f| home.join(f)).find(|p| p.is_file())
}

// ---------------------------------------------------------------- pictures in notes

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn b64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            out.push(if k <= c.len() { B64[(n >> (18 - 6 * k) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

pub fn b64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut n, mut bits) = (0u32, 0);
    for c in text.bytes().filter(|c| *c != b'=') {
        n = n << 6 | B64.iter().position(|b| *b == c)? as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((n >> bits) as u8);
        }
    }
    Some(out)
}

/// A face as one line of a note: its size and pixels.
pub fn face_text(f: &Face) -> String {
    format!("{}\t{}", f.size, b64_encode(&f.rgba))
}

pub fn face_from(size: &str, data: &str) -> Option<Face> {
    let size: usize = size.parse().ok().filter(|s| *s > 0 && *s <= 64)?;
    let rgba = b64_decode(data)?;
    (rgba.len() == size * size * 4).then_some(Face { size, rgba })
}

/// Up to two letters for someone without a picture: "Ana Müller on PC-7" is "AM".
pub fn initials(who: &str) -> String {
    let name = who.split(" on ").next().unwrap_or(who);
    let mut letters: Vec<char> = name.split_whitespace().filter_map(|w| w.chars().next()).collect();
    if letters.len() > 2 {
        letters = vec![letters[0], letters[letters.len() - 1]];
    }
    letters.into_iter().flat_map(char::to_uppercase).collect()
}

// ---------------------------------------------------------------- one look at a folder

/// Who has what open in `dir`, by item name in lower case: this PC's apps, and the people lock files name. `items` is the
/// folder's listing, as (name, is a folder). With `share_note`, this PC's open files go into this window's note in the
/// folder too, for other people's Daleris, when it's on a network share. Can take as long as a server takes to answer.
pub fn scan(dir: &Path, items: &[(String, bool)], share_note: Option<&str>) -> std::collections::HashMap<String, Vec<Opener>> {
    use std::collections::HashMap;
    let mut out: HashMap<String, Vec<Opener>> = HashMap::new();
    // nothing on a phone or camera is open in a way anyone can tell
    if crate::fscommon::is_shell(dir) {
        return out;
    }
    let names: Vec<&str> = items.iter().map(|(n, _)| n.as_str()).collect();
    let me = crate::marks::who();
    let my_names: Vec<String> = [Some(me.split(" on ").next().unwrap_or(&me).to_string()), std::env::var("USERNAME").ok(), std::env::var("USER").ok()].into_iter().flatten().map(|n| n.to_lowercase()).collect();
    // this PC first: what's open here is open by you
    let mut mine: Vec<(String, Vec<String>)> = Vec::new();
    for (name, is_dir) in items.iter().filter(|(n, d)| !d && !is_lock(n) && !crate::marks::is_mark(n)).take(400) {
        let _ = is_dir;
        let apps = local_apps(&dir.join(name));
        if !apps.is_empty() {
            mine.push((name.clone(), apps));
        }
    }
    let first_seen = remember_first_seen(dir, &mine);
    for (name, apps) in &mine {
        out.entry(name.to_lowercase()).or_default().push(Opener {
            who: tr("You").into(),
            app: Some(apps.join(", ")),
            since: first_seen.get(name).copied(),
            face: my_face(),
            me: true,
            from: tx("this PC"),
        });
    }
    // the people Office and LibreOffice name, unless it's you, who's already there
    for lock in names.iter().filter(|n| is_lock(n)) {
        let Some(target) = lock_target(lock, &names) else { continue };
        let Ok(bytes) = read_head(&dir.join(lock)) else { continue };
        let Some(owner) = lock_owner(lock, &bytes) else { continue };
        let key = target.to_lowercase();
        let you = my_names.contains(&owner.to_lowercase());
        if you && out.get(&key).is_some_and(|v| v.iter().any(|o| o.me)) {
            continue;
        }
        let since = std::fs::metadata(dir.join(lock)).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs());
        let app = if lock.starts_with(".~lock.") { "LibreOffice" } else { "Office" }; // i18n-ignore: program names
        out.entry(key).or_default().push(Opener {
            who: if you { tr("You").into() } else { owner },
            app: Some(app.into()),
            since,
            face: if you { my_face() } else { None },
            me: you,
            from: tx("its lock file"),
        });
    }
    if let Some(file) = share_note.filter(|_| crate::fsx::is_network_path(dir)) {
        share_open(dir, file, &me, &mine, &first_seen);
    }
    out
}

/// The first bytes of a lock file, which hold the name.
fn read_head(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut buf = vec![0u8; 512];
    let n = std::fs::File::open(path)?.read(&mut buf)?;
    buf.truncate(n);
    Ok(buf)
}

/// When each of this PC's open files in `dir` was first seen open, as long as it stays open.
fn remember_first_seen(dir: &Path, mine: &[(String, Vec<String>)]) -> std::collections::HashMap<String, u64> {
    use std::collections::HashMap;
    use std::sync::Mutex;
    static SEEN: Mutex<Option<HashMap<std::path::PathBuf, HashMap<String, u64>>>> = Mutex::new(None);
    let mut all = SEEN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let here = all.get_or_insert_with(HashMap::new).entry(dir.to_path_buf()).or_default();
    let now = crate::marks::now();
    here.retain(|n, _| mine.iter().any(|(m, _)| m == n));
    for (n, _) in mine {
        here.entry(n.clone()).or_insert(now);
    }
    here.clone()
}

/// Keeps this window's "open" note in a shared folder in line with what's open here: written when that changes, and
/// renewed every minute so it doesn't go stale; removed when nothing is open any more.
fn share_open(dir: &Path, file: &str, me: &str, mine: &[(String, Vec<String>)], since: &std::collections::HashMap<String, u64>) {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static WRITTEN: Mutex<Option<HashMap<std::path::PathBuf, (String, Instant)>>> = Mutex::new(None);
    let mut all = WRITTEN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let written = all.get_or_insert_with(HashMap::new);
    let what: String = mine.iter().map(|(n, a)| format!("{n}:{}", a.join(","))).collect::<Vec<_>>().join("|");
    if mine.is_empty() {
        if written.remove(dir).is_some() {
            crate::marks::remove(dir, file);
        }
        return;
    }
    if written.get(dir).is_some_and(|(w, at)| *w == what && at.elapsed() < Duration::from_secs(60)) {
        return;
    }
    let at = crate::marks::now();
    let marks: Vec<crate::marks::Mark> = mine
        .iter()
        .map(|(n, apps)| crate::marks::Mark {
            name: n.clone(),
            way: crate::marks::Way::Open,
            kind: 'o',
            frac: None,
            who: me.to_string(),
            at,
            started: since.get(n).copied().unwrap_or(at),
            done: None,
            total: None,
            left: None,
            app: Some(apps.join(", ")),
        })
        .collect();
    let face = my_face();
    if crate::marks::write(dir, file, &marks, face.as_deref().map(|f| (me, f))).is_ok() {
        written.insert(dir.to_path_buf(), (what, Instant::now()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_files_name_their_document_and_owner() {
        let names = ["Quarterly report.docx", "Budget.xlsx", "notes.odt", "~$arterly report.docx", "~$Budget.xlsx"];
        assert_eq!(lock_target("~$Budget.xlsx", &names), Some("Budget.xlsx"));
        assert_eq!(lock_target("~$arterly report.docx", &names), Some("Quarterly report.docx"), "Word drops two letters");
        assert_eq!(lock_target(".~lock.notes.odt#", &names), Some("notes.odt"));
        assert_eq!(lock_target("~$nothing.docx", &names), None);
        let mut office = vec![11u8];
        office.extend(b"Ana Mueller");
        office.extend([0u8; 40]);
        assert_eq!(lock_owner("~$Budget.xlsx", &office).as_deref(), Some("Ana Mueller"));
        assert_eq!(lock_owner(".~lock.notes.odt#", b"Ana Mueller,PC-7,/home/ana,08.11.2026 17:00,file:///x;").as_deref(), Some("Ana Mueller"));
    }

    #[test]
    fn faces_survive_a_note() {
        let f = Face { size: 2, rgba: (0..16).collect() };
        let text = face_text(&f);
        let (size, data) = text.split_once('\t').unwrap();
        assert_eq!(face_from(size, data), Some(f));
        assert_eq!(b64_encode(b"Ma"), "TWE=");
        assert_eq!(initials("Ana Müller on PC-7"), "AM");
        assert_eq!(initials("mydrift on DESKTOP"), "M");
    }
}
