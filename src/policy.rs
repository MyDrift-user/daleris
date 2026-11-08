//! What administrators manage. On Windows, Group Policy (GPO and GPP); on Linux and macOS, a file only they can change.
//!
//! Windows' fixed settings: `Software\Policies\Daleris`, machine first, then user. They always win
//! and are grayed out in the app. Default settings: the `Recommended` subkey below that, then
//! `Software\Daleris\Defaults` (for preferences). They apply until the person picks their
//! own value. Values use the names in `VALUES`; `PolicyDefinitions/README.txt` lists them for admins.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

#[derive(Clone, Copy)]
enum Kind {
    Bool,
    /// a number from 0 up to this
    Max(u32),
    /// 0 details, 1 large icons
    View,
    Text,
}

/// Registry value name, settings file key, what it holds.
const VALUES: [(&str, &str, Kind); 64] = [
    ("ShowHiddenItems", "hidden", Kind::Bool),
    ("HideDotfiles", "hide_dots", Kind::Bool),
    ("ExplorerLook", "explorer_look", Kind::Bool),
    ("ExplorerColours", "explorer_colours", Kind::Bool),
    ("ExplorerLines", "explorer_lines", Kind::Bool),
    ("ExplorerMica", "explorer_mica", Kind::Bool),
    ("ExplorerMicaBar", "explorer_mica_bar", Kind::Bool),
    ("OpenNewDrives", "open_new_drives", Kind::Bool),
    ("StartNewWindow", "start_new_window", Kind::Bool),
    ("ShareTransferNotes", "share_marks", Kind::Bool),
    ("DefaultSort", "sort_key", Kind::Text),
    ("DefaultSortAscending", "sort_asc", Kind::Bool),
    ("FoldersFirst", "dirs_first", Kind::Bool),
    ("DownloadsNewestFirst", "downloads_newest", Kind::Bool),
    ("ShowPreviewPane", "preview", Kind::Bool),
    ("ShowFileExtensions", "ext", Kind::Bool),
    ("DefaultView", "view", Kind::View),
    ("Theme", "theme", Kind::Max(2)),
    ("AccentColor", "accent2", Kind::Max(7)),
    ("IconPack", "iconpack", Kind::Text),
    ("RowHeight", "density", Kind::Max(2)),
    ("CustomTitleBar", "title_custom", Kind::Bool),
    ("TitleBarLogo", "title_logo", Kind::Bool),
    ("TitleBarName", "title_name2", Kind::Bool),
    ("TitleBarSettingsButton", "title_gear", Kind::Bool),
    ("TitleBarBrandSettings", "title_brand_settings", Kind::Bool),
    ("ReopenTabs", "restore_tabs2", Kind::Bool),
    ("NewTabOpens", "new_tab", Kind::Text),
    ("WindowsPropertiesDialog", "native_props", Kind::Bool),
    ("FolderSizes", "folder_sizes", Kind::Bool),
    ("SearchSubfolders", "nested_search", Kind::Bool),
    ("SearchRegex", "regex_search", Kind::Bool),
    ("SearchSkipAppData", "search_skip_appdata", Kind::Bool),
    ("SearchSkipWindows", "search_skip_windows", Kind::Bool),
    ("SearchSkipDeveloperFiles", "search_skip_dev", Kind::Bool),
    ("SearchSkipCloudFolders", "search_skip_cloud", Kind::Bool),
    ("SearchSkipTempFiles", "search_skip_temp", Kind::Bool),
    ("SearchSkipProgramParts", "search_skip_parts", Kind::Bool),
    ("SearchSkipHidden", "search_skip_hidden", Kind::Bool),
    ("SearchSkipSystem", "search_skip_system", Kind::Bool),
    ("SearchSkipFolders", SKIP_FOLDERS, Kind::Text),
    ("SearchSkipFolderNames", SKIP_NAMES, Kind::Text),
    ("SearchSkipFileTypes", SKIP_TYPES, Kind::Text),
    ("SearchSkipPatterns", SKIP_PATTERNS, Kind::Text),
    ("AutomaticUpdates", "auto_update", Kind::Bool),
    ("RibbonLeft", "rleft", Kind::Text),
    ("RibbonRight", "rright", Kind::Text),
    ("RibbonHidden", "rhide", Kind::Text),
    ("SidebarHiddenSections", SIDEBAR_HIDDEN, Kind::Text),
    ("SidebarPins", SIDEBAR_PINS, Kind::Text),
    ("SidebarOrder", SIDEBAR_ORDER, Kind::Text),
    ("ContextMenuHidden", MENU_HIDDEN, Kind::Text),
    ("ContextMenuMoreOptions", MENU_MORE, Kind::Text),
    ("ContextMenuOrder", MENU_ORDER, Kind::Text),
    ("CustomCommands", COMMANDS, Kind::Text),
    ("ExplorerOpenFolders", "exp_folders", Kind::Bool),
    ("ExplorerWinE", "exp_win_e", Kind::Bool),
    ("ExplorerWinEHook", "exp_win_e_hook", Kind::Bool),
    ("ExplorerSelectFile", "exp_select", Kind::Bool),
    ("ExplorerSingleWindow", "exp_single", Kind::Bool),
    ("ExplorerFileDialogs", "exp_dialogs", Kind::Bool),
    ("ExplorerDialogJump", "exp_jump", Kind::Bool),
    ("ExplorerDialogLog", "exp_dialog_log", Kind::Bool),
    ("ExplorerDialogExclusions", "exp_exclude", Kind::Text),
];

