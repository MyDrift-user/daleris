use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, pos2, vec2, Align2, Color32, CornerRadius, FontId, Key, Rect, Response, Sense, Stroke, StrokeKind, TextureHandle, Ui};

use crate::dirsize::DirSize;
use crate::fsx::{self, Cloud, CloudRoot, Drive, DriveKind, Entry, Place};
use crate::policy;
use crate::search::{self, Index, Query, Search};
use crate::shell::{self, ImgReq, Loader, Op};
use crate::theme::*;
use crate::util::*;

mod actions;
mod chrome;
mod file_icons;
mod list;
mod menu;
mod props;
mod settings_page;
mod sidebar;

use actions::*;
use file_icons::{Art, IconFor};

// ====================================================================== model

#[derive(Clone, PartialEq, Debug)]
pub enum Loc {
    ThisPc,
    Settings,
    Dir(PathBuf),
}

impl Loc {
    fn path(&self) -> Option<&Path> {
        match self {
            Loc::Dir(p) => Some(p),
            _ => None,
        }
    }
    fn title(&self) -> String {
        match self {
            Loc::ThisPc => "This PC".into(),
            Loc::Settings => "Settings".into(),
            Loc::Dir(p) => match p.file_name() {
                Some(n) => n.to_string_lossy().to_string(),
                None => p.to_string_lossy().trim_end_matches('\\').to_string(),
            },
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortKey {
    Name,
    Modified,
    Type,
    Size,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Details,
    Icons,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Subfolders,
    ThisFolder,
    AllDrives,
}

struct Rename {
    key: String,
    path: PathBuf,
    buf: String,
    focus: bool,
}

struct Tab {
    id: u64,
    loc: Loc,
    back: Vec<Loc>,
    fwd: Vec<Loc>,
    entries: Vec<Entry>,
    incoming: Vec<Entry>,
    view: Vec<u32>,
    dirty: bool,
    loading: bool,
    refreshing: bool,
    gen: u64,
    cancel: Arc<AtomicBool>,
    load_start: Instant,
    load_time: Option<Duration>,
    error: Option<String>,
    sel: HashSet<String>,
    sel_info: (usize, u64),
    cursor: Option<usize>,
    anchor: Option<usize>,
    scroll_to: Option<usize>,
    sort: SortKey,
    asc: bool,
    rename: Option<Rename>,
    query: String,
    scope: Scope,
    search: Option<Search>,
    watcher: Option<fsx::Watcher>,
    watched: Option<PathBuf>,
    refresh_at: Option<Instant>,
    select_after_load: Option<String>,
    in_cloud: bool,
    typeahead: (String, Instant),
    /// why the search text can't be used (an invalid regex)
    query_error: Option<String>,
    /// how many entries of each type (extension, is folder) the view shows: sizes the Type column
    type_stats: Vec<(String, bool, usize)>,
}

impl Tab {
    fn new(id: u64, loc: Loc) -> Tab {
        Tab {
            id,
            loc,
            back: Vec::new(),
            fwd: Vec::new(),
            entries: Vec::new(),
            incoming: Vec::new(),
            view: Vec::new(),
            dirty: false,
            loading: false,
            refreshing: false,
            gen: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            load_start: Instant::now(),
            load_time: None,
            error: None,
            sel: HashSet::new(),
            sel_info: (0, 0),
            cursor: None,
            anchor: None,
            scroll_to: None,
            sort: SortKey::Name,
            asc: true,
            rename: None,
            query: String::new(),
            scope: Scope::Subfolders,
            search: None,
            watcher: None,
            watched: None,
            refresh_at: None,
            select_after_load: None,
            in_cloud: false,
            typeahead: (String::new(), Instant::now()),
            query_error: None,
            type_stats: Vec::new(),
        }
    }
    fn searching(&self) -> bool {
        self.search.is_some()
    }
    fn dir(&self) -> Option<&Path> {
        self.loc.path()
    }
}

enum Msg {
    Batch { tab: u64, gen: u64, batch: Vec<Entry> },
    Done { tab: u64, gen: u64, err: Option<String> },
    Changed { tab: u64 },
    Drives(Vec<Drive>),
    DriveDetail(Drive),
    Clouds(Vec<CloudRoot>),
    OpDone(Result<(), String>),
    /// a server was mounted (Linux/macOS): where it appeared
    #[allow(dead_code)]
    Connected(Result<PathBuf, String>),
    PropSize { gen: u64, bytes: u64, files: usize, dirs: usize, done: bool },
    Preview { gen: u64, content: PvContent },
}

#[derive(Clone)]
enum PvContent {
    Loading,
    Text { text: String, truncated: bool, lines: usize },
    Binary,
    Folder { files: usize, dirs: usize, size: u64 },
    CloudOnly,
    Nothing,
}

struct Preview {
    path: Option<PathBuf>,
    entry: Option<Entry>,
    gen: u64,
    content: PvContent,
}

#[derive(Clone)]
struct Row {
    key: String,
    name: String,
    ext: String,
    is_dir: bool,
    size: u64,
    mtime: u64,
    attrs: u32,
    path: PathBuf,
    folder: Option<String>,
}

#[derive(Clone)]
struct DragFiles(Vec<PathBuf>);

#[derive(Clone)]
enum Act {
    OpenMany(Vec<PathBuf>),
    TerminalAt(PathBuf),
    PinTo(String, PathBuf),
    NewSectionWith(PathBuf),
    PasteInto(PathBuf),
    SetView(View),
    ToggleHidden,
    TogglePreview,
    OpenSettings(Option<settings_page::Page>),
    NativeProperties(PathBuf),
    /// mount a server address (Linux/macOS): url, user, password
    #[allow(dead_code)]
    ConnectServer(String, String, String),
    CustomizeMenu,
    Navigate(Loc),
    NavigateNewTab(Loc),
    Open(PathBuf),
    OpenWith(PathBuf),
    RunAdmin(PathBuf),
    OpenLocation(PathBuf),
    Back,
    Forward,
    Up,
    Refresh,
    NewTab,
    CloseTab(usize),
    SwitchTab(usize),
    Copy,
    Cut,
    Paste,
    CopyPath,
    StartRename,
    CommitRename,
    Delete(bool),
    NewFolder,
    NewTextFile,
    Properties(PathBuf),
    NativeMenu(bool),
    Unpin(PathBuf),
    KeepLocal(bool),
    Terminal,
    MapDrive,
    Disconnect(char),
    SelectAll,
    Drop { srcs: Vec<PathBuf>, dest: PathBuf, copy: bool },
    Import(Vec<PathBuf>),
    ShellInvoke(u32),
    CopyContents(PathBuf),
    OpenTarget(PathBuf),
    NewWindow(PathBuf),
    RunCustom(usize),
    Sort(SortKey, Option<bool>),
}

enum Slot {
    Pending,
    Ready(TextureHandle),
    Failed,
}

struct Shot {
    path: PathBuf,
    frames: u32,
    requested: bool,
}

/// Where a right-click menu item goes: in the menu, in its "Show more options" submenu, or nowhere.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MenuMode {
    Show,
    More,
    Hide,
}

impl MenuMode {
    fn word(self) -> &'static str {
        match self {
            MenuMode::Show => "show",
            MenuMode::More => "more",
            MenuMode::Hide => "hide",
        }
    }
    fn from_word(s: &str) -> Option<MenuMode> {
        Some(match s {
            "show" => MenuMode::Show,
            "more" => MenuMode::More,
            "hide" => MenuMode::Hide,
            _ => return None,
        })
    }
}

/// A user-defined right-click command, e.g. `"C:\Tools\app.exe" {path}`.
#[derive(Clone)]
struct CustomCmd {
    label: String,
    command: String,
    /// all | files | folders | background | ext:jpg,png
    applies: String,
}

/// A sidebar section: built-in ones ("quick", "cloud", "pc", "network") list live
/// places/drives plus pins; custom ones list only their pinned folders.
#[derive(Clone)]
struct SectionCfg {
    id: String,
    title: String,
    visible: bool,
    collapsed: bool,
    pins: Vec<PathBuf>,
}

impl SectionCfg {
    fn builtin(&self) -> bool {
        matches!(self.id.as_str(), "quick" | "cloud" | "pc" | "network")
    }
    fn accepts_pins(&self) -> bool {
        self.id == "quick" || !self.builtin()
    }
}

fn default_sections() -> Vec<SectionCfg> {
    [("quick", "Quick access"), ("cloud", "Cloud drives"), ("pc", "This PC"), ("network", "Network")]
        .iter()
        .map(|(id, t)| SectionCfg { id: id.to_string(), title: t.to_string(), visible: true, collapsed: false, pins: Vec::new() })
        .collect()
}

pub struct Settings {
    show_hidden: bool,
    show_preview: bool,
    show_ext: bool,
    view: View,
    tabs: Vec<PathBuf>,
    menu_rules: HashMap<String, (MenuMode, String)>,
    /// user order of right-click menu items (keys)
    menu_order: Vec<String>,
    custom: Vec<CustomCmd>,
    /// 0 = follow Windows, 1 = dark, 2 = light
    theme: u8,
    /// index into ACCENTS; ACCENTS.len() = Windows accent colour
    accent: usize,
    /// "system", "flat", or the name of a folder in the icon packs folder
    icon_pack: String,
    /// 0 compact, 1 normal, 2 comfortable
    density: u8,
    title_custom: bool,
    title_logo: bool,
    title_name: bool,
    title_gear: bool,
    sections: Vec<SectionCfg>,
    side_hidden: HashSet<String>,
    side_order: HashMap<String, Vec<String>>,
    restore_tabs: bool,
    /// use the Windows Properties dialog instead of ours
    native_props: bool,
    /// show each folder's total size (counted in the background)
    folder_sizes: bool,
    /// details-view column widths you set (column id to width)
    col_w: HashMap<String, f32>,
    /// search box 'Subfolders' switch (on = search every folder below)
    nested_search: bool,
    /// search box '.*' switch: the whole search text is a regular expression
    regex_search: bool,
    /// the ribbon bar: the items on each side, left to right, and the ones switched off
    ribbon_left: Vec<String>,
    ribbon_right: Vec<String>,
    ribbon_hidden: HashSet<String>,
    /// lines of the settings file for values a policy fixes right now, written back unchanged
    kept: Vec<String>,
}

/// Everything the ribbon bar can show: id, icon, name in Settings.
const RIBBON_ITEMS: [(&str, &str, &str); 14] = [
    ("new", I_ADD, "New"),
    ("sep1", I_GRIP, "Divider"),
    ("cut", I_CUT, "Cut"),
    ("copy", I_COPY, "Copy"),
    ("paste", I_PASTE, "Paste"),
    ("rename", I_RENAME, "Rename"),
    ("delete", I_DELETE, "Delete"),
    ("sep2", I_GRIP, "Divider"),
    ("sort", I_SORT, "Sort"),
    ("view", I_LIST, "View"),
    ("more", I_MORE, "Three dots (Windows menu for this folder)"),
    ("terminal", I_TERMINAL, "Open in Terminal"),
    ("hidden", I_EYE, "Show hidden items"),
    ("preview", I_PANE, "Preview pane"),
];
const RIBBON_LEFT: [&str; 10] = ["new", "sep1", "cut", "copy", "paste", "rename", "delete", "sep2", "sort", "view"];
const RIBBON_RIGHT: [&str; 4] = ["more", "terminal", "hidden", "preview"];

/// Test hook: DALERIS_TEST_SETTINGS=1 makes a screenshot run read and write the settings file like a
/// normal run (point APPDATA at a scratch folder first).
fn test_real_settings() -> bool {
    std::env::var_os("DALERIS_TEST_SETTINGS").is_some()
}

/// Top of the settings file. Holds no equals sign, as lines without one are skipped when reading.
const SETTINGS_HEADER: &str = "# Daleris settings. Open windows read this file again when they get focus.\n";

/// How a This PC tab is written in the settings file.

const THIS_PC_TAB: &str = "::thispc";

/// Should the window start without the Windows frame (our own title bar)?
/// Decided before the window exists so the frame never has to be swapped at start-up.
pub fn starts_with_custom_titlebar(screenshot: bool) -> bool {
    if std::env::var("DALERIS_TEST_NATIVE_TITLE").is_ok() {
        return false;
    }
    if screenshot && !test_real_settings() {
        Settings::defaults().title_custom
    } else {
        Settings::load().title_custom
    }
}

impl Settings {
    fn defaults() -> Settings {
        Settings {
            show_hidden: false,
            show_preview: true,
            show_ext: true,
            view: View::Details,
            tabs: Vec::new(),
            menu_rules: HashMap::new(),
            menu_order: Vec::new(),
            custom: Vec::new(),
            theme: 1,
            accent: ACCENTS.len(),
            icon_pack: if prefers_bundled_icons() { "flat" } else { "system" }.into(),
            density: 1,
            title_custom: true,
            title_logo: true,
            title_name: false,
            title_gear: true,
            sections: default_sections(),
            side_hidden: HashSet::new(),
            side_order: HashMap::new(),
            restore_tabs: false,
            native_props: false,
            folder_sizes: true,
            col_w: HashMap::new(),
            nested_search: true,
            regex_search: false,
            ribbon_left: RIBBON_LEFT.iter().map(|s| s.to_string()).collect(),
            ribbon_right: RIBBON_RIGHT.iter().map(|s| s.to_string()).collect(),
            ribbon_hidden: HashSet::new(),
            kept: Vec::new(),
        }
    }

