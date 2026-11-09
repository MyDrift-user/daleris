//! The right-click menu. Our own type-aware actions and the live Windows context
//! menu (classic shell extensions and app-registered commands) are turned into one
//! list of entries; the person's choices (hidden / in "Show more options" / order) are
//! applied to that list, and the very same list drives the WYSIWYG editor.

use std::collections::BTreeMap;

use super::sidebar::apply_order;
use super::*;
use crate::shell::{ShellEntry, ShellMenu};

/// Windows items that duplicate one of ours: hidden unless the person turns them on.
const DEFAULT_HIDDEN: &[&str] = &[
    "v:open", "v:opennewwindow", "v:opennewprocess", "v:opennewtab", "v:explore", "v:cut", "v:copy", "v:paste", "v:delete", "v:rename",
    "v:properties", "v:pintohome", "v:unpinfromhome", "v:copyaspath", "v:refresh", "v:find", "l:open", "l:cut", "l:copy", "l:paste",
    "l:delete", "l:rename", "l:properties", "l:copy as path", "l:pin to quick access", "l:unpin from quick access", "l:open in new window", // i18n-ignore: keys of Windows's menu items
    "l:open in new tab", "l:refresh", "l:view", "l:sort by", "l:group by", "l:always keep on this device", "l:free up space", // i18n-ignore: keys of Windows's menu items
    "l:open in terminal", "l:select all", // i18n-ignore: keys of Windows's menu items
    // German Windows
    "l:öffnen", "l:in neuem fenster öffnen", "l:in neuer registerkarte öffnen", "l:ausschneiden", "l:kopieren", "l:einfügen", "l:löschen", // i18n-ignore: keys of Windows's menu items
    "l:umbenennen", "l:eigenschaften", "l:als pfad kopieren", "l:an schnellzugriff anheften", "l:vom schnellzugriff lösen", "l:aktualisieren", // i18n-ignore: keys of Windows's menu items
    "l:ansicht", "l:sortieren nach", "l:gruppieren nach", "l:immer auf diesem gerät beibehalten", "l:speicherplatz freigeben", // i18n-ignore: keys of Windows's menu items
    "l:im terminal öffnen", "l:alles auswählen", // i18n-ignore: keys of Windows's menu items
    // Windows' "Run as administrator" duplicates ours
    "v:runas", "l:run as administrator", "l:als administrator ausführen", // i18n-ignore: keys of Windows's menu items
];

/// Rarely used items: tucked into "Show more options".
const DEFAULT_MORE: &[&str] = &[
    "app:more", "l:give access to", "l:include in library", "l:restore previous versions", "l:troubleshoot compatibility", "l:customize this folder", // i18n-ignore: keys of Windows's menu items
    "l:pin to start", "l:paste shortcut", "l:create shortcut", "l:share", "l:cast to device", "l:scan with microsoft defender", // i18n-ignore: keys of Windows's menu items
    // German Windows
    "l:zugriff gewähren auf", "l:in bibliothek aufnehmen", "l:vorgängerversionen wiederherstellen", "l:kompatibilitätsprobleme beheben", // i18n-ignore: keys of Windows's menu items
    "l:an \"start\" anheften", "l:verknüpfung einfügen", "l:verknüpfung erstellen", "l:freigeben", "l:freigabe", "l:an gerät streamen", // i18n-ignore: keys of Windows's menu items
    "l:wiedergabe auf gerät", "l:mit microsoft defender überprüfen", // i18n-ignore: keys of Windows's menu items
];

/// Group numbers decide where separators go. 90+ is the bottom block.
const G_QUICK: u32 = 0;
const G_OPEN: u32 = 10;
const G_EDIT: u32 = 15;
const G_SHELL: u32 = 20;
const G_ORG: u32 = 80;
const G_TAIL: u32 = 90;

#[derive(Clone)]
pub(super) struct QuickRow {
    cut: bool,
    paste_into: Option<PathBuf>,
    rename: bool,
    delete: bool,
}

#[derive(Clone)]
pub(super) enum MKind {
    Quick(QuickRow),
    Act(Act),
    Shell(u32),
    Sub(Vec<MEntry>),
}

#[derive(Clone)]
pub(super) struct MEntry {
    pub key: String,
    pub label: String,
    pub glyph: Option<&'static str>,
    pub tex: Option<egui::TextureId>,
    pub shortcut: &'static str,
    pub bold: bool,
    pub enabled: bool,
    pub group: u32,
    pub kind: MKind,
    /// Windows items carry their label key too, so rules match on either
    pub alt_key: Option<String>,
}

impl MEntry {
    fn act(key: &str, glyph: &'static str, label: impl Into<String>, shortcut: &'static str, group: u32, act: Act) -> MEntry {
        MEntry { key: key.into(), label: label.into(), glyph: Some(glyph), tex: None, shortcut, bold: false, enabled: true, group, kind: MKind::Act(act), alt_key: None }
    }
    fn sub(key: &str, glyph: &'static str, label: impl Into<String>, group: u32, kids: Vec<MEntry>) -> MEntry {
        MEntry { key: key.into(), label: label.into(), glyph: Some(glyph), tex: None, shortcut: "", bold: false, enabled: true, group, kind: MKind::Sub(kids), alt_key: None }
    }
    fn bold(mut self) -> MEntry {
        self.bold = true;
        self
    }
    /// Grayed out while a policy fixes this settings value.
    fn managed(mut self, key: &str) -> MEntry {
        self.enabled = !policy::is_fixed(key);
        self
    }
}

/// Sample targets the editor previews the menu for (index 5 = what was right-clicked last).
pub(super) const SAMPLES: [&str; 6] = [tx("Folder"), tx("Text file"), tx("Picture"), tx("Program"), tx("Folder background"), tx("What you right-clicked last")];

/// The open right-click menu.
pub(super) struct Popup {
    serial: u64,
    ctx: MenuCtx,
    background: bool,
    pos: egui::Pos2,
    /// key of the open submenu at each depth
    open: Vec<String>,
    /// screen areas of the menu and its submenus (last frame), for "clicked elsewhere"
    rects: Vec<Rect>,
    sizes: HashMap<String, egui::Vec2>,
    close: bool,
    opened: f64,
    /// (depth, key, since) of the row under the pointer, for the open/close delay
    hover: (usize, String, f64),
}

/// The visible menu: "Show" items, with one "Show more options" submenu (the "More" items,
/// the classic Windows menu and "Customize this menu…") in the bottom block, above Properties.
fn with_more_submenu(list: Vec<(MEntry, MenuMode)>) -> Vec<MEntry> {
    let mut more: Vec<MEntry> = list.iter().filter(|(_, m)| *m == MenuMode::More).map(|(e, _)| e.clone()).collect();
    let mut main: Vec<MEntry> = list.into_iter().filter(|(_, m)| *m == MenuMode::Show).map(|(e, _)| e).collect();
    more.push(MEntry::act("app:customize", I_SETTINGS, tr("Customize this menu…"), "", 999, Act::CustomizeMenu));
    let at = main.iter().position(|e| e.group >= G_TAIL).unwrap_or(main.len());
    main.insert(at, MEntry::sub("app:moresub", I_MORE, tr("Show more options"), G_TAIL, more));
    main
}

#[derive(Default)]
pub(super) struct Customizer {
    pub sample: usize,
    /// what was right-clicked last: (items, folder, background), shown as sample 5 in the editor
    pub target: Option<(Vec<PathBuf>, PathBuf, bool)>,
    /// submenus unfolded in the editor
    expanded: HashSet<String>,
    pub built_for: Option<usize>,
    shell: Option<ShellMenu>,
    tex: HashMap<u32, TextureHandle>,
    new_label: String,
    new_command: String,
    new_applies: String,
    new_exts: String,
}

/// Payload when dragging a row in the menu editor.
#[derive(Clone)]
struct MenuDrag(String);