const SIDEBAR_HIDDEN: &str = "sidebar_hidden";
// Settings with a form of their own, which policies set as text: see the helpers further down.
pub const SIDEBAR_PINS: &str = "sidebar_pins";
pub const SIDEBAR_ORDER: &str = "sidebar_order";
pub const MENU_HIDDEN: &str = "menu_hidden";
pub const MENU_MORE: &str = "menu_more";
pub const MENU_ORDER: &str = "menu_order";
pub const COMMANDS: &str = "custom_commands";
// What search never finds. A fixed list adds to the person's own instead of replacing it.
pub const SKIP_FOLDERS: &str = "search_skip_folders";
pub const SKIP_NAMES: &str = "search_skip_names";
pub const SKIP_TYPES: &str = "search_skip_types";
/// one settings file line per pattern, as a pattern can hold `|`
pub const SKIP_PATTERNS: &str = "search_skip_pattern";

/// Is this settings file key a list an administrator adds to, rather than one they set?
pub fn adds_to_own(key: &str) -> bool {
    matches!(key, SKIP_FOLDERS | SKIP_NAMES | SKIP_TYPES | SKIP_PATTERNS)
}

/// A list policy's items: one per line, and for all but patterns also separated by `|`.
pub fn items(key: &str, text: &str) -> Vec<String> {
    let split: &[char] = if key == SKIP_PATTERNS { &['\n'] } else { &['\n', '|'] };
    text.split(split).map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()
}