    /// What one side of the ribbon bar shows: no switched-off items, and no divider at
    /// the start, at the end or right after another divider.
    fn ribbon_shown(&self, side: &[String]) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for id in side {
            if self.ribbon_hidden.contains(id) || (id == "more" && !cfg!(windows)) {
                continue;
            }
            if id.starts_with("sep") && v.last().is_none_or(|l| l.starts_with("sep")) {
                continue;
            }
            v.push(id.clone());
        }
        if v.last().is_some_and(|l| l.starts_with("sep")) {
            v.pop();
        }
        v
    }

    /// Keep each item on the bar exactly once, whatever the settings file says.
    /// A layout fixed by policy shows only what it lists.
    fn fix_ribbon(&mut self) {
        let known = |id: &String| RIBBON_ITEMS.iter().any(|(k, _, _)| k == id);
        let mut seen: HashSet<String> = HashSet::new();
        self.ribbon_left.retain(|id| known(id) && seen.insert(id.clone()));
        self.ribbon_right.retain(|id| known(id) && seen.insert(id.clone()));
        let pol = policy::current();
        if pol.has_fixed("rleft") || pol.has_fixed("rright") {
            return;
        }
        for id in RIBBON_LEFT {
            if !seen.contains(id) {
                self.ribbon_left.push(id.to_string());
            }
        }
        for id in RIBBON_RIGHT {
            if !seen.contains(id) {
                self.ribbon_right.push(id.to_string());
            }
        }
    }

    fn row_h(&self) -> f32 {
        [24.0, 28.0, 34.0][self.density.min(2) as usize]
    }

    fn palette(&self) -> Palette {
        let dark = match self.theme {
            0 => system_dark(),
            2 => false,
            _ => true,
        };
        let accent = match ACCENTS.get(self.accent) {
            Some(a) => {
                if dark {
                    a.1
                } else {
                    a.2
                }
            }
            None => system_accent().unwrap_or(ACCENTS[0].1),
        };
        Palette::new(dark, accent)
    }

    fn section_mut(&mut self, id: &str) -> Option<&mut SectionCfg> {
        self.sections.iter_mut().find(|s| s.id == id)
    }

    fn is_pinned(&self, p: &Path) -> bool {
        self.sections.iter().any(|s| s.pins.iter().any(|x| fsx::same_path(x, p)))
    }
    fn file() -> Option<PathBuf> {
        config_dir().map(|d| d.join("settings.txt"))
    }
    /// Set one simple value from its text in the settings file; false if `k` is not one of them.
    fn set_value(&mut self, k: &str, v: &str) -> bool {
        let on = v == "1";
        let list = || v.split('|').map(str::trim).filter(|k| !k.is_empty()).map(String::from);
        match k {
            "hidden" => self.show_hidden = on,
            "preview" => self.show_preview = on,
            "ext" => self.show_ext = on,
            "view" => self.view = if v == "icons" { View::Icons } else { View::Details },
            "theme" => self.theme = v.parse().unwrap_or(1),
            "accent2" => self.accent = v.parse().unwrap_or(ACCENTS.len()),
            "native_props" => self.native_props = on,
            "folder_sizes" => self.folder_sizes = on,
            "nested_search" => self.nested_search = on,
            "regex_search" => self.regex_search = on,
            "rleft" => self.ribbon_left = list().collect(),
            "rright" => self.ribbon_right = list().collect(),
            "rhide" => self.ribbon_hidden = list().collect(),
            "iconpack" => self.icon_pack = if v == "system" && prefers_bundled_icons() { "flat".to_string() } else { v.to_string() },
            "density" => self.density = v.parse().unwrap_or(1),
            "title_custom" => self.title_custom = on,
            "title_logo" => self.title_logo = on,
            "title_name2" => self.title_name = on,
            "title_gear" => self.title_gear = on,
            "restore_tabs2" => self.restore_tabs = on,
            _ => return false,
        }
        true
    }