/// What was right-clicked.
pub(super) struct MenuCtx {
    row: Option<Row>,
    paths: Vec<PathBuf>,
    dir: Option<PathBuf>,
    cloud: Cloud,
    in_cloud: bool,
    pinned: bool,
    searching: bool,
    net_letter: Option<char>,
    view: View,
    sort: SortKey,
    asc: bool,
    dirs_first: bool,
    own_sort: bool,
}

impl App {
    /// Build the Windows menu for what was just right-clicked (on the UI thread,
    /// which is where shell extensions expect to run).
    pub(super) fn open_shell_menu(&mut self, items: &[PathBuf], dir: &Path, extended: bool) {
        self.shell_menu = None;
        self.shell_tex.clear();
        // This has to run here, and Windows would sit in it for as long as a dead server takes. So a look first.
        if fsx::is_network_path(dir) {
            let d = dir.to_path_buf();
            if bg::within(dir, Duration::from_millis(400), move || fsx::check_folder(&d)).is_none_or(|r| r.is_err()) {
                log::info!("no Windows menu: {} isn't answering", dir.display());
                return;
            }
        }
        let Some(sm) = ShellMenu::build(items, dir, extended) else { return };
        self.shell_tex = self.shell_textures(&sm);
        self.shell_menu = Some(sm);
    }

    /// Pick up item icons Windows attached after the menu was built (first ~2 s).
    pub(super) fn poll_late_icons(&mut self) {
        let fresh = |sm: &Option<ShellMenu>| sm.as_ref().is_some_and(|s| s.built.elapsed() < Duration::from_secs(2));
        if fresh(&self.shell_menu) {
            let found = self.shell_menu.as_ref().map(|s| s.late_icons()).unwrap_or_default();
            for (id, img) in found {
                if !self.shell_tex.contains_key(&id) {
                    let t = self.ctx.load_texture(format!("shm{id}"), egui::ColorImage::from_rgba_premultiplied([img.w, img.h], &img.rgba), egui::TextureOptions::LINEAR);
                    self.shell_tex.insert(id, t);
                }
            }
            self.ctx.request_repaint_after(Duration::from_millis(120));
        }
        if fresh(&self.customizer.shell) {
            let found = self.customizer.shell.as_ref().map(|s| s.late_icons()).unwrap_or_default();
            for (id, img) in found {
                if !self.customizer.tex.contains_key(&id) {
                    let t = self.ctx.load_texture(format!("shme{id}"), egui::ColorImage::from_rgba_premultiplied([img.w, img.h], &img.rgba), egui::TextureOptions::LINEAR);
                    self.customizer.tex.insert(id, t);
                }
            }
            self.ctx.request_repaint_after(Duration::from_millis(120));
        }
    }

    fn shell_textures(&mut self, sm: &ShellMenu) -> HashMap<u32, TextureHandle> {
        fn walk(app: &mut App, es: &[ShellEntry], out: &mut HashMap<u32, TextureHandle>) {
            for e in es.iter().filter(|e| !e.separator) {
                app.seen_shell.insert(e.key.clone(), e.label.clone());
                if let Some(img) = &e.icon {
                    let ci = egui::ColorImage::from_rgba_premultiplied([img.w, img.h], &img.rgba);
                    out.insert(e.id, app.ctx.load_texture(format!("shm{}", e.id), ci, egui::TextureOptions::LINEAR));
                }
                walk(app, &e.children, out);
            }
        }
        let mut out = HashMap::new();
        walk(self, &sm.entries, &mut out);
        out
    }

    pub(super) fn menu_ctx(&self, t: &Tab, row: Option<&Row>) -> MenuCtx {
        let paths = if row.is_some() { self.selected_paths(t) } else { Vec::new() };
        let cloud = row.map(|r| self.cloud_of(&r.path, r.attrs, t)).unwrap_or(Cloud::None);
        let target = row.map(|r| r.path.clone()).or_else(|| t.dir().map(|d| d.to_path_buf()));
        MenuCtx {
            in_cloud: target.as_ref().is_some_and(|p| t.in_cloud || self.clouds.iter().any(|c| fsx::is_under(p, &c.path))),
            pinned: target.as_ref().is_some_and(|p| self.settings.is_pinned(p)),
            net_letter: row.and_then(|r| self.drives.iter().find(|d| d.kind == DriveKind::Network && fsx::same_path(&d.root, &r.path)).map(|d| d.letter)),
            row: row.cloned(),
            paths,
            dir: t.dir().map(|d| d.to_path_buf()),
            cloud,
            searching: t.searching(),
            view: self.settings.view,
            sort: t.sort,
            asc: t.asc,
            dirs_first: t.dirs_first,
            own_sort: self.folder_has_own_sort(t),
        }
    }

    // ------------------------------------------------------------------ building entries

    fn shell_entries(&self, es: &[ShellEntry], tex: &HashMap<u32, TextureHandle>, top: bool) -> Vec<MEntry> {
        let mut out = Vec::new();
        let mut section = 0;
        for e in es {
            if e.separator {
                section += 1;
                continue;
            }
            let kind = if e.children.is_empty() { MKind::Shell(e.id) } else { MKind::Sub(self.shell_entries(&e.children, tex, false)) };
            let lkey = crate::shell::menu_key("", &e.label);
            out.push(MEntry {
                key: e.key.clone(),
                alt_key: (lkey != e.key).then_some(lkey),
                label: e.label.clone(),
                glyph: None,
                tex: tex.get(&e.id).map(|t| t.id()),
                shortcut: "",
                bold: e.default,
                enabled: !e.disabled,
                group: if top { G_SHELL + section.min(50) } else { section },
                kind,
            });
        }
        out
    }

    fn pin_entry(&self, path: &Path, pinned: bool) -> MEntry {
        if policy::current().sidebar_locked() {
            let mut off = MEntry::act("app:pin", I_PIN, tr("Pin to sidebar"), "", G_ORG, Act::Refresh);
            off.enabled = false;
            return off;
        }
        if pinned {
            return MEntry::act("app:pin", I_PIN, tr("Unpin from sidebar"), "", G_ORG, Act::Unpin(path.to_path_buf()));
        }
        let mut kids: Vec<MEntry> = self
            .settings
            .sections
            .iter()
            .filter(|s| s.accepts_pins())
            .map(|s| MEntry::act(&format!("pin:{}", s.id), I_PIN, tr_dyn(&s.title), "", 0, Act::PinTo(s.id.clone(), path.to_path_buf())))
            .collect();
        kids.push(MEntry::act("pin:new", I_ADD, tr("New section…"), "", 1, Act::NewSectionWith(path.to_path_buf())));
        MEntry::sub("app:pin", I_PIN, tr("Pin to sidebar"), G_ORG, kids)
    }

    fn custom_entries(&self, row: Option<&Row>) -> Vec<MEntry> {
        if policy::current().no_custom_commands {
            return Vec::new();
        }
        self.settings
            .commands()
            .iter()
            .enumerate()
            .filter(|(_, c)| custom_applies(c, row))
            .map(|(i, c)| MEntry::act(&format!("custom:{}", c.label.to_lowercase()), I_TERMINAL, c.label.clone(), "", G_ORG, Act::RunCustom(i)))
            .collect()
    }