#[derive(Default, Clone, PartialEq)]
pub struct Policy {
    /// (settings file key, value) that always apply
    pub fixed: Vec<(&'static str, String)>,
    /// (settings file key, value) used until the person picks their own
    pub recommended: Vec<(&'static str, String)>,
    pub lock_ribbon: bool,
    pub lock_sidebar: bool,
    pub lock_menu: bool,
    pub no_custom_commands: bool,
    /// no looking for updates at all: the administrator deploys new versions
    pub no_updates: bool,
}

impl Policy {
    /// Can the person not change this settings file key?
    pub fn is_fixed(&self, key: &str) -> bool {
        // the person can still add to a list the administrator adds to
        (self.fixed.iter().any(|(k, _)| *k == key) && !adds_to_own(key))
            || (self.lock_ribbon && matches!(key, "rleft" | "rright" | "rhide"))
            // the settings file's own lines for sidebar pins
            || (key == "spin" && self.has_fixed(SIDEBAR_PINS))
    }

    pub fn has_fixed(&self, key: &str) -> bool {
        self.fixed.iter().any(|(k, _)| *k == key)
    }

    /// Is anything in the app managed?
    pub fn any_fixed(&self) -> bool {
        !self.fixed.is_empty() || self.lock_ribbon || self.lock_sidebar || self.lock_menu || self.no_custom_commands || self.no_updates
    }

    /// The value a policy fixes for this settings file key.
    pub fn fixed_value(&self, key: &str) -> Option<&str> {
        self.fixed.iter().find(|(k, _)| *k == key).map(|(_, v)| v.as_str())
    }

    /// The items an administrator adds to a list, on top of the person's own.
    pub fn fixed_items(&self, key: &str) -> Vec<String> {
        self.fixed_value(key).map_or_else(Vec::new, |v| items(key, v))
    }

    /// The default a policy sets for this settings file key.
    pub fn recommended_value(&self, key: &str) -> Option<&str> {
        self.recommended.iter().find(|(k, _)| *k == key).map(|(_, v)| v.as_str())
    }

    pub fn ribbon_locked(&self) -> bool {
        self.is_fixed("rleft") || self.is_fixed("rright") || self.is_fixed("rhide")
    }

    pub fn sidebar_locked(&self) -> bool {
        self.lock_sidebar || self.has_fixed(SIDEBAR_HIDDEN) || self.has_fixed(SIDEBAR_PINS) || self.has_fixed(SIDEBAR_ORDER)
    }

    /// Can the person not rearrange the right-click menu?
    pub fn menu_locked(&self) -> bool {
        self.lock_menu || self.has_fixed(MENU_HIDDEN) || self.has_fixed(MENU_MORE) || self.has_fixed(MENU_ORDER)
    }

    /// Where a policy puts a right-click menu item, if anywhere: what's fixed, or failing that the default.
    /// A fixed list leaves every item it doesn't name where Daleris puts it.
    pub fn menu_place(&self, key: &str, fixed: bool) -> Option<&'static str> {
        let value = |k| if fixed { self.fixed_value(k) } else { self.recommended_value(k) };
        let names = |k| value(k).is_some_and(|v| list(v).iter().any(|x| x == key));
        if names(MENU_HIDDEN) {
            Some("hide")
        } else if names(MENU_MORE) {
            Some("more")
        } else if fixed && (self.has_fixed(MENU_HIDDEN) || self.has_fixed(MENU_MORE)) {
            Some("default")
        } else {
            None
        }
    }

    /// Sidebar sections that policy always hides.
    pub fn section_hidden(&self, id: &str) -> bool {
        self.fixed.iter().any(|(k, v)| *k == SIDEBAR_HIDDEN && v.split('|').any(|s| s.trim() == id))
    }

    /// Sidebar sections hidden by default the first time the app runs.
    pub fn section_hidden_by_default(&self, id: &str) -> bool {
        self.recommended.iter().any(|(k, v)| *k == SIDEBAR_HIDDEN && v.split('|').any(|s| s.trim() == id))
    }
}

fn slot() -> &'static RwLock<Arc<Policy>> {
    static SLOT: OnceLock<RwLock<Arc<Policy>>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(Arc::new(read())))
}

static CHANGED: AtomicBool = AtomicBool::new(false);

pub fn current() -> Arc<Policy> {
    slot().read().unwrap().clone()
}

pub fn is_fixed(key: &str) -> bool {
    slot().read().unwrap().is_fixed(key)
}

/// True once after the policy changed while the app was running.
pub fn take_changed() -> bool {
    CHANGED.swap(false, Ordering::AcqRel)
}