    /// The simple values as the settings file writes them.
    fn value_lines(&self) -> [(&'static str, String); 20] {
        let b = |x: bool| (x as u8).to_string();
        let mut off: Vec<&str> = self.ribbon_hidden.iter().map(|s| s.as_str()).collect();
        off.sort();
        [
            ("hidden", b(self.show_hidden)),
            ("preview", b(self.show_preview)),
            ("ext", b(self.show_ext)),
            ("view", if self.view == View::Icons { "icons".to_string() } else { "details".to_string() }),
            ("theme", self.theme.to_string()),
            ("accent2", self.accent.to_string()),
            ("iconpack", self.icon_pack.clone()),
            ("density", self.density.to_string()),
            ("title_custom", b(self.title_custom)),
            ("title_logo", b(self.title_logo)),
            ("title_name2", b(self.title_name)),
            ("title_gear", b(self.title_gear)),
            ("restore_tabs2", b(self.restore_tabs)),
            ("native_props", b(self.native_props)),
            ("folder_sizes", b(self.folder_sizes)),
            ("nested_search", b(self.nested_search)),
            ("regex_search", b(self.regex_search)),
            ("rleft", self.ribbon_left.join("|")),
            ("rright", self.ribbon_right.join("|")),
            ("rhide", off.join("|")),
        ]
    }

    /// What a value is when the person never chose one: built in, or the policy default.
    fn baseline() -> Settings {
        let mut s = Settings::defaults();
        for (k, v) in &policy::current().recommended {
            s.set_value(k, v);
        }
        s.fix_ribbon();
        s
    }

    /// Put every fixed policy value back. Runs each frame, so nothing can change one for long.
    fn enforce(&mut self, pol: &policy::Policy) {
        if !pol.any_fixed() {
            return;
        }
        for (k, v) in &pol.fixed {
            self.set_value(k, v);
        }
        if pol.lock_ribbon {
            let b = Settings::baseline();
            if !pol.has_fixed("rleft") {
                self.ribbon_left = b.ribbon_left;
            }
            if !pol.has_fixed("rright") {
                self.ribbon_right = b.ribbon_right;
            }
            if !pol.has_fixed("rhide") {
                self.ribbon_hidden = b.ribbon_hidden;
            }
        }
        if pol.ribbon_locked() {
            self.fix_ribbon();
        }
    }

    fn load() -> Settings {
        let pol = policy::current();
        let mut s = Settings::baseline();
        let mut sections: Vec<SectionCfg> = Vec::new();
        let mut legacy_pins: Vec<PathBuf> = Vec::new();
        let mut spins: Vec<(String, PathBuf)> = Vec::new();
        if let Some(text) = Self::file().and_then(|f| std::fs::read_to_string(f).ok()) {
            for line in text.lines() {
                let Some((k, v)) = line.split_once('=') else { continue };
                // the person's own choice waits underneath a fixed policy
                if pol.is_fixed(k) {
                    s.kept.push(line.to_string());
                    continue;
                }
                if s.set_value(k, v) {
                    continue;
                }
                match k {
                    "colw" => {
                        for part in v.split(';') {
                            if let Some((id, w)) = part.split_once(':') {
                                if let Ok(w) = w.parse::<f32>() {
                                    s.col_w.insert(id.to_string(), w.clamp(30.0, 3000.0));
                                }
                            }
                        }
                    }
                    "pin" => legacy_pins.push(PathBuf::from(v)),
                    "section" => {
                        let f: Vec<&str> = v.splitn(4, '\t').collect();
                        if f.len() == 4 {
                            sections.push(SectionCfg { id: f[0].into(), visible: f[1] == "1", collapsed: f[2] == "1", title: f[3].into(), pins: Vec::new() });
                        }
                    }
                    "spin" => {
                        if let Some((id, p)) = v.split_once('\t') {
                            spins.push((id.into(), PathBuf::from(p)));
                        }
                    }
                    "shide" => {
                        s.side_hidden.insert(v.to_string());
                    }
                    "sorder" => {
                        if let Some((id, keys)) = v.split_once('\t') {
                            s.side_order.insert(id.into(), keys.split('|').filter(|k| !k.is_empty()).map(String::from).collect());
                        }
                    }
                    "morder" => s.menu_order = v.split('|').filter(|k| !k.is_empty()).map(String::from).collect(),
                    "tab" => s.tabs.push(PathBuf::from(v)),
                    "menu" => {
                        let mut it = v.splitn(3, '\t');
                        if let (Some(m), Some(key)) = (it.next().and_then(MenuMode::from_word), it.next()) {
                            s.menu_rules.insert(key.to_string(), (m, it.next().unwrap_or(key).to_string()));
                        }
                    }
                    "cmd" => {
                        let mut it = v.splitn(3, '\t');
                        if let (Some(applies), Some(label), Some(command)) = (it.next(), it.next(), it.next()) {
                            s.custom.push(CustomCmd { label: label.into(), command: command.into(), applies: applies.into() });
                        }
                    }
                    _ => {}
                }
            }
        }
        if !sections.is_empty() {
            for d in default_sections() {
                if !sections.iter().any(|x| x.id == d.id) {
                    sections.push(d);
                }
            }
            s.sections = sections;
        } else {
            // the first run: sections the administrator hides by default
            for sec in &mut s.sections {
                sec.visible = !pol.section_hidden_by_default(&sec.id);
            }
        }
        for (id, p) in spins {
            if let Some(sec) = s.section_mut(&id) {
                sec.pins.push(p);
            }
        }
        if let Some(q) = s.section_mut("quick") {
            q.pins.extend(legacy_pins);
        }
        s.fix_ribbon();
        s.enforce(&pol);
        s
    }
    fn save(&self, tabs: &[PathBuf]) {
        let Some(f) = Self::file() else { return };
        let _ = std::fs::create_dir_all(f.parent().unwrap());
        let mut out = String::from(SETTINGS_HEADER);
        out += &self.body();
        for p in tabs {
            out += &format!("tab={}\n", p.display());
        }
        if let Err(e) = std::fs::write(&f, out) {
            log::warn!("couldn't save {}: {e}", f.display());
        }

    }

    /// Everything but the tab list, always in the same order, so two copies can be compared.
    fn body(&self) -> String {
        let b = |x: bool| x as u8;
        let pol = policy::current();
        let mut out = String::new();
        // only what the person chose, so a new default from the administrator still reaches them
        let base = Settings::baseline().value_lines();
        for ((k, v), (_, d)) in self.value_lines().iter().zip(base.iter()) {
            if v != d && !pol.is_fixed(k) {
                out += &format!("{k}={v}\n");
            }
        }
        for line in &self.kept {
            out += &format!("{line}\n");
        }
        for sec in &self.sections {
            out += &format!("section={}\t{}\t{}\t{}\n", sec.id, b(sec.visible), b(sec.collapsed), sec.title.replace('\t', " "));
            for p in &sec.pins {
                out += &format!("spin={}\t{}\n", sec.id, p.display());
            }
        }
        let mut hidden: Vec<&String> = self.side_hidden.iter().collect();
        hidden.sort();
        for k in hidden {
            out += &format!("shide={k}\n");
        }
        let mut orders: Vec<_> = self.side_order.iter().collect();
        orders.sort_by_key(|(id, _)| id.as_str());
        for (id, keys) in orders {
            out += &format!("sorder={id}\t{}\n", keys.join("|"));
        }
        if !self.col_w.is_empty() {
            let mut widths: Vec<_> = self.col_w.iter().collect();
            widths.sort_by_key(|(k, _)| k.as_str());
            out += &format!("colw={}\n", widths.iter().map(|(k, w)| format!("{k}:{w:.0}")).collect::<Vec<_>>().join(";"));
        }
        if !self.menu_order.is_empty() {
            out += &format!("morder={}\n", self.menu_order.join("|"));
        }
        let mut rules: Vec<_> = self.menu_rules.iter().collect();
        rules.sort_by_key(|(key, _)| key.as_str());
        for (key, (mode, label)) in rules {
            out += &format!("menu={}\t{key}\t{label}\n", mode.word());
        }
        for c in &self.custom {
            out += &format!("cmd={}\t{}\t{}\n", c.applies, c.label.replace('\t', " "), c.command.replace('\t', " "));
        }
        out
    }
}

pub struct App {
    tabs: Vec<Tab>,
    active: usize,
    next_id: u64,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    ctx: egui::Context,
    places: Vec<Place>,
    clouds: Vec<CloudRoot>,
    drives: Vec<Drive>,
    settings: Settings,
    /// the settings as this window last read or wrote them (several windows share one file)
    saved_body: std::cell::RefCell<String>,
    was_focused: bool,
    loader: Loader,
    icons: HashMap<String, Slot>,
    type_names: HashMap<String, String>,
    thumb_order: VecDeque<String>,
    indexes: Vec<Arc<Index>>,
    preview: Preview,
    toast: Option<(String, Instant, bool)>,
    focus_search: bool,
    editing_address: Option<String>,
    focus_address: bool,
    shot: Option<Shot>,
    /// The title the window has, as sending one also asks for another frame.
    window_title: String,
    #[cfg_attr(not(windows), allow(dead_code))]
    ctrl_v_down: bool,
    acts: Vec<Act>,
    started: Instant,
    /// the Windows menu behind the right-click menu that's open now
    shell_menu: Option<shell::ShellMenu>,
    shell_tex: HashMap<u32, TextureHandle>,
    /// Windows/app menu items seen this session (key → label), for the customiser
    seen_shell: std::collections::BTreeMap<String, String>,
    customizer: menu::Customizer,
    settings_page: settings_page::Page,
    /// sidebar section being renamed inline: (section id, text)
    renaming_section: Option<(String, String)>,
    maximized: bool,
    props: Option<props::PropsState>,
    /// folder sizes, counted in the background
    sizer: crate::dirsize::DirSizer,
    /// last time the list re-sorted because folder sizes came in
    size_resort: Instant,
    popup: Option<menu::Popup>,
    popup_serial: u64,
    /// last time Windows' theme/accent was checked (for 'follow Windows')
    sys_check: f64,
    /// the 'Connect to server' dialog (Linux/macOS): url, user, password
    connect: Option<(String, String, String)>,
}
const PER_FILE_ICON: [&str; 11] = ["exe", "lnk", "ico", "url", "cur", "ani", "scr", "msc", "cpl", "appref-ms", "website"];

impl App {
    pub fn new(cc: &eframe::CreationContext, start: Option<PathBuf>, query: Option<String>, screenshot: Option<PathBuf>) -> App {
        shell::init_com();
        if let Some(gl) = &cc.gl {
            use eframe::glow::{self, HasContext};
            // SAFETY: only reads two strings from the context eframe just made current.
            let (renderer, version) = unsafe { (gl.get_parameter_string(glow::RENDERER), gl.get_parameter_string(glow::VERSION)) };
            log::info!("graphics: {renderer}, OpenGL {version}");
        }
        install(&cc.egui_ctx);
        let ctx = cc.egui_ctx.clone();
        let (tx, rx) = channel();
        let places = fsx::known_folders();
        let mut settings = if screenshot.is_some() && !test_real_settings() { Settings::defaults() } else { Settings::load() };
        if std::env::var("DALERIS_TEST_NATIVE_TITLE").is_ok() {
            settings.title_custom = false;
        }
        policy::watch(cc.egui_ctx.clone());
        if std::env::var("DALERIS_VIEW").as_deref() == Ok("icons") {
            settings.view = View::Icons;
        }
        let home = places.iter().find(|p| p.name == "Home").map(|p| p.path.clone()).unwrap_or_else(|| PathBuf::from("C:\\"));
        let mut app = App {
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
            tx,
            rx,
            loader: Loader::new(ctx.clone()),
            ctx,
            places,
            clouds: Vec::new(),
            drives: Vec::new(),
            saved_body: std::cell::RefCell::new(settings.body()),
            was_focused: true,
            settings,
            icons: HashMap::new(),
            type_names: HashMap::new(),
            thumb_order: VecDeque::new(),
            indexes: Vec::new(),
            preview: Preview { path: None, entry: None, gen: 0, content: PvContent::Nothing },
            toast: None,
            focus_search: false,
            editing_address: None,
            focus_address: false,
            shot: screenshot.map(|path| Shot { path, frames: 0, requested: false }),
            window_title: String::new(),
            ctrl_v_down: false,
            acts: Vec::new(),
            started: Instant::now(),
            shell_menu: None,
            shell_tex: HashMap::new(),
            seen_shell: Default::default(),
            customizer: menu::Customizer::default(),
            settings_page: settings_page::Page::Appearance,
            renaming_section: None,
            maximized: false,
            props: None,
            sizer: crate::dirsize::DirSizer::new(cc.egui_ctx.clone()),
            size_resort: Instant::now(),
            popup: None,
            popup_serial: 0,
            sys_check: 0.0,
            connect: None,
        };
        app.apply_theme();
        app.refresh_drives();
        app.refresh_clouds();

        let mut locs: Vec<Loc> = Vec::new();
        if let Some(p) = start {
            locs.push(Loc::Dir(p));
        } else if (app.shot.is_none() || test_real_settings()) && app.settings.restore_tabs {
            locs.extend(app.settings.tabs.iter().filter_map(|p| {
                if p.as_os_str() == THIS_PC_TAB {
                    Some(Loc::ThisPc)
                } else {
                    p.is_dir().then(|| Loc::Dir(p.clone()))
                }
            }));
        }
        if locs.is_empty() {
            locs.push(Loc::Dir(home));
        }
        for l in locs {
            app.open_tab(l, false);
        }
        app.active = 0;
        // test hook: DALERIS_TEST_PROPS=<path> opens the Properties window
        if let Ok(p) = std::env::var("DALERIS_TEST_PROPS") {
            if !p.is_empty() {
                app.open_props(vec![PathBuf::from(p)]);
            }
        }
        // test hook: DALERIS_PAGE=appearance|sidebar|menu|files opens Settings on that page
        if let Ok(p) = std::env::var("DALERIS_PAGE") {
            app.settings_page = match p.as_str() {
                "sidebar" => settings_page::Page::Sidebar,
                "ribbon" => settings_page::Page::Ribbon,
                "menu" => settings_page::Page::ContextMenu,
                "files" => settings_page::Page::Files,
                _ => settings_page::Page::Appearance,
            };
            if !p.is_empty() {
                app.open_tab(Loc::Settings, true);
            }
        }
        // `--query text` opens with a search running in the first tab
        if let Some(q) = query.filter(|q| !q.trim().is_empty()) {
            let mut t = std::mem::replace(&mut app.tabs[0], Tab::new(0, Loc::ThisPc));
            t.query = q;
            app.query_changed(&mut t);
            app.tabs[0] = t;
        }
        app
    }