    pub(super) fn item_entries(&self, m: &MenuCtx, shell: Option<(&ShellMenu, &HashMap<u32, TextureHandle>)>) -> Vec<MEntry> {
        let Some(row) = &m.row else { return Vec::new() };
        let mut v: Vec<MEntry> = Vec::new();
        let n = m.paths.len().max(1);
        let single = n == 1;
        let kind = kind_of(&row.ext, row.is_dir);
        let is_root = row.is_dir && row.path.parent().is_none();
        let paste_into = (row.is_dir && single && shell::clipboard_has_files()).then(|| row.path.clone());
        v.push(MEntry {
            key: "app:quickrow".into(),
            label: tr("Cut · Copy · Paste · Rename · Delete").into(),
            glyph: Some(I_CUT),
            tex: None,
            shortcut: "",
            bold: false,
            enabled: true,
            group: G_QUICK,
            kind: MKind::Quick(QuickRow { cut: !is_root, paste_into, rename: single && !is_root, delete: !is_root }),
            alt_key: None,
        });
        let open_label = if kind == Kind::App && row.ext != "lnk" { tr("Run") } else { tr("Open") };
        let open = if row.is_dir { Act::Navigate(Loc::Dir(row.path.clone())) } else { Act::OpenMany(m.paths.clone()) };
        v.push(MEntry::act("app:open", I_FWD, open_label, tr("Enter"), G_OPEN, open).bold());
        if row.is_dir {
            v.push(MEntry::act("app:newtab", I_ADD, tr("Open in new tab"), tr("Ctrl+Enter"), G_OPEN, Act::NavigateNewTab(Loc::Dir(row.path.clone()))));
            v.push(MEntry::act("app:newwindow", I_PANE, tr("Open in new window"), "", G_OPEN, Act::NewWindow(row.path.clone())));
            if single {
                v.push(MEntry::act("app:terminal", I_TERMINAL, tr("Open in Terminal"), "", G_OPEN, Act::TerminalAt(row.path.clone())));
            }
        }
        if !row.is_dir && single {
            if matches!(row.ext.as_str(), "exe" | "msi" | "bat" | "cmd" | "lnk") {
                v.push(MEntry::act("app:runadmin", I_INFO, tr("Run as administrator"), "", G_OPEN, Act::RunAdmin(row.path.clone())));
            }
            if row.ext == "lnk" {
                v.push(MEntry::act("app:linktarget", I_NEWFOLDER, tr("Open target location"), "", G_OPEN, Act::OpenTarget(row.path.clone())));
            }
            if is_text_ext(&row.ext) && row.size <= 4 * 1024 * 1024 && m.cloud != Cloud::OnlineOnly {
                v.push(MEntry::act("app:copycontents", I_COPY, tr("Copy file contents"), "", G_OPEN, Act::CopyContents(row.path.clone())));
            }
        }
        if m.searching {
            v.push(MEntry::act("app:location", I_NEWFOLDER, tr("Open file location"), "", G_OPEN, Act::OpenLocation(row.path.clone())));
        }
        if let Some(l) = m.net_letter {
            v.push(MEntry::act("app:disconnect", I_CLOSE, tr("Disconnect network drive"), "", G_OPEN, Act::Disconnect(l)));
        }
        match shell {
            Some((sm, tex)) => v.extend(self.shell_entries(&sm.entries, tex, true)),
            None if !row.is_dir && single => v.push(MEntry::act("app:openwith", I_MORE, tr("Open with…"), "", G_OPEN, Act::OpenWith(row.path.clone()))),
            None => {}
        }
        if m.in_cloud {
            if m.cloud != Cloud::AlwaysLocal {
                v.push(MEntry::act("app:cloudkeep", I_CLOUD, tr("Always keep on this device"), "", G_ORG, Act::KeepLocal(true)));
            }
            if m.cloud != Cloud::OnlineOnly {
                v.push(MEntry::act("app:cloudfree", I_CLOUD, tr("Free up space"), "", G_ORG, Act::KeepLocal(false)));
            }
        }
        if row.is_dir && single {
            v.push(self.pin_entry(&row.path, m.pinned));
        }
        v.push(MEntry::act("app:copypath", I_COPY, if single { tr("Copy as path") } else { tr("Copy as paths") }, "", G_ORG, Act::CopyPath));
        v.extend(self.custom_entries(Some(row)));
        if !is_root {
            v.push(MEntry::act("app:deleteperm", I_DELETE, tr("Delete permanently"), tr("Shift+Del"), G_ORG, Act::Delete(true)));
        }
        if cfg!(windows) {
            v.push(MEntry::act("app:more", I_MORE, tr("Classic Windows menu"), tr("Shift+F10"), G_TAIL, Act::NativeMenu(true)));
        }
        v.push(MEntry::act("app:properties", I_INFO, tr("Properties"), tr("Alt+Enter"), G_TAIL + 1, Act::Properties(row.path.clone())));
        v
    }

    pub(super) fn background_entries(&self, m: &MenuCtx, shell: Option<(&ShellMenu, &HashMap<u32, TextureHandle>)>) -> Vec<MEntry> {
        let mut v: Vec<MEntry> = Vec::new();
        let check = |on: bool| if on { I_CHECK } else { " " };
        let mut view = vec![
            MEntry::act("view:details", check(m.view == View::Details), tr("Details"), "", 0, Act::SetView(View::Details)).managed("view"),
            MEntry::act("view:icons", check(m.view == View::Icons), tr("Large icons"), "", 0, Act::SetView(View::Icons)).managed("view"),
            MEntry::act("view:preview", check(self.settings.show_preview), tr("Preview pane"), tr("Alt+P"), 1, Act::TogglePreview).managed("preview"),
        ];
        // on Linux and macOS a leading dot is what makes a name hidden, so Ctrl+Shift+. belongs to hidden items there
        if cfg!(windows) {
            view.push(MEntry::act("view:hidden", check(self.settings.show_hidden), tr("Hidden items"), "", 1, Act::ToggleHidden).managed("hidden"));
            view.push(MEntry::act("view:dots", check(!self.settings.hide_dotfiles), tr("Dotfiles"), tr("Ctrl+Shift+."), 1, Act::ToggleDotfiles).managed("hide_dots"));
        } else {
            view.push(MEntry::act("view:hidden", check(self.settings.show_hidden), tr("Hidden items"), tr("Ctrl+Shift+."), 1, Act::ToggleHidden).managed("hidden"));
        }
        v.push(MEntry::sub("app:view", I_LIST, tr("View"), G_OPEN, view));
        // the key stays English: the person's menu rules are saved under it
        let mut sort: Vec<MEntry> = SortKey::ALL.into_iter().map(|k| MEntry::act(&format!("sort:{}", k.label()), check(m.sort == k), tr(k.label()), "", 0, Act::Sort(k, None))).collect();
        sort.push(MEntry::act("sort:asc", check(m.asc), tr("Ascending"), "", 1, Act::Sort(m.sort, Some(true))));
        sort.push(MEntry::act("sort:desc", check(!m.asc), tr("Descending"), "", 1, Act::Sort(m.sort, Some(false))));
        sort.push(MEntry::act("sort:dirs", check(m.dirs_first), tr("Folders first"), "", 2, Act::FoldersFirst(!m.dirs_first)));
        if m.own_sort {
            sort.push(MEntry::act("sort:reset", I_REFRESH, tr("Reset this folder to default"), "", 2, Act::ResetSort));
        }
        v.push(MEntry::sub("app:sort", I_SORT, tr("Sort by"), G_OPEN, sort));
        v.push(MEntry::act("app:refresh", I_REFRESH, tr("Refresh"), "F5", G_OPEN, Act::Refresh));
        let mut paste = MEntry::act("app:paste", I_PASTE, tr("Paste"), tr("Ctrl+V"), G_EDIT, Act::Paste);
        paste.enabled = shell::clipboard_has_files();
        v.push(paste);
        v.push(MEntry::act("app:new", I_NEWFOLDER, tr("New folder"), tr("Ctrl+Shift+N"), G_EDIT, Act::NewFolder));
        if shell.is_none() {
            v.push(MEntry::act("app:newtext", I_RENAME, tr("New text document"), "", G_EDIT, Act::NewTextFile));
        }
        if let Some(d) = &m.dir {
            v.push(MEntry::act("app:terminal", I_TERMINAL, tr("Open in Terminal"), "", G_EDIT, Act::TerminalAt(d.clone())));
        }
        v.push(MEntry::act("app:selectall", I_COPY, tr("Select all"), tr("Ctrl+A"), G_EDIT, Act::SelectAll));
        if let Some((sm, tex)) = shell {
            v.extend(self.shell_entries(&sm.entries, tex, true));
        }
        if let Some(d) = &m.dir {
            v.push(self.pin_entry(d, m.pinned));
            v.extend(self.custom_entries(None));
            if cfg!(windows) {
                v.push(MEntry::act("app:more", I_MORE, tr("Classic Windows menu"), tr("Shift+F10"), G_TAIL, Act::NativeMenu(false)));
            }
            v.push(MEntry::act("app:properties", I_INFO, tr("Properties"), "", G_TAIL + 1, Act::Properties(d.clone())));
        }
        v
    }

