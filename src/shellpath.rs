//! Places that only Windows's shell can show, such as a phone or camera connected by USB (a portable device), and how
//! Daleris writes them as a path. No disk and no Windows calls in here: the shell itself is in `shellns.rs`.
//!
//! Such a place has no file system path. Daleris keeps it in a `PathBuf` all the same: the device's shell parsing name,
//! which starts with `::{`, followed by the display names of the folders below it, as Explorer shows them:
//!
//! `::{20D04FE0-3AEA-1069-A2D8-08002B30309D}\\\?\usb#vid_18d1&pid_4ee1#…#{6ac27878-…}\Internal storage\DCIM`
//!
//! So `join`, `file_name` and `parent` work on it as on any other path, except above the device, where `parent_of`
//! says there's nothing (the device is in This PC).

// Linux and macOS have no such places: the parts only Windows calls stay unused there.
#![cfg_attr(not(windows), allow(dead_code))]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

/// This PC's parsing name. Portable devices are its children.
pub const THIS_PC: &str = "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}";

/// A device in This PC that has no drive letter, such as a phone or camera.
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    /// its shell parsing name, the root of every path on it
    pub root: PathBuf,
    /// what Windows calls it, "Pixel 8"
    pub name: String,
}

/// Is this a shell place rather than a file system path? Only Windows has them. Doesn't touch anything.
pub fn is_shell(p: &Path) -> bool {
    cfg!(windows) && is_shell_text(&p.to_string_lossy())
}

pub fn is_shell_text(s: &str) -> bool {
    s.starts_with("::{")
}

/// The root (a parsing name, which may itself hold backslashes) and the display names below it.
pub fn split(s: &str) -> Option<(&str, Vec<&str>)> {
    if !is_shell_text(s) {
        return None;
    }
    let mut root_end = s.find('}')? + 1;
    // a device: `\` and then `\\?\…` up to the next backslash, all one name
    let after = s[root_end..].trim_start_matches('\\');
    if after.starts_with("?\\") {
        let start = s.len() - after.len() + 2;
        root_end = s[start..].find('\\').map_or(s.len(), |i| start + i);
    }
    let segs = s[root_end..].split('\\').filter(|x| !x.is_empty()).collect();
    Some((&s[..root_end], segs))
}

/// The folder a shell place is in; None for the device itself, which is in This PC. Other paths as `Path::parent`.
pub fn parent_of(p: &Path) -> Option<PathBuf> {
    let s = p.to_string_lossy();
    match split(&s) {
        Some((_, segs)) if segs.is_empty() => None,
        Some((root, segs)) => Some(build(root, &segs[..segs.len() - 1])),
        None => p.parent().map(Path::to_path_buf),
    }
}

/// The path of `root` with these names below it.
pub fn build(root: &str, segs: &[&str]) -> PathBuf {
    let mut s = root.to_string();
    for seg in segs {
        s.push('\\');
        s.push_str(seg);
    }
    PathBuf::from(s)
}

/// The same place however it's written, for remembering things about it.
pub fn key(p: &Path) -> String {
    let s = p.to_string_lossy();
    match split(&s) {
        Some((root, segs)) => build(root, &segs).to_string_lossy().to_lowercase(),
        None => s.to_lowercase(),
    }
}

/// The root of the device `p` is on.
pub fn root_of(p: &Path) -> Option<PathBuf> {
    let s = p.to_string_lossy();
    split(&s).map(|(root, _)| PathBuf::from(root))
}

/// A display name made fit to be one part of a path: it can't hold a separator.
pub fn clean_segment(name: &str) -> String {
    let n: String = name.chars().map(|c| if c == '\\' || c == '/' { '_' } else { c }).collect();
    let n = n.trim().to_string();
    if n.is_empty() || n == "." || n == ".." { "_".to_string() } else { n }
}

/// The parts of the address bar for `p`, as (label, place): the device by its name, then each folder. "This PC"
/// comes before them, as for drives.
pub fn crumbs(p: &Path, device_name: &str) -> Vec<(String, PathBuf)> {
    let s = p.to_string_lossy();
    let Some((root, segs)) = split(&s) else { return Vec::new() };
    // This PC itself comes first anyway
    let mut out = if root.eq_ignore_ascii_case(THIS_PC) { Vec::new() } else { vec![(device_name.to_string(), PathBuf::from(root))] };
    for i in 0..segs.len() {
        out.push((segs[i].to_string(), build(root, &segs[..=i])));
    }
    out
}