    // ------------------------------------------------------------------ tabs & loading

    fn open_tab(&mut self, loc: Loc, activate: bool) {
        let mut t = Tab::new(self.next_id, loc);
        self.next_id += 1;
        if !self.settings.nested_search {
            t.scope = Scope::ThisFolder;
        }
        self.start_load(&mut t, false);
        self.tabs.push(t);
        if activate {
            self.active = self.tabs.len() - 1;
        }
    }

    fn start_load(&mut self, t: &mut Tab, refresh: bool) {
        t.cancel.store(true, Ordering::Relaxed);
        t.cancel = Arc::new(AtomicBool::new(false));
        t.gen += 1;
        t.error = None;
        t.incoming.clear();
        t.refresh_at = None;
        t.load_start = Instant::now();
        t.load_time = None;
        if !refresh {
            t.entries.clear();
            t.view.clear();
            t.sel.clear();
            t.sel_info = (0, 0);
            t.cursor = None;
            t.anchor = None;
            t.rename = None;
            t.search = None;
            t.query.clear();
            t.scroll_to = Some(0);
        }
        t.refreshing = refresh;
        match t.loc.clone() {
            Loc::ThisPc | Loc::Settings => {
                t.loading = false;
                t.watcher = None;
                t.watched = None;
                t.in_cloud = false;
                self.refresh_drives();
            }
            Loc::Dir(p) => {
                t.loading = true;
                t.in_cloud = self.clouds.iter().any(|c| fsx::is_under(&p, &c.path));
                let (tx, id, gen, cancel, ctx) = (self.tx.clone(), t.id, t.gen, t.cancel.clone(), self.ctx.clone());
                let p2 = p.clone();
                std::thread::Builder::new()
                    .name("list".into())
                    .spawn(move || {
                        let r = fsx::list_dir(&p2, &cancel, |batch| {
                            let _ = tx.send(Msg::Batch { tab: id, gen, batch });
                            ctx.request_repaint();
                        });
                        let _ = tx.send(Msg::Done { tab: id, gen, err: r.err() });
                        ctx.request_repaint();
                    })
                    .ok();
                if t.watched.as_deref() != Some(p.as_path()) {
                    let (tx, id, ctx) = (self.tx.clone(), t.id, self.ctx.clone());
                    t.watcher = Some(fsx::watch(&p, move || {
                        let _ = tx.send(Msg::Changed { tab: id });
                        ctx.request_repaint();
                    }));
                    t.watched = Some(p);
                }
            }
        }
    }

    fn navigate(&mut self, t: &mut Tab, loc: Loc) {
        if t.loc == loc && !t.searching() {
            return;
        }
        let old = std::mem::replace(&mut t.loc, loc);
        // going up: highlight the folder we came from
        if let (Loc::Dir(o), Loc::Dir(n)) = (&old, &t.loc) {
            if o.parent() == Some(n.as_path()) {
                t.select_after_load = o.file_name().map(|s| s.to_string_lossy().to_string());
            }
        }
        t.back.push(old);
        t.fwd.clear();
        self.editing_address = None;
        for k in self.loader.clear_thumbs() {
            self.icons.remove(&k);
        }
        self.start_load(t, false);
    }