    // ------------------------------------------------------------------ rules

    pub(super) fn entry_mode(&self, e: &MEntry) -> MenuMode {
        let keys = std::iter::once(e.key.as_str()).chain(e.alt_key.as_deref());
        // An administrator's fixed lists come first, then the person's own choice, then the administrator's defaults.
        let pol = policy::current();
        let placed = |fixed: bool| {
            let places: Vec<&str> = keys.clone().filter_map(|k| pol.menu_place(k, fixed)).collect();
            if places.contains(&"hide") {
                Some(Some(MenuMode::Hide))
            } else if places.contains(&"more") {
                Some(Some(MenuMode::More))
            } else if places.contains(&"default") {
                Some(None)
            } else {
                None
            }
        };
        match placed(true) {
            Some(Some(mode)) => return mode,
            Some(None) => return builtin_mode(keys),
            None => {}
        }
        for k in keys.clone() {
            if let Some((m, _)) = self.settings.menu_rules.get(k) {
                return *m;
            }
        }
        if let Some(Some(mode)) = placed(false) {
            return mode;
        }
        builtin_mode(keys)
    }

    /// The order the menu's items go in: an administrator's fixed order, the person's own, or the administrator's default.
    pub(super) fn menu_order(&self) -> Vec<String> {
        let pol = policy::current();
        if let Some(v) = pol.fixed_value(policy::MENU_ORDER) {
            return policy::list(v);
        }
        if !self.settings.menu_order.is_empty() {
            return self.settings.menu_order.clone();
        }
        pol.recommended_value(policy::MENU_ORDER).map(policy::list).unwrap_or_default()
    }
}

/// Where Daleris itself puts a menu item.
fn builtin_mode<'a>(keys: impl Iterator<Item = &'a str>) -> MenuMode {
    for k in keys {
        if DEFAULT_HIDDEN.contains(&k) {
            return MenuMode::Hide;
        }
        if DEFAULT_MORE.contains(&k) {
            return MenuMode::More;
        }
    }
    MenuMode::Show
}

impl App {
    /// Entries in the person's order, each with where it should appear.
    pub(super) fn arrange(&self, mut entries: Vec<MEntry>) -> Vec<(MEntry, MenuMode)> {
        apply_order(&mut entries, &self.menu_order(), |e| e.key.as_str());
        entries.into_iter().map(|e| (self.entry_mode(&e), e)).map(|(m, e)| (e, m)).collect()
    }

    fn set_mode(&mut self, e: &MEntry, mode: MenuMode) {
        let default = {
            let mut probe = self.settings.menu_rules.clone();
            probe.remove(&e.key);
            if let Some(a) = &e.alt_key {
                probe.remove(a);
            }
            let saved = std::mem::replace(&mut self.settings.menu_rules, probe);
            let d = self.entry_mode(e);
            self.settings.menu_rules = saved;
            d
        };
        if let Some(a) = &e.alt_key {
            self.settings.menu_rules.remove(a);
        }
        if mode == default {
            self.settings.menu_rules.remove(&e.key);
        } else {
            self.settings.menu_rules.insert(e.key.clone(), (mode, e.label.clone()));
        }
        self.save_settings();
    }

    // ------------------------------------------------------------------ the real menu (our own popup)

    /// Open the right-click menu at the pointer for an item (or the folder background).
    /// `target` (items, folder) is remembered so "Customize this menu…" can edit exactly this menu.
    pub(super) fn open_popup(&mut self, m: MenuCtx, background: bool, target: (Vec<PathBuf>, PathBuf)) {
        let pos = self.ctx.input(|i| i.pointer.interact_pos()).unwrap_or_default();
        self.popup_serial += 1;
        self.customizer.target = Some((target.0, target.1, background));
        self.popup = Some(Popup {
            serial: self.popup_serial,
            ctx: m,
            background,
            pos,
            open: Vec::new(),
            rects: Vec::new(),
            sizes: HashMap::new(),
            close: false,
            opened: self.ctx.input(|i| i.time),
            hover: (usize::MAX, String::new(), 0.0),
        });
    }

    /// Draw the open right-click menu and its submenus. Submenus open to the right of
    /// their item, or to the left when there is no room, and move up near the screen bottom.
    pub(super) fn popup_ui(&mut self, ctx: &egui::Context) {
        let Some(mut pp) = self.popup.take() else { return };
        let (pressed, ptr, now, esc) = ctx.input(|i| (i.pointer.any_pressed(), i.pointer.interact_pos(), i.time, i.key_pressed(Key::Escape)));
        if esc {
            return;
        }
        if pressed && now - pp.opened > 0.08 {
            if let Some(p) = ptr {
                if !pp.rects.iter().any(|r| r.expand(2.0).contains(p)) {
                    return; // clicked somewhere else: close
                }
            }
        }
        let sm = self.shell_menu.take();
        let tex = std::mem::take(&mut self.shell_tex);
        let entries = if pp.background { self.background_entries(&pp.ctx, sm.as_ref().map(|s| (s, &tex))) } else { self.item_entries(&pp.ctx, sm.as_ref().map(|s| (s, &tex))) };
        self.shell_menu = sm;
        self.shell_tex = tex;
        let main = with_more_submenu(self.arrange(entries));
        pp.rects.clear();
        let anchor = Rect::from_min_size(pp.pos, egui::Vec2::ZERO);
        self.popup_level(ctx, &mut pp, &main, 0, anchor);
        if !pp.close {
            self.popup = Some(pp);
        }
    }

    fn popup_level(&mut self, ctx: &egui::Context, pp: &mut Popup, entries: &[MEntry], depth: usize, anchor: Rect) {
        let key = format!("{depth}:{}", if depth == 0 { "" } else { pp.open.get(depth - 1).map(|s| s.as_str()).unwrap_or("") });
        let size = pp.sizes.get(&key).copied().unwrap_or(vec2(320.0, 200.0));
        let screen = ctx.content_rect().shrink(4.0);
        let pos = if depth == 0 {
            let mut p = anchor.min;
            if p.x + size.x > screen.right() {
                p.x = (p.x - size.x).max(screen.left());
            }
            if p.y + size.y > screen.bottom() {
                p.y = (screen.bottom() - size.y).max(screen.top());
            }
            p
        } else {
            let mut x = anchor.right() + 9.0;
            if x + size.x > screen.right() {
                x = (anchor.left() - 9.0 - size.x).max(screen.left());
            }
            let mut y = anchor.top() - 7.0;
            if y + size.y > screen.bottom() {
                y = (screen.bottom() - size.y).max(screen.top());
            }
            pos2(x, y)
        };
        let id = egui::Id::unique(("ctxpop", pp.serial, key.clone()));
        let width = if depth == 0 { 310.0 } else { 280.0 };
        let shown = egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(pos).constrain(false).show(ctx, |ui| {
            egui::Frame::menu(ui.style()).show(ui, |ui| {
                ui.set_width(width);
                let mut last: Option<u32> = None;
                for e in entries {
                    if last.is_some_and(|g| g != e.group) && !(last >= Some(G_TAIL) && e.group >= G_TAIL) {
                        ui.separator();
                    }
                    last = Some(e.group);
                    self.popup_entry(ctx, ui, pp, e, depth);
                }
            });
        });
        let r = shown.response.rect;
        pp.sizes.insert(key, r.size());
        pp.rects.push(r);
    }

