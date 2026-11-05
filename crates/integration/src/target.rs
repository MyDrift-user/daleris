//! Reads what Explorer's own command line, or a shell verb, asks to open. Tolerant on purpose: `"%1"` turns `C:\` into
//! `C:"`, and apps pass `/select,` and `/e,` in every combination.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::channel;
use std::time::Duration;

/// This PC's shell name. Daleris has its own page for it.
const THIS_PC: &str = "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Nothing named: a plain new window.
    Nothing,
    Dir(PathBuf),
    /// Open the parent folder with this item selected.
    Select(PathBuf),
    ThisPc,
    /// A `shell:` or `::{…}` name. Might be a folder on disk, which only Windows can tell (see `explorer::resolve`).
    Shell(String),
    /// A path that isn't there. Explorer gets it, as it knows how to say so.
    Missing(OsString),
}

/// What's at a path, as far as could be found out in time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathKind {
    Dir,
    File,
    Missing,
    /// no answer in time, or no permission to look. Could be a folder on a server that isn't there.
    Unknown,
}

/// How many looks may be stuck at once. Past that nobody waits for another.
const MAX_STUCK: usize = 16;
static LOOKING: AtomicUsize = AtomicUsize::new(0);

/// A look at `path` that gives up after `wait`. A dead server can hold up a look for 20 seconds or more, and whoever
/// is waiting for this is usually someone's window. The look itself goes on until the system gives it back.
pub fn kind_within(path: &Path, wait: Duration) -> PathKind {
    if LOOKING.fetch_add(1, Ordering::AcqRel) >= MAX_STUCK {
        LOOKING.fetch_sub(1, Ordering::AcqRel);
        return PathKind::Unknown;
    }
    let (tx, rx) = channel();
    let path = path.to_path_buf();
    let started = std::thread::Builder::new().name("look".into()).spawn(move || {
        let _ = tx.send(look(&path));
        LOOKING.fetch_sub(1, Ordering::AcqRel);
    });
    if started.is_err() {
        LOOKING.fetch_sub(1, Ordering::AcqRel);
        return PathKind::Unknown;
    }
    rx.recv_timeout(wait).unwrap_or(PathKind::Unknown)
}

fn look(path: &Path) -> PathKind {
    match std::fs::metadata(path) {
        Ok(m) if m.is_dir() => PathKind::Dir,
        Ok(_) => PathKind::File,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => PathKind::Missing,
        Err(_) => PathKind::Unknown,
    }
}

/// The target of a command line, given how to check the disk.
pub fn parse(args: &[OsString], is_dir: impl Fn(&Path) -> bool, exists: impl Fn(&Path) -> bool) -> Target {
    let mut select = false;
    let mut named: Option<OsString> = None;
    for arg in args {
        // Text that isn't valid Unicode can't be a switch, so it's a path. Windows allows those.
        let Some(text) = arg.to_str() else {
            named.get_or_insert_with(|| arg.clone());
            continue;
        };
        let mut rest = text.trim();
        while rest.starts_with('/') || rest.starts_with(',') {
            rest = rest.trim_start_matches(',').trim_start();
            let lower = rest.to_ascii_lowercase();
            if lower.starts_with("/select") {
                select = true;
                rest = &rest["/select".len()..];
            } else if lower.starts_with("/root") {
                rest = &rest["/root".len()..];
            } else if lower.starts_with("/separate") {
                rest = &rest["/separate".len()..];
            } else if lower.starts_with("/e") || lower.starts_with("/n") {
                rest = &rest[2..];
            } else {
                break;
            }
            // `/select,path` leaves `,path`; nothing else may follow a switch's name
            if !(rest.is_empty() || rest.starts_with(',') || rest.starts_with(' ')) {
                rest = "";
            }
            rest = rest.trim_start();
        }
        if !rest.is_empty() && named.is_none() {
            named = Some(OsString::from(rest));
        }
    }
    let Some(named) = named else { return Target::Nothing };
    let cleaned = clean(&named);
    if let Some(text) = cleaned.to_str() {
        if let Some(t) = shell_name(text) {
            return t;
        }
    }
    let path = PathBuf::from(&cleaned);
    if !exists(&path) {
        return Target::Missing(named);
    }
    // Selecting the top of a drive has no parent to open, so that's just the drive.
    if (select || !is_dir(&path)) && path.parent().is_some() {
        Target::Select(path)
    } else {
        Target::Dir(path)
    }
}

fn shell_name(text: &str) -> Option<Target> {
    let lower = text.to_ascii_lowercase();
    if !(lower.starts_with("::{") || lower.starts_with("shell:")) {
        return None;
    }
    let bare = if lower.starts_with("shell:") { &text["shell:".len()..] } else { text };
    if bare.eq_ignore_ascii_case(THIS_PC) || bare.eq_ignore_ascii_case("MyComputerFolder") {
        return Some(Target::ThisPc);
    }
    Some(Target::Shell(text.to_string()))
}

/// Undoes what quoting and URLs do to a path.
fn clean(arg: &OsStr) -> OsString {
    let Some(text) = arg.to_str() else { return arg.to_os_string() };
    let mut t = text.trim().trim_matches('"').trim().to_string();
    if t.len() >= 5 && t.is_char_boundary(5) && t[..5].eq_ignore_ascii_case("file:") {
        t = from_file_url(&t);
    }
    // `C:` alone means whatever folder the program is in on that drive. Nobody clicking a drive means that.
    let b = t.as_bytes();
    if b.len() == 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        t.push('\\');
    }
    OsString::from(t)
}

