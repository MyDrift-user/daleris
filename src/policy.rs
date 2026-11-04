//! Group Policy (GPO and GPP) for the Windows build.
//!
//! Fixed settings: `Software\Policies\Daleris`, machine first, then user. They always win
//! and are grayed out in the app. Default settings: the `Recommended` subkey below that, then
//! `Software\Daleris\Defaults` (for preferences). They apply until the person picks their
//! own value. Values use the names in `VALUES`; `policy/README.txt` lists them for admins.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

#[derive(Clone, Copy)]
#[cfg_attr(not(windows), allow(dead_code))]
enum Kind {
    Bool,
    /// a number from 0 up to this
    Max(u32),
    /// 0 details, 1 large icons
    View,
    Text,
}

/// Registry value name, settings file key, what it holds.
#[cfg_attr(not(windows), allow(dead_code))]
const VALUES: [(&str, &str, Kind); 22] = [
    ("ShowHiddenItems", "hidden", Kind::Bool),
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
    ("ReopenTabs", "restore_tabs2", Kind::Bool),
    ("WindowsPropertiesDialog", "native_props", Kind::Bool),
    ("FolderSizes", "folder_sizes", Kind::Bool),
    ("SearchSubfolders", "nested_search", Kind::Bool),
    ("SearchRegex", "regex_search", Kind::Bool),
    ("AutomaticUpdates", "auto_update", Kind::Bool),
    ("RibbonLeft", "rleft", Kind::Text),
    ("RibbonRight", "rright", Kind::Text),
    ("RibbonHidden", "rhide", Kind::Text),
    ("SidebarHiddenSections", SIDEBAR_HIDDEN, Kind::Text),
];

const SIDEBAR_HIDDEN: &str = "sidebar_hidden";

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
}

impl Policy {
    /// Can the person not change this settings file key?
    pub fn is_fixed(&self, key: &str) -> bool {
        self.fixed.iter().any(|(k, _)| *k == key) || (self.lock_ribbon && matches!(key, "rleft" | "rright" | "rhide"))
    }

    pub fn has_fixed(&self, key: &str) -> bool {
        self.fixed.iter().any(|(k, _)| *k == key)
    }

    /// Is anything in the app managed?
    pub fn any_fixed(&self) -> bool {
        !self.fixed.is_empty() || self.lock_ribbon || self.lock_sidebar || self.lock_menu || self.no_custom_commands
    }

    pub fn ribbon_locked(&self) -> bool {
        self.is_fixed("rleft") || self.is_fixed("rright") || self.is_fixed("rhide")
    }

    pub fn sidebar_locked(&self) -> bool {
        self.lock_sidebar || self.has_fixed(SIDEBAR_HIDDEN)
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

/// Pick up `gpupdate` and new policies while running: check the registry now and then.
pub fn watch(ctx: eframe::egui::Context) {
    if !cfg!(windows) {
        return;
    }
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

#[cfg(not(windows))]
fn read() -> Policy {
    Policy::default()
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
    let mut p = Policy::default();
    for (name, key, kind) in VALUES {
        if let Some(v) = first(&fixed_at, name).and_then(|raw| to_setting(kind, raw)) {
            p.fixed.push((key, v));
        }
        if let Some(v) = first(&default_at, name).and_then(|raw| to_setting(kind, raw)) {
            p.recommended.push((key, v));
        }
    }
    let on = |name: &str| first(&fixed_at, name).is_some_and(|v| v.trim() != "0" && !v.trim().is_empty());
    p.lock_ribbon = on("LockRibbon");
    p.lock_sidebar = on("LockSidebar");
    p.lock_menu = on("LockContextMenu");
    p.no_custom_commands = on("DisableCustomCommands");
    p
}

/// A registry value as the text the settings file would hold, if it is valid.
#[cfg(windows)]
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

/// A DWORD or string value as text (preferences can write either).
#[cfg(windows)]
fn reg_value(root: windows::Win32::System::Registry::HKEY, key: &str, name: &str) -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::*;
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (key, name) = (wide(key), wide(name));
    let mut buf = [0u16; 2048];
    let mut len = std::mem::size_of_val(&buf) as u32;
    let mut ty = REG_VALUE_TYPE(0);
    let rc = unsafe { RegGetValueW(root, PCWSTR(key.as_ptr()), PCWSTR(name.as_ptr()), RRF_RT_REG_DWORD | RRF_RT_REG_SZ, Some(&mut ty), Some(buf.as_mut_ptr() as *mut _), Some(&mut len)) };
    if rc.is_err() {
        return None;
    }
    if ty == REG_DWORD {
        return Some((buf[0] as u32 | (buf[1] as u32) << 16).to_string());
    }
    let chars = (len as usize / 2).min(buf.len());
    Some(String::from_utf16_lossy(&buf[..chars]).trim_end_matches('\0').to_string())
}
