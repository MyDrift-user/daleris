//! The registry changes behind each switch, all under HKCU, and how they're undone.
//!
//! Before writing a value this records what was there, and whether the key existed, under `Software\Daleris\Integration`.
//! Undoing puts that back, and only if the value is still the one written here: anything another program set since
//! stays. So `daleris --restore-explorer` removes what Daleris wrote and nothing else.

use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use windows::core::PCWSTR;

use windows::Win32::System::Registry::*;
use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};

use crate::Feature;

pub const FOLDER_OPEN: &str = r"Software\Classes\Folder\shell\open\command";
pub const FOLDER_EXPLORE: &str = r"Software\Classes\Folder\shell\explore\command";
pub const WIN_E: &str = r"Software\Classes\CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}\shell\opennewwindow\command";
pub const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
pub const RUN_ONCE: &str = r"Software\Microsoft\Windows\CurrentVersion\RunOnce";
pub const RUN_VALUE: &str = "Daleris Win+E";
pub const RUN_JUMP_VALUE: &str = "Daleris Ctrl+G";
const CLEANUP_VALUE: &str = "Daleris cleanup";
/// `CLSID_FileOpenDialog` and `CLSID_FileSaveDialog`.
pub const DIALOG_CLSIDS: [&str; 2] = ["{DC1C5A9C-E88A-4DDE-A5A1-60F82A20AEF7}", "{C0B4E2F3-BA21-4773-8DBA-335EC946EB8B}"];
/// Everything of ours lives below this, except what Windows itself has to find elsewhere.
pub const OWN_KEY: &str = r"Software\Daleris\Integration";
const POLICY_KEY: &str = r"Software\Policies\Daleris";
/// What the window and the Ctrl + G watcher tell each other, under `OWN_KEY`.
pub const JUMP_FOLDER: &str = "JumpFolder";
pub const JUMP_PID: &str = "JumpFolderPid";
pub const JUMP_PROBLEM: &str = "JumpProblem";
/// The file dialog DLL's log, in `logs_dir`.
pub const DIALOG_LOG: &str = "file-dialogs.log";

/// One value this program sets: key, value name ("" for the default value), what it holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Write {
    pub key: String,
    pub name: String,
    pub data: String,
}

fn w(key: impl Into<String>, name: &str, data: impl Into<String>) -> Write {
    Write { key: key.into(), name: name.into(), data: data.into() }
}

/// Where the support files ended up. See `install_support_files` in the program.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Support {
    pub launcher: PathBuf,
    pub dll64: PathBuf,
    /// 32-bit apps keep Windows' dialog without this.
    pub dll32: Option<PathBuf>,
}

/// The values one switch sets. Those that only tell the launcher or the DLL to do something are `Flag-` values of ours.
pub fn writes(feature: Feature, s: &Support) -> Vec<Write> {
    let launcher = s.launcher.display();
    let flag = |f: Feature| w(OWN_KEY, &format!("Flag-{}", f.id()), "1");
    match feature {
        Feature::Folders => [FOLDER_OPEN, FOLDER_EXPLORE]
            .into_iter()
            .flat_map(|key| [w(key, "", format!("\"{launcher}\" --folder \"%1\"")), w(key, "DelegateExecute", "")])
            .collect(),
        Feature::WinE => vec![w(WIN_E, "", format!("\"{launcher}\" --win-e")), w(WIN_E, "DelegateExecute", "")],
        Feature::WinEHook => vec![w(RUN, RUN_VALUE, format!("\"{launcher}\" --win-e-hook"))],
        Feature::Select | Feature::SingleWindow | Feature::DialogLog => vec![flag(feature)],
        Feature::DialogJump => vec![flag(feature), w(RUN, RUN_JUMP_VALUE, format!("\"{launcher}\" --dialog-jump"))],
        Feature::Dialogs => {
            let mut v = vec![flag(feature)];
            for clsid in DIALOG_CLSIDS {
                let key = format!(r"Software\Classes\CLSID\{clsid}\InprocServer32");
                v.push(w(key.clone(), "", s.dll64.display().to_string()));
                v.push(w(key, "ThreadingModel", "Apartment"));
                if let Some(dll32) = &s.dll32 {
                    let key = format!(r"Software\Classes\WOW6432Node\CLSID\{clsid}\InprocServer32");
                    v.push(w(key.clone(), "", dll32.display().to_string()));
                    v.push(w(key, "ThreadingModel", "Apartment"));
                }
            }
            v
        }
    }
}

