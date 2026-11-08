use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, pos2, vec2, Align2, Color32, CornerRadius, FontId, Key, Rect, Response, Sense, Stroke, StrokeKind, TextureHandle, Ui};

use daleris_integration::target;

use crate::dirsize::DirSize;
use crate::bg;
use crate::fsx::{self, Cloud, CloudRoot, Drive, DriveKind, Entry, FailKind, FolderError, Place};
use crate::listing::{self, folder_key, DateRange, ListFilter, SizeRange, SortKey, SortRow, SortSpec};
use crate::policy;
use crate::search::{self, Index, Query, Search};
use crate::shell::{self, ImgReq, Loader, Op};
use crate::theme::*;
use crate::util::*;

mod actions;
mod chrome;
mod file_icons;
mod phosphor_icons;
mod integration_page;
mod list;
mod menu;
mod picker;
mod openers;
mod ops;
mod props;
mod settings_page;
mod sidebar;

use actions::*;
use chrome::{action_button, page_column, primary_button, secondary_button};
use picker::field_dropdown;
pub use integration_page::{handle_integration_command, Wanted};
pub use picker::{end_with_parent as end_pick_with_parent, read_request as read_pick_request, Picker};
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

/// An item seen changing size: since when, its size then and now, and when it last changed.
#[derive(Clone, Copy, Debug)]
struct Growth {
    first: Instant,
    first_size: u64,
    last: Instant,
    size: u64,
}

struct Tab {
    id: u64,
    loc: Loc,
    /// what other people's Daleris says it's doing here, by item name in lower case
    marks: HashMap<String, crate::marks::Mark>,
    /// items whose size changed, or that appeared, while this folder was shown: someone is writing them
    growing: HashMap<String, Growth>,
    /// who has what open, by item name in lower case: this PC and lock files, and other people's notes
    open_here: HashMap<String, Vec<crate::presence::Opener>>,
    open_notes: HashMap<String, Vec<crate::marks::Mark>>,
    /// when this folder was last looked at for that
    presence_at: Option<Instant>,
    back: Vec<Loc>,
    fwd: Vec<Loc>,
    entries: Vec<Entry>,
    incoming: Vec<Entry>,
    view: Vec<u32>,
    dirty: bool,
    loading: bool,
    refreshing: bool,
    generation: u64,
    cancel: Arc<AtomicBool>,
    load_start: Instant,
    load_time: Option<Duration>,
    error: Option<FolderError>,
    /// trying again without being asked: what's on screen stays until there's an answer
    quiet: bool,
    /// when to try again by itself, and how long to wait the time after
    retry_at: Option<Instant>,
    retry_wait: Duration,
    /// Network folders get looked at now and then, as nothing else says when a server goes away.
    probe_at: Instant,
    probe_since: Option<Instant>,
    /// that look is taking its time
    stalled: bool,
    /// a reconnect with a possible password prompt is under way
    reconnecting: bool,
    sel: HashSet<String>,
    sel_info: (usize, u64),
    cursor: Option<usize>,
    anchor: Option<usize>,
    scroll_to: Option<usize>,
    sort: SortKey,
    asc: bool,
    dirs_first: bool,
    /// all entries in sorted order, of which `view` is what passes the filters
    order: Vec<u32>,
    /// what `order` was made for
    order_for: Option<(SortSpec, usize, u64)>,
    /// goes up each time `entries` is replaced
    stamp: u64,
    /// a big folder is being sorted away from the window
    sorting: bool,
    sort_token: u64,
    last_rebuild: Instant,
    /// the filter panel's choices
    filter: ListFilter,
    filter_open: bool,
    /// a custom extension being typed in the filter panel
    filter_ext: String,
    /// how many items the view would show without the filter panel
    shown_of: usize,
    /// where the filter button is, for the panel to open under it
    filter_anchor: Option<Rect>,
    /// (extension, how many) of what's in the folder, for the filter panel to offer
    type_stats_all: Vec<(String, usize)>,
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
            generation: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            load_start: Instant::now(),
            load_time: None,
            error: None,
            quiet: false,
            retry_at: None,
            retry_wait: FIRST_RETRY,
            probe_at: Instant::now(),
            probe_since: None,
            stalled: false,
            reconnecting: false,
            sel: HashSet::new(),
            sel_info: (0, 0),
            cursor: None,
            anchor: None,
            scroll_to: None,
            sort: SortKey::Name,
            asc: true,
            dirs_first: true,
            order: Vec::new(),
            order_for: None,
            stamp: 0,
            sorting: false,
            sort_token: 0,
            last_rebuild: Instant::now(),
            filter: ListFilter::default(),
            filter_open: false,
            filter_ext: String::new(),
            shown_of: 0,
            filter_anchor: None,
            type_stats_all: Vec::new(),
            rename: None,
            query: String::new(),
            scope: Scope::Subfolders,
            search: None,
            watcher: None,
            watched: None,
            refresh_at: None,
            marks: HashMap::new(),
            growing: HashMap::new(),
            open_here: HashMap::new(),
            open_notes: HashMap::new(),
            presence_at: None,
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
    fn spec(&self) -> SortSpec {
        SortSpec { key: self.sort, asc: self.asc, dirs_first: self.dirs_first }
    }
    fn set_spec(&mut self, s: SortSpec) {
        self.sort = s.key;
        self.asc = s.asc;
        self.dirs_first = s.dirs_first;
    }
    /// Is the filter panel hiding things that would show otherwise?
    fn filtered(&self) -> bool {
        self.filter.is_active() && !self.searching()
    }
}

/// How long until a folder that failed is tried again by itself, doubling up to `LAST_RETRY`.
const FIRST_RETRY: Duration = Duration::from_secs(5);
const LAST_RETRY: Duration = Duration::from_secs(30);
/// How often an open network folder is looked at, and when a look that hasn't come back counts as trouble.
const PROBE_EVERY: Duration = Duration::from_secs(8);
const PROBE_SLOW: Duration = Duration::from_secs(4);
/// A network drive that hasn't said how it is by now looks unavailable until it does.
const DRIVE_SLOW: Duration = Duration::from_secs(4);
/// How often drives are looked at when nothing else asks for it.
const DRIVE_POLL: Duration = Duration::from_secs(15);
/// Folders with more entries than this are sorted away from the window.
const BIG_FOLDER: usize = 30_000;

enum Msg {
    /// a big folder's sort is done
    Sorted { tab: u64, token: u64, order: Vec<u32>, made_for: (SortSpec, usize, u64) },
    /// what a look at an open network folder found
    Probe { tab: u64, generation: u64, result: Result<(), FolderError> },
    Reconnected { tab: u64, result: Result<(), FolderError> },
    /// a drive came back while something was using it
    DrivesChanged,
    /// the contents of a file, read away from the window
    FileText(Result<String, String>),
    Batch { tab: u64, generation: u64, batch: Vec<Entry> },
    Done { tab: u64, generation: u64, err: Option<FolderError> },
    /// the notes in a tab's folder, read
    Marks { tab: u64, notes: crate::marks::Notes },
    /// who has what open in a tab's folder, from this PC and lock files
    Presence { tab: u64, open: HashMap<String, Vec<crate::presence::Opener>> },
    Changed { tab: u64 },
    Drives(Vec<Drive>),
    DriveDetail(Drive),
    Clouds(Vec<CloudRoot>),
    OpDone(Result<(), String>),
    /// a server was mounted (Linux/macOS): where it appeared
    #[allow(dead_code)]
    Connected(Result<PathBuf, String>),
    PropSize { generation: u64, bytes: u64, files: usize, dirs: usize, done: bool },
    Preview { generation: u64, content: PvContent },
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
    generation: u64,
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
    ctime: u64,
    attrs: u32,
    path: PathBuf,
    folder: Option<String>,
}

#[derive(Clone)]
struct DragFiles(Vec<PathBuf>);

/// What a drop does, from the keys held when it's let go, as in Explorer.
#[derive(Clone, Copy, PartialEq, Debug)]
enum DropMode {
    /// Move on the same drive, copy to another one.
    Auto,
    /// Ctrl
    Copy,
    /// Shift
    Move,
}

impl DropMode {
    fn of(m: egui::Modifiers) -> DropMode {
        if m.ctrl {
            DropMode::Copy
        } else if m.shift {
            DropMode::Move
        } else {
            DropMode::Auto
        }
    }
}

/// Why dropping here does nothing.
#[derive(Clone, Copy, PartialEq, Debug)]
enum NoDrop {
    /// A folder can't go inside itself.
    IntoItself,
    /// Moving things to the folder they're already in.
    AlreadyThere,
    /// nothing can be added there now
    Cant(fsx::NoWrite),
}