    /// Hovering a row for a moment opens its submenu (or closes another one), like Windows.
    fn dwell(&self, ctx: &egui::Context, pp: &mut Popup, depth: usize, key: &str) -> bool {
        let now = ctx.input(|i| i.time);
        if pp.hover.0 != depth || pp.hover.1 != key {
            pp.hover = (depth, key.to_string(), now);
        }
        let ready = now - pp.hover.2 > 0.2;
        if !ready {
            ctx.request_repaint_after(Duration::from_millis(60));
        }
        ready
    }

    fn popup_entry(&mut self, ctx: &egui::Context, ui: &mut Ui, pp: &mut Popup, e: &MEntry, depth: usize) {
        match &e.kind {
            MKind::Quick(q) => {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let mut clicked: Option<Act> = None;
                    if icon_button(ui, I_CUT, tr("Cut (Ctrl+X)"), q.cut).clicked() {
                        clicked = Some(Act::Cut);
                    }
                    if icon_button(ui, I_COPY, tr("Copy (Ctrl+C)"), true).clicked() {
                        clicked = Some(Act::Copy);
                    }
                    if let Some(dest) = &q.paste_into {
                        if icon_button(ui, I_PASTE, tr("Paste into this folder"), true).clicked() {
                            clicked = Some(Act::PasteInto(dest.clone()));
                        }
                    }
                    if icon_button(ui, I_RENAME, tr("Rename (F2)"), q.rename).clicked() {
                        clicked = Some(Act::StartRename);
                    }
                    if icon_button(ui, I_DELETE, tr("Delete (Del)"), q.delete).clicked() {
                        clicked = Some(Act::Delete(false));
                    }
                    if let Some(a) = clicked {
                        self.acts.push(a);
                        pp.close = true;
                    }
                });
            }
            MKind::Sub(kids) => {
                let kids: Vec<MEntry> = kids.iter().filter(|k| self.entry_mode(k) != MenuMode::Hide).cloned().collect();
                if kids.is_empty() {
                    return;
                }
                let is_open = pp.open.get(depth) == Some(&e.key);
                let resp = pop_row(ui, e, true, is_open);
                if !is_open && (resp.clicked() || (resp.hovered() && self.dwell(ctx, pp, depth, &e.key))) {
                    pp.open.truncate(depth);
                    pp.open.push(e.key.clone());
                }
                if pp.open.get(depth) == Some(&e.key) {
                    self.popup_level(ctx, pp, &kids, depth + 1, resp.rect);
                }
            }
            MKind::Act(_) | MKind::Shell(_) => {
                let resp = pop_row(ui, e, false, false);
                if resp.hovered() && pp.open.len() > depth && self.dwell(ctx, pp, depth, &e.key) {
                    pp.open.truncate(depth);
                }
                if resp.clicked() && e.enabled {
                    self.acts.push(match &e.kind {
                        MKind::Act(a) => a.clone(),
                        MKind::Shell(id) => Act::ShellInvoke(*id),
                        _ => unreachable!(),
                    });
                    pp.close = true;
                }
            }
        }
    }

    // ------------------------------------------------------------------ WYSIWYG editor

    fn sample_path(&self, i: usize) -> Option<PathBuf> {
        let docs = self.places.iter().find(|p| p.name == "Documents").map(|p| p.path.clone()); // i18n-ignore: place names stay English inside the program
        let win = PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into()));
        let first = |c: Vec<PathBuf>| c.into_iter().find(|p| p.exists());
        match i {
            0 | 4 => docs,
            1 => first(vec![win.join("win.ini"), win.join("system.ini")]),
            2 => first(vec![win.join(r"Web\Wallpaper\Windows\img0.jpg"), win.join(r"Web\Screen\img100.jpg"), win.join(r"Web\Wallpaper\Windows\img19.jpg")]),
            _ => first(vec![win.join(r"System32\notepad.exe"), win.join("notepad.exe")]),
        }
    }

    /// (items, folder, background) the editor previews for sample `i`.
    fn sample_target(&self, i: usize) -> Option<(Vec<PathBuf>, PathBuf, bool)> {
        if i == 5 {
            return self.customizer.target.clone();
        }
        let p = self.sample_path(i)?;
        Some(if i == 4 { (Vec::new(), p, true) } else { (vec![p.clone()], p.parent().map(|x| x.to_path_buf()).unwrap_or_default(), false) })
    }

    fn sample_ctx(&self, items: &[PathBuf], dir: &Path, background: bool) -> MenuCtx {
        let row = items.first().filter(|_| !background).and_then(|p| { let q = p.clone(); bg::within(p, Duration::from_millis(300), move || fsx::stat(&q)).flatten().map(|e| (p, e)) }).map(|(p, e)| Row {
            key: e.name.clone(),
            ext: e.ext.clone(),
            is_dir: e.is_dir,
            size: e.size,
            mtime: e.mtime,
            ctime: e.ctime,
            attrs: e.attrs,
            name: e.name,
            path: p.clone(),
            folder: None,
        });
        MenuCtx {
            paths: if background { Vec::new() } else { items.to_vec() },
            dir: Some(dir.to_path_buf()),
            cloud: Cloud::None,
            in_cloud: false,
            pinned: false,
            searching: false,
            net_letter: None,
            view: self.settings.view,
            sort: SortKey::Name,
            asc: true,
            dirs_first: true,
            own_sort: false,
            row,
        }
    }

    pub(super) fn menu_editor(&mut self, ui: &mut Ui) {
        ui.label(egui::RichText::new(tr("Right-click menu")).heading());
        ui.label(
            egui::RichText::new(tr(
                "This is your menu exactly as it will look. Drag items to reorder them, or drag them into \"Show more options\" or \"Hidden\". \
                 Items from Windows and installed apps (7-Zip, Git, VS Code, Photos…) are included.",
            ))
            .color(DIM()),
        );
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(tr("Preview the menu for")).color(DIM()));
            for (i, s) in SAMPLES.iter().enumerate() {
                if i == 5 && self.customizer.target.is_none() {
                    continue;
                }
                if ui.selectable_label(self.customizer.sample == i, tr(s)).clicked() {
                    self.customizer.sample = i;
                }
            }
        });
        if let (5, Some((items, dir, bg))) = (self.customizer.sample, &self.customizer.target) {
            let what = if *bg {
                trf!("Showing the menu for the background of {folder}", folder = dir.display())
            } else {
                let list = items.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ");
                trf!("Showing the menu for {items}", items = list)
            };
            ui.label(egui::RichText::new(what).small().color(FAINT()));
        }
        ui.add_space(8.0);
        let sample = self.customizer.sample;
        let Some((items, dir, background)) = self.sample_target(sample) else {
            ui.label(egui::RichText::new(tr("No sample file of this kind was found on this PC.")).color(FAINT()));
            return;
        };
        if self.customizer.built_for != Some(sample) {
            self.customizer.built_for = Some(sample);
            self.customizer.shell = ShellMenu::build(&items, &dir, false);
            let sm = self.customizer.shell.take();
            self.customizer.tex = sm.as_ref().map(|s| self.shell_textures(s)).unwrap_or_default();
            self.customizer.shell = sm;
        }
        let m = self.sample_ctx(&items, &dir, background);
        let sm = self.customizer.shell.take();
        let tex = std::mem::take(&mut self.customizer.tex);
        let entries = if background { self.background_entries(&m, sm.as_ref().map(|s| (s, &tex))) } else { self.item_entries(&m, sm.as_ref().map(|s| (s, &tex))) };
        self.customizer.shell = sm;
        self.customizer.tex = tex;
        let list = self.arrange(entries);

        let pol = policy::current();
        if pol.menu_locked() {
            settings_page::managed_note(ui);
            ui.add_space(6.0);
        }
        let menu_enabled = !pol.menu_locked();
        let wide = ui.available_width() > 820.0;
        let menu_w = if wide { (ui.available_width() * 0.56).min(520.0) } else { ui.available_width().min(560.0) };
        let mut handled = false;
        let draw = |app: &mut App, ui: &mut Ui, handled: &mut bool| {
            ui.vertical(|ui| {
                ui.set_width(menu_w);
                app.editor_zone(ui, &list, MenuMode::Show, tr("Menu"), "", handled);
            });
            if wide {
                ui.add_space(16.0);
            }
            ui.vertical(|ui| {
                ui.set_width(if wide { ui.available_width().max(240.0) } else { menu_w });
                app.editor_zone(ui, &list, MenuMode::More, tr("Show more options submenu"), tr("Rarely used items, one level down."), handled);
                ui.add_space(12.0);
                app.editor_zone(ui, &list, MenuMode::Hide, tr("Not shown"), tr("Never shown. Includes Windows' own copies of commands this menu already has."), handled);
                ui.add_space(8.0);
                if ui.button(tr("Reset this menu to defaults")).clicked() {
                    let keys: HashSet<String> = list.iter().flat_map(|(e, _)| std::iter::once(e.key.clone()).chain(e.alt_key.clone())).collect();
                    app.settings.menu_rules.retain(|k, _| !keys.contains(k));
                    app.settings.menu_order.retain(|k| !keys.contains(k));
                    app.save_settings();
                }
            });
        };
        ui.add_enabled_ui(menu_enabled, |ui| {
            if wide {
                ui.horizontal_top(|ui| draw(self, ui, &mut handled));
            } else {
                draw(self, ui, &mut handled);
            }
        });
        // the dragged item follows the pointer
        if let (Some(d), Some(pos)) = (egui::DragAndDrop::payload::<MenuDrag>(ui.ctx()), ui.ctx().pointer_interact_pos()) {
            if let Some((e, _)) = list.iter().find(|(e, _)| e.key == d.0) {
                let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("menudrag")));
                let g = painter.layout_no_wrap(e.label.clone(), FontId::proportional(13.5), TEXT());
                let r = Rect::from_min_size(pos + vec2(12.0, -14.0), g.size() + vec2(20.0, 12.0));
                painter.rect_filled(r, CornerRadius::same(6), RAISED());
                painter.rect_stroke(r, CornerRadius::same(6), Stroke::new(1.0_f32, ACCENT()), StrokeKind::Inside);
                painter.galley(r.min + vec2(10.0, 6.0), g, TEXT());
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            }
        }
        ui.add_space(18.0);
        if pol.no_custom_commands {
            ui.label(egui::RichText::new(tr("Your own commands")).strong());
            settings_page::managed_note(ui);
        } else if pol.has_fixed(policy::COMMANDS) {
            ui.label(egui::RichText::new(tr("Your own commands")).strong());
            settings_page::managed_note(ui);
            for cmd in self.settings.commands() {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&cmd.label).strong());
                    ui.label(egui::RichText::new(&cmd.command).monospace().color(DIM()));
                });
            }
        } else {
            ui.add_enabled_ui(menu_enabled, |ui| self.custom_commands_editor(ui));
        }
        ui.add_space(12.0);
        ui.add_enabled_ui(menu_enabled, |ui| self.other_shell_items(ui));
    }

    /// One drop zone of the editor (the menu, the "Show more options" submenu, or hidden).
    fn editor_zone(&mut self, ui: &mut Ui, list: &[(MEntry, MenuMode)], zone: MenuMode, title: &str, sub: &str, handled: &mut bool) {
        ui.label(egui::RichText::new(title).strong());
        if !sub.is_empty() {
            ui.label(egui::RichText::new(sub).color(DIM()).small());
        }
        let dragging = egui::DragAndDrop::has_payload_of_type::<MenuDrag>(ui.ctx());
        let (_, dropped) = ui.dnd_drop_zone::<MenuDrag, ()>(menu_frame(), |ui| {
            ui.set_width(ui.available_width());
            let items: Vec<&(MEntry, MenuMode)> = list.iter().filter(|(_, m)| *m == zone).collect();
            let mut last: Option<u32> = None;
            // where the "Show more options" submenu sits: first in the bottom block, as in the real menu
            let more_row = |ui: &mut Ui| {
                ui.add(egui::Separator::default().spacing(4.0));
                ui.horizontal(|ui| {
                    ui.add_space(26.0);
                    ui.label(egui::RichText::new(format!("{}   {}  ›", ic(I_MORE), tr("Show more options"))).font(icon_font(13.5)).color(FAINT()));
                });
            };
            let mut more_placed = zone != MenuMode::Show;
            for (e, _) in &items {
                if !more_placed && e.group >= G_TAIL {
                    more_row(ui);
                    more_placed = true;
                    last = Some(G_TAIL);
                }
                if zone == MenuMode::Show && last.is_some_and(|g| g != e.group && !(g >= G_TAIL && e.group >= G_TAIL)) {
                    ui.add(egui::Separator::default().spacing(4.0));
                }
                last = Some(e.group);
                self.editor_row(ui, list, e, zone, handled, 0.0, None);
                self.editor_children(ui, list, e, zone, handled, 1);
            }
            if !more_placed {
                more_row(ui);
            }
            // hidden items from inside submenus are listed here too, so they can be brought back
            if zone == MenuMode::Hide {
                for (child, parent) in self.hidden_children(list) {
                    self.editor_row(ui, list, &child, zone, handled, 0.0, Some(&parent));
                }
            }
            if (items.is_empty() && !(zone == MenuMode::Hide && !self.hidden_children(list).is_empty())) || dragging {
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), if items.is_empty() { 40.0 } else { 26.0 }), Sense::hover());
                ui.painter().text(r.center(), Align2::CENTER_CENTER, if dragging { tr("Drop here") } else { tr("Drag items here") }, FontId::proportional(12.5), FAINT());
            }
        });
        if let Some(d) = dropped {
            if !*handled {
                *handled = true;
                self.move_entry(list, &d.0, None, zone);
            }
        }
    }

    /// One row: drawn exactly like the menu item, and draggable as a whole.
    /// `indent` > 0 for items inside an unfolded submenu; `parent` names the submenu of a hidden child.
    #[allow(clippy::too_many_arguments)]
    fn editor_row(&mut self, ui: &mut Ui, list: &[(MEntry, MenuMode)], e: &MEntry, zone: MenuMode, handled: &mut bool, indent: f32, parent: Option<&str>) {
        let w = ui.available_width();
        let (full, row) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click_and_drag());
        let rect = Rect::from_min_max(full.min + vec2(indent, 0.0), full.max);
        let is_sub = matches!(e.kind, MKind::Sub(_));
        if is_sub && row.clicked() {
            if !self.customizer.expanded.remove(&e.key) {
                self.customizer.expanded.insert(e.key.clone());
            }
        }
        let being_dragged = egui::DragAndDrop::payload::<MenuDrag>(ui.ctx()).is_some_and(|d| d.0 == e.key);
        let drop_here = row.dnd_hover_payload::<MenuDrag>().is_some_and(|d| d.0 != e.key);
        if row.hovered() && !being_dragged {
            ui.painter().rect_filled(rect, CornerRadius::same(4), HOVER());
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        if drop_here {
            ui.painter().rect_filled(Rect::from_min_size(rect.left_top() - vec2(0.0, 1.5), vec2(rect.width(), 3.0)), CornerRadius::same(2), ACCENT());
        }
        if row.drag_started() {
            row.dnd_set_drag_payload(MenuDrag(e.key.clone()));
        }
        if let Some(d) = row.dnd_release_payload::<MenuDrag>() {
            if !*handled && d.0 != e.key {
                *handled = true;
                self.move_entry(list, &d.0, Some(&e.key), zone);
            }
        }
        let fade = if being_dragged { 0.35 } else if zone == MenuMode::Hide { 0.55 } else { 1.0 };
        ui.painter().text(pos2(rect.left() + 11.0, rect.center().y), Align2::CENTER_CENTER, ic(I_GRIP), icon_font(11.0), FAINT().gamma_multiply(fade));
        let ic_c = pos2(rect.left() + 36.0, rect.center().y);
        if let Some(t) = e.tex {
            ui.painter().image(t, Rect::from_center_size(ic_c, vec2(16.0, 16.0)), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE.gamma_multiply(fade));
        } else if let Some(gl) = e.glyph {
            ui.painter().text(ic_c, Align2::CENTER_CENTER, ic(gl), icon_font(13.0), (if e.bold { ACCENT() } else { DIM() }).gamma_multiply(fade));
        }
        let mut label = e.label.clone();
        if let Some(p) = parent {
            label = trf!("{item}   (in {submenu})", item = label, submenu = p);
        }
        if is_sub {
            let open = self.customizer.expanded.contains(&e.key);
            ui.painter().text(rect.right_center() - vec2(14.0, 0.0), Align2::CENTER_CENTER, ic(if open { I_CHEV_D } else { I_CHEV_R }), icon_font(10.0), DIM().gamma_multiply(fade));
        }
        let col = TEXT().gamma_multiply(fade);
        let gal = truncated(ui, &label, FontId::proportional(13.5), col, rect.width() - 64.0 - if e.shortcut.is_empty() && !is_sub { 0.0 } else { 70.0 });
        let pos = pos2(rect.left() + 54.0, rect.center().y - gal.size().y / 2.0);
        ui.painter().galley(pos, gal.clone(), col);
        if e.bold {
            ui.painter().galley(pos + vec2(0.6, 0.0), gal, col);
        }
        if !e.shortcut.is_empty() && !is_sub {
            ui.painter().text(rect.right_center() - vec2(10.0, 0.0), Align2::RIGHT_CENTER, e.shortcut, FontId::proportional(12.0), FAINT().gamma_multiply(fade));
        }
    }

    /// The (non-hidden) items of an unfolded submenu, indented under it.
    fn editor_children(&mut self, ui: &mut Ui, list: &[(MEntry, MenuMode)], e: &MEntry, zone: MenuMode, handled: &mut bool, level: u32) {
        let MKind::Sub(kids) = &e.kind else { return };
        if !self.customizer.expanded.contains(&e.key) {
            return;
        }
        for k in kids {
            if self.entry_mode(k) == MenuMode::Hide {
                continue;
            }
            self.editor_row(ui, list, k, zone, handled, 22.0 * level as f32, None);
            self.editor_children(ui, list, k, zone, handled, level + 1);
        }
    }

    /// Hidden items that live inside submenus, with the submenu's name.
    fn hidden_children(&self, list: &[(MEntry, MenuMode)]) -> Vec<(MEntry, String)> {
        fn walk(app: &App, e: &MEntry, out: &mut Vec<(MEntry, String)>) {
            if let MKind::Sub(kids) = &e.kind {
                for k in kids {
                    if app.entry_mode(k) == MenuMode::Hide {
                        out.push((k.clone(), e.label.clone()));
                    } else {
                        walk(app, k, out);
                    }
                }
            }
        }
        let mut out = Vec::new();
        for (e, _) in list {
            walk(self, e, &mut out);
        }
        out
    }

    /// Put `key` before `before` (or at the end of `zone`) and give it that zone's mode.
    fn move_entry(&mut self, list: &[(MEntry, MenuMode)], key: &str, before: Option<&str>, zone: MenuMode) {
        let Some((entry, mode)) = list.iter().find(|(e, _)| e.key == key).cloned() else {
            // an item inside a submenu: it stays in its submenu, it can only be shown or hidden
            if let Some(child) = find_child(list.iter().map(|(e, _)| e), key) {
                self.set_mode(&child, if zone == MenuMode::Hide { MenuMode::Hide } else { MenuMode::Show });
            }
            return;
        };
        let mut keys: Vec<String> = list.iter().map(|(e, _)| e.key.clone()).collect();
        keys.retain(|k| k != key);
        let at = match before.and_then(|b| keys.iter().position(|k| k == b)) {
            Some(i) => i,
            None => list
                .iter()
                .filter(|(e, m)| *m == zone && e.key != key)
                .filter_map(|(e, _)| keys.iter().position(|k| *k == e.key))
                .max()
                .map(|i| i + 1)
                .unwrap_or(keys.len()),
        };
        keys.insert(at, key.to_string());
        let rest: Vec<String> = self.menu_order().iter().filter(|k| !keys.contains(k)).cloned().collect();
        keys.extend(rest);
        self.settings.menu_order = keys;
        if mode != zone {
            self.set_mode(&entry, zone); // also saves
        } else {
            self.save_settings();
        }
    }

    fn custom_commands_editor(&mut self, ui: &mut Ui) {
        ui.label(egui::RichText::new(tr("Your own commands")).strong());
        ui.label(egui::RichText::new(tr("Run any program on the right-clicked items. Use {path}, {paths}, {name} and {dir} in the command.")).color(DIM()));
        ui.add_space(4.0);
        let mut remove = None;
        for (i, cmd) in self.settings.custom.iter().enumerate() {
            ui.horizontal(|ui| {
                if ui.small_button(egui::RichText::new(ic(I_DELETE)).font(icon_font(12.0))).on_hover_text(tr("Remove")).clicked() {
                    remove = Some(i);
                }
                ui.label(egui::RichText::new(&cmd.label).strong());
                ui.label(egui::RichText::new(trf!("for {what}", what = applies_text(&cmd.applies))).color(DIM()).small());
                ui.label(egui::RichText::new(&cmd.command).monospace().color(DIM()));
            });
        }
        if let Some(i) = remove {
            self.settings.custom.remove(i);
            self.save_settings();
        }
        let c = &mut self.customizer;
        if c.new_applies.is_empty() {
            c.new_applies = "all".into();
        }
        let mut add = false;
        egui::Grid::new("newcmd").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
            ui.label(tr("Name"));
            ui.add(egui::TextEdit::singleline(&mut c.new_label).hint_text(tr("Open in VS Code")).desired_width(320.0));
            ui.end_row();
            ui.label(tr("Command"));
            ui.add(egui::TextEdit::singleline(&mut c.new_command).hint_text("code {paths}").desired_width(320.0));
            ui.end_row();
            ui.label(tr("Show for"));
            ui.horizontal_wrapped(|ui| {
                for (v, l) in [("all", tr("Everything")), ("files", tr("Files")), ("folders", tr("Folders")), ("background", tr("Folder background")), ("ext", tr("File types:"))] {
                    ui.radio_value(&mut c.new_applies, v.to_string(), l);
                }
            });
            ui.end_row();
            if c.new_applies == "ext" {
                ui.label("");
                ui.add(egui::TextEdit::singleline(&mut c.new_exts).hint_text("jpg, png, gif").desired_width(200.0));
                ui.end_row();
            }
            ui.label("");
            let ready = !c.new_label.trim().is_empty() && !c.new_command.trim().is_empty();
            add = ui.add_enabled(ready, egui::Button::new(egui::RichText::new(format!("{}  {}", ic(I_ADD), tr("Add command"))).font(icon_font(13.0)))).clicked();
            ui.end_row();
        });
        if add {
            let c = &mut self.customizer;
            let applies = if c.new_applies == "ext" {
                format!("ext:{}", c.new_exts.split([',', ' ', ';']).map(|e| e.trim().trim_start_matches('.').to_lowercase()).filter(|e| !e.is_empty()).collect::<Vec<_>>().join(","))
            } else {
                c.new_applies.clone()
            };
            let cmd = CustomCmd { label: c.new_label.trim().into(), command: c.new_command.trim().into(), applies };
            c.new_label.clear();
            c.new_command.clear();
            self.settings.custom.push(cmd);
            self.save_settings();
        }
    }

    /// Windows items seen on other file types this session, with their rule.
    fn other_shell_items(&mut self, ui: &mut Ui) {
        let mut all: BTreeMap<String, String> = self.seen_shell.clone();
        for (k, (_, l)) in &self.settings.menu_rules {
            if k.starts_with("v:") || k.starts_with("l:") {
                all.entry(k.clone()).or_insert(l.clone());
            }
        }
        if all.is_empty() {
            return;
        }
        egui::CollapsingHeader::new(egui::RichText::new(trf!("All Windows & app items seen so far ({n})", n = crate::util::fmt_count(all.len()))).strong()).default_open(false).show(ui, |ui| {
            egui::Grid::new("shellrules").num_columns(2).striped(true).spacing(vec2(16.0, 4.0)).show(ui, |ui| {
                for (key, label) in all {
                    let probe = MEntry { key: key.clone(), alt_key: None, label: label.clone(), glyph: None, tex: None, shortcut: "", bold: false, enabled: true, group: 0, kind: MKind::Shell(0) };
                    let mut mode = self.entry_mode(&probe);
                    ui.label(&label);
                    ui.horizontal(|ui| {
                        for (mm, word) in [(MenuMode::Show, tr("In menu")), (MenuMode::More, tr("In \"Show more options\"")), (MenuMode::Hide, tr("Not shown"))] {
                            if ui.radio_value(&mut mode, mm, word).changed() {
                                self.set_mode(&probe, mm);
                            }
                        }
                    });
                    ui.end_row();
                }
            });
        });
    }
}

