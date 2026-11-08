//! Notes Daleris leaves in a shared folder while it copies, moves or deletes there, so Daleris on other PCs can show
//! who is doing what to which item. Each window writes its own small hidden file, so two people never write the same
//! one, and removes it when it's done. A note that hasn't been renewed for a while belongs to a window that went away
//! and is ignored.

use std::path::Path;
use std::time::{Duration, SystemTime};

/// The start of every note's file name; the rest says which PC and window wrote it.
pub const PREFIX: &str = ".daleris-transit-";

/// A note older than this is from a window that stopped without cleaning up.
const STALE: Duration = Duration::from_secs(120);

pub fn is_mark(name: &str) -> bool {
    name.starts_with(PREFIX)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Way {
    Arriving,
    Leaving,
    Source,
}

/// What one note says about one item in the folder.
#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    /// the item's name in the folder
    pub name: String,
    pub way: Way,
    /// 'c' copy, 'm' move, 'd' delete
    pub kind: char,
    pub frac: Option<f32>,
    /// "person on PC"
    pub who: String,
    /// when it was written, in seconds since 1970
    pub at: u64,
    /// when the copy, move or delete started, likewise
    pub started: u64,
    /// this item's bytes done and in all, when it's the one being worked on
    pub done: Option<u64>,
    pub total: Option<u64>,
    /// seconds left for this item, as of `at`
    pub left: Option<u64>,
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Who's doing it, as other people see it: the person signed in to this PC, by the name the system knows them by.
pub fn who() -> String {
    let var = |names: &[&str]| names.iter().find_map(|n| std::env::var(n).ok()).filter(|v| !v.is_empty());
    let user = display_name().or_else(|| var(&["USERNAME", "USER"])).unwrap_or_else(|| "someone".into());
    match var(&["COMPUTERNAME", "HOSTNAME"]) {
        Some(host) => format!("{user} on {host}"),
        None => user,
    }
}

/// The signed-in person's full name: the directory's for a domain account, the PC's own for a local one.
#[cfg(windows)]
fn display_name() -> Option<String> {
    use windows::Win32::NetworkManagement::NetManagement::{NetApiBufferFree, NetUserGetInfo, USER_INFO_2};
    use windows::Win32::Security::Authentication::Identity::{GetUserNameExW, NameDisplay};
    let mut buf = [0u16; 256];
    let mut len = buf.len() as u32;
    if unsafe { GetUserNameExW(NameDisplay, Some(windows::core::PWSTR(buf.as_mut_ptr())), &mut len) } && len > 0 {
        return Some(String::from_utf16_lossy(&buf[..len as usize]));
    }
    let account = crate::util::wide(std::env::var_os("USERNAME")?);
    let mut info: *mut u8 = std::ptr::null_mut();
    if unsafe { NetUserGetInfo(windows::core::PCWSTR::null(), windows::core::PCWSTR(account.as_ptr()), 2, &mut info) } != 0 || info.is_null() {
        return None;
    }
    let name = unsafe { (*(info as *const USER_INFO_2)).usri2_full_name.to_string().ok() };
    unsafe { NetApiBufferFree(Some(info as *const _)) };
    name.filter(|n| !n.trim().is_empty())
}

/// The full name in the account's details, before any office or phone that follow it.
#[cfg(not(windows))]
fn display_name() -> Option<String> {
    let pw = unsafe { libc::getpwuid(libc::getuid()) };
    if pw.is_null() || unsafe { (*pw).pw_gecos }.is_null() {
        return None;
    }
    let gecos = unsafe { std::ffi::CStr::from_ptr((*pw).pw_gecos) }.to_string_lossy().into_owned();
    gecos.split(',').next().map(str::trim).filter(|n| !n.is_empty()).map(String::from)
}

/// This window's note for job number `job`.
pub fn file_name(job: u64) -> String {
    let host = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_default();
    let host: String = host.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    format!("{PREFIX}{host}-{}-{job}", std::process::id())
}

/// One line per item: version 2, way, kind, progress, written, started, bytes done, in all, seconds left, who, name.
/// What isn't known is left empty. Tabs can't be in names.
pub fn encode(marks: &[Mark]) -> String {
    let opt = |v: Option<u64>| v.map(|v| v.to_string()).unwrap_or_default();
    marks
        .iter()
        .map(|m| {
            let way = match m.way {
                Way::Arriving => 'a',
                Way::Leaving => 'l',
                Way::Source => 's',
            };
            let frac = m.frac.map(|f| format!("{f:.3}")).unwrap_or_default();
            format!("2\t{way}\t{}\t{frac}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n", m.kind, m.at, m.started, opt(m.done), opt(m.total), opt(m.left), m.who.replace('\t', " "), m.name)
        })
        .collect()
}

/// What a note says, leaving out what's too old to be true as of `now`. Reads the first version too, from older
/// Daleris, which said less.
pub fn parse(text: &str, now: u64) -> Vec<Mark> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            let (way, kind, frac, at, started, done, total, left, who, name) = match f.as_slice() {
                ["1", way, kind, frac, at, who, name @ ..] => (*way, *kind, *frac, *at, *at, "", "", "", *who, name.join("\t")),
                ["2", way, kind, frac, at, started, done, total, left, who, name @ ..] => (*way, *kind, *frac, *at, *started, *done, *total, *left, *who, name.join("\t")),
                _ => return None,
            };
            if name.is_empty() {
                return None;
            }
            let at: u64 = at.parse().ok()?;
            if now.saturating_sub(at) > STALE.as_secs() {
                return None;
            }
            let way = match way {
                "a" => Way::Arriving,
                "l" => Way::Leaving,
                "s" => Way::Source,
                _ => return None,
            };
            Some(Mark {
                name,
                way,
                kind: kind.chars().next()?,
                frac: frac.parse().ok(),
                who: who.to_string(),
                at,
                started: started.parse().unwrap_or(at),
                done: done.parse().ok(),
                total: total.parse().ok(),
                left: left.parse().ok(),
            })
        })
        .collect()
}