/// What dropping `srcs` on `dest` does, and to which of them.
fn drop_plan(srcs: &[PathBuf], dest: &Path, mode: DropMode) -> Result<(Op, Vec<PathBuf>), NoDrop> {
    let srcs: Vec<PathBuf> = srcs.iter().filter(|s| !fsx::same_path(s, dest) && !fsx::is_under(dest, s)).cloned().collect();
    if srcs.is_empty() {
        return Err(NoDrop::IntoItself);
    }
    // across drives a plain drop copies, like Explorer
    let same_drive = srcs.iter().all(|s| s.components().next() == dest.components().next());
    let op = match mode {
        DropMode::Copy => Op::Copy,
        DropMode::Move => Op::Move,
        DropMode::Auto if same_drive => Op::Move,
        DropMode::Auto => Op::Copy,
    };
    if op == Op::Move && srcs.iter().all(|s| s.parent().is_some_and(|p| fsx::same_path(p, dest))) {
        return Err(NoDrop::AlreadyThere);
    }
    Ok((op, srcs))
}

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
    /// one of `newitems::templates()`
    NewFrom(usize),
    Properties(PathBuf),
    NativeMenu(bool),
    Unpin(PathBuf),
    KeepLocal(bool),
    Terminal,
    MapDrive,
    Disconnect(char),
    SelectAll,
    /// `elevate`: the system does it, asking for administrator rights
    Drop { srcs: Vec<PathBuf>, dest: PathBuf, mode: DropMode, elevate: bool },
    Import(Vec<PathBuf>),
    ShellInvoke(u32),
    CopyContents(PathBuf),
    OpenTarget(PathBuf),
    NewWindow(PathBuf),
    RunCustom(usize),
    Sort(SortKey, Option<bool>),
    FoldersFirst(bool),
    ResetSort,
    ToggleDotfiles,
    /// the loading page's button: stop waiting
    CancelLoad,
    /// the failure page's button
    Retry,
    /// connect the mapped drive this folder is on, asking for a password if it takes one
    Reconnect,
    /// Windows's Credential Manager, where the passwords for servers are kept
    CredentialManager,
    /// Windows's prompt for a BitLocker drive's password
    Unlock(PathBuf),
    /// a tab to another place in the strip
    MoveTab(usize, usize),
    /// a tab dragged off the strip: to the Daleris window under the pointer, or to a window of its own
    TabOut(usize),
    ClearFilter,
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
    /// test hook: more pictures on the way, at these frames, as `<name>-<frame>.png`
    more: Vec<u32>,
    saving: Option<u32>,
    /// Where the window's picture lands once egui has taken it.
    taken: Arc<std::sync::Mutex<Option<Arc<egui::ColorImage>>>>,
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
    /// names starting with a dot, whatever their attributes say
    hide_dotfiles: bool,
    /// look like Windows Explorer: its colours, its icons, no logo or name in the title bar. Windows only
    explorer_look: bool,
    /// open a tab when Windows opens a drive that was just connected, such as a USB stick. Windows only
    open_new_drives: bool,
    /// starting Daleris again opens a new window, not a tab in the open one. Windows only
    start_new_window: bool,
    /// on network shares, leave a note while copying or moving, so other people's Daleris shows it
    share_marks: bool,
    /// the sort a folder gets until someone chooses one for it
    sort_key: SortKey,
    sort_asc: bool,
    dirs_first: bool,
    /// Downloads gets its own order out of the box
    downloads_newest: bool,
    /// what was chosen per folder, by `folder_key`
    folder_sorts: BTreeMap<String, SortSpec>,
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
    /// "system", "flat" (the Daleris pack), "phosphor", or the name of a folder in the icon packs folder
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
    auto_update: bool,
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
    /// what search never finds: a tick for each item of the ready-made groups, in the order of `search::SKIP_GROUPS`
    skip_groups: Vec<Vec<bool>>,
    skip_hidden: bool,
    skip_system: bool,
    /// the person's own lists. What an administrator adds stays in the policy, see `search_skip`.
    skip_folders: Vec<String>,
    skip_names: Vec<String>,
    skip_types: Vec<String>,
    skip_patterns: Vec<String>,
    /// the ribbon bar: the items on each side, left to right, and the ones switched off
    ribbon_left: Vec<String>,
    ribbon_right: Vec<String>,
    ribbon_hidden: HashSet<String>,
    /// the Windows Explorer switches. See integration_page.rs.
    exp_folders: bool,
    exp_win_e: bool,
    exp_win_e_hook: bool,
    exp_select: bool,
    exp_single: bool,
    exp_dialogs: bool,
    exp_jump: bool,
    exp_dialog_log: bool,
    /// apps that keep the Windows file dialog, separated by |
    exp_exclude: String,
    /// lines of the settings file for values a policy fixes right now, written back unchanged
    kept: Vec<String>,
}

/// Everything the ribbon bar can show: id, icon, name in Settings.
const RIBBON_ITEMS: [(&str, &str, &str); 15] = [
    ("new", I_ADD, "New"),
    ("sep1", I_GRIP, "Divider"),
    ("cut", I_CUT, "Cut"),
    ("copy", I_COPY, "Copy"),
    ("paste", I_PASTE, "Paste"),
    ("rename", I_RENAME, "Rename"),
    ("delete", I_DELETE, "Delete"),
    ("sep2", I_GRIP, "Divider"),
    ("sort", I_SORT, "Sort"),
    ("filter", I_FILTER, "Filter"),
    ("view", I_LIST, "View"),
    ("more", I_MORE, "Three dots (Windows menu for this folder)"),
    ("terminal", I_TERMINAL, "Open in Terminal"),
    ("hidden", I_EYE, "Show hidden items"),
    ("preview", I_PANE, "Preview pane"),
];
const RIBBON_LEFT: [&str; 11] = ["new", "sep1", "cut", "copy", "paste", "rename", "delete", "sep2", "sort", "filter", "view"];
const RIBBON_RIGHT: [&str; 4] = ["more", "terminal", "hidden", "preview"];

/// Test hook: DALERIS_TEST_SETTINGS=1 makes a screenshot run read and write the settings file like a
/// normal run (point APPDATA at a scratch folder first).
/// A number from a test hook's environment variable.
fn test_number(name: &str, default: u32) -> u32 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn test_real_settings() -> bool {
    std::env::var_os("DALERIS_TEST_SETTINGS").is_some()
}

/// Top of the settings file. Holds no equals sign, as lines without one are skipped when reading.
const SETTINGS_HEADER: &str = "# Daleris settings. Open windows read this file again when they get focus.\n";

/// How a This PC tab is written in the settings file.
const THIS_PC_TAB: &str = "::thispc";

/// Should the window start without the Windows frame (our own title bar)?
/// Decided before the window exists so the frame never has to be swapped at start-up.
/// How soon after a drive is connected Windows's opening it counts as that, not as someone opening it.
#[cfg(windows)]
const NEW_DRIVE: Duration = Duration::from_secs(20);

/// Whether starting Daleris again, from Start or the taskbar, opens a window of its own rather than a tab in the open one.
#[cfg(windows)]
pub fn starts_in_new_window() -> bool {
    Settings::load().start_new_window
}

/// Windows opening a drive because it was just connected, when the person turned that off in Settings.
#[cfg(windows)]
pub fn unwanted_new_drive(dir: &Path) -> bool {
    !Settings::load().open_new_drives && is_new_drive(dir)
}