/// Find an item anywhere inside the submenus of `entries`.
fn find_child<'a>(entries: impl Iterator<Item = &'a MEntry>, key: &str) -> Option<MEntry> {
    for e in entries {
        if let MKind::Sub(kids) = &e.kind {
            if let Some(k) = kids.iter().find(|k| k.key == key) {
                return Some(k.clone());
            }
            if let Some(k) = find_child(kids.iter(), key) {
                return Some(k);
            }
        }
    }
    None
}

fn menu_frame() -> egui::Frame {
    egui::Frame::new().fill(RAISED()).stroke(Stroke::new(1.0_f32, LINE())).corner_radius(8).inner_margin(egui::Margin::same(7))
}

/// A menu row (our own or Windows'), with its icon or glyph.
/// `chevron` marks a submenu; `open` keeps it highlighted while its submenu shows.
fn pop_row(ui: &mut Ui, e: &MEntry, chevron: bool, open: bool) -> Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 28.0), if e.enabled { Sense::click() } else { Sense::hover() });
    if e.enabled && (resp.hovered() || open) {
        ui.painter().rect_filled(rect, CornerRadius::same(4), HOVER());
    }
    if chevron {
        ui.painter().text(rect.right_center() - vec2(12.0, 0.0), Align2::CENTER_CENTER, ic(I_CHEV_R), icon_font(10.0), DIM());
    }
    let c = rect.left_center() + vec2(14.0, 0.0);
    if let Some(t) = e.tex {
        ui.painter().image(t, Rect::from_center_size(c, vec2(16.0, 16.0)), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    } else if let Some(g) = e.glyph {
        ui.painter().text(c, Align2::CENTER_CENTER, ic(g), icon_font(14.0), if e.bold { ACCENT() } else { DIM() });
    }
    let col = if e.enabled { TEXT() } else { FAINT() };
    let g = truncated(ui, &e.label, FontId::proportional(13.5), col, w - 44.0 - if e.shortcut.is_empty() { 0.0 } else { 70.0 });
    let pos = rect.left_center() + vec2(32.0, -g.size().y / 2.0);
    ui.painter().galley(pos, g.clone(), col);
    if e.bold {
        ui.painter().galley(pos + vec2(0.6, 0.0), g, col); // faux bold
    }
    if !e.shortcut.is_empty() && !chevron {
        ui.painter().text(rect.right_center() - vec2(10.0, 0.0), Align2::RIGHT_CENTER, e.shortcut, FontId::proportional(12.0), FAINT());
    }
    resp
}