/// Pick up `gpupdate` and new policies while running: check now and then.
pub fn watch(ctx: eframe::egui::Context) {
    let every = std::env::var("DALERIS_TEST_POLICY_POLL_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(30_000);
    let _ = std::thread::Builder::new().name("policy".into()).spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(every));
        let new = read();
        if *current() != new {
            *slot().write().unwrap() = Arc::new(new);
            CHANGED.store(true, Ordering::Release);
            ctx.request_repaint();
        }
    });
}

/// Builds the policy from where it's kept: what's fixed under a value name, and what's a default.
fn build(fixed: impl Fn(&str) -> Option<String>, default: impl Fn(&str) -> Option<String>) -> Policy {
    let mut p = Policy::default();
    for (name, key, kind) in VALUES {
        if let Some(v) = fixed(name).and_then(|raw| to_setting(kind, raw)) {
            p.fixed.push((key, v));
        }
        if let Some(v) = default(name).and_then(|raw| to_setting(kind, raw)) {
            p.recommended.push((key, v));
        }
    }
    let on = |name: &str| fixed(name).is_some_and(|v| v.trim() != "0" && !v.trim().is_empty());
    p.lock_ribbon = on("LockRibbon");
    p.lock_sidebar = on("LockSidebar");
    p.lock_menu = on("LockContextMenu");
    p.no_custom_commands = on("DisableCustomCommands");
    p.no_updates = on("DisableUpdates");
    p
}

#[cfg(not(windows))]
const POLICY_FILE: &str = if cfg!(target_os = "macos") { "/Library/Application Support/Daleris/policy.conf" } else { "/etc/daleris/policy.conf" };

#[cfg(not(windows))]
fn read() -> Policy {
    let mut paths = vec![POLICY_FILE.to_string()];
    // test hook: another file after the real one, so it can only add to a policy, never overrule it
    paths.extend(std::env::var("DALERIS_TEST_POLICY_FILE"));
    let files: Vec<_> = paths.iter().filter_map(|p| std::fs::read_to_string(p).ok()).map(|text| parse_file(&text)).collect();
    build(|name| files.iter().find_map(|(fixed, _)| value_of(fixed, name)), |name| files.iter().find_map(|(_, default)| value_of(default, name)))
}

/// A value from the policy file. A list takes a line per item, so its lines are joined, one item per line.
#[cfg(any(not(windows), test))]
fn value_of(lines: &[(String, String)], name: &str) -> Option<String> {
    let mut found = lines.iter().filter(|(k, _)| k == name).map(|(_, v)| v.as_str());
    let first = found.next()?;
    let list = VALUES.iter().any(|(n, k, _)| *n == name && adds_to_own(k));
    Some(if list { std::iter::once(first).chain(found).collect::<Vec<_>>().join("\n") } else { first.to_string() })
}

/// `Name=value` lines, as (fixed, default). Lines after `[Recommended]` are defaults, and `[Fixed]` goes back.
#[cfg(any(not(windows), test))]
#[allow(clippy::type_complexity)]
fn parse_file(text: &str) -> (Vec<(String, String)>, Vec<(String, String)>) {
    let (mut fixed, mut default) = (Vec::new(), Vec::new());
    let mut defaults = false;
    for line in text.lines().map(str::trim) {
        match line {
            "[Fixed]" => defaults = false,
            "[Recommended]" => defaults = true,
            _ if line.starts_with('#') => {}
            _ => {
                if let Some((k, v)) = line.split_once('=') {
                    let entry = (k.trim().to_string(), v.trim().to_string());
                    if defaults {
                        default.push(entry)
                    } else {
                        fixed.push(entry)
                    }
                }
            }
        }
    }
    (fixed, default)
}