    fn refresh_drives(&mut self) {
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let quick = fsx::drives_quick();
            let _ = tx.send(Msg::Drives(quick.clone()));
            ctx.request_repaint();
            for mut d in quick {
                let tx = tx.clone();
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    fsx::drive_details(&mut d);
                    let _ = tx.send(Msg::DriveDetail(d));
                    ctx.request_repaint();
                });
            }
        });
    }

    fn refresh_clouds(&mut self) {
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Clouds(fsx::cloud_roots()));
            ctx.request_repaint();
        });
    }

    fn pump(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Batch { tab, gen, batch } => {
                    if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab && t.gen == gen) {
                        if t.refreshing {
                            t.incoming.extend(batch);
                        } else {
                            t.entries.extend(batch);
                            t.dirty = true;
                        }
                    }
                }
                Msg::Done { tab, gen, err } => {
                    if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab && t.gen == gen) {
                        if t.refreshing {
                            if err.is_none() {
                                t.entries = std::mem::take(&mut t.incoming);
                            }
                            t.refreshing = false;
                        }
                        t.loading = false;
                        t.load_time = Some(t.load_start.elapsed());
                        t.error = err;
                        t.dirty = true;
                    }
                }
                Msg::Changed { tab } => {
                    if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) {
                        // coalesce bursts of change notifications
                        t.refresh_at = Some(Instant::now() + Duration::from_millis(120));
                    }
                }
                Msg::Drives(d) => {
                    let old = std::mem::take(&mut self.drives);
                    self.drives = d
                        .into_iter()
                        .map(|n| old.iter().find(|o| o.letter == n.letter && o.ready).map(|o| Drive { remote: n.remote.clone(), ..o.clone() }).unwrap_or(n))
                        .collect();
                }
                Msg::DriveDetail(d) => {
                    if let Some(x) = self.drives.iter_mut().find(|x| x.letter == d.letter) {
                        *x = d;
                    }
                }
                Msg::Clouds(c) => {
                    self.clouds = c;
                    for t in &mut self.tabs {
                        if let Loc::Dir(p) = &t.loc {
                            t.in_cloud = self.clouds.iter().any(|c| fsx::is_under(p, &c.path));
                        }
                    }
                }
                Msg::Connected(r) => match r {
                    Ok(p) => {
                        self.refresh_drives();
                        self.toast(format!("Connected: {}", p.display()), false);
                        self.acts.push(Act::Navigate(Loc::Dir(p)));
                    }
                    Err(e) => self.toast(e, true),
                },
                Msg::OpDone(r) => {
                    // copies, moves and deletes change folder sizes
                    self.sizer.clear();
                    if let Err(e) = r {
                        self.toast(e, true);
                    }
                }
                Msg::PropSize { gen, bytes, files, dirs, done } => {
                    if let Some(p) = self.props.as_mut().filter(|p| p.gen == gen) {
                        p.size = (bytes, files, dirs, done);
                    }
                }
                Msg::Preview { gen, content } => {
                    if gen == self.preview.gen {
                        self.preview.content = content;
                    }
                }
            }
        }
        while let Ok(r) = self.loader.rx.try_recv() {
            if let Some(tn) = r.type_name {
                self.type_names.insert(r.key.clone(), tn);
            }
            let slot = match r.img {
                Some(img) => {
                    let ci = if img.premultiplied {
                        egui::ColorImage::from_rgba_premultiplied([img.w, img.h], &img.rgba)
                    } else {
                        egui::ColorImage::from_rgba_unmultiplied([img.w, img.h], &img.rgba)
                    };
                    let opts = egui::TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..egui::TextureOptions::LINEAR };
                    Slot::Ready(self.ctx.load_texture(&r.key, ci, opts))
                }
                None => Slot::Failed,
            };
            if r.key.starts_with("t:") {
                self.thumb_order.push_back(r.key.clone());
                while self.thumb_order.len() > 800 {
                    if let Some(k) = self.thumb_order.pop_front() {
                        self.icons.remove(&k);
                    }
                }
            }
            self.icons.insert(r.key, slot);
        }
        let now = Instant::now();
        let due: Vec<usize> = self.tabs.iter().enumerate().filter(|(_, t)| t.refresh_at.is_some_and(|at| at <= now) && !t.loading).map(|(i, _)| i).collect();
        for i in due {
            let mut t = std::mem::replace(&mut self.tabs[i], Tab::new(0, Loc::ThisPc));
            self.start_load(&mut t, true);
            self.tabs[i] = t;
        }
        if let Some(at) = self.tabs.iter().filter_map(|t| t.refresh_at).min() {
            self.ctx.request_repaint_after(at.saturating_duration_since(now));
        }
    }

    fn toast(&mut self, s: impl Into<String>, err: bool) {
        let s = s.into();
        if err {
            log::warn!("{s}");
        }
        self.toast = Some((s, Instant::now(), err));

    }

    fn type_name(&self, ext: &str, is_dir: bool) -> String {
        if is_dir {
            return "File folder".into();
        }
        if let Some(t) = self.type_names.get(&format!("e:{ext}")) {
            return t.clone();
        }
        if ext.is_empty() {
            "File".into()
        } else {
            format!("{} File", ext.to_uppercase())
        }
    }

    // ------------------------------------------------------------------ view building

    fn rebuild_view(&self, t: &mut Tab) {
        let filter = (t.scope == Scope::ThisFolder && !t.query.trim().is_empty()).then(|| if self.settings.regex_search { Query::regex_whole(&t.query) } else { Query::new(&t.query) });
        let show_hidden = self.settings.show_hidden;
        let mut v: Vec<u32> = (0..t.entries.len() as u32)
            .filter(|&i| {
                let e = &t.entries[i as usize];
                (show_hidden || !e.hidden()) && filter.as_ref().is_none_or(|q| search::matches_entry(q, e))
            })
            .collect();
        let es = &t.entries;
        let (asc, sort) = (t.asc, t.sort);
        // sorting by size: folders count with their total size (when known)
        let base = t.dir().map(|d| d.to_path_buf()).unwrap_or_default();
        let sizes: Vec<u64> = if sort == SortKey::Size {
            es.iter().map(|e| if e.is_dir && self.settings.folder_sizes { self.sizer.known(&base.join(&e.name)).unwrap_or(0) } else { e.size }).collect()
        } else {
            Vec::new()
        };
        v.sort_unstable_by(|&a, &b| {
            let (x, y) = (&es[a as usize], &es[b as usize]);
            if x.is_dir != y.is_dir {
                return y.is_dir.cmp(&x.is_dir);
            }
            let o = match sort {
                SortKey::Name => std::cmp::Ordering::Equal,
                SortKey::Modified => x.mtime.cmp(&y.mtime),
                SortKey::Size => sizes[a as usize].cmp(&sizes[b as usize]),
                SortKey::Type => x.ext.cmp(&y.ext),
            }
            .then_with(|| natural_cmp(&x.lower, &y.lower));
            if asc {
                o
            } else {
                o.reverse()
            }
        });
        t.view = v;
        let mut counts: HashMap<(String, bool), usize> = HashMap::new();
        for &i in &t.view {
            let e = &t.entries[i as usize];
            *counts.entry((e.ext.clone(), e.is_dir)).or_default() += 1;
        }
        t.type_stats = counts.into_iter().map(|((ext, d), n)| (ext, d, n)).collect();
        if let Some(name) = t.select_after_load.take() {
            if let Some(pos) = t.view.iter().position(|&i| t.entries[i as usize].name.eq_ignore_ascii_case(&name)) {
                t.sel.clear();
                t.sel.insert(t.entries[t.view[pos] as usize].name.clone());
                t.cursor = Some(pos);
                t.anchor = Some(pos);
                t.scroll_to = Some(pos);
            } else if t.loading || t.refreshing {
                t.select_after_load = Some(name);
            }
        }
        if !t.loading {
            let names: HashSet<&str> = t.entries.iter().map(|e| e.name.as_str()).collect();
            t.sel.retain(|k| names.contains(k.as_str()));
            if let Some(c) = t.cursor {
                if c >= t.view.len() {
                    t.cursor = t.view.len().checked_sub(1);
                }
            }
        }
        self.update_sel_info(t);
    }

    fn update_sel_info(&self, t: &mut Tab) {
        if t.searching() {
            t.sel_info = (t.sel.len(), 0);
            return;
        }
        let mut size = 0;
        if !t.sel.is_empty() {
            let base = t.dir().map(|d| d.to_path_buf()).unwrap_or_default();
            for e in t.entries.iter().filter(|e| t.sel.contains(&e.name)) {
                if !e.is_dir {
                    size += e.size;
                } else if self.settings.folder_sizes {
                    // selected folders add their total size once it's known
                    size += self.sizer.known(&base.join(&e.name)).unwrap_or(0);
                }
            }
        }
        t.sel_info = (t.sel.len(), size);
    }

    fn row_count(&self, t: &Tab) -> usize {
        match &t.search {
            Some(s) => s.results.len(),
            None => t.view.len(),
        }
    }

    fn rows(&self, t: &Tab, range: std::ops::Range<usize>) -> Vec<Row> {
        match &t.search {
            Some(s) => {
                let data = s.index.data.read().unwrap();
                range
                    .filter_map(|i| s.results.get(i))
                    .map(|&ix| {
                        let it = data.items[ix as usize];
                        let name = data.name(ix).to_string();
                        let path = data.full_path(ix);
                        let is_dir = it.attrs & fsx::A_DIR != 0;
                        Row {
                            key: path.to_string_lossy().to_string(),
                            ext: ext_of(&name, is_dir),
                            name,
                            is_dir,
                            size: it.size,
                            mtime: it.mtime,
                            attrs: it.attrs,
                            folder: Some(search::display_folder(&path)),
                            path,
                        }
                    })
                    .collect()
            }
            None => {
                let base = t.dir().map(|p| p.to_path_buf()).unwrap_or_default();
                range
                    .filter_map(|i| t.view.get(i))
                    .map(|&ix| {
                        let e = &t.entries[ix as usize];
                        Row {
                            key: e.name.clone(),
                            name: e.name.clone(),
                            ext: e.ext.clone(),
                            is_dir: e.is_dir,
                            size: e.size,
                            mtime: e.mtime,
                            attrs: e.attrs,
                            path: base.join(&e.name),
                            folder: None,
                        }
                    })
                    .collect()
            }
        }
    }

    fn selected_paths(&self, t: &Tab) -> Vec<PathBuf> {
        if t.searching() {
            let mut v: Vec<PathBuf> = t.sel.iter().map(PathBuf::from).collect();
            v.sort();
            v
        } else if let Some(d) = t.dir() {
            t.view.iter().map(|&i| &t.entries[i as usize]).filter(|e| t.sel.contains(&e.name)).map(|e| d.join(&e.name)).collect()
        } else {
            Vec::new()
        }
    }

    fn cloud_of(&self, path: &Path, attrs: u32, t: &Tab) -> Cloud {
        let in_root = if t.searching() { self.clouds.iter().any(|c| fsx::is_under(path, &c.path)) } else { t.in_cloud };
        fsx::cloud_state(attrs, in_root)
    }

    // ------------------------------------------------------------------ icons

    fn icon_key(&self, path: &Path, ext: &str, is_dir: bool, attrs: u32) -> (String, ImgReq) {
        let ps = path.to_string_lossy().to_string();
        if is_dir {
            let is_root = path.parent().is_none() || fsx::is_server_root(path).is_some();
            let special = attrs & (fsx::A_READONLY | fsx::A_SYSTEM) != 0 || self.places.iter().any(|p| fsx::same_path(&p.path, path));
            if is_root || special {
                let key = format!("p:{}", ps.to_lowercase());
                return (key.clone(), ImgReq::Icon { key, path: ps, by_attrs: false, is_dir });
            }
            return ("dir".into(), ImgReq::Icon { key: "dir".into(), path: "folder".into(), by_attrs: true, is_dir });
        }
        let online_only = attrs & (fsx::A_RECALL_ON_DATA | fsx::A_OFFLINE) != 0;
        if PER_FILE_ICON.contains(&ext) && !online_only {
            let key = format!("p:{}", ps.to_lowercase());
            return (key.clone(), ImgReq::Icon { key, path: ps, by_attrs: false, is_dir });
        }
        let key = format!("e:{ext}");
        (key.clone(), ImgReq::Icon { key, path: format!("x.{ext}"), by_attrs: true, is_dir })
    }

    fn tex(&mut self, key: &str, req: impl FnOnce() -> ImgReq) -> Option<egui::TextureId> {
        match self.icons.get(key) {
            Some(Slot::Ready(t)) => Some(t.id()),
            Some(_) => None,
            None => {
                self.icons.insert(key.to_string(), Slot::Pending);
                self.loader.request(req());
                None
            }
        }
    }

    fn place_of(&self, path: &Path) -> Option<&Place> {
        self.places.iter().find(|p| fsx::same_path(&p.path, path))
    }

    /// Texture from a custom icon pack (`<pack>\<name>.png`), loaded once and cached.
    fn pack_tex(&mut self, pack: &str, name: &str) -> Option<egui::TextureId> {
        let key = format!("pack:{pack}:{name}");
        if let Some(slot) = self.icons.get(&key) {
            return if let Slot::Ready(t) = slot { Some(t.id()) } else { None };
        }
        let file = settings_page::icon_pack_dir().join(pack).join(format!("{name}.png"));
        let slot = match settings_page::load_png(&file) {
            Some(img) => Slot::Ready(self.ctx.load_texture(&key, img, egui::TextureOptions::LINEAR)),
            None => Slot::Failed,
        };
        let id = if let Slot::Ready(t) = &slot { Some(t.id()) } else { None };
        self.icons.insert(key, slot);
        id
    }

    /// An icon of the built-in pack. Pictures are drawn once per size and kept, so they stay sharp and cheap.
    fn draw_builtin(&mut self, ui: &Ui, rect: Rect, target: &IconFor) {
        let art = file_icons::resolve(target, pal().dark);
        let Art::Svg { id, svg } = &art else {
            file_icons::paint_font_art(ui.painter(), rect, &art, target.dim);
            return;
        };
        let wanted = (rect.height() * ui.ctx().pixels_per_point()).ceil() as u32;
        let side = [16, 20, 24, 32, 40, 48, 64, 96, 128, 192, 256].into_iter().find(|&s| s >= wanted).unwrap_or(256);
        let key = format!("svg:{id}:{side}");
        if !self.icons.contains_key(&key) {
            let slot = match file_icons::render_svg(&file_icons::svg_bytes(svg), side) {
                Some(img) => Slot::Ready(self.ctx.load_texture(&key, img, egui::TextureOptions::LINEAR)),
                None => {
                    log::warn!("couldn't draw the icon {id}");
                    Slot::Failed
                }
            };
            self.icons.insert(key.clone(), slot);
        }
        if let Some(Slot::Ready(tex)) = self.icons.get(&key) {
            let tint = if target.dim { Color32::from_white_alpha(130) } else { Color32::WHITE };
            ui.painter().image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
        } else {
            file_icons::paint_font_art(ui.painter(), rect, &Art::Page { color: Color32::GRAY, label: None }, target.dim);
        }
    }

    fn draw_icon(&mut self, ui: &Ui, rect: Rect, path: &Path, ext: &str, is_dir: bool, attrs: u32) {

        let dim = attrs & fsx::A_HIDDEN != 0;
        let tint = if dim { Color32::from_white_alpha(130) } else { Color32::WHITE };
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        let place = if is_dir { self.place_of(path).map(|p| (p.name.clone(), p.kf)) } else { None };
        let is_root = is_dir && path.parent().is_none();
        let drive = if is_dir { self.drives.iter().find(|d| fsx::same_path(&d.root, path)).map(|d| d.kind) } else { None };
        let is_drive = is_root || drive.is_some();
        let cloud = if is_dir { self.clouds.iter().find(|c| fsx::same_path(&c.path, path)).map(|c| c.provider.clone()) } else { None };
        let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        let place_name = place.as_ref().map(|p| p.0.clone());

        let kind = kind_of(ext, is_dir);
        let target = IconFor { kind, ext, name: &name, place: place_name.as_deref(), drive, is_root, is_system: is_dir && is_system_root(path), cloud: cloud.as_deref(), dim };


        match self.settings.icon_pack.clone().as_str() {
            "system" => {}
            "flat" => {
                self.draw_builtin(ui, rect, &target);
                return;
            }
            pack => {
                let mut names: Vec<String> = Vec::new();
                if let Some((n, _)) = &place {
                    names.push(n.to_lowercase());
                }
                if is_drive {
                    names.push("drive".into());
                }
                if is_dir {
                    names.push("folder".into());
                } else {
                    if !ext.is_empty() {
                        names.push(ext.to_string());
                    }
                    names.push(format!("{kind:?}").to_lowercase());
                    names.push("file".into());
                }
                for n in names {
                    if let Some(id) = self.pack_tex(pack, &n) {
                        ui.painter().image(id, rect, uv, tint);
                        return;
                    }
                }
            }
        }
        // Only Windows has system icons to ask for; a custom pack's gaps get the built-in ones.
        if prefers_bundled_icons() {
            self.draw_builtin(ui, rect, &target);
            return;
        }
        // Known folders use the icon Explorer shows (their shell identity), not the disk folder's.
        let (key, req) = match place {
            Some((name, kf)) => {
                let key = format!("kf:{name}");
                (key.clone(), ImgReq::KnownFolder { key, id: kf })
            }
            None => self.icon_key(path, ext, is_dir, attrs),
        };
        if let Some(id) = self.tex(&key, || req) {
            ui.painter().image(id, rect, uv, tint);
        } else {
            self.draw_builtin(ui, rect, &target);
        }
    }

    fn draw_thumb(&mut self, ui: &Ui, rect: Rect, path: &Path, row_ext: &str, is_dir: bool, attrs: u32, mtime: u64) {
        let px = (rect.width().max(rect.height()) * ui.ctx().pixels_per_point()).round().clamp(32.0, 512.0) as i32;
        let online_only = attrs & (fsx::A_RECALL_ON_DATA | fsx::A_OFFLINE) != 0;
        let kind = kind_of(row_ext, is_dir);
        let own_icons = self.settings.icon_pack != "system";
        // with an icon pack, a picture or video only replaces its icon when it has a real preview
        let packed = own_icons && (online_only || !matches!(kind, Kind::Image | Kind::Video));
        if packed || (is_dir && self.place_of(path).is_some()) {
            let side = rect.width().min(rect.height()) * 0.8;
            self.draw_icon(ui, Rect::from_center_size(rect.center(), vec2(side, side)), path, row_ext, is_dir, attrs);
            return;
        }
        let media = matches!(kind, Kind::Image | Kind::Video | Kind::Document | Kind::Folder | Kind::App) && !online_only;
        let key = if media { format!("t:{px}:{own_icons}:{}:{mtime}", path.to_string_lossy().to_lowercase()) } else { format!("t:{px}:e:{row_ext}:{is_dir}") };
        let p = path.to_path_buf();
        let k2 = key.clone();
        match self.tex(&key, || ImgReq::Thumb { key: k2, path: p, size: px, icon_only: !media, preview_only: own_icons }) {
            Some(id) => {
                let size = match self.icons.get(&key) {
                    Some(Slot::Ready(t)) => t.size_vec2() / ui.ctx().pixels_per_point(),
                    _ => rect.size(),
                };
                let r = fit_rect(rect, size);
                ui.painter().image(id, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            None => {
                // the same size as icons that never ask for a preview, when a pack draws both
                let scale = if own_icons { 0.8 } else { 0.6 };
                let r = Rect::from_center_size(rect.center(), vec2(1.0, 1.0) * rect.width().min(rect.height()) * scale);
                self.draw_icon(ui, r, path, row_ext, is_dir, attrs);
            }
        }
    }
}

/// Fit an image into `area`, keeping aspect ratio; small images are not blown up past 2x.
fn fit_rect(area: Rect, img: egui::Vec2) -> Rect {
    if img.x <= 0.0 || img.y <= 0.0 {
        return area;
    }
    let s = (area.width() / img.x).min(area.height() / img.y).min(2.0);
    Rect::from_center_size(area.center(), img * s)
}

/// Cloud status glyphs as Explorer shows them: cloud = online-only,
/// green outline check = available locally, solid green check = always keep on device.
fn cloud_badge(p: &egui::Painter, c: egui::Pos2, st: Cloud) {
    match st {
        Cloud::None => {}
        Cloud::OnlineOnly => {
            p.text(c, Align2::CENTER_CENTER, ic(I_CLOUD), icon_font(14.0), CLOUD_BLUE());
        }
        Cloud::Local | Cloud::AlwaysLocal => {
            let r = 6.5;
            if st == Cloud::AlwaysLocal {
                p.circle_filled(c, r, GREEN());
            } else {
                p.circle_stroke(c, r - 0.5, Stroke::new(1.4_f32, GREEN()));
            }
            let col = if st == Cloud::AlwaysLocal { Color32::WHITE } else { GREEN() };
            p.line_segment([c + vec2(-3.0, 0.2), c + vec2(-0.8, 2.4)], Stroke::new(1.5_f32, col));
            p.line_segment([c + vec2(-0.8, 2.4), c + vec2(3.2, -2.2)], Stroke::new(1.5_f32, col));
        }
    }
}

fn cloud_text(st: Cloud) -> &'static str {
    match st {
        Cloud::None => "",
        Cloud::OnlineOnly => "Available when online",
        Cloud::Local => "Available on this device",
        Cloud::AlwaysLocal => "Always available on this device",
    }
}

