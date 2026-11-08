//! What the New menu can make besides a folder and a text document: on Windows the same list Explorer's New menu shows,
//! which programs add to the registry, and on Linux the files in the Templates folder.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

// only Windows has programs that make their own, and shortcuts
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Clone, Debug, PartialEq)]
pub enum How {
    Empty,
    Bytes(Vec<u8>),
    CopyOf(PathBuf),
    /// a program makes it; `%1` is where
    Command(String),
    /// an empty .lnk, then Windows's shortcut wizard fills it in
    Shortcut,
}

#[derive(Clone, Debug)]
pub struct Template {
    /// the menu's wording
    pub label: String,
    /// the new item's name without a number, such as "New Compressed (zipped) Folder.zip"
    pub name: String,
    pub how: How,
}

/// Read once, when the New menu first opens.
pub fn templates() -> &'static [Template] {
    static ALL: OnceLock<Vec<Template>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut v = find();
        v.sort_by_key(|t| t.label.to_lowercase());
        v.dedup_by(|a, b| a.label.eq_ignore_ascii_case(&b.label));
        v
    })
}

/// The program and its arguments, with `%1` (or `"%1"`) as `path`.
pub fn command_line(cmd: &str, path: &Path) -> (String, String) {
    let p = path.to_string_lossy();
    let cmd = cmd.trim();
    let (prog, rest) = match cmd.strip_prefix('"') {
        Some(r) => r.split_once('"').map(|(a, b)| (a, b)).unwrap_or((r, "")),
        None => cmd.split_once(' ').unwrap_or((cmd, "")),
    };
    let args = rest.trim().replace("\"%1\"", &format!("\"{p}\"")).replace("%1", &format!("\"{p}\""));
    (prog.to_string(), args)
}

#[cfg(not(windows))]
fn find() -> Vec<Template> {
    let Some(dir) = templates_dir() else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
    rd.flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let label = Path::new(&name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| name.clone());
            Template { label, name, how: How::CopyOf(e.path()) }
        })
        .collect()
}

/// The Templates folder of `xdg-user-dirs`, if there is one; macOS has none.
#[cfg(not(windows))]
fn templates_dir() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
    let named = std::fs::read_to_string(config.join("user-dirs.dirs")).ok().and_then(|s| {
        let line = s.lines().find(|l| l.trim_start().starts_with("XDG_TEMPLATES_DIR="))?;
        let v = line.split_once('=')?.1.trim().trim_matches('"').to_string();
        Some(PathBuf::from(v.replace("$HOME", &home.to_string_lossy())))
    });
    let dir = named.unwrap_or_else(|| home.join("Templates"));
    // pointing at the home folder itself means there's none
    (dir != home && dir.is_dir()).then_some(dir)
}

#[cfg(windows)]
fn find() -> Vec<Template> {
    use windows::Win32::System::Registry::*;
    // Explorer keeps what its New menu shows; without that, every file type is looked at
    let cached = reg::multi(HKEY_CURRENT_USER, r"Software\Microsoft\Windows\CurrentVersion\Explorer\Discardable\PostSetup\ShellNew", "Classes");
    let types = cached.filter(|c| !c.is_empty()).unwrap_or_else(|| reg::subkeys(HKEY_CLASSES_ROOT, "").into_iter().filter(|k| k.starts_with('.')).collect());
    // folders and text documents are Daleris's own items; a library only belongs in Libraries
    types.iter().filter(|t| t.starts_with('.') && !matches!(t.to_lowercase().as_str(), ".txt" | ".library-ms")).filter_map(|ext| template(ext)).collect()
}

#[cfg(windows)]
fn template(ext: &str) -> Option<Template> {
    use windows::Win32::System::Registry::HKEY_CLASSES_ROOT as CR;
    let prog = reg::string(CR, ext, "").filter(|p| !p.is_empty());
    // under the type itself, under its program, or under another program it lists
    let mut places = vec![format!(r"{ext}\ShellNew")];
    if let Some(p) = &prog {
        places.push(format!(r"{ext}\{p}\ShellNew"));
    }
    places.extend(reg::subkeys(CR, ext).into_iter().map(|s| format!(r"{ext}\{s}\ShellNew")));
    let key = places.into_iter().find(|k| reg::exists(CR, k))?;
    let text = |name: &str| reg::string(CR, &key, name).map(|s| crate::fsx::resolve_indirect(&s)).filter(|s| !s.is_empty());
    let has = |name: &str| reg::raw(CR, &key, name).is_some();
    let how = if ext.eq_ignore_ascii_case(".lnk") {
        How::Shortcut
    } else if let Some(c) = reg::string(CR, &key, "Command") {
        How::Command(c)
    } else if let Some((ty, bytes)) = reg::raw(CR, &key, "Data") {
        // text data is written as it reads, without the closing zero
        const REG_SZ: u32 = 1;
        if ty == REG_SZ { How::Bytes(String::from_utf16_lossy(&wide_of(&bytes)).trim_end_matches('\0').as_bytes().to_vec()) } else { How::Bytes(bytes) }
    } else if let Some(f) = reg::string(CR, &key, "FileName") {
        How::CopyOf(template_file(&f)?)
    } else if has("NullFile") {
        How::Empty
    } else {
        return None;
    };
    let friendly = prog.as_deref().and_then(|p| reg::string(CR, p, "FriendlyTypeName").map(|s| crate::fsx::resolve_indirect(&s)).filter(|s| !s.is_empty()).or_else(|| reg::string(CR, p, "").filter(|s| !s.is_empty())));
    // "&" marks a menu's shortcut letter in Windows's wording
    let label = text("MenuText").map(|s| without_marks(&s)).or(friendly).unwrap_or_else(|| ext.trim_start_matches('.').to_uppercase());
    let base = text("ItemName").unwrap_or_else(|| format!("New {label}"));
    let name = if base.to_lowercase().ends_with(&ext.to_lowercase()) { base } else { format!("{base}{ext}") };
    Some(Template { label, name, how })
}