#[cfg(windows)]
fn read() -> Policy {
    use windows::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    const POLICY: &str = r"Software\Policies\Daleris";
    const RECOMMENDED: &str = r"Software\Policies\Daleris\Recommended";
    const DEFAULTS: &str = r"Software\Daleris\Defaults";
    let mut fixed_at: Vec<(HKEY, String)> = vec![(HKEY_LOCAL_MACHINE, POLICY.into()), (HKEY_CURRENT_USER, POLICY.into())];
    let mut default_at: Vec<(HKEY, String)> = vec![
        (HKEY_LOCAL_MACHINE, RECOMMENDED.into()),
        (HKEY_CURRENT_USER, RECOMMENDED.into()),
        (HKEY_LOCAL_MACHINE, DEFAULTS.into()),
        (HKEY_CURRENT_USER, DEFAULTS.into()),
    ];
    // test hook: extra keys after the real ones, so it can only add to a policy, never overrule it
    if let Ok(test) = std::env::var("DALERIS_TEST_POLICY_KEY") {
        fixed_at.push((HKEY_CURRENT_USER, test.clone()));
        default_at.push((HKEY_CURRENT_USER, format!(r"{test}\Recommended")));
        default_at.push((HKEY_CURRENT_USER, format!(r"{test}\Defaults")));
    }
    let first = |at: &[(HKEY, String)], name: &str| at.iter().find_map(|(root, key)| reg_value(*root, key, name));
    build(|name| first(&fixed_at, name), |name| first(&default_at, name))
}

// ---------------------------------------------------------------- settings with a form of their own, as text

/// Items separated by `|`, without blanks.
pub fn list(text: &str) -> Vec<String> {
    text.split('|').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()
}

/// `%NAME%` replaced by that environment variable, and on Linux and macOS a leading `~` by the home folder.
pub fn expand(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%').and_then(|end| std::env::var(&after[..end]).ok().map(|v| (end, v))) {
            Some((end, value)) => {
                out.push_str(&value);
                rest = &after[end + 1..];
            }
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    if cfg!(not(windows)) {
        if let (Some(tail), Some(home)) = (out.strip_prefix('~'), std::env::var_os("HOME")) {
            if tail.is_empty() || tail.starts_with('/') {
                return format!("{}{tail}", home.to_string_lossy());
            }
        }
    }
    out
}

/// Sidebar pins: `section>folder` items, or just `folder` for Quick access.
pub fn pins(text: &str) -> Vec<(String, std::path::PathBuf)> {
    list(text)
        .iter()
        .map(|item| match item.split_once('>') {
            Some((section, path)) => (section.trim().to_string(), std::path::PathBuf::from(expand(path.trim()))),
            None => ("quick".to_string(), std::path::PathBuf::from(expand(item))),
        })
        .filter(|(_, p)| !p.as_os_str().is_empty())
        .collect()
}

/// Custom commands: `name>command` items, or `name>which items>command`, where which items is all, files, folders,
/// background or ext:jpg,png.
pub fn commands(text: &str) -> Vec<(String, String, String)> {
    list(text)
        .iter()
        .filter_map(|item| {
            let parts: Vec<&str> = item.splitn(3, '>').map(str::trim).collect();
            match parts.as_slice() {
                [name, command] => Some((name.to_string(), "all".to_string(), command.to_string())),
                [name, applies, command] => Some((name.to_string(), applies.to_string(), command.to_string())),
                _ => None,
            }
        })
        .filter(|(name, _, command)| !name.is_empty() && !command.is_empty())
        .collect()
}

/// A policy's text as the text the settings file would hold, if it is valid.
fn to_setting(kind: Kind, raw: String) -> Option<String> {
    let num = || raw.trim().parse::<u32>().ok();
    match kind {
        Kind::Bool => num().map(|n| if n != 0 { "1".to_string() } else { "0".to_string() }),
        Kind::Max(max) => num().filter(|n| *n <= max).map(|n| n.to_string()),
        Kind::View => match num() {
            Some(0) => Some("details".into()),
            Some(1) => Some("icons".into()),
            _ => None,
        },
        Kind::Text => Some(raw),
    }
}