/// Width of a single line of text.
fn text_w(ui: &Ui, text: &str, size: f32) -> f32 {
    ui.fonts(|f| f.layout_no_wrap(text.to_string(), FontId::proportional(size), Color32::WHITE).size().x)
}

fn truncated(ui: &Ui, text: &str, font: FontId, color: Color32, width: f32) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = egui::text::TextWrapping { max_width: width.max(1.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    ui.fonts(|f| f.layout_job(job))
}

/// Name with matched search terms highlighted.
fn highlighted(ui: &Ui, text: &str, terms: &[String], width: f32) -> Arc<egui::Galley> {
    let lower = text.to_lowercase();
    let mut marks = vec![false; text.len()];
    if lower.len() == text.len() {
        for t in terms.iter().filter(|t| !t.is_empty()) {
            let mut from = 0;
            while let Some(p) = lower[from..].find(t.as_str()) {
                let s = from + p;
                marks[s..s + t.len()].iter_mut().for_each(|m| *m = true);
                from = s + t.len();
            }
        }
    }
    let normal = egui::TextFormat { font_id: FontId::proportional(13.5), color: TEXT(), ..Default::default() };
    let hi = egui::TextFormat { font_id: FontId::proportional(13.5), color: if pal().dark { Color32::from_rgb(0xff, 0xd3, 0x6b) } else { Color32::from_rgb(0xb3, 0x6b, 0x00) }, ..Default::default() };
    let mut job = egui::text::LayoutJob::default();
    let mut start = 0;
    let mut cur = marks.first().copied().unwrap_or(false);
    for (i, _) in text.char_indices() {
        let m = marks[i];
        if m != cur {
            job.append(&text[start..i], 0.0, if cur { hi.clone() } else { normal.clone() });
            start = i;
            cur = m;
        }
    }
    job.append(&text[start..], 0.0, if cur { hi } else { normal });
    job.wrap = egui::text::TextWrapping { max_width: width.max(1.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    ui.fonts(|f| f.layout_job(job))
}

fn icon_button(ui: &mut Ui, glyph: &'static str, tip: &str, enabled: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(32.0, 30.0), if enabled { Sense::click() } else { Sense::hover() });
    if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(5), HOVER());
    }
    let col = if enabled { TEXT() } else { FAINT() };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, ic(glyph), icon_font(15.0), col);
    resp.on_hover_text(tip)
}