/// `file:///C:/a%20b` to `C:\a b`, and `file://server/share` to `\\server\share`.
fn from_file_url(url: &str) -> String {
    let rest = &url["file:".len()..];
    let path = if let Some(p) = rest.strip_prefix("///") {
        p.to_string()
    } else if let Some(p) = rest.strip_prefix("//") {
        if p.to_ascii_lowercase().starts_with("localhost/") {
            p["localhost/".len()..].to_string()
        } else {
            format!("//{p}")
        }
    } else {
        rest.to_string()
    };
    percent_decode(&path).replace('/', "\\")
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> Target {
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        // what's on this pretend disk: the folders C:\ and C:\a b, and the file C:\a b\c,d.txt
        let dirs = [r"C:\", r"C:\a b"];
        let file = r"C:\a b\c,d.txt";
        parse(&args, |p| dirs.iter().any(|d| p == Path::new(d)), |p| p == Path::new(file) || dirs.iter().any(|d| p == Path::new(d)))
    }

    #[test]
    fn no_arguments_or_only_switches_name_nothing() {
        assert_eq!(run(&[]), Target::Nothing);
        assert_eq!(run(&["/n,"]), Target::Nothing);
        assert_eq!(run(&["/e,", "/n"]), Target::Nothing);
    }

    #[cfg(windows)]
    #[test]
    fn a_drive_survives_the_quote_that_percent_one_leaves_behind() {
        for arg in [r#"C:""#, r"C:\", "C:", r#""C:\""#] {
            assert_eq!(run(&[arg]), Target::Dir(PathBuf::from(r"C:\")), "{arg}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_file_opens_its_folder_and_selects_it() {
        assert_eq!(run(&[r"C:\a b\c,d.txt"]), Target::Select(PathBuf::from(r"C:\a b\c,d.txt")));
    }

    #[cfg(windows)]
    #[test]
    fn explorers_switches_are_read_in_any_combination() {
        let want = Target::Select(PathBuf::from(r"C:\a b\c,d.txt"));
        assert_eq!(run(&[r"/select,C:\a b\c,d.txt"]), want);
        assert_eq!(run(&["/select,", r"C:\a b\c,d.txt"]), want);
        assert_eq!(run(&["/select", r"C:\a b\c,d.txt"]), want);
        assert_eq!(run(&[r"/e,/select,C:\a b\c,d.txt"]), want);
        assert_eq!(run(&[r#"/select,"C:\a b\c,d.txt""#]), want);
        assert_eq!(run(&[r"/root,C:\a b"]), Target::Dir(PathBuf::from(r"C:\a b")));
        assert_eq!(run(&[r"/select,C:\a b"]), Target::Select(PathBuf::from(r"C:\a b")));
        assert_eq!(run(&[r"/select,C:\"]), Target::Dir(PathBuf::from(r"C:\")));
    }

    #[cfg(windows)]
    #[test]
    fn urls_become_paths() {
        assert_eq!(run(&["file:///C:/a%20b"]), Target::Dir(PathBuf::from(r"C:\a b")));
        assert_eq!(run(&["FILE:///C:/a%20b/c,d.txt"]), Target::Select(PathBuf::from(r"C:\a b\c,d.txt")));
        assert_eq!(from_file_url("file://server/share/x"), r"\\server\share\x");
        assert_eq!(from_file_url("file://localhost/C:/x"), r"C:\x");
    }

    #[test]
    fn shell_names_are_kept_for_windows_to_resolve() {
        assert_eq!(run(&["::{20D04FE0-3AEA-1069-A2D8-08002B30309D}"]), Target::ThisPc);
        assert_eq!(run(&["shell:::{20d04fe0-3aea-1069-a2d8-08002b30309d}"]), Target::ThisPc);
        assert_eq!(run(&["shell:MyComputerFolder"]), Target::ThisPc);
        assert_eq!(run(&["Shell:mycomputerfolder"]), Target::ThisPc);
        assert_eq!(run(&["shell:Downloads"]), Target::Shell("shell:Downloads".into()));
        assert_eq!(run(&["::{645FF040-5081-101B-9F08-00AA002F954E}"]), Target::Shell("::{645FF040-5081-101B-9F08-00AA002F954E}".into()));
    }

    #[test]
    fn what_isnt_there_is_left_for_explorer() {
        assert_eq!(run(&[r"C:\nope"]), Target::Missing(OsString::from(r"C:\nope")));
    }

    #[test]
    fn a_stray_percent_doesnt_break_decoding() {
        assert_eq!(percent_decode("a%2"), "a%2");
        assert_eq!(percent_decode("a%zz%41"), "a%zzA");
    }

    #[cfg(windows)]
    #[test]
    fn a_name_that_isnt_unicode_still_works() {
        use std::os::windows::ffi::OsStringExt;
        let odd = OsString::from_wide(&[0x43, 0x3a, 0x5c, 0xd800]);
        let args = vec![odd.clone()];
        assert_eq!(parse(&args, |_| true, |_| true), Target::Dir(PathBuf::from(odd)));
    }
}