/// "&Shortcut" as "Shortcut"; "&&" is a real "&".
#[cfg_attr(not(windows), allow(dead_code))]
fn without_marks(s: &str) -> String {
    s.split("&&").map(|p| p.replace('&', "")).collect::<Vec<_>>().join("&")
}

#[cfg(windows)]
fn wide_of(bytes: &[u8]) -> Vec<u16> {
    bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect()
}

/// A template's file: a full path, or one in the Templates folders or Windows\ShellNew.
#[cfg(windows)]
fn template_file(name: &str) -> Option<PathBuf> {
    let p = PathBuf::from(crate::policy::expand(name));
    if p.is_absolute() {
        return p.is_file().then_some(p);
    }
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    [
        env("APPDATA").map(|d| d.join(r"Microsoft\Windows\Templates")),
        env("ProgramData").map(|d| d.join(r"Microsoft\Windows\Templates")),
        env("SystemRoot").map(|d| d.join("ShellNew")),
    ]
    .into_iter()
    .flatten()
    .map(|d| d.join(&p))
    .find(|f| f.is_file())
}

#[cfg(windows)]
mod reg {
    use crate::util::{from_wide, wide};
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::*;
    use windows::core::PCWSTR;

    /// A value's type and bytes.
    pub fn raw(root: HKEY, key: &str, name: &str) -> Option<(u32, Vec<u8>)> {
        let (k, n) = (wide(key), wide(name));
        let mut ty = REG_VALUE_TYPE(0);
        let mut len = 0u32;
        unsafe {
            if RegGetValueW(root, PCWSTR(k.as_ptr()), PCWSTR(n.as_ptr()), RRF_RT_ANY | RRF_NOEXPAND, Some(&mut ty), None, Some(&mut len)) != ERROR_SUCCESS {
                return None;
            }
            let mut buf = vec![0u8; len as usize];
            if RegGetValueW(root, PCWSTR(k.as_ptr()), PCWSTR(n.as_ptr()), RRF_RT_ANY | RRF_NOEXPAND, Some(&mut ty), Some(buf.as_mut_ptr() as *mut _), Some(&mut len)) != ERROR_SUCCESS {
                return None;
            }
            buf.truncate(len as usize);
            Some((ty.0, buf))
        }
    }

    pub fn string(root: HKEY, key: &str, name: &str) -> Option<String> {
        let (ty, b) = raw(root, key, name)?;
        (ty == REG_SZ.0 || ty == REG_EXPAND_SZ.0).then(|| from_wide(&super::wide_of(&b)))
    }

    pub fn multi(root: HKEY, key: &str, name: &str) -> Option<Vec<String>> {
        let (ty, b) = raw(root, key, name)?;
        (ty == REG_MULTI_SZ.0).then(|| String::from_utf16_lossy(&super::wide_of(&b)).split('\0').filter(|s| !s.is_empty()).map(String::from).collect())
    }

    pub fn exists(root: HKEY, key: &str) -> bool {
        let k = wide(key);
        let mut hk = HKEY::default();
        let ok = unsafe { RegOpenKeyExW(root, PCWSTR(k.as_ptr()), Some(0), KEY_READ, &mut hk) } == ERROR_SUCCESS;
        if ok {
            let _ = unsafe { RegCloseKey(hk) };
        }
        ok
    }

    pub fn subkeys(root: HKEY, key: &str) -> Vec<String> {
        let k = wide(key);
        let mut hk = HKEY::default();
        let mut out = Vec::new();
        unsafe {
            if RegOpenKeyExW(root, PCWSTR(k.as_ptr()), Some(0), KEY_READ, &mut hk) != ERROR_SUCCESS {
                return out;
            }
            let mut buf = [0u16; 256];
            for i in 0.. {
                let mut len = buf.len() as u32;
                if RegEnumKeyExW(hk, i, Some(windows::core::PWSTR(buf.as_mut_ptr())), &mut len, None, None, None, None) != ERROR_SUCCESS {
                    break;
                }
                out.push(from_wide(&buf[..len as usize]));
            }
            let _ = RegCloseKey(hk);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_marks_are_left_out() {
        assert_eq!(without_marks("&Verkn{fc}pfung"), "Verkn{fc}pfung");
        assert_eq!(without_marks("Tom && &Jerry"), "Tom & Jerry");
    }

    #[test]
    fn a_command_gets_the_new_items_path() {
        let p = Path::new(if cfg!(windows) { r"C:\Work\New contact.contact" } else { "/work/New contact.contact" });
        let q = p.display();
        assert_eq!(command_line(r#""C:\Program Files\Windows Mail\Wab.exe" /CreateContact "%1""#, p), (r"C:\Program Files\Windows Mail\Wab.exe".into(), format!("/CreateContact \"{q}\"")));
        assert_eq!(command_line("rundll32.exe appwiz.cpl,NewLinkHere %1", p), ("rundll32.exe".into(), format!("appwiz.cpl,NewLinkHere \"{q}\"")));
    }

    #[cfg(windows)]
    #[test]
    fn finds_what_explorer_offers() {
        let all = templates();
        // every Windows has the zipped folder
        let zip = all.iter().find(|t| t.name.to_lowercase().ends_with(".zip")).expect("a zipped folder");
        assert!(matches!(&zip.how, How::Bytes(b) if b.starts_with(b"PK")), "{zip:?}");
        assert!(!all.iter().any(|t| t.name.to_lowercase().ends_with(".library-ms")));
    }
}