fn menu_item(ui: &mut Ui, glyph: &'static str, label: &str, shortcut: &str) -> bool {
    let w = ui.available_width().max(230.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(4), HOVER());
    }
    ui.painter().text(rect.left_center() + vec2(14.0, 0.0), Align2::CENTER_CENTER, ic(glyph), icon_font(14.0), DIM());
    ui.painter().text(rect.left_center() + vec2(32.0, 0.0), Align2::LEFT_CENTER, label, FontId::proportional(13.5), TEXT());
    if !shortcut.is_empty() {
        ui.painter().text(rect.right_center() - vec2(10.0, 0.0), Align2::RIGHT_CENTER, shortcut, FontId::proportional(12.0), FAINT());
    }
    let clicked = resp.clicked();
    if clicked {
        ui.close_menu();
    }
    clicked
}

// ====================================================================== frame

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pump();
        self.poll_late_icons();
        self.follow_windows_theme(ctx);
        self.sync_settings(ctx);
        self.settings.enforce(&policy::current());
        let mut t = std::mem::replace(&mut self.tabs[self.active], Tab::new(0, Loc::ThisPc));

        // folder sizes came in: re-sort (when sorted by size, at most every half second) and update the selection total
        if self.sizer.take_changed() {
            if t.sort == SortKey::Size && self.size_resort.elapsed() > Duration::from_millis(500) {
                self.size_resort = Instant::now();
                t.dirty = true;
            } else if t.sort == SortKey::Size {
                self.sizer.mark_changed();
                ctx.request_repaint_after(Duration::from_millis(500));
            }
            self.update_sel_info(&mut t);
        }
        if t.dirty {
            t.dirty = false;
            self.rebuild_view(&mut t);
        }
        if let Some(s) = &mut t.search {
            if s.pump() {
                t.sel_info = (t.sel.len(), 0);
            }
            if s.busy() {
                ctx.request_repaint_after(Duration::from_millis(60));
            }
        }

        self.keyboard(ctx, &mut t);
        self.dropped_files(ctx, &t);
        let in_settings = t.loc == Loc::Settings;
        self.tab_bar(ctx, &mut t);
        self.toolbar(ctx, &mut t);
        if !in_settings {
            self.command_bar(ctx, &mut t);
        }
        self.status_bar(ctx, &t);
        self.sidebar(ctx, &t);
        if self.settings.show_preview && !in_settings {
            self.preview_panel(ctx, &t);
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG())).show(ctx, |ui| {
            if in_settings {
                self.settings_ui(ui);
            } else if t.loc == Loc::ThisPc && !t.searching() {
                self.this_pc(ui);
            } else {
                match self.settings.view {
                    View::Details => self.details(ui, &mut t),
                    View::Icons => self.grid(ui, &mut t),
                }
            }
        });
        self.resize_grips(ctx);
        self.popup_ui(ctx);
        self.connect_dialog(ctx);
        self.props_window(ctx, &t);
        self.drag_overlay(ctx);
        self.toast_overlay(ctx);

        self.tabs[self.active] = t;
        for a in std::mem::take(&mut self.acts) {
            self.apply(a);
        }
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        }
        let title = format!("{} · Daleris", self.tabs[self.active].loc.title());
        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }

        self.screenshot(ctx);
        // Theme, accent and drive changes are looked for while drawing, so look now and then when nothing happens.
        ctx.request_repaint_after(Duration::from_secs(2));
    }

    /// Test hook for screenshot runs: DALERIS_TEST_CLICK="right:x,y;move:x,y;click:x,y;double:x,y;wheel:x,y" (logical
    /// points). Each step is sent through egui 20 frames after the previous one (first at frame 60), so a
    /// screenshot shows what real clicks do.
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        let Some(shot) = &self.shot else { return };
        let Ok(spec) = std::env::var("DALERIS_TEST_CLICK") else { return };
        let f = shot.frames;
        for (i, step) in spec.split(';').enumerate() {
            let Some((kind, xy)) = step.split_once(':') else { continue };
            let nums: Vec<f32> = xy.split(',').filter_map(|v| v.trim().parse().ok()).collect();
            if nums.len() < 2 {
                continue;
            }
            let pos = pos2(nums[0], nums[1]);
            let at = 60 + 20 * i as u32;
            // "drag:x1,y1,x2,y2": press at the first point, move in steps, release at the second
            if kind == "drag" && nums.len() >= 4 {
                let to = pos2(nums[2], nums[3]);
                let button = egui::PointerButton::Primary;
                match f.wrapping_sub(at - 8) {
                    0 => raw.events.push(egui::Event::PointerMoved(pos)),
                    8 => raw.events.push(egui::Event::PointerButton { pos, button, pressed: true, modifiers: Default::default() }),
                    d @ 9..=14 => raw.events.push(egui::Event::PointerMoved(pos.lerp(to, (d - 8) as f32 / 6.0))),
                    16 => raw.events.push(egui::Event::PointerButton { pos: to, button, pressed: false, modifiers: Default::default() }),
                    _ => {}
                }
                continue;
            }
            if f == at - 15 || f == at - 8 {
                raw.events.push(egui::Event::PointerMoved(pos));
            }
            if f == at && kind == "ctrl_comma" {
                raw.events.push(egui::Event::Key { key: Key::Comma, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND });
            } else if f == at && kind == "wheel" {
                raw.events.push(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: vec2(0.0, -10_000_000.0), modifiers: Default::default() });
            } else if f == at && kind != "move" {
                let (button, times) = match kind {
                    "right" => (egui::PointerButton::Secondary, 1),
                    "click" => (egui::PointerButton::Primary, 1),
                    _ => (egui::PointerButton::Primary, 2),
                };
                for _ in 0..times {
                    for pressed in [true, false] {
                        raw.events.push(egui::Event::PointerButton { pos, button, pressed, modifiers: Default::default() });
                    }
                }
            }
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // nothing changed in this window: keep what another window saved, only update the tabs
        if self.writes_settings() && self.settings.body() == *self.saved_body.borrow() {
            Settings::load().save(&self.tab_list());
        } else {
            self.save_settings();
        }
    }
}

impl App {
    /// Test runs (screenshots, forced title bar) must never overwrite the person's settings.
    fn writes_settings(&self) -> bool {
        (self.shot.is_none() || test_real_settings()) && std::env::var_os("DALERIS_TEST_NATIVE_TITLE").is_none()
    }

    /// The open tabs as written in the settings file (id 0 stands in for the active tab during a frame).
    fn tab_list(&self) -> Vec<PathBuf> {
        self.tabs
            .iter()
            .filter(|t| t.id != 0)
            .filter_map(|t| if t.loc == Loc::ThisPc { Some(PathBuf::from(THIS_PC_TAB)) } else { t.dir().map(|p| p.to_path_buf()) })
            .collect()
    }

    fn save_settings(&self) {
        if self.writes_settings() {
            self.settings.save(&self.tab_list());
            *self.saved_body.borrow_mut() = self.settings.body();
        }
    }

    /// Every window shares one settings file: save when leaving a window, and pick up
    /// what another window changed when coming back, so a window never undoes those changes.
    fn sync_settings(&mut self, ctx: &egui::Context) {
        let policy_changed = policy::take_changed();
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
        if focused == self.was_focused && !policy_changed {
            return;
        }
        self.was_focused = focused;
        if !self.writes_settings() {
            return;
        }
        if !focused || policy_changed {
            if self.settings.body() != *self.saved_body.borrow() {
                self.save_settings();
            }
            if !policy_changed {
                return;
            }
        }
        let disk = Settings::load();
        let body = disk.body();
        if body == *self.saved_body.borrow() && !policy_changed {
            return;
        }
        let refilter = disk.show_hidden != self.settings.show_hidden || disk.folder_sizes != self.settings.folder_sizes;
        let title = disk.title_custom != self.settings.title_custom;
        self.settings = disk;
        *self.saved_body.borrow_mut() = body;
        if refilter {
            self.refilter_tabs();
        }
        self.apply_theme();
        if title {
            self.set_titlebar();
        }
    }

    /// What a tab lists depends on a setting that changed: rebuild every tab and rerun its search.
    fn refilter_tabs(&mut self) {
        for i in 0..self.tabs.len() {
            self.tabs[i].dirty = true;
            if self.tabs[i].searching() {
                let mut t = std::mem::replace(&mut self.tabs[i], Tab::new(0, Loc::ThisPc));
                self.query_changed(&mut t);
                self.tabs[i] = t;
            }
        }
    }