/// A DWORD or string value as text (preferences can write either). A multi-string value comes back one item per line.
#[cfg(windows)]
fn reg_value(root: windows::Win32::System::Registry::HKEY, key: &str, name: &str) -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::*;
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (key, name) = (wide(key), wide(name));
    // room for a long list of folders or patterns
    let mut buf = vec![0u16; 32 * 1024];
    let mut len = (buf.len() * 2) as u32;
    let mut ty = REG_VALUE_TYPE(0);
    let rc = unsafe { RegGetValueW(root, PCWSTR(key.as_ptr()), PCWSTR(name.as_ptr()), RRF_RT_REG_DWORD | RRF_RT_REG_SZ | RRF_RT_REG_MULTI_SZ, Some(&mut ty), Some(buf.as_mut_ptr() as *mut _), Some(&mut len)) };
    if rc.is_err() {
        return None;
    }
    if ty == REG_DWORD {
        return Some((buf[0] as u32 | (buf[1] as u32) << 16).to_string());
    }
    let chars = (len as usize / 2).min(buf.len());
    let text = String::from_utf16_lossy(&buf[..chars]);
    if ty == REG_MULTI_SZ {
        return Some(text.split('\0').filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n"));
    }
    Some(text.trim_end_matches('\0').to_string())
}

/// Is this settings file key one a policy can set?
#[cfg(test)]
pub fn covers(key: &str) -> bool {
    VALUES.iter().any(|(_, k, _)| *k == key)
}