fn applies_text(a: &str) -> String {
    match a {
        "all" => tr("everything").into(),
        "files" => tr("files").into(),
        "folders" => tr("folders").into(),
        "background" => tr("folder background").into(),
        other => other.strip_prefix("ext:").map(|e| format!(".{}", e.replace(',', ", ."))).unwrap_or_else(|| other.to_string()),
    }
}

fn custom_applies(c: &CustomCmd, row: Option<&Row>) -> bool {
    match (c.applies.as_str(), row) {
        ("background", None) => true,
        ("all", Some(_)) => true,
        ("files", Some(r)) => !r.is_dir,
        ("folders", Some(r)) => r.is_dir,
        (a, Some(r)) if a.starts_with("ext:") => !r.is_dir && a[4..].split(',').any(|e| e == r.ext),
        _ => false,
    }
}

/// Run a custom command: `{path}` first item, `{paths}` all items, `{name}`, `{dir}`.
pub(super) fn run_custom(c: &CustomCmd, paths: &[PathBuf], dir: &Path) -> Result<(), String> {
    // quote a path for the system's command line
    let q = |p: &Path| if cfg!(windows) { format!("\"{}\"", p.display()) } else { format!("'{}'", p.display().to_string().replace('\'', "'\\''")) };
    let first = paths.first().map(|p| q(p)).unwrap_or_else(|| q(dir));
    let all = if paths.is_empty() { q(dir) } else { paths.iter().map(|p| q(p)).collect::<Vec<_>>().join(" ") };
    let name = paths.first().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mut line = c.command.clone();
    if !["{path}", "{paths}", "{name}", "{dir}"].iter().any(|p| line.contains(p)) {
        line.push_str(" {paths}");
    }
    let line = line.replace("{paths}", &all).replace("{path}", &first).replace("{name}", &name).replace("{dir}", &q(dir));
    let line = line.trim();
    #[cfg(not(windows))]
    {
        // Linux / macOS: the shell runs it (handles quoting, scripts and PATH lookups)
        return std::process::Command::new("sh").arg("-c").arg(line).current_dir(dir).spawn().map(|_| ()).map_err(|e| trf!("Couldn't run '{name}': {error}", name = c.label, error = e));
    }
    #[cfg(windows)]
    run_custom_windows(c, line, dir)
}

#[cfg(windows)]
fn run_custom_windows(c: &CustomCmd, line: &str, dir: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let (prog, rest) = if let Some(r) = line.strip_prefix('"') {
        match r.find('"') {
            Some(e) => (&r[..e], r[e + 1..].trim_start()),
            None => (r, ""),
        }
    } else {
        line.split_once(' ').unwrap_or((line, ""))
    };
    let mut cmd = std::process::Command::new(prog);
    cmd.raw_arg(rest).current_dir(dir);
    match cmd.spawn() {
        Ok(_) => Ok(()),
        // not an .exe (a .bat, a script, a document): let the shell run it
        Err(_) => std::process::Command::new("cmd.exe")
            .raw_arg(format!("/C \"{line}\""))
            .current_dir(dir)
            .creation_flags(0x0800_0000)
            .spawn()
            .map(|_| ())
            .map_err(|e| trf!("Couldn't run '{name}': {error}", name = c.label, error = e)),
    }
}