/// Writes the note for `dir`, hidden so it stays out of other people's way.
pub fn write(dir: &Path, file: &str, marks: &[Mark]) -> std::io::Result<()> {
    let path = dir.join(file);
    std::fs::write(&path, encode(marks))?;
    #[cfg(windows)]
    let _ = crate::fsx::set_attrs(&path, None, Some(true));
    Ok(())
}

pub fn remove(dir: &Path, file: &str) {
    let _ = std::fs::remove_file(dir.join(file));
}

/// Every note in `dir` that's still current, by item name in lower case. Several people on one item: the newest wins.
pub fn read_all(dir: &Path, names: &[String]) -> std::collections::HashMap<String, Mark> {
    let now = now();
    let mut out = std::collections::HashMap::new();
    for n in names.iter().filter(|n| is_mark(n)) {
        let Ok(text) = std::fs::read_to_string(dir.join(n)) else { continue };
        for m in parse(&text, now) {
            let key = m.name.to_lowercase();
            if out.get(&key).is_none_or(|o: &Mark| o.at <= m.at) {
                out.insert(key, m);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_reads_back_and_goes_stale() {
        let m = Mark { name: "Q3 report.xlsx".into(), way: Way::Arriving, kind: 'c', frac: Some(0.4), who: "Ana on PC-7".into(), at: 1_000, started: 990, done: Some(400), total: Some(1_000), left: Some(15) };
        let source = Mark { name: "Photos".into(), way: Way::Source, frac: None, done: None, total: None, left: None, ..m.clone() };
        let text = encode(&[m.clone(), source.clone()]);
        assert_eq!(parse(&text, 1_030), vec![m, source]);
        assert!(parse(&text, 1_000 + STALE.as_secs() + 1).is_empty(), "a window that went away leaves nothing behind");
        assert!(parse("3\ta\tc\t\t1000\tx\ty\n", 1_000).is_empty(), "a newer kind of note is left alone");
        assert!(is_mark(&file_name(3)));
    }

    #[test]
    fn notes_from_the_first_version_still_read() {
        let old = parse("1\tl\tm\t0.500\t1000\tAna on PC-7\tOld plans\n", 1_010);
        assert_eq!(old.len(), 1);
        assert_eq!((old[0].name.as_str(), old[0].way, old[0].started, old[0].left), ("Old plans", Way::Leaving, 1_000, None));
    }

    #[test]
    #[cfg(windows)]
    fn who_is_the_signed_in_person() {
        let w = who();
        assert!(!w.is_empty() && w != "someone", "{w}");
    }
}