/// The value name a settings file key goes by.
#[cfg(test)]
pub fn name_of(key: &str) -> Option<&'static str> {
    VALUES.iter().find(|(_, k, _)| *k == key).map(|(n, _, _)| *n)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADMX: &str = include_str!("../PolicyDefinitions/Daleris.admx");
    const LOCKS: [&str; 5] = ["LockRibbon", "LockSidebar", "LockContextMenu", "DisableCustomCommands", "DisableUpdates"];

    #[test]
    fn the_templates_offer_every_value_as_fixed_and_as_default() {
        for (name, _, _) in VALUES {
            assert!(ADMX.contains(&format!(r#"name="Fixed_{name}""#)), "no fixed policy for {name}");
            assert!(ADMX.contains(&format!(r#"name="Rec_{name}""#)), "no default policy for {name}");
        }
        for name in LOCKS {
            assert!(ADMX.contains(&format!(r#"<policy name="{name}""#)), "no policy for {name}");
        }
    }

    #[test]
    fn the_templates_name_everything_they_use_in_every_language() {
        let en = include_str!("../PolicyDefinitions/en-US/Daleris.adml");
        let de = include_str!("../PolicyDefinitions/de-DE/Daleris.adml");
        for (from, to) in [("$(string.", r#"<string id=""#), ("$(presentation.", r#"<presentation id=""#)] {
            for used in ADMX.split(from).skip(1).map(|s| &s[..s.find(')').unwrap()]) {
                for (language, text) in [("en-US", en), ("de-DE", de)] {
                    assert!(text.contains(&format!(r#"{to}{used}""#)), "{language} has no {used}");
                }
            }
        }
    }

    #[test]
    fn the_admin_guide_lists_every_value() {
        let guide = include_str!("../PolicyDefinitions/README.txt");
        for name in VALUES.iter().map(|(name, _, _)| *name).chain(LOCKS) {
            assert!(guide.contains(name), "PolicyDefinitions/README.txt doesn't mention {name}");
        }
    }

    #[test]
    fn pins_and_commands_are_read_from_text() {
        let p = pins(" C:\\Users\\%CARGO_PKG_NAME%\\Work | network>\\\\server\\share ||");
        assert_eq!(p, [("quick".to_string(), "C:\\Users\\daleris\\Work".into()), ("network".to_string(), "\\\\server\\share".into())]);
        assert_eq!(expand("%NOT_SET_ANYWHERE%\\x"), "%NOT_SET_ANYWHERE%\\x", "an unknown variable stays as written");
        let c = commands("Edit>code \"{path}\" | Resize>ext:jpg,png>magick {path} -resize 50% | nonsense | >empty");
        assert_eq!(c, [("Edit".to_string(), "all".to_string(), "code \"{path}\"".to_string()), ("Resize".to_string(), "ext:jpg,png".to_string(), "magick {path} -resize 50%".to_string())]);
    }

    #[test]
    fn menu_items_go_where_the_policy_puts_them() {
        let p = Policy { fixed: vec![(MENU_HIDDEN, "app:terminal|v:open".into())], recommended: vec![(MENU_MORE, "app:sort".into())], ..Default::default() };
        assert_eq!(p.menu_place("v:open", true), Some("hide"));
        assert_eq!(p.menu_place("app:newtab", true), Some("default"), "a fixed list sets the whole menu");
        assert_eq!(p.menu_place("app:sort", false), Some("more"));
        assert_eq!(p.menu_place("app:newtab", false), None);
        assert!(p.menu_locked());
        assert!(!Policy { recommended: vec![(MENU_HIDDEN, "app:sort".into())], ..Default::default() }.menu_locked(), "a default leaves it open");
        assert!(Policy { fixed: vec![(SIDEBAR_PINS, "C:\\x".into())], ..Default::default() }.is_fixed("spin"));
    }

    #[test]
    fn a_policy_file_is_read() {
        let (fixed, default) = parse_file("# a comment\nAutomaticUpdates = 0\nnonsense\n[Recommended]\nTheme=2\n[Fixed]\nDisableUpdates=1\n");
        assert_eq!(fixed, [("AutomaticUpdates".to_string(), "0".to_string()), ("DisableUpdates".to_string(), "1".to_string())]);
        assert_eq!(default, [("Theme".to_string(), "2".to_string())]);
    }

    #[test]
    fn search_lists_are_read_from_lines_and_pipes() {
        assert_eq!(items(SKIP_NAMES, "node_modules | .git\nbuild\n\n"), ["node_modules", ".git", "build"]);
        assert_eq!(items(SKIP_PATTERNS, r"\.(key|pem)$"), [r"\.(key|pem)$"], "a pattern keeps its |");
        let (fixed, default) = parse_file("SearchSkipPatterns=\\\\Payroll\\\\\nSearchSkipPatterns=\\.pfx$\nTheme=1\nTheme=2\n[Recommended]\nSearchSkipFileTypes=iso\n");
        assert_eq!(value_of(&fixed, "SearchSkipPatterns").as_deref(), Some("\\\\Payroll\\\\\n\\.pfx$"), "a line per item");
        assert_eq!(value_of(&fixed, "Theme").as_deref(), Some("1"), "anything else: the first line");
        assert_eq!(value_of(&default, "SearchSkipFileTypes").as_deref(), Some("iso"));
    }

    #[test]
    fn a_fixed_search_list_adds_to_the_persons_own() {
        let p = build(|name| (name == "SearchSkipFolderNames").then(|| "Payroll|HR".to_string()), |_| None);
        assert_eq!(p.fixed_items(SKIP_NAMES), ["Payroll", "HR"]);
        assert!(!p.is_fixed(SKIP_NAMES), "the person can still add their own");
        assert!(p.any_fixed());
    }

    #[test]
    fn values_that_make_no_sense_are_ignored() {
        let p = build(|name| ["Theme", "RowHeight", "AutomaticUpdates"].contains(&name).then(|| "9".to_string()), |_| None);
        assert_eq!(p.fixed_value("theme"), None);
        assert_eq!(p.fixed_value("density"), None);
        assert_eq!(p.fixed_value("auto_update"), Some("1"));
    }
}