    fn screenshot(&mut self, ctx: &egui::Context) {
        let Some(shot) = &mut self.shot else { return };
        shot.frames += 1;
        let img = ctx.input(|i| i.raw.events.iter().find_map(|e| if let egui::Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None }));
        if let Some(img) = img {
            if let Some(dir) = shot.path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = save_png(&shot.path, &img);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if shot.frames >= 120 && !shot.requested {
            shot.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        if shot.frames > 600 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint();
    }

    // ------------------------------------------------------------------ keyboard

    fn keyboard(&mut self, ctx: &egui::Context, t: &mut Tab) {
        if self.popup.is_some() {
            return; // the open menu owns Enter / Escape
        }
        let typing = ctx.wants_keyboard_input();
        let (mods, events, focused) = ctx.input(|i| (i.modifiers, i.events.clone(), i.focused));
        let n = self.row_count(t);
        let mut moved: Option<usize> = None;

        // Ctrl+V with only files on the clipboard never reaches egui as an event; read the key directly.
        #[cfg(windows)]
        {
            let v_down = focused && mods.command && (unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(0x56) } as u16 & 0x8000) != 0;
            if v_down && !self.ctrl_v_down && !typing {
                self.acts.push(Act::Paste);
            }
            self.ctrl_v_down = v_down;
        }
        #[cfg(not(windows))]
        let _ = focused;

        for ev in &events {
            match ev {
                egui::Event::Copy if !typing => self.acts.push(Act::Copy),
                // Linux/macOS: Ctrl/Cmd+V arrives as a paste event; it may carry paths copied in another file manager
                #[cfg(not(windows))]
                egui::Event::Paste(text) if !typing => {
                    shell::clipboard_offer_text(text);
                    self.acts.push(Act::Paste);
                }
                egui::Event::Cut if !typing => self.acts.push(Act::Cut),
                egui::Event::Text(s) if !typing && !mods.command && n > 0 && !t.searching() => {
                    // type-ahead: jump to the next name starting with what was typed
                    // (an Esc earlier in this frame can have ended a search with more rows than the folder)
                    let n = n.min(t.view.len());
                    let (buf, at) = &mut t.typeahead;
                    if at.elapsed() > Duration::from_millis(900) {
                        buf.clear();
                    }
                    buf.push_str(&s.to_lowercase());
                    *at = Instant::now();
                    let prefix = buf.clone();
                    let start = t.cursor.map(|c| if prefix.chars().count() == 1 { c + 1 } else { c }).unwrap_or(0);
                    let hit = (0..n).map(|k| (start + k) % n).find(|&i| t.entries[t.view[i] as usize].lower.starts_with(&prefix));
                    if let Some(i) = hit {
                        moved = Some(i);
                    }
                }
                egui::Event::Key { key, pressed: true, modifiers, .. } => {
                    let (k, m) = (*key, *modifiers);
                    match k {
                        Key::T if m.command => self.acts.push(Act::NewTab),
                        Key::W if m.command => self.acts.push(Act::CloseTab(self.active)),
                        Key::Tab if m.ctrl => {
                            let len = self.tabs.len();
                            let next = if m.shift { (self.active + len - 1) % len } else { (self.active + 1) % len };
                            self.acts.push(Act::SwitchTab(next));
                        }
                        Key::F if m.command => self.focus_search = true,
                        Key::F3 => self.focus_search = true,
                        Key::L if m.command => self.start_address_edit(t),
                        Key::D if m.alt => self.start_address_edit(t),
                        Key::F5 => self.acts.push(Act::Refresh),
                        Key::ArrowLeft if m.alt => self.acts.push(Act::Back),
                        Key::ArrowRight if m.alt => self.acts.push(Act::Forward),
                        Key::ArrowUp if m.alt => self.acts.push(Act::Up),
                        Key::N if m.command && m.shift => self.acts.push(Act::NewFolder),
                        Key::P if m.alt && !policy::is_fixed("preview") => self.settings.show_preview = !self.settings.show_preview,
                        _ => {}
                    }
                    const NUMS: [Key; 9] = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
                    if let Some(d) = NUMS.iter().position(|&x| x == k) {
                        if m.command {
                            let last = self.tabs.len() - 1;
                            self.acts.push(Act::SwitchTab(if d == 8 { last } else { d.min(last) }));
                        }
                    }
                    if typing {
                        continue;
                    }
                    let page = 18;
                    let cur = t.cursor;
                    let icons = self.settings.view == View::Icons;
                    match k {
                        Key::ArrowDown if n > 0 => moved = Some(cur.map(|c| (c + 1).min(n - 1)).unwrap_or(0)),
                        Key::ArrowUp if n > 0 && !m.alt => moved = Some(cur.map(|c| c.saturating_sub(1)).unwrap_or(0)),
                        Key::ArrowRight if n > 0 && icons && !m.alt => moved = Some(cur.map(|c| (c + 1).min(n - 1)).unwrap_or(0)),
                        Key::ArrowLeft if n > 0 && icons && !m.alt => moved = Some(cur.map(|c| c.saturating_sub(1)).unwrap_or(0)),
                        Key::PageDown if n > 0 => moved = Some(cur.map(|c| (c + page).min(n - 1)).unwrap_or(0)),
                        Key::PageUp if n > 0 => moved = Some(cur.map(|c| c.saturating_sub(page)).unwrap_or(0)),
                        Key::Home if n > 0 => moved = Some(0),
                        Key::End if n > 0 => moved = Some(n - 1),
                        Key::Enter if m.alt => {
                            if let Some(p) = self.selected_paths(t).into_iter().next().or_else(|| t.dir().map(|d| d.to_path_buf())) {
                                self.acts.push(Act::Properties(p));
                            }
                        }
                        Key::Enter => {
                            for p in self.selected_paths(t) {
                                if p.is_dir() {
                                    self.acts.push(if m.command { Act::NavigateNewTab(Loc::Dir(p)) } else { Act::Navigate(Loc::Dir(p)) });
                                    break;
                                }
                                self.acts.push(Act::Open(p));
                            }
                        }
                        Key::Backspace => self.acts.push(Act::Up),
                        Key::Delete => self.acts.push(Act::Delete(m.shift)),
                        Key::F2 => self.acts.push(Act::StartRename),
                        Key::Comma if m.command => self.acts.push(Act::OpenSettings(None)),
                        Key::F10 if m.shift =>self.acts.push(Act::NativeMenu(!t.sel.is_empty())),
                        Key::A if m.command => self.acts.push(Act::SelectAll),
                        Key::Escape => {
                            if t.searching() || !t.query.is_empty() {
                                t.query.clear();
                                t.search = None;
                                t.dirty = true;
                            } else {
                                t.sel.clear();
                                self.update_sel_info(t);
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        if let Some(i) = moved {
            if let Some(r) = self.rows(t, i..i + 1).first() {
                if mods.shift {
                    let a = t.anchor.unwrap_or(i);
                    let (lo, hi) = (a.min(i), a.max(i));
                    t.sel = self.rows(t, lo..hi + 1).into_iter().map(|r| r.key).collect();
                } else {
                    t.sel.clear();
                    t.sel.insert(r.key.clone());
                    t.anchor = Some(i);
                }
                t.cursor = Some(i);
                t.scroll_to = Some(i);
                self.update_sel_info(t);
            }
        }
    }

    fn dropped_files(&mut self, ctx: &egui::Context, t: &Tab) {
        let files: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if !files.is_empty() && t.dir().is_some() {
            self.acts.push(Act::Import(files));
        }
    }

    fn start_address_edit(&mut self, t: &Tab) {
        self.editing_address = Some(match &t.loc {
            Loc::Dir(p) => p.to_string_lossy().to_string(),
            _ => String::new(),
        });
        self.focus_address = true;
    }
}

/// The network button's wording: Windows maps drive letters, Linux/macOS connect to servers.
fn network_label() -> &'static str {
    if cfg!(windows) {
        "Map network drive…"
    } else {
        "Connect to server…"
    }
}

impl App {
    /// The drive or mount a path lives on (the deepest mount point that contains it).
    fn drive_for(&self, p: &Path) -> Option<&Drive> {
        self.drives.iter().filter(|d| fsx::is_under(p, &d.root)).max_by_key(|d| d.root.as_os_str().len())
    }

    /// Linux/macOS: "Connect to server" for smb://, sftp://, ftp://, dav(s)://, nfs:// (afp:// on macOS).
    fn connect_dialog(&mut self, ctx: &egui::Context) {
        let Some((mut url, mut user, mut pass)) = self.connect.take() else { return };
        let (mut go, mut cancel) = (false, false);
        egui::Window::new("Connect to server")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.set_width(420.0);
                ui.label(egui::RichText::new("Server address").color(DIM()));
                let r = ui.add(egui::TextEdit::singleline(&mut url).desired_width(f32::INFINITY).hint_text("smb://server/share"));
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    go = true;
                }
                ui.label(
                    egui::RichText::new("smb://server/share · sftp://user@host/path · ftp://host · davs://host/path · nfs://host/export")
                        .small()
                        .color(FAINT()),
                );
                ui.add_space(8.0);
                egui::Grid::new("connect_creds").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                    ui.label(egui::RichText::new("User name").color(DIM()));
                    ui.add(egui::TextEdit::singleline(&mut user).desired_width(260.0).hint_text("optional"));
                    ui.end_row();
                    ui.label(egui::RichText::new("Password").color(DIM()));
                    ui.add(egui::TextEdit::singleline(&mut pass).password(true).desired_width(260.0).hint_text("optional"));
                    ui.end_row();
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Mounted shares (and FUSE mounts like sshfs or rclone) show under Network.").small().color(FAINT()));
                });
                ui.add_space(6.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    cancel = ui.button("Cancel").clicked();
                    go |= ui.add(egui::Button::new(egui::RichText::new("Connect").color(Color32::WHITE)).fill(ACCENT())).clicked();
                });
            });
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            cancel = true;
        }
        if go && url.contains("://") && url.split("://").nth(1).is_some_and(|r| !r.is_empty()) {
            self.acts.push(Act::ConnectServer(url.trim().to_string(), user, pass));
        } else if !cancel {
            self.connect = Some((url, user, pass));
        }
    }
}