// ---------------------------------------------------------------- plain registry access

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

/// A view of HKCU, with a prefix in tests so they never touch anything real.
#[derive(Clone, Debug, Default)]
pub struct Hive {
    prefix: String,
}

impl Hive {
    pub fn real() -> Hive {
        Hive::default()
    }

    /// All keys below this one instead. For tests.
    pub fn below(prefix: &str) -> Hive {
        Hive { prefix: format!("{prefix}\\") }
    }

    fn path(&self, key: &str) -> Vec<u16> {
        wide(&format!("{}{key}", self.prefix))
    }

    fn open(&self, key: &str, access: REG_SAM_FLAGS) -> Option<HKEY> {
        let mut h = HKEY::default();
        let k = self.path(key);
        unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr()), 0, access, &mut h) }.is_ok().then_some(h)
    }

    pub fn key_exists(&self, key: &str) -> bool {
        self.open(key, KEY_READ).map(|h| unsafe { RegCloseKey(h) }).is_some()
    }

    /// (type, bytes) of a value.
    fn read_raw(&self, key: &str, name: &str) -> Option<(u32, Vec<u8>)> {
        let k = self.open(key, KEY_READ)?;
        let n = wide(name);
        let mut ty = REG_VALUE_TYPE(0);
        let mut len = 0;
        let mut result = None;
        if unsafe { RegQueryValueExW(k, PCWSTR(n.as_ptr()), None, Some(&mut ty), None, Some(&mut len)) }.is_ok() {
            let mut buf = vec![0u8; len as usize];
            if unsafe { RegQueryValueExW(k, PCWSTR(n.as_ptr()), None, Some(&mut ty), Some(buf.as_mut_ptr()), Some(&mut len)) }.is_ok() {
                buf.truncate(len as usize);
                result = Some((ty.0, buf));
            }
        }
        let _ = unsafe { RegCloseKey(k) };
        result
    }

    fn write_raw(&self, key: &str, name: &str, ty: u32, data: &[u8]) -> bool {
        let k = self.path(key);
        let mut h = HKEY::default();
        if unsafe { RegCreateKeyExW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr()), 0, None, REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut h, None) }.is_err() {
            return false;
        }
        let n = wide(name);
        let ok = unsafe { RegSetValueExW(h, PCWSTR(n.as_ptr()), 0, REG_VALUE_TYPE(ty), Some(data)) }.is_ok();
        let _ = unsafe { RegCloseKey(h) };
        ok
    }

    pub fn read_text(&self, key: &str, name: &str) -> Option<String> {
        let (ty, bytes) = self.read_raw(key, name)?;
        (ty == REG_SZ.0 || ty == REG_EXPAND_SZ.0).then(|| text_of(&bytes))
    }

    pub fn write_text(&self, key: &str, name: &str, data: &str) -> bool {
        self.write_raw(key, name, REG_SZ.0, &bytes_of(data))
    }

    fn write_dword(&self, key: &str, name: &str, v: u32) -> bool {
        self.write_raw(key, name, REG_DWORD.0, &v.to_le_bytes())
    }

    pub fn delete_value(&self, key: &str, name: &str) -> bool {
        let Some(h) = self.open(key, KEY_SET_VALUE) else { return false };
        let n = wide(name);
        let ok = unsafe { RegDeleteValueW(h, PCWSTR(n.as_ptr())) }.is_ok();
        let _ = unsafe { RegCloseKey(h) };
        ok
    }

    /// Deletes a key if nothing is in it.
    fn delete_if_empty(&self, key: &str) -> bool {
        let Some(h) = self.open(key, KEY_READ) else { return false };
        let (mut subkeys, mut values) = (0, 0);
        let got = unsafe { RegQueryInfoKeyW(h, windows::core::PWSTR::null(), None, None, Some(&mut subkeys), None, None, Some(&mut values), None, None, None, None) };
        let _ = unsafe { RegCloseKey(h) };
        if got.is_err() || subkeys != 0 || values != 0 {
            return false;
        }
        let k = self.path(key);
        unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr())) }.is_ok()
    }

    /// Deletes a whole key of our own.
    fn delete_tree(&self, key: &str) {
        let k = self.path(key);
        let _ = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr())) };
        let _ = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr())) };
    }

    fn subkey_names(&self, key: &str) -> Vec<String> {
        let Some(h) = self.open(key, KEY_READ) else { return Vec::new() };
        let mut out = Vec::new();
        for i in 0.. {
            let mut buf = [0u16; 256];
            let mut len = buf.len() as u32;
            if unsafe { RegEnumKeyExW(h, i, windows::core::PWSTR(buf.as_mut_ptr()), &mut len, None, windows::core::PWSTR::null(), None, None) }.is_err() {
                break;
            }
            out.push(String::from_utf16_lossy(&buf[..len as usize]));
        }
        let _ = unsafe { RegCloseKey(h) };
        out
    }

    // ------------------------------------------------------------ switches

    fn backup_key(&self, feature: Feature) -> String {
        format!(r"{OWN_KEY}\Backup\{}", feature.id())
    }

    /// Sets a switch's values, remembering what each one replaced. Safe to repeat, which also moves them to new paths.
    pub fn apply(&self, feature: Feature, support: &Support) -> Result<(), String> {
        let backup = self.backup_key(feature);
        let wanted = writes(feature, support);
        // a value from before that this version doesn't set any more
        for entry in self.subkey_names(&backup) {
            let at = format!(r"{backup}\{entry}");
            let (Some(key), Some(name)) = (self.read_text(&at, "Key"), self.read_text(&at, "Name")) else { continue };
            if !wanted.iter().any(|w| w.key == key && w.name == name) {
                self.undo_entry(&at);
            }
        }
        for (i, write) in wanted.iter().enumerate() {
            let at = format!(r"{backup}\{}", self.find_entry(&backup, write).unwrap_or_else(|| self.free_entry(&backup, i)));
            if !self.key_exists(&at) {
                self.remember(&at, write)?;
            }
            if !self.write_text(&at, "Ours", &write.data) || !self.write_text(&write.key, &write.name, &write.data) {
                return Err(format!("Couldn't write {}", write.key));
            }
        }
        if self.prefix.is_empty() {
            notify_shell(feature);
        }
        Ok(())
    }

    fn find_entry(&self, backup: &str, write: &Write) -> Option<String> {
        self.subkey_names(backup).into_iter().find(|e| {
            let at = format!(r"{backup}\{e}");
            self.read_text(&at, "Key").is_some_and(|k| k.eq_ignore_ascii_case(&write.key)) && self.read_text(&at, "Name").is_some_and(|n| n == write.name)
        })
    }

    fn free_entry(&self, backup: &str, from: usize) -> String {
        (from..).map(|i| format!("{i:03}")).find(|e| !self.key_exists(&format!(r"{backup}\{e}"))).unwrap_or_default()
    }

    fn remember(&self, at: &str, write: &Write) -> Result<(), String> {
        // the keys that don't exist yet, top first, as those are the ones to remove again
        let mut created = Vec::new();
        let mut path = String::new();
        for part in write.key.split('\\') {
            if !path.is_empty() {
                path.push('\\');
            }
            path.push_str(part);
            if !self.key_exists(&path) {
                created.push(path.clone());
            }
        }
        let prior = self.read_raw(&write.key, &write.name);
        let mut ledger = self.created_keys();
        ledger.extend(created);
        ledger.sort();
        ledger.dedup();
        let ok = self.write_text(at, "Key", &write.key)
            && self.write_text(at, "Name", &write.name)
            && self.write_text(OWN_KEY, "CreatedKeys", &ledger.join("|"))
            && self.write_dword(at, "Existed", prior.is_some() as u32)
            && prior.as_ref().is_none_or(|(ty, data)| self.write_dword(at, "PriorType", *ty) && self.write_raw(at, "PriorData", REG_BINARY.0, data));
        ok.then_some(()).ok_or_else(|| "Couldn't save what's being replaced, so nothing was changed".to_string())
    }

    /// Puts one value back, if it's still ours, and removes the keys that were only there for it.
    fn undo_entry(&self, at: &str) {
        let (Some(key), Some(name), Some(ours)) = (self.read_text(at, "Key"), self.read_text(at, "Name"), self.read_text(at, "Ours")) else {
            self.delete_tree(at);
            return;
        };
        if self.read_text(&key, &name).as_deref() == Some(ours.as_str()) {
            let existed = self.read_raw(at, "Existed").is_some_and(|(_, d)| d.first() == Some(&1));
            match (existed, self.read_raw(at, "PriorType"), self.read_raw(at, "PriorData")) {
                (true, Some((_, ty)), Some((_, data))) if ty.len() == 4 => {
                    self.write_raw(&key, &name, u32::from_le_bytes([ty[0], ty[1], ty[2], ty[3]]), &data);
                }
                (true, Some((_, ty)), None) if ty.len() == 4 => {
                    self.write_raw(&key, &name, u32::from_le_bytes([ty[0], ty[1], ty[2], ty[3]]), &[]);
                }
                _ => {
                    self.delete_value(&key, &name);
                }
            }
        }
        self.delete_tree(at);
        self.prune_created();
    }

    /// Keys that exist only because of something written here, however many switches shared them.
    fn created_keys(&self) -> Vec<String> {
        self.read_text(OWN_KEY, "CreatedKeys").unwrap_or_default().split('|').filter(|k| !k.is_empty()).map(String::from).collect()
    }

    /// Removes those that have nothing in them any more, deepest first. Anything else put there since keeps them.
    fn prune_created(&self) {
        let mut keys = self.created_keys();
        keys.sort_by_key(|k| std::cmp::Reverse(k.len()));
        let left: Vec<String> = keys.into_iter().filter(|k| self.key_exists(k) && !self.delete_if_empty(k)).collect();
        if self.key_exists(OWN_KEY) {
            self.write_text(OWN_KEY, "CreatedKeys", &left.join("|"));
        }
    }

    /// Takes a switch's values out again.
    pub fn remove(&self, feature: Feature) {
        let backup = self.backup_key(feature);
        let mut entries = self.subkey_names(&backup);
        entries.sort();
        for e in entries {
            self.undo_entry(&format!(r"{backup}\{e}"));
        }
        self.delete_tree(&backup);
        self.prune_own();
        if self.prefix.is_empty() {
            notify_shell(feature);
        }
    }

    /// Is every value of the switch in place, and pointing at files that exist?
    pub fn is_in_place(&self, feature: Feature) -> bool {
        let backup = self.backup_key(feature);
        let entries = self.subkey_names(&backup);
        !entries.is_empty()
            && entries.iter().all(|e| {
                let at = format!(r"{backup}\{e}");
                match (self.read_text(&at, "Key"), self.read_text(&at, "Name"), self.read_text(&at, "Ours")) {
                    (Some(k), Some(n), Some(ours)) => self.read_text(&k, &n).as_deref() == Some(ours.as_str()),
                    _ => false,
                }
            })
    }

    /// Is a switch's flag set? For the launcher and DLL, which have no settings file to go by.
    pub fn flag(&self, feature: Feature) -> bool {
        self.read_text(OWN_KEY, &format!("Flag-{}", feature.id())).as_deref() == Some("1")
    }

    /// Removes every switch, and everything of ours, and says what was left that someone else had changed.
    pub fn restore_all(&self) {
        for f in Feature::ALL {
            self.remove(f);
        }
        self.delete_tree(&format!(r"{OWN_KEY}\Backup"));
        self.delete_tree(OWN_KEY);
        self.delete_if_empty(r"Software\Daleris");
    }

    /// Our key once nothing's left in it but the bookkeeping.
    fn prune_own(&self) {
        let backup = format!(r"{OWN_KEY}\Backup");
        self.delete_if_empty(&backup);
    }

    pub fn program_path(&self) -> Option<PathBuf> {
        self.read_text(OWN_KEY, "Program").map(PathBuf::from)
    }

    pub fn set_program_path(&self, exe: &Path) -> bool {
        self.write_text(OWN_KEY, "Program", &exe.display().to_string())
    }

    /// The person's list of apps the DLL leaves alone.
    pub fn exclusions(&self) -> String {
        self.read_text(OWN_KEY, "DialogExclusions").unwrap_or_default()
    }

    pub fn set_exclusions(&self, text: &str) -> bool {
        self.write_text(OWN_KEY, "DialogExclusions", text)
    }
}