/// What the address bar shows for `p` when it's edited, as Explorer writes it: `This PC\Pixel 8\Internal storage\DCIM`.
pub fn address_text(p: &Path, this_pc: &str, device_name: &str) -> String {
    let mut parts = vec![this_pc.to_string()];
    parts.extend(crumbs(p, device_name).into_iter().map(|(l, _)| l));
    parts.join("\\")
}

/// The place typed into the address bar, if it's on a device: its parsing name as it is, or the names Explorer shows,
/// starting with This PC (in any of `this_pc`'s spellings) or the device's name.
pub fn parse_address(text: &str, this_pc: &[&str], devices: &[Device]) -> Option<PathBuf> {
    let text = text.trim().trim_matches('"');
    if is_shell_text(text) {
        return Some(PathBuf::from(text));
    }
    let mut parts: Vec<&str> = text.split(['\\', '/']).map(str::trim).filter(|x| !x.is_empty()).collect();
    if parts.first().is_some_and(|f| this_pc.iter().any(|t| t.eq_ignore_ascii_case(f))) {
        parts.remove(0);
    }
    let first = parts.first()?;
    let d = devices.iter().find(|d| d.name.eq_ignore_ascii_case(first))?;
    Some(build(&d.root.to_string_lossy(), &parts[1..]))
}

/// Whether a copy, move or delete involving these has to be done by Windows's shell: anything on a device.
pub fn needs_shell_op(srcs: &[PathBuf], dest: Option<&Path>) -> bool {
    srcs.iter().map(PathBuf::as_path).chain(dest).any(is_shell)
}

/// The names of the devices seen, by root, for titles and the address bar where no device list is at hand.
static NAMES: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

pub fn remember_name(root: &Path, name: &str) {
    NAMES.lock().unwrap_or_else(PoisonError::into_inner).get_or_insert_with(HashMap::new).insert(key(root), name.to_string());
}

/// What the device `p` is on is called, if it's been seen.
pub fn device_name(p: &Path) -> Option<String> {
    let root = root_of(p)?;
    NAMES.lock().unwrap_or_else(PoisonError::into_inner).as_ref()?.get(&key(&root)).cloned()
}