#[cfg(windows)]
fn is_new_drive(dir: &Path) -> bool {
    dir.parent().is_none() && fsx::connected_for(dir).is_some_and(|d| d < NEW_DRIVE)
}

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
            hide_dotfiles: false,
            explorer_look: false,
            open_new_drives: true,
            start_new_window: true,
            share_marks: true,
            sort_key: SortKey::Name,
            sort_asc: true,
            dirs_first: true,
            downloads_newest: true,
            folder_sorts: BTreeMap::new(),
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
            auto_update: true,
            native_props: false,
            folder_sizes: true,
            col_w: HashMap::new(),
            nested_search: true,
            regex_search: false,
            skip_groups: (0..search::SKIP_GROUPS.len()).map(|g| vec![false; search::group_items(g).len()]).collect(),
            skip_hidden: false,
            skip_system: true,
            skip_folders: Vec::new(),
            skip_names: Vec::new(),
            skip_types: Vec::new(),
            skip_patterns: Vec::new(),
            ribbon_left: RIBBON_LEFT.iter().map(|s| s.to_string()).collect(),
            ribbon_right: RIBBON_RIGHT.iter().map(|s| s.to_string()).collect(),
            ribbon_hidden: HashSet::new(),
            exp_folders: false,
            exp_win_e: false,
            exp_win_e_hook: false,
            exp_select: false,
            exp_single: false,
            exp_dialogs: false,
            exp_jump: false,
            exp_dialog_log: false,
            exp_exclude: String::new(),
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

    /// The custom commands in force: an administrator's fixed list, or the person's own.
    fn commands(&self) -> Vec<CustomCmd> {
        match policy::current().fixed_value(policy::COMMANDS) {
            Some(v) => policy::commands(v).into_iter().map(|(label, applies, command)| CustomCmd { label, command, applies }).collect(),
            None => self.custom.clone(),
        }
    }

    /// Sidebar pins from a policy: each in its section, or in Quick access if there's no such section.
    fn set_pins(&mut self, text: &str) {
        for sec in &mut self.sections {
            sec.pins.clear();
        }
        for (id, path) in policy::pins(text) {
            let id = if self.sections.iter().any(|s| s.id == id) { id } else { "quick".to_string() };
            if let Some(sec) = self.section_mut(&id) {
                sec.pins.push(path);
            }
        }
    }

    /// Sidebar sections in a policy's order. Sections it doesn't name keep their order after those it does.
    fn order_sections(&mut self, text: &str) {
        let order = policy::list(text);
        self.sections.sort_by_key(|s| order.iter().position(|id| *id == s.id).unwrap_or(order.len()));
    }

    fn looks_like_explorer(&self) -> bool {
        cfg!(windows) && self.explorer_look
    }

    /// The icon pack in use: Windows's icons when Daleris looks like Explorer.
    fn pack(&self) -> &str {
        if self.looks_like_explorer() && !prefers_bundled_icons() { "system" } else { &self.icon_pack }
    }

    fn shows_logo(&self) -> bool {
        self.title_logo && !self.looks_like_explorer()
    }

    fn shows_name(&self) -> bool {
        self.title_name && !self.looks_like_explorer()
    }

    /// "Hide dotfiles" only means something on Windows: on Linux and macOS a leading dot is what makes a name hidden,
    /// so "Show hidden items" already covers them there.
    fn hides_dotfiles(&self) -> bool {
        cfg!(windows) && self.hide_dotfiles
    }

    /// The row height in use: Explorer's look has Explorer's compact rows.
    fn density(&self) -> u8 {
        if self.looks_like_explorer() { 0 } else { self.density }
    }

    fn row_h(&self) -> f32 {
        [24.0, 28.0, 34.0][self.density().min(2) as usize]
    }

    fn palette(&self) -> Palette {
        if self.looks_like_explorer() {
            return Palette::explorer(system_dark(), system_accent().unwrap_or(ACCENTS[0].1));
        }
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
        if let Some(i) = search::SKIP_GROUPS.iter().position(|g| g.0 == k) {
            self.skip_groups[i] = search::group_ticks(i, v);
            return true;
        }
        match k {
            "hidden" => self.show_hidden = on,
            "hide_dots" => self.hide_dotfiles = on,
            "explorer_look" => self.explorer_look = on,
            "open_new_drives" => self.open_new_drives = on,
            "start_new_window" => self.start_new_window = on,
            "share_marks" => self.share_marks = on,
            "sort_key" => self.sort_key = SortKey::from_word(v).unwrap_or(SortKey::Name),
            "sort_asc" => self.sort_asc = on,
            "dirs_first" => self.dirs_first = on,
            "downloads_newest" => self.downloads_newest = on,
            "preview" => self.show_preview = on,
            "ext" => self.show_ext = on,
            "view" => self.view = if v == "icons" { View::Icons } else { View::Details },
            "theme" => self.theme = v.parse().unwrap_or(1),
            "accent2" => self.accent = v.parse().unwrap_or(ACCENTS.len()),
            "native_props" => self.native_props = on,
            "folder_sizes" => self.folder_sizes = on,
            "nested_search" => self.nested_search = on,
            "regex_search" => self.regex_search = on,
            "search_skip_hidden" => self.skip_hidden = on,
            "search_skip_system" => self.skip_system = on,
            policy::SKIP_FOLDERS => self.skip_folders = policy::items(k, v),
            policy::SKIP_NAMES => self.skip_names = policy::items(k, v),
            policy::SKIP_TYPES => self.skip_types = policy::items(k, v).iter().filter_map(|t| search::file_type(t)).collect(),
            // a whole list from a policy: the settings file has a line per pattern, see `load`
            policy::SKIP_PATTERNS => self.skip_patterns = policy::items(k, v),
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
            "auto_update" => self.auto_update = on,
            "exp_folders" => self.exp_folders = on,
            "exp_win_e" => self.exp_win_e = on,
            "exp_win_e_hook" => self.exp_win_e_hook = on,
            "exp_select" => self.exp_select = on,
            "exp_single" => self.exp_single = on,
            "exp_dialogs" => self.exp_dialogs = on,
            "exp_jump" => self.exp_jump = on,
            "exp_dialog_log" => self.exp_dialog_log = on,
            "exp_exclude" => self.exp_exclude = v.to_string(),
            _ => return false,
        }
        true
    }

    /// The simple values as the settings file writes them.
    fn value_lines(&self) -> Vec<(&'static str, String)> {
        let b = |x: bool| (x as u8).to_string();
        let mut off: Vec<&str> = self.ribbon_hidden.iter().map(|s| s.as_str()).collect();
        off.sort();
        let mut lines = vec![
            ("hidden", b(self.show_hidden)),
            ("hide_dots", b(self.hide_dotfiles)),
            ("explorer_look", b(self.explorer_look)),
            ("open_new_drives", b(self.open_new_drives)),
            ("start_new_window", b(self.start_new_window)),
            ("share_marks", b(self.share_marks)),
            ("sort_key", self.sort_key.word().to_string()),
            ("sort_asc", b(self.sort_asc)),
            ("dirs_first", b(self.dirs_first)),
            ("downloads_newest", b(self.downloads_newest)),
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
            ("auto_update", b(self.auto_update)),
            ("native_props", b(self.native_props)),
            ("folder_sizes", b(self.folder_sizes)),
            ("nested_search", b(self.nested_search)),
            ("regex_search", b(self.regex_search)),
            ("rleft", self.ribbon_left.join("|")),
            ("rright", self.ribbon_right.join("|")),
            ("rhide", off.join("|")),
            ("exp_folders", b(self.exp_folders)),
            ("exp_win_e", b(self.exp_win_e)),
            ("exp_win_e_hook", b(self.exp_win_e_hook)),
            ("exp_select", b(self.exp_select)),
            ("exp_single", b(self.exp_single)),
            ("exp_dialogs", b(self.exp_dialogs)),
            ("exp_jump", b(self.exp_jump)),
            ("exp_dialog_log", b(self.exp_dialog_log)),
            ("exp_exclude", self.exp_exclude.clone()),
            ("search_skip_hidden", b(self.skip_hidden)),
            ("search_skip_system", b(self.skip_system)),
            (policy::SKIP_FOLDERS, self.skip_folders.join("|")),
            (policy::SKIP_NAMES, self.skip_names.join("|")),
            (policy::SKIP_TYPES, self.skip_types.join("|")),
        ];
        lines.extend(search::SKIP_GROUPS.iter().enumerate().map(|(i, g)| (g.0, search::group_value(i, &self.skip_groups[i]))));
        lines
    }

    /// What search never finds: the person's own rules, with what the administrator adds to the lists.
    fn search_skip(&self, pol: &policy::Policy) -> search::Skip {
        let with_fixed = |own: &[String], key: &str| own.iter().cloned().chain(pol.fixed_items(key)).collect::<Vec<_>>();
        search::Skip::new(
            &self.skip_groups,
            self.skip_hidden,
            self.skip_system,
            &with_fixed(&self.skip_folders, policy::SKIP_FOLDERS),
            &with_fixed(&self.skip_names, policy::SKIP_NAMES),
            &with_fixed(&self.skip_types, policy::SKIP_TYPES),
            &with_fixed(&self.skip_patterns, policy::SKIP_PATTERNS),
        )
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
            // a list the administrator adds to stays the person's own: see `search_skip`
            if !policy::adds_to_own(k) {
                self.set_value(k, v);
            }
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
        if let Some(v) = pol.fixed_value(policy::SIDEBAR_PINS) {
            self.set_pins(v);
        }
        if let Some(v) = pol.fixed_value(policy::SIDEBAR_ORDER) {
            self.order_sections(v);
        }
    }

    fn load() -> Settings {
        let pol = policy::current();
        let mut s = Settings::baseline();
        let mut sections: Vec<SectionCfg> = Vec::new();
        let mut legacy_pins: Vec<PathBuf> = Vec::new();
        let mut spins: Vec<(String, PathBuf)> = Vec::new();
        let saved = Self::file().and_then(|f| std::fs::read_to_string(f).ok());
        let first_run = saved.is_none();
        let mut own_patterns = false;
        if let Some(text) = saved {
            for line in text.lines() {
                let Some((k, v)) = line.split_once('=') else { continue };
                // a line per pattern; the first one replaces the default list, and an empty one stands for an empty list
                if k == policy::SKIP_PATTERNS {
                    if !own_patterns {
                        s.skip_patterns.clear();
                        own_patterns = true;
                    }
                    if !v.trim().is_empty() {
                        s.skip_patterns.push(v.to_string());
                    }
                    continue;
                }
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
                    "fsort" => {
                        if let Some((folder, spec)) = v.split_once('\t') {
                            if let Some(spec) = SortSpec::decode(spec) {
                                s.folder_sorts.insert(folder.to_string(), spec);
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
        // the first run: what the administrator suggests for the sidebar and the person's own commands
        if first_run {
            if let Some(v) = pol.recommended_value(policy::SIDEBAR_PINS) {
                s.set_pins(v);
            }
            if let Some(v) = pol.recommended_value(policy::SIDEBAR_ORDER) {
                s.order_sections(v);
            }
            if let Some(v) = pol.recommended_value(policy::COMMANDS) {
                s.custom = policy::commands(v).into_iter().map(|(label, applies, command)| CustomCmd { label, command, applies }).collect();
            }
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
        let baseline = Settings::baseline();
        let base = baseline.value_lines();
        for ((k, v), (_, d)) in self.value_lines().iter().zip(base.iter()) {
            if v != d && !pol.is_fixed(k) {
                out += &format!("{k}={v}\n");
            }
        }
        if self.skip_patterns != baseline.skip_patterns {
            if self.skip_patterns.is_empty() {
                out += &format!("{}=\n", policy::SKIP_PATTERNS);
            }
            for p in &self.skip_patterns {
                out += &format!("{}={p}\n", policy::SKIP_PATTERNS);
            }
        }
        for line in &self.kept {
            out += &format!("{line}\n");
        }
        for sec in &self.sections {
            out += &format!("section={}\t{}\t{}\t{}\n", sec.id, b(sec.visible), b(sec.collapsed), sec.title.replace('\t', " "));
            // pins a policy fixes aren't the person's: theirs wait in `kept`
            if !pol.is_fixed("spin") {
                for p in &sec.pins {
                    out += &format!("spin={}\t{}\n", sec.id, p.display());
                }
            }
        }
        for (folder, spec) in &self.folder_sorts {
            out += &format!("fsort={folder}\t{}\n", spec.encode());
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
    /// The folder dragged files are over this frame, for the label under the pointer.
    drop_hint: Option<PathBuf>,
    /// A tab dragged files have been held over since then: it opens after a moment, so they can go into its folders.
    drag_tab_hover: Option<(usize, Instant)>,
    focus_search: bool,
    editing_address: Option<String>,
    focus_address: bool,
    shot: Option<Shot>,
    updater: Option<crate::update::Updater>,
    auto_update_on: Arc<AtomicBool>,
    /// the version whose notice the person answered with Later
    update_notice_closed: Option<String>,
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
    /// what's being typed into the four "Never found in search" lists
    skip_new: [String; 4],
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
    /// a look at the drive list is under way
    drives_busy: Arc<AtomicBool>,
    /// network drives that haven't said how they are yet, since when
    drive_wait: HashMap<char, Instant>,
    /// what dropping into a folder would run into, checked once per drag
    drop_checks: HashMap<PathBuf, Arc<std::sync::OnceLock<Result<(), fsx::NoWrite>>>>,
    drive_poll: Instant,
    /// the preview that's wanted now, so that ones nobody wants any more don't run
    preview_wanted: Arc<std::sync::atomic::AtomicU64>,
    /// the 'Connect to server' dialog (Linux/macOS): url, user, password
    /// copies, moves and deletes that are running, see `ops`
    ops: Vec<Arc<crate::fileops::Job>>,
    /// what's about to be deleted for good, once the person says so
    confirm_delete: Option<Vec<PathBuf>>,
    /// the pictures other people's notes carry, by who
    faces: HashMap<String, Arc<crate::presence::Face>>,
    /// the tab being dragged, by its place in the strip
    tab_drag: Option<usize>,
    /// tabs other Daleris windows hand to this one (Windows)
    #[cfg(windows)]
    tab_requests: Option<Receiver<daleris_integration::pipe::Request>>,
    connect: Option<(String, String, String)>,
    /// this process is an app's Open or Save dialog. See picker.rs.
    picker: Option<Picker>,
    /// the Windows Explorer switches as last applied to the system
    explorer_applied: Option<(Vec<(crate::integration::Feature, bool)>, String)>,
    explorer_status_at: Option<Instant>,
    explorer_live: Vec<(crate::integration::Feature, crate::integration::Live)>,
    /// folders new launches send to this window, while it takes them
    #[cfg(windows)]
    requests: Option<Receiver<daleris_integration::pipe::Request>>,
    #[cfg(windows)]
    serving: Arc<AtomicBool>,
    /// what the Ctrl + G watcher was last told this window shows
    jump_told: Option<Option<PathBuf>>,
}
const PER_FILE_ICON: [&str; 11] = ["exe", "lnk", "ico", "url", "cur", "ani", "scr", "msc", "cpl", "appref-ms", "website"];

impl App {
    pub fn new(cc: &eframe::CreationContext, wanted: Wanted, query: Option<String>, screenshot: Option<PathBuf>, picker: Option<Picker>) -> App {
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
        let mut places = fsx::known_folders();
        let mut settings = if screenshot.is_some() && !test_real_settings() { Settings::defaults() } else { Settings::load() };
        if std::env::var("DALERIS_TEST_NATIVE_TITLE").is_ok() {
            settings.title_custom = false;
        }
        policy::watch(cc.egui_ctx.clone());
        if std::env::var("DALERIS_VIEW").as_deref() == Ok("icons") {
            settings.view = View::Icons;
        }
        let home = places.iter().find(|p| p.name == "Home").map(|p| p.path.clone()).unwrap_or_else(|| PathBuf::from("C:\\"));
        let auto_update_on = Arc::new(AtomicBool::new(settings.auto_update));
        // test runs must never look for updates
        let updater = (screenshot.is_none() && picker.is_none()).then(|| crate::update::Updater::start(cc.egui_ctx.clone(), auto_update_on.clone()));
        // test hook: Downloads moved elsewhere, which also keeps other people's files out of test pictures
        if let Some(d) = std::env::var_os("DALERIS_TEST_DOWNLOADS") {
            if let Some(p) = places.iter_mut().find(|p| p.name == "Downloads") {
                p.path = PathBuf::from(d);
            }
        }
        // test hook: what a high-DPI screen looks like
        if let Some(z) = std::env::var("DALERIS_TEST_ZOOM").ok().and_then(|z| z.parse().ok()) {
            cc.egui_ctx.set_zoom_factor(z);
        }
        #[cfg(windows)]
        if std::env::var_os("DALERIS_TEST_BACKGROUND").is_some() {
            shell::test_never_activate();
        }
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
            preview: Preview { path: None, entry: None, generation: 0, content: PvContent::Nothing },
            toast: None,
            drop_hint: None,
            drag_tab_hover: None,
            focus_search: false,
            editing_address: None,
            focus_address: false,
            shot: screenshot.map(|path| Shot {
                path,
                frames: 0,
                requested: false,
                more: std::env::var("DALERIS_TEST_SHOT_MORE").unwrap_or_default().split(',').filter_map(|f| f.parse().ok()).collect(),
                saving: None,
                taken: Arc::default(),
            }),
            updater,
            auto_update_on: auto_update_on.clone(),
            update_notice_closed: None,
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
            skip_new: Default::default(),
            maximized: false,
            props: None,
            sizer: crate::dirsize::DirSizer::new(cc.egui_ctx.clone()),
            size_resort: Instant::now(),
            popup: None,
            popup_serial: 0,
            sys_check: 0.0,
            drives_busy: Arc::new(AtomicBool::new(false)),
            drive_wait: HashMap::new(),
            drop_checks: HashMap::new(),
            drive_poll: Instant::now(),
            preview_wanted: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            ops: Vec::new(),
            confirm_delete: None,
            faces: HashMap::new(),
            tab_drag: None,
            #[cfg(windows)]
            tab_requests: None,
            connect: None,
            picker,
            explorer_applied: None,
            explorer_status_at: None,
            explorer_live: Vec::new(),
            #[cfg(windows)]
            requests: None,
            #[cfg(windows)]
            serving: Arc::new(AtomicBool::new(false)),
            jump_told: None,
        };
        app.apply_theme();
        app.refresh_drives();
        app.refresh_clouds();
        app.watch_drives();
        #[cfg(windows)]
        {
            app.tab_requests = crate::integration::serve_window_requests(cc.egui_ctx.clone());
        }

        let mut locs: Vec<Loc> = Vec::new();
        if wanted.this_pc {
            locs.push(Loc::ThisPc);
        } else if let Some(p) = wanted.dir {
            locs.push(Loc::Dir(p));
        } else if app.picker.is_none() && (app.shot.is_none() || test_real_settings()) && app.settings.restore_tabs {
            // a look for each tab, and no more than this in all: a server that's down can say so once the window is up
            let deadline = Instant::now() + Duration::from_millis(400);
            locs.extend(app.settings.tabs.iter().filter_map(|p| {
                if p.as_os_str() == THIS_PC_TAB {
                    return Some(Loc::ThisPc);
                }
                let wait = deadline.saturating_duration_since(Instant::now());
                (target::kind_within(p, wait) != target::PathKind::Missing).then(|| Loc::Dir(p.clone()))
            }));
        }
        if locs.is_empty() {
            locs.push(Loc::Dir(home));
        }
        for l in locs {
            app.open_tab(l, false);
        }
        app.active = 0;
        app.tabs[0].select_after_load = wanted.select;
        // test hook: DALERIS_TEST_PROPS=<path> opens the Properties window
        if let Ok(p) = std::env::var("DALERIS_TEST_PROPS") {
            if !p.is_empty() {
                app.open_props(vec![PathBuf::from(p)]);
            }
        }
        // test hook: DALERIS_PAGE=appearance|sidebar|menu|files|about opens Settings on that page
        if let Ok(p) = std::env::var("DALERIS_PAGE") {
            app.settings_page = match p.as_str() {
                "sidebar" => settings_page::Page::Sidebar,
                "ribbon" => settings_page::Page::Ribbon,
                "menu" => settings_page::Page::ContextMenu,
                "files" => settings_page::Page::Files,
                "explorer" => settings_page::Page::Explorer,
                "about" => settings_page::Page::About,
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
        t.generation += 1;
        if !std::mem::take(&mut t.quiet) {
            t.error = None;
        }
        t.retry_at = None;
        t.stalled = false;
        t.probe_since = None;
        t.probe_at = Instant::now() + PROBE_EVERY;
        t.incoming.clear();
        t.refresh_at = None;
        t.load_start = Instant::now();
        t.load_time = None;
        if !refresh {
            t.entries.clear();
            t.view.clear();
            t.order.clear();
            t.order_for = None;
            t.stamp += 1;
            t.shown_of = 0;
            if let Loc::Dir(p) = &t.loc {
                t.set_spec(self.spec_for(p));
            }
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
                let (tx, id, generation, cancel, ctx) = (self.tx.clone(), t.id, t.generation, t.cancel.clone(), self.ctx.clone());
                let p2 = p.clone();
                // Watching starts once the folder has been read: on a dead server that too can hang.
                if t.watched.as_deref() != Some(p.as_path()) {
                    t.watcher = None;
                    t.watched = None;
                }
                bg::submit(&p, "list", move || {
                    // the person left while this was waiting its turn
                    if cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    let list = || {
                        fsx::list_dir(&p2, &cancel, |batch| {
                            let _ = tx.send(Msg::Batch { tab: id, generation, batch });
                            ctx.request_repaint();
                        })
                    };
                    let mut r = list();
                    // Windows doesn't always connect a mapped drive by itself. No prompt here: nobody asked for one.
                    if matches!(&r, Err(e) if e.kind == FailKind::Disconnected) && !cancel.load(Ordering::Relaxed) {
                        match fsx::reconnect_for(&p2, 0, false) {
                            Ok(()) => {
                                let _ = tx.send(Msg::DrivesChanged);
                                r = list();
                            }
                            // the server answered, and said what's wrong with the account: that's what to show
                            Err(e) if matches!(e.kind, FailKind::SignIn | FailKind::PasswordExpired | FailKind::AccountLocked | FailKind::OtherUser | FailKind::AccessDenied) => r = Err(e),
                            Err(_) => {}
                        }
                    }
                    // a drive that can't be read may be one BitLocker keeps locked, which can be unlocked from here
                    if let Err(e) = &mut r {
                        if matches!(e.kind, FailKind::NotReady | FailKind::Other) && !fsx::is_network_path(&p2) && p2.ancestors().last().is_some_and(fsx::bitlocker_locked) {
                            e.kind = FailKind::Locked;
                        }
                    }
                    let _ = tx.send(Msg::Done { tab: id, generation, err: r.err() });
                    ctx.request_repaint();
                });
            }
        }
    }

    /// The sort a folder opens with.
    fn spec_for(&self, folder: &Path) -> SortSpec {
        let s = &self.settings;
        let downloads = self.places.iter().find(|p| p.name == "Downloads").map(|p| p.path.as_path());
        listing::sort_for(Some(folder), &s.folder_sorts, downloads, s.downloads_newest, SortSpec { key: s.sort_key, asc: s.sort_asc, dirs_first: s.dirs_first })
    }

    /// The person chose how this folder is sorted: from now on, that's how it's sorted, wherever it opens.
    fn set_sort(&mut self, t: &mut Tab, spec: SortSpec) {
        t.set_spec(spec);
        t.dirty = true;
        self.sort_search(t);
        if !t.searching() {
            if let Some(d) = t.dir() {
                self.settings.folder_sorts.insert(folder_key(d), spec);
                // a list nobody will go through by hand. It only needs to not grow forever.
                while self.settings.folder_sorts.len() > 2000 {
                    self.settings.folder_sorts.pop_first();
                }
                self.save_settings();
            }
        }
        // other tabs on this folder follow
        if let Some(d) = t.dir().map(Path::to_path_buf) {
            for o in self.tabs.iter_mut().filter(|o| o.dir().is_some_and(|x| fsx::same_path(x, &d))) {
                o.set_spec(spec);
                o.dirty = true;
            }
        }
    }

    fn folder_has_own_sort(&self, t: &Tab) -> bool {
        t.dir().is_some_and(|d| self.settings.folder_sorts.contains_key(&folder_key(d)))
    }

    /// Back to what this folder would have if nobody had chosen anything for it.
    fn reset_sort(&mut self, t: &mut Tab) {
        let Some(d) = t.dir().map(Path::to_path_buf) else { return };
        self.settings.folder_sorts.remove(&folder_key(&d));
        self.save_settings();
        let spec = self.spec_for(&d);
        t.set_spec(spec);
        t.dirty = true;
        for o in self.tabs.iter_mut().filter(|o| o.dir().is_some_and(|x| fsx::same_path(x, &d))) {
            o.set_spec(spec);
            o.dirty = true;
        }
    }

    /// What a network path points at: `\\server\share` for a mapped drive too.
    fn remote_of(&self, p: &Path) -> Option<String> {
        let s = p.to_string_lossy();
        if s.starts_with(r"\\") {
            let mut parts = s.trim_start_matches('\\').splitn(3, '\\');
            return match (parts.next(), parts.next()) {
                (Some(a), Some(b)) if !b.is_empty() => Some(format!(r"\\{a}\{b}")),
                (Some(a), _) => Some(format!(r"\\{a}")),
                _ => None,
            };
        }
        self.drive_for(p).filter(|d| d.kind == DriveKind::Network).and_then(|d| d.remote.clone())
    }

    /// Is this network drive unavailable, as far as can be told without waiting for it?
    fn drive_offline(&self, d: &Drive) -> bool {
        d.kind == DriveKind::Network && (d.disconnected || (d.checked && !d.ready) || self.drive_wait.get(&d.letter).is_some_and(|w| w.elapsed() >= DRIVE_SLOW))
    }

    /// The window's frame was cut short by a bug. It's in the log; all that's left to do is keep the window usable.
    fn recover_from_panic(&mut self) {
        self.acts.clear();
        self.popup = None;
        if self.tabs.get(self.active).is_some_and(|t| t.id == 0) {
            let mut t = Tab::new(self.next_id, Loc::ThisPc);
            self.next_id += 1;
            self.start_load(&mut t, false);
            self.tabs[self.active] = t;
        }
        self.toast("Something went wrong in this window, and the tab was reset. The details are in the log.", true);
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

    /// Notices a drive coming or going, such as a USB stick, as it happens. The window only draws when something
    /// happens in it, so without this a new drive waited for the next mouse move.
    fn watch_drives(&self) {
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        bg::spawn("drive-watch", move || {
            let mut last = fsx::drive_signature();
            loop {
                std::thread::sleep(Duration::from_secs(1));
                let now = fsx::drive_signature();
                if now != last {
                    last = now;
                    if tx.send(Msg::DrivesChanged).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            }
        });
    }

    fn refresh_drives(&mut self) {
        self.look_at_drives(true);
    }

    /// The drive list right away, and each drive's details as they come, however long that drive takes. A drive that
    /// is still being looked at isn't looked at again, however often this is called. `all`: not only the network
    /// drives and new ones, which wakes up other drives that have gone to sleep.
    fn look_at_drives(&mut self, all: bool) {
        static LOOKING: std::sync::Mutex<Vec<char>> = std::sync::Mutex::new(Vec::new());
        if self.drives_busy.swap(true, Ordering::AcqRel) {
            return;
        }
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let busy = self.drives_busy.clone();
        let known: Vec<char> = self.drives.iter().map(|d| d.letter).collect();
        let started = bg::spawn("drives", move || {
            let quick = fsx::drives_quick();
            let _ = tx.send(Msg::Drives(quick.clone()));
            ctx.request_repaint();
            busy.store(false, Ordering::Release);
            for mut d in quick {
                if d.checked || !(all || d.kind == DriveKind::Network || !known.contains(&d.letter)) {
                    continue;
                }
                {
                    let mut looking = LOOKING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    if looking.contains(&d.letter) {
                        continue;
                    }
                    looking.push(d.letter);
                }
                let (tx, ctx, root) = (tx.clone(), ctx.clone(), d.root.clone());
                bg::submit(&root, "drive", move || {
                    fsx::drive_details(&mut d);
                    LOOKING.lock().unwrap_or_else(std::sync::PoisonError::into_inner).retain(|l| *l != d.letter);
                    let _ = tx.send(Msg::DriveDetail(d));
                    ctx.request_repaint();
                });
            }
        });
        if !started {
            self.drives_busy.store(false, Ordering::Release);
        }
    }

    fn refresh_clouds(&mut self) {
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        bg::spawn("clouds", move || {
            let _ = tx.send(Msg::Clouds(fsx::cloud_roots()));
            ctx.request_repaint();
        });
    }

    fn pump(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Sorted { tab, token, order, made_for } => {
                    if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab && t.sort_token == token) {
                        t.sorting = false;
                        t.dirty = true;
                        if made_for == (t.spec(), t.entries.len(), t.stamp) {
                            t.order = order;
                            t.order_for = Some(made_for);
                        }
                    }
                }
                Msg::Probe { tab, generation, result } => {
                    if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab && t.generation == generation) {
                        t.probe_since = None;
                        t.stalled = false;
                        t.probe_at = Instant::now() + PROBE_EVERY;
                        if let Err(e) = result {
                            log::warn!("{} went away: {}", t.loc.title(), e.detail);
                            t.retry_wait = FIRST_RETRY;
                            t.retry_at = e.may_recover().then(|| Instant::now() + FIRST_RETRY);
                            t.error = Some(e);
                            t.watcher = None;
                            t.watched = None;
                        }
                    }
                }
                Msg::Reconnected { tab, result } => {
                    if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) {
                        t.reconnecting = false;
                        match result {
                            Ok(()) => t.refresh_at = Some(Instant::now()),
                            // the person said no to the password prompt: nothing has changed
                            Err(e) if e.kind == FailKind::Cancelled => {}
                            Err(e) => t.error = Some(e),
                        }
                    }
                    self.look_at_drives(false);
                }
                Msg::DrivesChanged => self.look_at_drives(false),
                Msg::FileText(r) => match r {
                    Ok(text) => {
                        shell::clipboard_set_text(&text);
                        self.toast("File contents copied", false);
                    }
                    Err(e) => self.toast(format!("Couldn't read the file: {e}"), true),
                },
                Msg::Batch { tab, generation, batch } => {
                    if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab && t.generation == generation) {
                        if t.refreshing {
                            t.incoming.extend(batch);
                        } else {
                            t.entries.extend(batch);
                            t.dirty = true;
                        }
                    }
                }
                Msg::Presence { tab, open } => {
                    if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) {
                        t.open_here = open;
                    }
                }
                Msg::Marks { tab, notes } => {
                    self.faces.extend(notes.faces);
                    if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) {
                        t.marks = notes.transit;
                        t.open_notes = notes.open;
                    }
                }
                Msg::Done { tab, generation, err } => {
                    if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab && t.generation == generation) {
                        if t.refreshing {
                            if err.is_none() {
                                // anything that grew, shrank or turned up since the last look is being written by someone
                                let before: HashMap<&str, u64> = t.entries.iter().map(|e| (e.lower.as_str(), e.size)).collect();
                                let now = Instant::now();
                                let changed: Vec<(String, u64, u64)> = t
                                    .incoming
                                    .iter()
                                    .filter(|e| !e.is_dir && !crate::marks::is_mark(&e.name) && before.get(e.lower.as_str()) != Some(&e.size))
                                    .map(|e| (e.lower.clone(), before.get(e.lower.as_str()).copied().unwrap_or(0), e.size))
                                    .collect();
                                drop(before);
                                for (k, was, size) in changed {
                                    let g = t.growing.entry(k).or_insert(Growth { first: now, first_size: was, last: now, size });
                                    // a pause long enough to have stopped counts as starting again
                                    if g.last.elapsed() > Duration::from_secs(8) {
                                        *g = Growth { first: now, first_size: was, last: now, size };
                                    }
                                    g.last = now;
                                    g.size = size;
                                }
                                t.entries = std::mem::take(&mut t.incoming);
                                t.order.clear();
                                t.order_for = None;
                                t.stamp += 1;
                            }
                            t.refreshing = false;
                        }
                        t.loading = false;
                        t.load_time = Some(t.load_start.elapsed());
                        match &err {
                            None => {
                                t.retry_wait = FIRST_RETRY;
                                t.retry_at = None;
                                // who has what open: look again now the listing is here
                                t.presence_at = None;
                                // other people's notes, read off the window's thread as the folder may be on a slow server
                                let notes: Vec<String> = t.entries.iter().filter(|e| crate::marks::is_mark(&e.name)).map(|e| e.name.clone()).collect();
                                if notes.is_empty() {
                                    t.marks.clear();
                                    t.open_notes.clear();
                                } else if let Loc::Dir(p) = &t.loc {
                                    let (tx, id, ctx, dir) = (self.tx.clone(), t.id, self.ctx.clone(), p.clone());
                                    bg::submit(p, "notes", move || {
                                        let _ = tx.send(Msg::Marks { tab: id, notes: crate::marks::read_all(&dir, &notes) });
                                        ctx.request_repaint();
                                    });
                                }
                                if let (Loc::Dir(p), None) = (&t.loc, &t.watcher) {
                                    let (tx, id, ctx) = (self.tx.clone(), t.id, self.ctx.clone());
                                    t.watcher = Some(fsx::watch(p, move || {
                                        let _ = tx.send(Msg::Changed { tab: id });
                                        ctx.request_repaint();
                                    }));
                                    t.watched = Some(p.clone());
                                }
                            }
                            Some(e) => {
                                log::warn!("{} can't be opened: {}", t.loc.title(), e.detail);
                                t.watcher = None;
                                t.watched = None;
                                // again and again, less and less often, until it answers
                                t.retry_at = e.may_recover().then(|| Instant::now() + t.retry_wait);
                                t.retry_wait = (t.retry_wait * 2).min(LAST_RETRY);
                            }
                        }
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
                        .map(|n| match old.iter().find(|o| o.letter == n.letter && o.checked && !n.checked) {
                            // what's known until the new look comes back
                            Some(o) => Drive { remote: n.remote.clone(), disconnected: n.disconnected, ..o.clone() },
                            None => n,
                        })
                        .collect();
                    fsx::set_network_roots(self.drives.iter().filter(|d| d.kind == DriveKind::Network).map(|d| d.root.clone()).collect());
                    self.drive_wait.retain(|l, _| self.drives.iter().any(|d| d.letter == *l));
                    for d in self.drives.iter().filter(|d| d.kind == DriveKind::Network && !d.checked) {
                        self.drive_wait.entry(d.letter).or_insert_with(Instant::now);
                    }
                }
                Msg::DriveDetail(d) => {
                    self.drive_wait.remove(&d.letter);
                    if let Some(x) = self.drives.iter_mut().find(|x| x.letter == d.letter) {
                        d.ready.then(|| log::debug!("{} is available", d.title()));
                        *x = Drive { disconnected: x.disconnected && !d.ready, ..d };
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
                Msg::PropSize { generation, bytes, files, dirs, done } => {
                    if let Some(p) = self.props.as_mut().filter(|p| p.generation == generation) {
                        p.size = (bytes, files, dirs, done);
                    }
                }
                Msg::Preview { generation, content } => {
                    if generation == self.preview.generation {
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
        self.look_after_network(now);
    }

    /// Things nobody asked for that keep the network side honest: trying a failed folder again, noticing a server
    /// that has gone away, and drives coming and going. Only the tab on screen: the others catch up when they're shown.
    fn look_after_network(&mut self, now: Instant) {
        if now.duration_since(self.drive_poll) >= DRIVE_POLL {
            self.drive_poll = now;
            self.look_at_drives(false);
        }
        let i = self.active.min(self.tabs.len().saturating_sub(1));
        let Some(t) = self.tabs.get_mut(i) else { return };
        if t.retry_at.is_some_and(|at| at <= now) && !t.loading && !t.reconnecting {
            let mut t = std::mem::replace(&mut self.tabs[i], Tab::new(0, Loc::ThisPc));
            t.quiet = true;
            self.start_load(&mut t, true);
            self.tabs[i] = t;
            return;
        }
        let Loc::Dir(p) = &t.loc else { return };
        if t.error.is_some() || t.loading || !fsx::is_network_path(p) {
            return;
        }
        match t.probe_since {
            Some(since) => {
                t.stalled = now.duration_since(since) >= PROBE_SLOW;
                self.ctx.request_repaint_after(Duration::from_secs(1));
            }
            None if now >= t.probe_at => {
                t.probe_since = Some(now);
                let (tx, ctx, tab, generation, path) = (self.tx.clone(), self.ctx.clone(), t.id, t.generation, p.clone());
                bg::submit(p, "probe", move || {
                    let result = fsx::check_folder(&path);
                    let _ = tx.send(Msg::Probe { tab, generation, result });
                    ctx.request_repaint();
                });
            }
            None => {}
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
        self.ensure_order(t);
        let (show_hidden, hide_dots) = (self.settings.show_hidden, self.settings.hides_dotfiles());
        let picker = self.picker.as_ref();
        let panel = t.filter.is_active().then(|| t.filter.compile_now());
        // searching this folder only never finds what search never finds either
        let skip = filter.as_ref().and_then(|_| t.dir()).map(|d| (self.settings.search_skip(&policy::current()), d.to_path_buf()));
        let skip_all = skip.as_ref().is_some_and(|(s, d)| s.skips_start(d));
        let mut v: Vec<u32> = Vec::with_capacity(t.order.len());
        let mut shown_of = 0;
        for &i in &t.order {
            let Some(e) = t.entries.get(i as usize) else { continue };
            // the notes are for Daleris, never for the list
            if !listing::is_visible(e.hidden(), e.is_dotfile(), show_hidden, hide_dots) || crate::marks::is_mark(&e.name) || picker.is_some_and(|p| !p.shows(&e.name, e.is_dir)) {
                continue;
            }
            shown_of += 1;
            if skip_all || skip.as_ref().is_some_and(|(s, d)| s.skips(d, &e.name, e.attrs)) {
                continue;
            }
            if filter.as_ref().is_none_or(|q| search::matches_entry(q, e)) && panel.as_ref().is_none_or(|f| f.matches(&e.lower, &e.ext, e.is_dir, e.size, e.mtime)) {
                v.push(i);
            }
        }
        t.view = v;
        t.shown_of = shown_of;
        let mut counts: HashMap<(&str, bool), usize> = HashMap::new();
        for &i in &t.view {
            let e = &t.entries[i as usize];
            *counts.entry((e.ext.as_str(), e.is_dir)).or_default() += 1;
        }
        t.type_stats = counts.into_iter().map(|((ext, d), n)| (ext.to_string(), d, n)).collect();
        if t.filter_open {
            let mut all: HashMap<&str, usize> = HashMap::new();
            for e in t.entries.iter().filter(|e| !e.is_dir && !e.ext.is_empty()) {
                *all.entry(e.ext.as_str()).or_default() += 1;
            }
            t.type_stats_all = all.into_iter().map(|(e, n)| (e.to_string(), n)).collect();
        }
        t.last_rebuild = Instant::now();
        if let Some(name) = t.select_after_load.take() {
            if let Some(pos) = t.view.iter().position(|&i| t.entries[i as usize].name.eq_ignore_ascii_case(&name)) {
                log::debug!("selected {name} after loading");
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
            if !t.sel.is_empty() {
                let names: HashSet<&str> = t.entries.iter().map(|e| e.name.as_str()).collect();
                t.sel.retain(|k| names.contains(k.as_str()));
            }
            if let Some(c) = t.cursor {
                if c >= t.view.len() {
                    t.cursor = t.view.len().checked_sub(1);
                }
            }
        }
        self.update_sel_info(t);
    }

    /// Puts `t.order` in line with the sort and the entries. A big folder keeps its old order, with what's new at the
    /// end, until a thread has come back with the new one.
    fn ensure_order(&self, t: &mut Tab) {
        let made_for = (t.spec(), t.entries.len(), t.stamp);
        if t.order_for == Some(made_for) {
            return;
        }
        let n = t.entries.len();
        let big = n >= BIG_FOLDER;
        if big {
            let have = t.order.len().min(n);
            t.order.truncate(have);
            t.order.extend(have as u32..n as u32);
            if t.sorting {
                return;
            }
        }
        let base = t.dir().map(|d| d.to_path_buf()).unwrap_or_default();
        let by_size = t.sort == SortKey::Size && self.settings.folder_sizes;
        let rows: Vec<SortRow> = t
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| SortRow {
                index: i as u32,
                is_dir: e.is_dir,
                lower: e.lower.clone(),
                ext: e.ext.clone(),
                // folders count with their total size, when known
                size: if e.is_dir && by_size { self.sizer.known(&base.join(&e.name)).unwrap_or(0) } else { e.size },
                mtime: e.mtime,
                ctime: e.ctime,
            })
            .collect();
        if !big {
            t.order = listing::sorted_indexes(rows, made_for.0);
            t.order_for = Some(made_for);
            return;
        }
        t.sorting = true;
        t.sort_token += 1;
        let (tx, ctx, tab, token) = (self.tx.clone(), self.ctx.clone(), t.id, t.sort_token);
        if !bg::spawn("sort", move || {
            let order = listing::sorted_indexes(rows, made_for.0);
            let _ = tx.send(Msg::Sorted { tab, token, order, made_for });
            ctx.request_repaint();
        }) {
            t.sorting = false;
        }
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
                            ctime: it.mtime,
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
                            ctime: e.ctime,
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
        let art = if self.settings.pack() == "phosphor" { phosphor_icons::resolve(target) } else { file_icons::resolve(target) };
        let Art::Svg { id, svg, label } = &art else {
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
            if let Some(label) = label {
                file_icons::paint_label(ui.painter(), rect, label, target.dim);
            }
        } else {
            file_icons::paint_font_art(ui.painter(), rect, &Art::Page { color: Color32::GRAY, label: None }, target.dim);
        }
    }

    /// The window a password prompt should belong to.
    fn owner_window(&self) -> isize {
        #[cfg(windows)]
        {
            shell::main_hwnd().0 as isize
        }
        #[cfg(not(windows))]
        {
            0
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


        match self.settings.pack().to_string().as_str() {
            "system" => {}
            "flat" | "phosphor" => {
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
        let (key, req) = match place.filter(|_| !fsx::is_network_path(path)) {
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
        let own_icons = self.settings.pack() != "system";
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
    ui.fonts_mut(|f| f.layout_no_wrap(text.to_string(), FontId::proportional(size), Color32::WHITE).size().x)
}

fn truncated(ui: &Ui, text: &str, font: FontId, color: Color32, width: f32) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = egui::text::TextWrapping { max_width: width.max(1.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    ui.fonts_mut(|f| f.layout_job(job))
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
    ui.fonts_mut(|f| f.layout_job(job))
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
        ui.close();
    }
    clicked
}

// ====================================================================== frame

impl App {
    fn frame(&mut self, root: &mut Ui) {
        let ctx = &root.ctx().clone();
        let frame_start = Instant::now();
        self.pump();
        self.poll_late_icons();
        self.follow_windows_theme(ctx);
        self.sync_settings(ctx);
        self.settings.enforce(&policy::current());
        self.explorer_tick(ctx);
        let mut t = std::mem::replace(&mut self.tabs[self.active], Tab::new(0, Loc::ThisPc));
        // Settings changes every other tab; this one is out of the list while it's drawn, so it catches up here
        self.follow_subfolder_setting(&mut t);

        // folder sizes came in: re-sort (when sorted by size, at most every half second) and update the selection total
        if self.sizer.take_changed() {
            if t.sort == SortKey::Size && self.size_resort.elapsed() > Duration::from_millis(500) {
                self.size_resort = Instant::now();
                t.order_for = None;
                t.dirty = true;
            } else if t.sort == SortKey::Size {
                self.sizer.mark_changed();
                ctx.request_repaint_after(Duration::from_millis(500));
            }
            self.update_sel_info(&mut t);
        }
        // a big folder still coming in isn't sorted again for every batch
        if t.dirty && t.loading && t.entries.len() >= BIG_FOLDER && t.last_rebuild.elapsed() < Duration::from_millis(400) {
            ctx.request_repaint_after(Duration::from_millis(100));
        } else if t.dirty {
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
        // A picker is a dialog: what's for managing files or the window it doesn't need.
        if self.picker.is_some() {
            self.picker_top(root, &mut t);
            self.picker_bar(root, &mut t);
            self.sidebar(root, &t);
        } else {
            self.tab_bar(root, &mut t);
            self.toolbar(root, &mut t);
            if !in_settings {
                self.command_bar(root, &mut t);
            }
            self.status_bar(root, &t);
            self.sidebar(root, &t);
            if self.settings.show_preview && !in_settings {
                self.preview_panel(root, &t);
            }
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG())).show(root, |ui| {
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
        self.filter_panel(ctx, &mut t);
        if self.picker.is_none() {
            self.resize_grips(ctx);
        }
        self.popup_ui(ctx);
        self.connect_dialog(ctx);
        self.props_window(ctx, &t);
        self.drag_overlay(ctx);
        self.toast_overlay(ctx);
        self.update_notice(ctx);
        self.file_ops(ctx);

        self.tabs[self.active] = t;
        for a in std::mem::take(&mut self.acts) {
            self.apply(a);
        }
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        }
        self.look_for_openers();
        self.auto_update_on.store(self.settings.auto_update, Ordering::Relaxed);
        let title = self.picker.as_ref().map_or_else(|| format!("{} · Daleris", self.tabs[self.active].loc.title()), Picker::window_title);
        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }

        self.screenshot(ctx);
        // Theme, accent and drive changes are looked for while drawing, so look now and then when nothing happens.
        ctx.request_repaint_after(Duration::from_secs(2));
        // whatever made this slow held up the whole window, so it's worth knowing about
        if frame_start.elapsed() > Duration::from_millis(500) && ctx.cumulative_pass_nr() > 2 {
            log::warn!("the window was stuck for {} ms", frame_start.elapsed().as_millis());
        }
        // test hook: a bug in the window's own code must not take the app with it
        if self.shot.as_ref().is_some_and(|s| s.frames == 90) && std::env::var("DALERIS_TEST_CRASH").as_deref() == Ok("frame") {
            panic!("test panic in a frame");
        }
    }
}

impl eframe::App for App {
    /// The window's own colour behind the panels: at a scale like 125% their edges fall between pixels, and the
    /// default, nearly black, showed through as a line under the ribbon bar.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        BG().to_normalized_gamma_f32()
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        // Unwinding out of this would end at the window procedure, which Rust turns into an abort: the whole app gone,
        // with whatever was going on in it.
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.frame(ui))).is_err() {
            self.recover_from_panic();
        }
    }

    /// Test hook for screenshot runs: DALERIS_TEST_CLICK="right:x,y;move:x,y;click:x,y;double:x,y;wheel:x,y" (logical
    /// points). Each step is sent through egui 20 frames after the previous one (first at frame 60), so a
    /// screenshot shows what real clicks do.
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        let Some(shot) = &self.shot else { return };
        // a test window never has real focus; keyboard steps still have to reach the widgets
        raw.focused = true;
        let Ok(spec) = std::env::var("DALERIS_TEST_CLICK") else { return };
        let f = shot.frames;
        let gap = test_number("DALERIS_TEST_STEP_FRAMES", 20);
        let mut acts = Vec::new();
        for (i, step) in spec.split(';').enumerate() {
            let Some((kind, xy)) = step.split_once(':') else { continue };
            let due = f == 60 + gap * i as u32;
            // "key:Tab", "key:shift+Tab", "text:abc", "nav:path", "newtab:path", "closetab:1", "tab:0"
            match kind {
                "key" => {
                    if due {
                        let (shift, name) = xy.strip_prefix("shift+").map_or((false, xy), |n| (true, n));
                        if let Some(key) = Key::from_name(name) {
                            let modifiers = egui::Modifiers { shift, ..Default::default() };
                            for pressed in [true, false] {
                                raw.events.push(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers });
                            }
                        }
                    }
                    continue;
                }
                "text" | "nav" | "newtab" | "tab" | "closetab" | "cancel" => {
                    if due {
                        match kind {
                            "text" => raw.events.push(egui::Event::Text(xy.to_string())),
                            "nav" => acts.push(Act::Navigate(if xy == "thispc" { Loc::ThisPc } else { Loc::Dir(PathBuf::from(xy)) })),
                            "newtab" => acts.push(Act::NavigateNewTab(Loc::Dir(PathBuf::from(xy)))),
                            "tab" => acts.push(Act::SwitchTab(xy.parse().unwrap_or(0))),
                            "cancel" => acts.push(Act::CancelLoad),
                            _ => acts.push(Act::CloseTab(xy.parse().unwrap_or(0))),
                        }
                    }
                    continue;
                }
                _ => {}
            }
            let nums: Vec<f32> = xy.split(',').filter_map(|v| v.trim().parse().ok()).collect();
            if nums.len() < 2 {
                continue;
            }
            let pos = pos2(nums[0], nums[1]);
            let at = 60 + gap * i as u32;
            // "drag:x1,y1,x2,y2[,hold]": press at the first point, move in steps, wait `hold` frames, release at the second
            if kind == "drag" && nums.len() >= 4 {
                let to = pos2(nums[2], nums[3]);
                let hold = nums.get(4).map_or(0, |h| *h as u32);
                let button = egui::PointerButton::Primary;
                match f.wrapping_sub(at - 8) {
                    0 => raw.events.push(egui::Event::PointerMoved(pos)),
                    8 => raw.events.push(egui::Event::PointerButton { pos, button, pressed: true, modifiers: Default::default() }),
                    d @ 9..=14 => raw.events.push(egui::Event::PointerMoved(pos.lerp(to, (d - 8) as f32 / 6.0))),
                    d if (15..16 + hold).contains(&d) => raw.events.push(egui::Event::PointerMoved(to)),
                    d if d == 16 + hold => raw.events.push(egui::Event::PointerButton { pos: to, button, pressed: false, modifiers: Default::default() }),
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
                raw.events.push(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: vec2(0.0, -10_000_000.0), phase: egui::TouchPhase::Move, source: egui::MouseWheelSource::Wheel, modifiers: Default::default() });
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
        self.acts.extend(acts);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(p) = &mut self.picker {
            p.cancel_if_unanswered();
        }
        if self.jump_told.is_some() {
            crate::integration::clear_jump_folder();
        }
        // nothing changed in this window: keep what another window saved, only update the tabs
        if self.writes_settings() && self.settings.body() == *self.saved_body.borrow() {
            Settings::load().save(&self.tab_list());
        } else {
            self.save_settings();
        }
        if let Some(updater) = &self.updater {
            updater.run_pending_on_exit();
        }
    }
}

impl App {
    /// Test runs (screenshots, forced title bar) must never overwrite the person's settings.
    fn writes_settings(&self) -> bool {
        self.picker.is_none() && (self.shot.is_none() || test_real_settings()) && std::env::var_os("DALERIS_TEST_NATIVE_TITLE").is_none()
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
        let refilter = disk.show_hidden != self.settings.show_hidden || disk.hide_dotfiles != self.settings.hide_dotfiles || disk.folder_sizes != self.settings.folder_sizes;
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
        /// Asks egui for a picture of the window, which arrives in `slot` a frame or two later.
        fn take_picture(ctx: &egui::Context, slot: &Arc<std::sync::Mutex<Option<Arc<egui::ColorImage>>>>) {
            let (slot, repaint) = (slot.clone(), ctx.clone());
            ctx.request_screenshot(move |image| {
                *slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(image);
                repaint.request_repaint();
            });
        }
        let Some(shot) = &mut self.shot else { return };
        shot.frames += 1;
        // off screen there's nothing to keep it to the screen's pace
        if std::env::var_os("DALERIS_TEST_BACKGROUND").is_some() {
            std::thread::sleep(Duration::from_millis(16));
        }
        let img = shot.taken.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
        if let (Some(img), Some(frame)) = (&img, shot.saving) {
            shot.saving = None;
            let stem = shot.path.with_extension("");
            let _ = save_png(Path::new(&format!("{}-{frame}.png", stem.display())), img);
            ctx.request_repaint();
            return;
        }
        if shot.saving.is_some() {
            ctx.request_repaint();
            return;
        }
        if let Some(pos) = shot.more.iter().position(|f| *f == shot.frames) {
            shot.more.remove(pos);
            shot.saving = Some(shot.frames);
            take_picture(ctx, &shot.taken);
            ctx.request_repaint();
            return;
        }
        if let Some(img) = img {
            if let Some(dir) = shot.path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = save_png(&shot.path, &img);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        let at = test_number("DALERIS_TEST_SHOT_FRAMES", 120);
        if shot.frames >= at && !shot.requested {
            shot.requested = true;
            take_picture(ctx, &shot.taken);
        }
        if shot.frames > at + 480 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint();
    }

    // ------------------------------------------------------------------ keyboard

    fn keyboard(&mut self, ctx: &egui::Context, t: &mut Tab) {
        if self.popup.is_some() || self.op_dialog_open() {
            return; // the open menu owns Enter / Escape
        }
        // Escape while dragging files puts them down where they were
        if egui::DragAndDrop::has_payload_of_type::<DragFiles>(ctx) && ctx.input(|i| i.key_pressed(Key::Escape)) {
            egui::DragAndDrop::clear_payload(ctx);
            return;
        }
        let typing = ctx.egui_wants_keyboard_input();
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
                        // as in Explorer: the folder in front, in a window of its own
                        Key::N if m.command => {
                            if let Some(d) = t.dir() {
                                self.acts.push(Act::NewWindow(d.to_path_buf()));
                            }
                        }
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
                                // no answer in time: going there will say what's wrong
                                if matches!(target::kind_within(&p, Duration::from_millis(150)), target::PathKind::Dir | target::PathKind::Unknown) {
                                    self.acts.push(if m.command { Act::NavigateNewTab(Loc::Dir(p)) } else { Act::Navigate(Loc::Dir(p)) });
                                    break;
                                }
                                self.acts.push(Act::Open(p));
                            }
                        }
                        // on Linux and macOS a leading dot is what hides a name, so this shows hidden items, as in Finder
                        Key::Period if m.command && m.shift => self.acts.push(if cfg!(windows) { Act::ToggleDotfiles } else { Act::ToggleHidden }),
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
        let files: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Anything new in the settings file has to be something administrators can set too.
    #[test]
    fn folder_sorts_survive_the_settings_file() {
        let mut s = Settings::defaults();
        let spec = SortSpec { key: SortKey::Created, asc: false, dirs_first: false };
        s.folder_sorts.insert("c:\\some folder".to_string(), spec);
        let body = s.body();
        let line = body.lines().find(|l| l.starts_with("fsort=")).expect("written");
        let (folder, written) = line["fsort=".len()..].split_once(char::from(9)).expect("two parts");
        assert_eq!(folder, "c:\\some folder");
        assert_eq!(SortSpec::decode(written), Some(spec));
    }

    #[test]
    fn the_new_defaults_are_what_was_asked_for() {
        let s = Settings::defaults();
        assert!(!s.hide_dotfiles, "dotfiles show unless someone hides them");
        assert!(s.dirs_first && s.sort_asc && s.sort_key == SortKey::Name);
        assert!(s.downloads_newest);
    }

    #[test]
    fn a_fixed_search_list_adds_to_the_persons_own_and_stays_out_of_the_file() {
        let mut s = Settings::defaults();
        s.skip_names = vec!["Backup".into()];
        s.skip_patterns = vec![r"\.(key|pem)$".into()];
        let pol = policy::Policy { fixed: vec![(policy::SKIP_NAMES, "Payroll\nHR".into()), ("search_skip_dev", "1".into())], ..Default::default() };
        s.enforce(&pol);
        assert_eq!(s.skip_names, ["Backup"], "the person's own list stays as it was");
        assert!(s.skip_groups[2].iter().all(|t| *t), "a fixed group is set like any other value: all of it");
        let skip = s.search_skip(&pol);
        let dir = Path::new("x");
        assert!(skip.skips(dir, "payroll", fsx::A_DIR) && skip.skips(dir, "hr", fsx::A_DIR) && skip.skips(dir, "backup", fsx::A_DIR));
        let body = s.body();
        assert!(body.contains("search_skip_names=Backup\n") && !body.contains("Payroll"), "only the person's own items are saved");
        assert!(body.contains("search_skip_pattern=\\.(key|pem)$\n"), "a pattern gets a line of its own");
        s.skip_types = policy::items(policy::SKIP_TYPES, "*.ISO|bak").iter().filter_map(|t| search::file_type(t)).collect();
        assert_eq!(s.skip_types, [".iso", ".bak"]);
    }

    #[test]
    fn dotfiles_are_hidden_on_their_own_only_on_windows() {
        let s = Settings { hide_dotfiles: true, ..Settings::defaults() };
        assert_eq!(s.hides_dotfiles(), cfg!(windows), "on Linux and macOS, Show hidden items decides");
        assert!(!Settings::defaults().hides_dotfiles());
    }

    #[test]
    #[cfg(windows)]
    fn drops_follow_explorers_rules() {
        let p = |s: &str| PathBuf::from(s);
        let files = [p(r"C:\Work\a.txt"), p(r"C:\Work\Plans")];
        assert_eq!(drop_plan(&files, &p(r"C:\Archive"), DropMode::Auto).map(|(op, _)| op), Ok(Op::Move), "the same drive moves");
        assert_eq!(drop_plan(&files, &p(r"D:\Backup"), DropMode::Auto).map(|(op, _)| op), Ok(Op::Copy), "another drive copies");
        assert_eq!(drop_plan(&files, &p(r"D:\Backup"), DropMode::Move).map(|(op, _)| op), Ok(Op::Move), "Shift moves anyway");
        assert_eq!(drop_plan(&files, &p(r"C:\Archive"), DropMode::Copy).map(|(op, _)| op), Ok(Op::Copy), "Ctrl copies anyway");
        assert_eq!(drop_plan(&files, &p(r"C:\Work"), DropMode::Auto).map(|(op, _)| op), Err(NoDrop::AlreadyThere));
        assert_eq!(drop_plan(&files, &p(r"C:\Work"), DropMode::Copy).map(|(op, _)| op), Ok(Op::Copy), "a copy next to the original is fine");
        assert_eq!(drop_plan(&[p(r"C:\Work\Plans")], &p(r"C:\Work\Plans\2026"), DropMode::Auto).map(|(op, _)| op), Err(NoDrop::IntoItself));
        let (_, kept) = drop_plan(&files, &p(r"C:\Work\Plans"), DropMode::Auto).unwrap();
        assert_eq!(kept, [p(r"C:\Work\a.txt")], "the folder dropped on itself stays where it is");
    }

    #[test]
    fn a_policy_can_set_every_setting() {
        for (key, _) in Settings::defaults().value_lines() {
            assert!(policy::covers(key), "no policy sets {key}: add it to VALUES in policy.rs and to PolicyDefinitions/");
        }
    }
}