fn bytes_of(s: &str) -> Vec<u8> {
    s.encode_utf16().chain(std::iter::once(0)).flat_map(|w| w.to_le_bytes()).collect()
}

fn text_of(bytes: &[u8]) -> String {
    let words: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    let end = words.iter().position(|&c| c == 0).unwrap_or(words.len());
    OsString::from_wide(&words[..end]).to_string_lossy().into_owned()
}

/// Folder verbs are cached by the shell.
fn notify_shell(feature: Feature) {
    if matches!(feature, Feature::Folders | Feature::WinE) {
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    }
}

// ---------------------------------------------------------------- policy, as the launcher and DLL need it

/// What a policy fixes this switch to, if anything. HKLM first, as everywhere in Daleris.
pub fn policy_fixed(feature: Feature) -> Option<bool> {
    let name = wide(feature.policy_name());
    let key = wide(POLICY_KEY);
    let test = std::env::var("DALERIS_TEST_POLICY_KEY").ok().map(|k| wide(&k));
    let roots: Vec<(HKEY, &[u16])> = [(HKEY_LOCAL_MACHINE, key.as_slice()), (HKEY_CURRENT_USER, key.as_slice())].into_iter().chain(test.iter().map(|k| (HKEY_CURRENT_USER, k.as_slice()))).collect();
    for (root, k) in roots {
        let mut data = 0u32;
        let mut len = 4;
        if unsafe { RegGetValueW(root, PCWSTR(k.as_ptr()), PCWSTR(name.as_ptr()), RRF_RT_REG_DWORD, None, Some(&mut data as *mut u32 as *mut _), Some(&mut len)) }.is_ok() {
            return Some(data != 0);
        }
        let mut buf = [0u16; 16];
        let mut len = std::mem::size_of_val(&buf) as u32;
        if unsafe { RegGetValueW(root, PCWSTR(k.as_ptr()), PCWSTR(name.as_ptr()), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut len)) }.is_ok() {
            let end = buf.iter().position(|&c| c == 0).unwrap_or(0);
            let text = String::from_utf16_lossy(&buf[..end]);
            if let Ok(n) = text.trim().parse::<u32>() {
                return Some(n != 0);
            }
        }
    }
    None
}