/// A shell place's title: its own name, or the device's for the device itself.
pub fn title(p: &Path, unknown_device: &str) -> String {
    let s = p.to_string_lossy();
    match split(&s) {
        Some((_, segs)) if !segs.is_empty() => segs[segs.len() - 1].to_string(),
        _ => device_name(p).unwrap_or_else(|| unknown_device.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEV: &str = r"::{20D04FE0-3AEA-1069-A2D8-08002B30309D}\\\?\usb#vid_18d1&pid_4ee1#0123456789#{6ac27878-a6fa-4155-ba85-f98f491d4f33}";

    fn on_dev(rest: &str) -> PathBuf {
        PathBuf::from(format!(r"{DEV}{rest}"))
    }

    #[test]
    fn a_device_root_is_one_name() {
        let (root, segs) = split(DEV).unwrap();
        assert_eq!(root, DEV);
        assert!(segs.is_empty());
        let deep = format!(r"{DEV}\Internal storage\DCIM\Camera");
        let (root, segs) = split(&deep).unwrap();
        assert_eq!(root, DEV);
        assert_eq!(segs, ["Internal storage", "DCIM", "Camera"]);
        // a shell folder that isn't a device: its class id is the root
        let (root, segs) = split(r"::{645FF040-5081-101B-9F08-00AA002F954E}\x").unwrap();
        assert_eq!((root, segs), ("::{645FF040-5081-101B-9F08-00AA002F954E}", vec!["x"]));
        assert_eq!(split(r"C:\Users"), None);
    }

    #[test]
    fn paths_on_a_device_go_up_and_down_like_any_other() {
        let dcim = on_dev(r"\Internal storage\DCIM");
        assert_eq!(parent_of(&dcim), Some(on_dev(r"\Internal storage")));
        assert_eq!(parent_of(&on_dev(r"\Internal storage")), Some(PathBuf::from(DEV)));
        assert_eq!(parent_of(Path::new(DEV)), None, "the device is in This PC");
        // join and file_name, as the list uses them; only Windows joins with a backslash
        if !cfg!(windows) {
            return;
        }
        let photo = dcim.join("IMG_0001.jpg");
        assert_eq!(photo.file_name().unwrap(), "IMG_0001.jpg");
        assert_eq!(parent_of(&photo), Some(dcim.clone()));
        assert_eq!(key(&photo), key(&PathBuf::from(format!(r"{}\", photo.display()))), "a trailing separator is the same place");
        assert_eq!(root_of(&photo), Some(PathBuf::from(DEV)));
        if cfg!(windows) {
            assert_eq!(parent_of(Path::new(r"C:\a\b")), Some(PathBuf::from(r"C:\a")));
        }
    }

    #[test]
    fn only_shell_places_are_shell_places() {
        assert_eq!(is_shell(Path::new(DEV)), cfg!(windows));
        assert!(!is_shell(Path::new(r"C:\Users")));
        assert!(!is_shell(Path::new(r"\\server\share")));
        assert!(!is_shell(Path::new("/home/me")));
    }

    #[test]
    fn names_never_hold_a_separator() {
        assert_eq!(clean_segment("a/b\\c"), "a_b_c");
        assert_eq!(clean_segment("  Internal storage "), "Internal storage");
        assert_eq!(clean_segment(".."), "_");
    }

    #[test]
    fn the_address_reads_as_explorer_writes_it() {
        let p = on_dev(r"\Internal storage\DCIM");
        assert_eq!(address_text(&p, "This PC", "Pixel 8"), r"This PC\Pixel 8\Internal storage\DCIM");
        assert_eq!(address_text(Path::new(DEV), "Dieser PC", "Pixel 8"), r"Dieser PC\Pixel 8");
        let crumbs = crumbs(&p, "Pixel 8");
        assert_eq!(crumbs.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), ["Pixel 8", "Internal storage", "DCIM"]);
        assert_eq!(crumbs[0].1, PathBuf::from(DEV));
        assert_eq!(crumbs[2].1, p);
        // no arrows or middle dots
        assert!(!address_text(&p, "This PC", "Pixel 8").contains(['\u{2192}', '\u{203a}', '\u{b7}', '>']));
    }

    #[test]
    fn a_typed_address_finds_the_device() {
        let devices = [Device { root: PathBuf::from(DEV), name: "Pixel 8".into() }];
        let this_pc = ["This PC", "Dieser PC"];
        assert_eq!(parse_address(r"This PC\Pixel 8\Internal storage\DCIM", &this_pc, &devices), Some(on_dev(r"\Internal storage\DCIM")));
        assert_eq!(parse_address(r"dieser pc\pixel 8\", &this_pc, &devices), Some(PathBuf::from(DEV)));
        assert_eq!(parse_address(r"Pixel 8/DCIM", &this_pc, &devices), Some(on_dev(r"\DCIM")));
        assert_eq!(parse_address(DEV, &this_pc, &devices), Some(PathBuf::from(DEV)));
        assert_eq!(parse_address(r"C:\Users", &this_pc, &devices), None);
        assert_eq!(parse_address(r"This PC\Galaxy", &this_pc, &devices), None);
    }

    #[test]
    fn anything_on_a_device_goes_to_windows() {
        let local = vec![PathBuf::from(r"C:\a.txt")];
        let phone = vec![on_dev(r"\DCIM\a.jpg")];
        assert!(!needs_shell_op(&local, Some(Path::new(r"D:\"))));
        assert!(!needs_shell_op(&local, None));
        assert_eq!(needs_shell_op(&local, Some(&on_dev(r"\DCIM"))), cfg!(windows));
        assert_eq!(needs_shell_op(&phone, Some(Path::new(r"C:\Pictures"))), cfg!(windows));
        assert_eq!(needs_shell_op(&phone, None), cfg!(windows));
    }

    #[test]
    fn titles_use_the_devices_name() {
        assert_eq!(title(&on_dev(r"\Internal storage"), "Device"), "Internal storage");
        let other = r"::{20D04FE0-3AEA-1069-A2D8-08002B30309D}\\\?\usb#vid_1#unnamed#{6ac27878-a6fa-4155-ba85-f98f491d4f33}";
        assert_eq!(title(Path::new(other), "Device"), "Device");
        remember_name(Path::new(other), "Camera");
        assert_eq!(title(Path::new(other), "Device"), "Camera");
        assert_eq!(device_name(&PathBuf::from(format!(r"{other}\DCIM"))).as_deref(), Some("Camera"));
    }
}