// ---------------------------------------------------------------- files left in use

/// The folder for the launcher and DLL: `%LOCALAPPDATA%\Daleris\integration`. Per person, so nothing here can stop
/// another person's dialogs, or an upgrade of a PC-wide install, by being in use or being gone.
pub fn support_dir() -> Option<PathBuf> {
    local_dir("integration")
}

/// Where `file-dialogs.log` and `dialog-jump.log` go, next to the program's own log.
pub fn logs_dir() -> Option<PathBuf> {
    local_dir("logs")
}

fn local_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|a| PathBuf::from(a).join("Daleris").join(name))
}

/// Removes the support folder now, or at next sign-in for what's still loaded in a running app.
/// A running launcher can't remove itself: `after_exit` hands that to a short-lived `cmd`.
pub fn remove_support_files(hive: &Hive, after_exit: bool) {
    let Some(dir) = support_dir() else { return };
    if !dir.exists() {
        return;
    }
    if !after_exit {
        let _ = std::fs::remove_dir_all(&dir);
    }
    if !dir.exists() {
        return;
    }
    let system = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows")).join("System32").join("cmd.exe");
    let quoted = format!(r#""{}" /c rmdir /s /q "{}""#, system.display(), dir.display());
    if after_exit {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let wait = format!(r#"/c ping -n 3 127.0.0.1 >nul & rmdir /s /q "{}""#, dir.display());
        let _ = std::process::Command::new(&system).raw_arg(wait).creation_flags(CREATE_NO_WINDOW).spawn();
    } else {
        hive.write_text(RUN_ONCE, CLEANUP_VALUE, &quoted);
    }
}

/// Takes away a removal that was waiting for the next sign-in.
pub fn clear_cleanup_note(hive: &Hive) {
    hive.delete_value(RUN_ONCE, CLEANUP_VALUE);
}


#[cfg(test)]
mod tests {
    use super::*;

    /// A key of its own per test, removed afterwards even if the test fails.
    struct Scratch(Hive, String);

    impl Scratch {
        fn new(name: &str) -> Scratch {
            let key = format!(r"Software\DalerisTest-{}-{name}", std::process::id());
            let h = Hive::below(&key);
            Scratch(h, key)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            Hive::real().delete_tree(&self.1);
        }
    }

    fn support() -> Support {
        Support { launcher: PathBuf::from(r"C:\x\daleris-open.exe"), dll64: PathBuf::from(r"C:\x\d64.dll"), dll32: Some(PathBuf::from(r"C:\x\d32.dll")) }
    }

    #[test]
    fn turning_a_switch_on_and_off_leaves_nothing_behind() {
        let s = Scratch::new("clean");
        for f in Feature::ALL {
            s.0.apply(f, &support()).unwrap();
            assert!(s.0.is_in_place(f), "{f:?}");
            s.0.remove(f);
            assert!(!s.0.is_in_place(f), "{f:?}");
            for write in writes(f, &support()) {
                // the bookkeeping's own key is only cleared by restoring everything
                if write.key != OWN_KEY {
                    assert!(!s.0.key_exists(&write.key), "{f:?} {}", write.key);
                }
            }
            assert!(!s.0.key_exists(r"Software\Classes\Folder"), "{f:?}");
            assert!(!s.0.key_exists(r"Software\Classes\CLSID"), "{f:?}");
            assert!(!s.0.key_exists(r"Software\Classes\WOW6432Node"), "{f:?}");
        }
    }

    #[test]
    fn what_was_there_before_comes_back() {
        let s = Scratch::new("prior");
        s.0.write_text(FOLDER_OPEN, "", "other.exe");
        s.0.write_text(FOLDER_OPEN, "Keep", "me");
        s.0.apply(Feature::Folders, &support()).unwrap();
        assert!(s.0.read_text(FOLDER_OPEN, "").unwrap().contains("daleris-open"));
        // applying twice must not save our own value as what came before
        s.0.apply(Feature::Folders, &support()).unwrap();
        s.0.remove(Feature::Folders);
        assert_eq!(s.0.read_text(FOLDER_OPEN, "").as_deref(), Some("other.exe"));
        assert_eq!(s.0.read_text(FOLDER_OPEN, "Keep").as_deref(), Some("me"));
        assert_eq!(s.0.read_text(FOLDER_OPEN, "DelegateExecute"), None);
        assert_eq!(s.0.read_text(FOLDER_EXPLORE, ""), None);
    }

    #[test]
    fn what_another_program_changed_since_is_left_alone() {
        let s = Scratch::new("foreign");
        s.0.apply(Feature::Folders, &support()).unwrap();
        s.0.write_text(FOLDER_OPEN, "", "somebody-else.exe");
        assert!(!s.0.is_in_place(Feature::Folders), "shows that it's no longer active");
        s.0.remove(Feature::Folders);
        assert_eq!(s.0.read_text(FOLDER_OPEN, "").as_deref(), Some("somebody-else.exe"));
        assert_eq!(s.0.read_text(FOLDER_EXPLORE, ""), None);
    }

    #[test]
    fn a_new_path_replaces_the_old_without_losing_what_came_before_it() {
        let s = Scratch::new("move");
        s.0.write_text(WIN_E, "", "before.exe");
        s.0.apply(Feature::WinE, &support()).unwrap();
        let moved = Support { launcher: PathBuf::from(r"D:\new\daleris-open.exe"), ..support() };
        s.0.apply(Feature::WinE, &moved).unwrap();
        assert!(s.0.read_text(WIN_E, "").unwrap().contains(r"D:\new"));
        assert!(s.0.is_in_place(Feature::WinE));
        s.0.remove(Feature::WinE);
        assert_eq!(s.0.read_text(WIN_E, "").as_deref(), Some("before.exe"));
    }

    #[test]
    fn a_dll_no_longer_there_stops_being_registered() {
        let s = Scratch::new("no32");
        s.0.apply(Feature::Dialogs, &support()).unwrap();
        let key32 = format!(r"Software\Classes\WOW6432Node\CLSID\{}\InprocServer32", DIALOG_CLSIDS[0]);
        assert!(s.0.read_text(&key32, "").is_some());
        s.0.apply(Feature::Dialogs, &Support { dll32: None, ..support() }).unwrap();
        assert_eq!(s.0.read_text(&key32, ""), None);
        assert!(s.0.is_in_place(Feature::Dialogs));
    }

    #[test]
    fn restoring_removes_all_of_it_and_nothing_else() {
        let s = Scratch::new("restore");
        s.0.write_text(r"Software\Classes\Folder\shell\other\command", "", "someone-elses.exe");
        for f in Feature::ALL {
            s.0.apply(f, &support()).unwrap();
        }
        s.0.set_program_path(Path::new(r"C:\daleris.exe"));
        s.0.set_exclusions("foo.exe");
        s.0.restore_all();
        assert!(!s.0.key_exists(OWN_KEY));
        assert!(!s.0.key_exists(r"Software\Daleris"));
        assert!(!s.0.key_exists(r"Software\Classes\Folder\shell\open"));
        assert!(!s.0.key_exists(r"Software\Classes\CLSID"));
        assert_eq!(s.0.read_text(r"Software\Classes\Folder\shell\other\command", "").as_deref(), Some("someone-elses.exe"));
    }

    #[test]
    fn a_key_two_switches_share_goes_when_the_last_one_does_in_either_order() {
        for (first, second) in [(Feature::WinE, Feature::Dialogs), (Feature::Dialogs, Feature::WinE)] {
            let s = Scratch::new(&format!("shared-{}", first.id()));
            s.0.apply(first, &support()).unwrap();
            s.0.apply(second, &support()).unwrap();
            s.0.remove(first);
            assert!(s.0.key_exists(r"Software\Classes\CLSID"), "the other switch still needs it");
            s.0.remove(second);
            assert!(!s.0.key_exists(r"Software\Classes\CLSID"));
            assert!(!s.0.key_exists(r"Software\Classes"));
        }
    }

    #[test]
    fn flags_only_exist_for_switches_without_their_own_keys() {
        let s = Scratch::new("flags");
        assert!(!s.0.flag(Feature::Select));
        s.0.apply(Feature::Select, &support()).unwrap();
        assert!(s.0.flag(Feature::Select));
        s.0.remove(Feature::Select);
        assert!(!s.0.flag(Feature::Select));
    }
}
