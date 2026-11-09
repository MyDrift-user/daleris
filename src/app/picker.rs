//! Daleris as an app's Open or Save dialog: `daleris --pick`, started by the DLL in `crates/dialogs`.
//!
//! It's the normal list and sidebar, made into a dialog: nothing of the main window's that's for managing files or
//! tabs, and the fields and buttons of a dialog along the bottom. The request comes on standard input and the answer goes to
//! standard output, pipes only the DLL and this process hold. This process has one job, so it writes no settings and
//! looks for no updates. If the app goes away the pipe breaks and this ends too.

use std::io::{BufRead, Write};
use std::sync::Mutex;

use daleris_integration::target::{kind_within, PathKind};
use daleris_integration::pick::{first_extension, matches_filter, Control, ControlKind, HostDecoder, HostMessage, Kind, PickReply, PickRequest, STATE_ENABLED, STATE_VISIBLE};

use super::*;

pub struct Picker {
    pub request: PickRequest,
    name: String,
    filter: usize,
    /// a name to replace, waiting for a yes or no
    confirm_overwrite: Option<PathBuf>,
    error: Option<String>,
    told_ready: bool,
    answered: bool,
    /// what selection last filled the name box in, so that it only follows when the selection changes
    seen_selection: Vec<PathBuf>,
    frames: u32,
    focus_name: bool,
    /// the controls the app added, as it last described them
    controls: Vec<Control>,
    /// the app is handling a control, since when
    busy: Option<Instant>,
    /// this window stepped back from being on top, for a window of the app's to be seen
    stepped_back: bool,
    /// where the app was last told the person is
    told: Option<(usize, PathBuf, String, PathBuf)>,
    /// is the section with the app's controls open? Nobody has said yet, when it's None.
    controls_open: Option<bool>,
    /// what Tab goes through, as drawn this frame
    tab_ids: Vec<egui::Id>,
    name_tab_index: usize,
}

/// What the app said since the last frame. The thread reading it can't reach the window any other way.
static FROM_APP: Mutex<Vec<HostMessage>> = Mutex::new(Vec::new());

/// How long this looks at a path before saying it doesn't know. It's a dialog someone is waiting in.
const LOOK: Duration = Duration::from_millis(400);

/// How long the app has to deal with a control before this steps back for it. Most handlers are over before this.
const STEP_BACK_AFTER: Duration = Duration::from_millis(300);

/// Reads the request. Nothing else may run until it's here, and a parent that never sends one ends this.
pub fn read_request() -> Option<PickRequest> {
    let stdin = std::io::stdin();
    let mut text = String::new();
    let mut handle = stdin.lock();
    loop {
        let mut line = String::new();
        if handle.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let done = line.trim_end() == "end";
        text.push_str(&line);
        if done {
            break;
        }
    }
    drop(handle);
    PickRequest::decode(&text)
}

fn tell(reply: &PickReply) {
    let mut out = std::io::stdout();
    let _ = out.write_all(reply.encode().as_bytes());
    let _ = out.flush();
}

/// Listens to the app. When it goes away this does too, as nobody would be left to give the answer to.
pub fn end_with_parent() {
    let _ = std::thread::Builder::new().name("parent".into()).spawn(|| {
        let mut line = String::new();
        let mut decoder = HostDecoder::default();
        while matches!(std::io::stdin().read_line(&mut line), Ok(n) if n > 0) {
            if let Some(message) = decoder.feed(&line) {
                FROM_APP.lock().unwrap_or_else(|e| e.into_inner()).push(message);
            }
            line.clear();
        }
        std::process::exit(0);
    });
}

impl Picker {
    pub fn new(request: PickRequest) -> Picker {
        Picker {
            name: request.file_name.clone(),
            filter: request.filter_index.min(request.filters.len().saturating_sub(1)),
            confirm_overwrite: None,
            error: None,
            told_ready: false,
            answered: false,
            seen_selection: Vec::new(),
            frames: 0,
            focus_name: true,
            controls: request.controls.clone(),
            busy: None,
            stepped_back: false,
            told: None,
            controls_open: None,
            tab_ids: Vec::new(),
            name_tab_index: 0,
            request,
        }
    }

    pub fn start_folder(&self) -> Option<PathBuf> {
        let p = PathBuf::from(&self.request.folder);
        if self.request.folder.is_empty() {
            return None;
        }
        // a folder on a server that's down opens as that, with the page that says so
        matches!(kind_within(&p, LOOK), PathKind::Dir | PathKind::Unknown).then_some(p)
    }

    pub fn window_title(&self) -> String {
        let what = if !self.request.title.is_empty() {
            self.request.title.clone()
        } else {
            match self.request.kind {
                Kind::Open => tr("Open").to_string(),
                Kind::Save => tr("Save as").to_string(),
                Kind::Folder => tr("Select folder").to_string(),
            }
        };
        if self.request.host.is_empty() {
            what
        } else {
            format!("{what} · {}", self.request.host)
        }
    }

    /// Should this entry show in the list?
    pub fn shows(&self, name: &str, is_dir: bool) -> bool {
        if is_dir {
            return true;
        }
        if self.request.kind == Kind::Folder {
            return false;
        }
        self.request.filters.get(self.filter).is_none_or(|(_, spec)| matches_filter(name, spec))
    }

    fn answer(&mut self, reply: PickReply) {
        self.answered = true;
        tell(&reply);
        // the app has what it asked for, and this process has nothing else to do
        std::process::exit(0);
    }

    /// The window was closed some way other than the buttons.
    pub fn cancel_if_unanswered(&mut self) {
        if !self.answered {
            self.answer(PickReply::Cancelled);
        }
    }

    pub fn answer_cancel(&mut self) {
        self.answer(PickReply::Cancelled);
    }

    /// A file was opened in the list, which here means it was chosen.
    pub fn choose(&mut self, paths: Vec<PathBuf>, dir: &Path) -> Option<PathBuf> {
        if self.request.kind == Kind::Save {
            if let Some(name) = paths.first().and_then(|p| p.file_name()) {
                self.name = name.to_string_lossy().into_owned();
            }
            return self.accept(dir, &[]);
        }
        self.accept(dir, &paths)
    }

    /// What OK means now. Gives back a folder to go to when that's what was typed.
    fn accept(&mut self, dir: &Path, selected: &[PathBuf]) -> Option<PathBuf> {
        self.error = None;
        let typed = self.name.trim().trim_matches('"').to_string();
        match self.request.kind {
            Kind::Folder => {
                let chosen = if !typed.is_empty() && kind_within(&dir.join(&typed), LOOK) == PathKind::Dir {
                    dir.join(&typed)
                } else {
                    selected.iter().find(|p| kind_within(p, LOOK) == PathKind::Dir).cloned().unwrap_or_else(|| dir.to_path_buf())
                };
                self.answer(PickReply::Picked { filter_index: self.filter, paths: vec![chosen.into_os_string()] });
                None
            }
            Kind::Open => {
                let from_selection: Vec<PathBuf> = selected.iter().filter(|p| kind_within(p, LOOK) != PathKind::Dir).cloned().collect();
                let matches_selection = from_selection.first().and_then(|p| p.file_name()).is_some_and(|n| n.to_string_lossy() == typed);
                let paths = if !from_selection.is_empty() && (typed.is_empty() || matches_selection || from_selection.len() > 1) {
                    if self.request.multi {
                        from_selection
                    } else {
                        from_selection.into_iter().take(1).collect()
                    }
                } else if typed.is_empty() {
                    return selected.iter().find(|p| kind_within(p, LOOK) == PathKind::Dir).cloned();
                } else if self.request.multi && self.name.trim().starts_with('"') {
                    split_names(&self.name).into_iter().map(|n| dir.join(n)).collect()
                } else {
                    vec![dir.join(&typed)]
                };
                if let Some(folder) = paths.iter().find(|p| kind_within(p, LOOK) == PathKind::Dir) {
                    return Some(folder.clone());
                }
                if self.request.must_exist {
                    if let Some(missing) = paths.iter().find(|p| kind_within(p, LOOK) == PathKind::Missing) {
                        self.error = Some(trf!("{path} isn't there.", path = missing.display()));
                        return None;
                    }
                }
                self.answer(PickReply::Picked { filter_index: self.filter, paths: paths.into_iter().map(PathBuf::into_os_string).collect() });
                None
            }
            Kind::Save => {
                if typed.is_empty() {
                    self.error = Some(tr("Type a name to save as.").into());
                    return selected.iter().find(|p| kind_within(p, LOOK) == PathKind::Dir).cloned();
                }
                if typed.contains(['<', '>', '|', '?', '*']) {
                    self.error = Some(tr("A file name can't hold < > | ? or *.").into());
                    return None;
                }
                let path = self.with_extension(dir.join(&typed));
                let there = kind_within(&path, LOOK);
                if there == PathKind::Dir {
                    self.name.clear();
                    return Some(path);
                }
                if self.request.must_exist && path.parent().is_some_and(|p| kind_within(p, LOOK) == PathKind::Missing) {
                    let folder = path.parent().unwrap_or(Path::new("")).display().to_string();
                    self.error = Some(trf!("{path} isn't there.", path = folder));
                    return None;
                }
                if self.request.overwrite_prompt && there == PathKind::File {
                    self.confirm_overwrite = Some(path);
                    return None;
                }
                self.answer(PickReply::Picked { filter_index: self.filter, paths: vec![path.into_os_string()] });
                None
            }
        }
    }

    /// As the real dialog does: a name without an extension gets the one of the file type chosen.
    fn with_extension(&self, path: PathBuf) -> PathBuf {
        if path.extension().is_some() {
            return path;
        }
        let from_filter = self.request.filters.get(self.filter).and_then(|(_, s)| first_extension(s));
        let ext = from_filter.or(Some(self.request.default_ext.as_str()).filter(|e| !e.is_empty()));
        match ext {
            Some(e) => {
                let mut name = path.into_os_string();
                name.push(".");
                name.push(e.trim_start_matches('.'));
                PathBuf::from(name)
            }
            None => path,
        }
    }

    /// Test hook: DALERIS_TEST_PICK answers on its own, to run the whole round trip with nobody at the keyboard.
    /// `ok:<path>` or `ok:<path>|<path>`, `cancel`, `windows`, `die`, `hang`, `silent`.
    /// DALERIS_TEST_PICK_CONTROLS does things to the controls first, one per frame from frame 5, as
    /// `check:<id>=1;item:<id>=<item>;button:<id>;activate:<id>;text:<id>=<text>`.
    fn test_answer(&mut self) {
        if let Ok(actions) = std::env::var("DALERIS_TEST_PICK_CONTROLS") {
            if let Some(action) = actions.split(';').nth((self.frames as usize).wrapping_sub(5)) {
                let (what, rest) = action.split_once(':').unwrap_or((action, ""));
                let (id, value) = rest.split_once('=').unwrap_or((rest, ""));
                let id = id.parse().unwrap_or(0);
                match what {
                    "check" => self.do_check(id, value == "1"),
                    "item" => self.do_item(id, value.parse().unwrap_or(0)),
                    "button" => tell(&PickReply::Button { id }),
                    "activate" => tell(&PickReply::Activating { id }),
                    "text" => self.do_text(id, value.to_string()),
                    _ => {}
                }
            }
        }
        let Ok(spec) = std::env::var("DALERIS_TEST_PICK") else { return };
        // later than the actions above, however many of those there are
        let actions = std::env::var("DALERIS_TEST_PICK_CONTROLS").map_or(0, |a| a.split(';').count() as u32);
        if self.frames != 20 + actions * 2 {
            return;
        }
        // lets the app finish with what those did
        if actions > 0 {
            std::thread::sleep(Duration::from_millis(300));
        }
        match spec.as_str() {
            "cancel" => self.answer(PickReply::Cancelled),
            "windows" => self.answer(PickReply::UseWindows),
            "die" => std::process::abort(),
            "hang" | "silent" => loop {
                std::thread::sleep(Duration::from_secs(60));
            },
            other => {
                if let Some(list) = other.strip_prefix("ok:") {
                    let paths = list.split('|').map(std::ffi::OsString::from).collect();
                    self.answer(PickReply::Picked { filter_index: self.filter, paths });
                }
            }
        }
    }
}

impl Picker {
    fn do_check(&mut self, id: u32, on: bool) {
        if let Some(c) = self.controls.iter_mut().find(|c| c.id == id) {
            c.checked = on;
        }
        tell(&PickReply::Check { id, on });
    }

    fn do_item(&mut self, control: u32, item: u32) {
        if let Some(c) = self.controls.iter_mut().find(|c| c.id == control && c.kind != ControlKind::Menu) {
            c.selected = Some(item);
        }
        tell(&PickReply::Item { control, item });
    }

    fn do_text(&mut self, id: u32, text: String) {
        if let Some(c) = self.controls.iter_mut().find(|c| c.id == id) {
            c.text = text.clone();
        }
        tell(&PickReply::Text { id, text });
    }

    /// What the app said since the last frame. Gives back a folder it wants this in.
    fn read_app(&mut self, ctx: &egui::Context, t: &mut Tab) -> Option<PathBuf> {
        let mut go_to = None;
        for message in std::mem::take(&mut *FROM_APP.lock().unwrap_or_else(|e| e.into_inner())) {
            match message {
                HostMessage::Controls(list) => self.controls = list,
                HostMessage::Busy(true) => self.busy = Some(Instant::now()),
                HostMessage::Busy(false) => self.busy = None,
                HostMessage::SetName(name) => self.name = name,
                HostMessage::SetFilter(i) => {
                    self.filter = i.min(self.request.filters.len().saturating_sub(1));
                    t.dirty = true;
                }
                HostMessage::SetFolder(p) => go_to = Some(PathBuf::from(p)),
            }
        }
        // A handler that's taking a while may be showing a window of its own, which has to be able to come in front.
        let step_back = self.busy.is_some_and(|b| b.elapsed() >= STEP_BACK_AFTER);
        if step_back != self.stepped_back {
            self.stepped_back = step_back;
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(if step_back { egui::WindowLevel::Normal } else { egui::WindowLevel::AlwaysOnTop }));
            #[cfg(windows)]
            if step_back {
                use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY};
                let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
            }
        }
        go_to
    }

    /// Tells the app where the person is, when that changes.
    fn tell_app(&mut self, dir: Option<&Path>, selected: &[PathBuf]) {
        let now = (self.filter, dir.map(Path::to_path_buf).unwrap_or_default(), self.name.clone(), selected.first().cloned().unwrap_or_default());
        if self.told.as_ref() != Some(&now) {
            tell(&PickReply::State { filter_index: now.0, folder: now.1.clone().into_os_string(), name: now.2.clone(), selection: now.3.clone().into_os_string() });
            self.told = Some(now);
        }
    }

    /// What the app added: a section of its own, above the name box. Labels share a column, groups are set apart, and
    /// a lot of controls take two columns, then fold away, so the name box and the buttons always stay in view.
    fn controls_ui(&mut self, ui: &mut Ui) {
        let shown = |c: &Control| c.state & STATE_VISIBLE != 0 && c.kind != ControlKind::OpenDropDown;
        let count = self.controls.iter().filter(|c| shown(c) && c.kind != ControlKind::Separator).count();
        if count == 0 {
            return;
        }
        // prominent ones first, as Windows' dialog puts those by the buttons; then in the order they were added
        let top = |c: &Control| shown(c) && c.group.is_none();
        let mut order: Vec<usize> = (0..self.controls.len()).filter(|&i| top(&self.controls[i]) && self.controls[i].prominent).collect();
        order.extend((0..self.controls.len()).filter(|&i| top(&self.controls[i]) && !self.controls[i].prominent));
        // no line where a section starts or ends
        while order.last().is_some_and(|&i| self.controls[i].kind == ControlKind::Separator) {
            order.pop();
        }
        while order.first().is_some_and(|&i| self.controls[i].kind == ControlKind::Separator) {
            order.remove(0);
        }
        // a small window has better things to show first
        let tall_enough = ui.ctx().content_rect().height() >= 560.0;
        let open = *self.controls_open.get_or_insert(count <= FOLD_CONTROLS_OVER && tall_enough);
        let whose = if self.request.host.is_empty() { tr("Options").to_string() } else { trf!("Options from {app}", app = self.request.host) };
        let frame = egui::Frame::new().fill(BG()).stroke(Stroke::new(1.0_f32, LINE())).corner_radius(8).inner_margin(egui::Margin::symmetric(12, 8));
        frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            let (r, head) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
            let chev = if open { I_CHEV_D } else { I_CHEV_R };
            ui.painter().text(r.left_center() + vec2(6.0, 0.0), Align2::CENTER_CENTER, ic(chev), icon_font(9.0), DIM());
            let title = if open { whose.clone() } else { format!("{whose} ({count})") };
            ui.painter().text(r.left_center() + vec2(18.0, 0.0), Align2::LEFT_CENTER, title, FontId::proportional(12.5), DIM());
            if head.clicked() {
                self.controls_open = Some(!open);
            }
            if !open {
                return;
            }
            ui.add_space(6.0);
            // the label column: as wide as the longest label needs, within reason
            let longest = self.controls.iter().filter(|c| shown(c) && matches!(c.kind, ControlKind::Edit | ControlKind::Combo | ControlKind::Radio)).map(|c| plain(&c.label)).map(|l| ui.fonts_mut(|f| f.layout_no_wrap(l, FontId::proportional(13.5), TEXT()).size().x)).fold(0.0_f32, f32::max);
            let width = ui.available_width();
            // how many rows each top-level control takes, so that the two columns come out even
            let rows = |c: &Control| -> usize {
                let _ = c;
                1
            };
            let weights: Vec<usize> = order
                .iter()
                .map(|&i| {
                    let c = &self.controls[i];
                    let inside: usize = if c.kind == ControlKind::Group { self.controls.iter().filter(|m| m.group == Some(c.id) && shown(m)).map(rows).sum() } else { 0 };
                    rows(c) + inside
                })
                .collect();
            let total: usize = weights.iter().sum();
            let two = total >= 5 && width >= 620.0;
            // room for what's indented inside a group as well
            let label_w = (longest + 24.0).clamp(0.0, if two { (width / 2.0 - 180.0).max(90.0) } else { 240.0 });
            let max_h = (ui.ctx().content_rect().height() * 0.34).clamp(90.0, 280.0);
            egui::ScrollArea::vertical().id_salt("picker_controls").max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
                if two {
                    // where the two sides come out most even
                    let mut half = 0;
                    let mut sum: usize = 0;                    let mut best = usize::MAX;
                    for i in 0..=order.len() {
                        let gap = (sum * 2).abs_diff(total);
                        if gap < best {
                            best = gap;
                            half = i;
                        }
                        sum += weights.get(i).copied().unwrap_or(0);
                    }
                    let (left, right) = order.split_at(half);
                    // a line that separates nothing goes
                    let is_line = |i: &&usize| self.controls[**i].kind == ControlKind::Separator;
                    let trim = |part: &[usize]| -> Vec<usize> {
                        let mut v: Vec<usize> = part.iter().copied().skip_while(|i| is_line(&i)).collect();
                        while v.last().is_some_and(|i| is_line(&i)) {
                            v.pop();
                        }
                        v
                    };
                    let (left, right) = (trim(left), trim(right));
                    ui.horizontal_top(|ui| {
                        let w = (ui.available_width() - 20.0) / 2.0;
                        for (side, part) in [&left, &right].into_iter().enumerate() {
                            ui.allocate_ui_with_layout(vec2(w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                                ui.set_width(w);
                                self.control_rows(ui, part, label_w);
                            });
                            if side == 0 {
                                ui.add_space(20.0);
                            }
                        }
                    });
                } else {
                    self.control_rows(ui, &order, label_w);
                }
            });
        });
        ui.add_space(10.0);
    }

    fn control_rows(&mut self, ui: &mut Ui, list: &[usize], label_w: f32) {
        for &i in list {
            self.control_row(ui, i, label_w);
            if self.controls[i].kind == ControlKind::Group {
                let id = self.controls[i].id;
                let members: Vec<usize> = (0..self.controls.len()).filter(|&m| self.controls[m].group == Some(id) && self.controls[m].state & STATE_VISIBLE != 0).collect();
                // what's in a group sits inside it
                egui::Frame::new().inner_margin(egui::Margin { left: 12, right: 0, top: 0, bottom: 4 }).show(ui, |ui| {
                    for m in members {
                        self.control_row(ui, m, (label_w - 12.0).max(0.0));
                    }
                });
            }
        }
    }

    /// One row of the section: its label in the label column, the control beside it.
    fn control_row(&mut self, ui: &mut Ui, index: usize, label_w: f32) {
        let c = self.controls[index].clone();
        let label = plain(&c.label);
        ui.spacing_mut().item_spacing.y = 4.0;
        match c.kind {
            ControlKind::Group => {
                ui.add_space(4.0);
                if !label.is_empty() {
                    ui.add(egui::Label::new(egui::RichText::new(label).size(12.5).strong().color(DIM())).wrap());
                }
                ui.add_space(2.0);
            }
            ControlKind::Separator => {
                ui.add_space(3.0);
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0_f32, LINE()));
                ui.add_space(3.0);
            }
            // those without a label of their own line up with the fields
            ControlKind::Check | ControlKind::Text | ControlKind::Button | ControlKind::Menu => {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.add_space(label_w);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        self.control_ui(ui, index);
                    });
                });
            }
            ControlKind::Edit | ControlKind::Combo | ControlKind::OpenDropDown | ControlKind::Radio => {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.allocate_ui_with_layout(vec2(label_w, 28.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.set_min_size(vec2(label_w, 28.0));
                        ui.set_max_width(label_w);
                        let shown = ui.add(egui::Label::new(egui::RichText::new(&label).size(13.5)).truncate());
                        if shown.rect.width() + 1.0 >= label_w {
                            let _ = shown.on_hover_text(&label);
                        }
                    });
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        self.control_ui(ui, index);
                    });
                });
            }
        }
    }

    /// One control. `&` in a label marks a keyboard shortcut in Windows' dialog.
    fn control_ui(&mut self, ui: &mut Ui, index: usize) {
        let c = self.controls[index].clone();
        let id = c.id;
        let label = plain(&c.label);
        // what's in a disabled group is disabled too
        let group_on = c.group.and_then(|g| self.controls.iter().find(|o| o.id == g)).is_none_or(|g| g.state & STATE_ENABLED != 0);
        let enabled = group_on && c.state & STATE_ENABLED != 0;
        let items: Vec<_> = c.items.iter().filter(|i| i.state & STATE_VISIBLE != 0).collect();
        let field_w = ui.available_width().clamp(120.0, 340.0);
        match c.kind {
            ControlKind::Group | ControlKind::Separator => {}
            ControlKind::Check => {
                let mut on = c.checked;
                let r = ui.add_enabled(enabled, egui::Checkbox::new(&mut on, label));
                self.in_tab_order(ui, &r);
                if r.changed() {
                    self.do_check(id, on);
                }
            }
            ControlKind::Edit => {
                let mut text = c.text.clone();
                let r = ui.add_enabled(enabled, egui::TextEdit::singleline(&mut text).desired_width(field_w).margin(egui::Margin::symmetric(8, 5)).lock_focus(true));
                self.tab_ids.push(r.id);
                if r.changed() {
                    self.do_text(id, text);
                }
            }
            ControlKind::Text => {
                ui.add_enabled(enabled, egui::Label::new(egui::RichText::new(label).color(DIM())).wrap());
            }
            ControlKind::Button => {
                let r = ui.add_enabled(enabled, egui::Button::new(label).fill(RAISED()).stroke(Stroke::new(1.0_f32, FAINT())).corner_radius(6).min_size(vec2(84.0, 28.0)));
                self.in_tab_order(ui, &r);
                if r.clicked() {
                    tell(&PickReply::Button { id });
                }
            }
            ControlKind::Radio => {
                // a line of them, wrapping if it has to. Only `add_enabled` in here: a scope wouldn't wrap.
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    for item in items {
                        let on = enabled && item.state & STATE_ENABLED != 0;
                        let r = ui.add_enabled(on, egui::RadioButton::new(c.selected == Some(item.id), plain(&item.label)));
                        self.in_tab_order(ui, &r);
                        if r.clicked() && c.selected != Some(item.id) {
                            self.do_item(id, item.id);
                        }
                    }
                });
            }
            ControlKind::Combo | ControlKind::OpenDropDown => {
                let current = c.items.iter().find(|i| Some(i.id) == c.selected).map(|i| plain(&i.label)).unwrap_or_default();
                let list: Vec<(String, bool)> = items.iter().map(|i| (plain(&i.label), i.state & STATE_ENABLED != 0)).collect();
                let at = items.iter().position(|i| Some(i.id) == c.selected);
                let width = if c.kind == ControlKind::OpenDropDown { 130.0 } else { field_w };
                let (r, picked) = field_dropdown(ui, ("picker_control", id), width, &current, &list, at, enabled);
                self.in_tab_order(ui, &r);
                if r.clicked() {
                    tell(&PickReply::Activating { id });
                }
                if let Some(i) = picked {
                    self.do_item(id, items[i].id);
                }
            }
            ControlKind::Menu => {
                if !enabled {
                    ui.add_enabled(false, egui::Button::new(label).min_size(vec2(84.0, 28.0)));
                    return;
                }
                // a button as well, not loose text
                ui.visuals_mut().widgets.inactive.weak_bg_fill = RAISED();
                ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::new(1.0_f32, FAINT());
                let menu = ui.menu_button(format!("{label}  ▾"), |ui| {
                    for item in &items {
                        if ui.add_enabled(item.state & STATE_ENABLED != 0, egui::Button::new(plain(&item.label))).clicked() {
                            self.do_item(id, item.id);
                            ui.close();
                        }
                    }
                });
                if menu.response.clicked() {
                    tell(&PickReply::Activating { id });
                }
            }
        }
    }

    /// Tab goes through what's in this part of the dialog, in the order it's added here. It would also go through
    /// every row of the list otherwise, and the sidebar.
    fn in_tab_order(&mut self, ui: &Ui, r: &Response) {
        if !r.enabled() {
            return;
        }
        self.tab_ids.push(r.id);
        ui.memory_mut(|m| m.set_focus_lock_filter(r.id, TAB_IS_OURS));
        if r.has_focus() {
            ui.painter().rect_stroke(r.rect.expand(2.0), CornerRadius::same(7), Stroke::new(1.5_f32, ACCENT()), StrokeKind::Outside);
        }
    }

    fn move_focus(&mut self, ctx: &egui::Context) {
        let (tab, back) = ctx.input(|i| (i.key_pressed(Key::Tab), i.modifiers.shift));
        if !tab || self.tab_ids.is_empty() || egui::Popup::is_any_open(ctx) {
            return;
        }
        let at = ctx.memory(|m| m.focused()).and_then(|f| self.tab_ids.iter().position(|i| *i == f));
        let n = self.tab_ids.len();
        let next = match at {
            Some(i) if back => (i + n - 1) % n,
            Some(i) => (i + 1) % n,
            None => self.name_tab_index,
        };
        ctx.memory_mut(|m| m.request_focus(self.tab_ids[next]));
    }

    /// The name box. Selects what's there when it first gets focus, without the extension, as the real dialog does.
    fn name_box(&mut self, ui: &mut Ui, width: f32, hint: &str) -> Response {
        let edit = egui::TextEdit::singleline(&mut self.name)
            .id_salt("picker_name")
            .desired_width(width)
            .margin(egui::Margin::symmetric(8, 6))
            .hint_text(egui::RichText::new(hint).color(FAINT()))
            .lock_focus(true);
        let r = ui.add(edit);
        if std::mem::take(&mut self.focus_name) {
            r.request_focus();
            let chars = self.name.chars().count();
            let end = match self.name.rfind('.') {
                Some(dot) if self.request.kind == Kind::Save && dot > 0 => self.name[..dot].chars().count(),
                _ => chars,
            };
            if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), r.id) {
                st.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(end))));
                st.store(ui.ctx(), r.id);
            } else {
                let mut st = egui::text_edit::TextEditState::default();
                st.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(end))));
                st.store(ui.ctx(), r.id);
            }
        }
        self.name_tab_index = self.tab_ids.len();
        self.in_tab_order_quiet(ui, &r);
        r
    }

    /// The same, for the name box, which shows its focus by itself.
    fn in_tab_order_quiet(&mut self, _ui: &Ui, r: &Response) {
        self.tab_ids.push(r.id);
    }

    fn name_label(&self) -> String {
        if !self.request.name_label.is_empty() {
            plain(&self.request.name_label).trim_end_matches(':').to_string()
        } else if self.request.kind == Kind::Folder {
            tr("Folder").to_string()
        } else {
            tr("File name").to_string()
        }
    }

    fn ok_label(&self) -> String {
        if !self.request.ok_label.is_empty() {
            return plain(&self.request.ok_label);
        }
        match self.request.kind {
            Kind::Open => tr("Open").to_string(),
            Kind::Save => tr("Save").to_string(),
            Kind::Folder => tr("Select folder").to_string(),
        }
    }
}

/// A label without Windows' mark for a keyboard shortcut. `&&` is a real `&`.
fn plain(label: &str) -> String {
    label.replace("&&", "\u{1}").replace('&', "").replace('\u{1}', "&")
}

/// Tab moves focus in this dialog's own order: see `in_tab_order`.
const TAB_IS_OURS: egui::EventFilter = egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: false };

/// How many of an app's controls show without being asked for.
const FOLD_CONTROLS_OVER: usize = 10;

/// A dropdown that looks like one: a field with an arrow at its end. Gives back the choice made, if any.
pub(super) fn field_dropdown(ui: &mut Ui, salt: impl std::hash::Hash + std::fmt::Debug, width: f32, current: &str, items: &[(String, bool)], at: Option<usize>, enabled: bool) -> (Response, Option<usize>) {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 30.0), if enabled { Sense::click() } else { Sense::hover() });
    let popup = ui.make_persistent_id(salt).with("popup");
    let open = egui::Popup::is_id_open(ui.ctx(), popup);
    let fill = if !enabled {
        BG()
    } else if resp.hovered() || open {
        HOVER()
    } else {
        RAISED()
    };
    let line = if open || resp.has_focus() { ACCENT() } else { FAINT() };
    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
    ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0_f32, line), StrokeKind::Inside);
    let text_col = if enabled { TEXT() } else { FAINT() };
    let gal = truncated(ui, current, FontId::proportional(13.5), text_col, width - 40.0);
    let full = gal.size().x >= width - 41.0;
    ui.painter().galley(pos2(rect.left() + 10.0, rect.center().y - gal.size().y / 2.0), gal, text_col);
    ui.painter().text(rect.right_center() - vec2(14.0, 0.0), Align2::CENTER_CENTER, ic(I_CHEV_D), icon_font(10.0), if enabled { DIM() } else { FAINT() });
    let mut picked = None;
    if enabled {
        // up and down change it without opening it, as in the real dialog
        if resp.has_focus() && !open {
            let (up, down) = ui.input(|i| (i.key_pressed(Key::ArrowUp), i.key_pressed(Key::ArrowDown)));
            let step = |from: usize, forward: bool| {
                let mut i = from;
                loop {
                    i = if forward { i + 1 } else { i.checked_sub(1)? };
                    match items.get(i) {
                        Some((_, true)) => return Some(i),
                        Some(_) => {}
                        None => return None,
                    }
                }
            };
            if up {
                picked = at.and_then(|a| step(a, false));
            } else if down {
                picked = match at {
                    Some(a) => step(a, true),
                    None => items.iter().position(|(_, on)| *on),
                };
            }
        }
        // It opens and closes with a click on the field, which a click-to-close popup must not take for a click outside.
        egui::Popup::from_toggle_button_response(&resp).id(popup).close_behavior(egui::PopupCloseBehavior::CloseOnClick).show(|ui| {
            ui.set_min_width(width - 12.0);
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for (i, (label, on)) in items.iter().enumerate() {
                    let row = ui.add_enabled(*on, egui::Button::selectable(at == Some(i), label));
                    if row.clicked() {
                        picked = Some(i);
                    }
                }
            });
        });
    }
    let resp = if full { resp.on_hover_text(current) } else { resp };
    (resp, picked)
}

/// A row of the field grid: the label in its column, then what `add` adds.
fn field_row(ui: &mut Ui, label_w: f32, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let (r, _) = ui.allocate_exact_size(vec2(label_w, 30.0), Sense::hover());
        let gal = truncated(ui, label, FontId::proportional(13.5), TEXT(), label_w - 8.0);
        ui.painter().galley(pos2(r.left(), r.center().y - gal.size().y / 2.0), gal, TEXT());
        add(ui);
    });
}

/// `"a.txt" "b.txt"`, as the real dialog shows several chosen files.
fn quoted_names(names: &[String]) -> String {
    names.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join(" ")
}

/// The other way round. A name that isn't quoted is one name, spaces and all.
fn split_names(text: &str) -> Vec<String> {
    let t = text.trim();
    if !t.starts_with('"') {
        return if t.is_empty() { Vec::new() } else { vec![t.to_string()] };
    }
    t.split('"').enumerate().filter(|(i, s)| i % 2 == 1 && !s.is_empty()).map(|(_, s)| s.to_string()).collect()
}

impl App {
    /// The dialog's top: where it is and how the list looks. Nothing for managing tabs, or the window.
    /// The bottom of the dialog: the app's controls, the name, the type and the buttons.
    pub(super) fn picker_bar(&mut self, root: &mut Ui, t: &mut Tab) {
        let ctx = &root.ctx().clone();
        let Some(mut p) = self.picker.take() else { return };
        p.frames += 1;
        p.tab_ids.clear();
        if !p.told_ready && std::env::var("DALERIS_TEST_PICK").as_deref() != Ok("silent") {
            p.told_ready = true;
            tell(&PickReply::Ready);
        }
        p.test_answer();
        if ctx.input(|i| i.viewport().close_requested()) {
            p.cancel_if_unanswered();
        }
        if let Some(folder) = p.read_app(ctx, t) {
            self.acts.push(Act::Navigate(Loc::Dir(folder)));
        }
        let selected = self.selected_paths(t);
        if selected != p.seen_selection {
            p.seen_selection = selected.clone();
            let dirs: HashSet<&str> = t.entries.iter().filter(|e| e.is_dir).map(|e| e.name.as_str()).collect();
            let is_dir = |s: &PathBuf| s.file_name().is_some_and(|n| dirs.contains(n.to_string_lossy().as_ref()));
            let names = |want_dirs: bool| -> Vec<String> { selected.iter().filter(|s| is_dir(s) == want_dirs).filter_map(|s| s.file_name().map(|n| n.to_string_lossy().into_owned())).collect() };
            match p.request.kind {
                Kind::Folder => {
                    if let [one] = names(true).as_slice() {
                        p.name = one.clone();
                    }
                }
                _ => {
                    let files = names(false);
                    if let [one] = files.as_slice() {
                        p.name = one.clone();
                    } else if files.len() > 1 {
                        p.name = quoted_names(&files);
                    }
                }
            }
        }
        let dir = t.dir().map(|d| d.to_path_buf());
        p.tell_app(dir.as_deref(), &selected);
        let mut go_to: Option<PathBuf> = None;
        let frame = egui::Frame::new().fill(PANEL()).inner_margin(egui::Margin { left: 16, right: 16, top: 12, bottom: 14 });
        egui::Panel::bottom("picker").frame(frame).show_separator_line(true).show(root, |ui| {
            // fields look like fields whether they have focus or not
            ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::new(1.0_f32, FAINT());
            ui.visuals_mut().widgets.hovered.bg_stroke = Stroke::new(1.0_f32, DIM());
            ui.visuals_mut().extreme_bg_color = RAISED();
            if let Some(path) = p.confirm_overwrite.clone() {
                egui::Frame::new().fill(RAISED()).stroke(Stroke::new(1.0_f32, WARN())).corner_radius(8).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        // the name alone, as a whole path can be wider than the window, and take the buttons with it
                        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        let room = ui.available_width() - 210.0;
                        let gal = truncated(ui, &trf!("{name} already exists. Replace it?", name = name), FontId::proportional(13.5), TEXT(), room);
                        let (r, _) = ui.allocate_exact_size(vec2(room, 30.0), Sense::hover());
                        ui.painter().galley(pos2(r.left(), r.center().y - gal.size().y / 2.0), gal, TEXT());
                        if action_button(ui, tr("Replace"), true).clicked() {
                            p.answer(PickReply::Picked { filter_index: p.filter, paths: vec![path.clone().into_os_string()] });
                        }
                        if action_button(ui, tr("Keep it"), false).clicked() {
                            p.confirm_overwrite = None;
                        }
                    });
                });
                ui.add_space(10.0);
            }
            if p.busy.is_some_and(|b| b.elapsed() >= STEP_BACK_AFTER) {
                ui.label(egui::RichText::new(trf!("Waiting for {app}", app = p.request.host)).color(DIM()));
                ui.add_space(4.0);
            }
            let waiting = p.busy.is_some();
            ui.add_enabled_ui(!waiting, |ui| p.controls_ui(ui));

            // the name and the type, on one grid
            let filters = p.request.filters.clone();
            let name_label = p.name_label();
            let type_label = if p.request.kind == Kind::Save { tr("Save as type") } else { tr("File type") };
            let measure = |ui: &Ui, s: &str| ui.fonts_mut(|f| f.layout_no_wrap(s.to_string(), FontId::proportional(13.5), TEXT()).size().x);
            let widest = if filters.is_empty() { measure(ui, &name_label) } else { measure(ui, &name_label).max(measure(ui, type_label)) };
            let label_w = (widest + 14.0).min(ui.available_width() * 0.3);
            let field_w = ui.available_width() - label_w;
            let hint = match p.request.kind {
                Kind::Folder => t.loc.title(),
                _ => String::new(),
            };
            let mut typed_enter = false;
            field_row(ui, label_w, &name_label, |ui| {
                let edit = p.name_box(ui, field_w - 1.0, &hint);
                typed_enter = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            });
            if !filters.is_empty() {
                ui.add_space(6.0);
                field_row(ui, label_w, type_label, |ui| {
                    let list: Vec<(String, bool)> = filters.iter().map(|(n, s)| (if s.is_empty() || n.contains(s.as_str()) { n.clone() } else { format!("{n} ({s})") }, true)).collect();
                    let current = list.get(p.filter).map(|(l, _)| l.clone()).unwrap_or_default();
                    let (r, picked) = field_dropdown(ui, "picker_filter", field_w, &current, &list, Some(p.filter), true);
                    p.in_tab_order(ui, &r);
                    if let Some(i) = picked {
                        p.filter = i;
                        t.dirty = true;
                    }
                });
            }
            if let Some(e) = &p.error {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add_space(label_w);
                    ui.add(egui::Label::new(egui::RichText::new(e).color(Color32::from_rgb(232, 96, 96))).truncate());
                });
            }
            ui.add_space(14.0);

            // the buttons: what the dialog is for at the right, the way out of it apart at the left
            let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
            let ok_text = p.ok_label();
            let ok_w = (measure(ui, &ok_text) + 32.0).max(96.0);
            let cancel_r = Rect::from_min_size(row.right_top() - vec2(96.0, 0.0), vec2(96.0, 32.0));
            let ok_r = Rect::from_min_size(cancel_r.left_top() - vec2(ok_w + 8.0, 0.0), vec2(ok_w, 32.0));
            // the choices an app puts on the OK button
            let menu_at = p.controls.iter().position(|c| c.kind == ControlKind::OpenDropDown && c.state & STATE_VISIBLE != 0);
            if let Some(index) = menu_at {
                let r = Rect::from_min_size(ok_r.left_top() - vec2(138.0, -1.0), vec2(130.0, 30.0));
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r).layout(egui::Layout::left_to_right(egui::Align::Center)));
                p.control_ui(&mut child, index);
            }
            let ok = ui.put(ok_r, primary_button(&ok_text, !waiting));
            p.in_tab_order(ui, &ok);
            if (ok.clicked() || typed_enter) && dir.is_some() && !waiting {
                go_to = p.accept(dir.as_deref().unwrap_or(Path::new(".")), &selected);
            }
            // Enter with nothing in particular focused means the same, unless the list has something to open
            let idle_enter = ctx.input(|i| i.key_pressed(Key::Enter)) && ctx.memory(|m| m.focused().is_none()) && selected.is_empty() && !typed_enter && self.editing_address.is_none() && !t.filter_open;
            if idle_enter && dir.is_some() && !waiting {
                go_to = p.accept(dir.as_deref().unwrap_or(Path::new(".")), &selected);
            }
            let cancel = ui.put(cancel_r, secondary_button(tr("Cancel")));
            p.in_tab_order(ui, &cancel);
            let quiet = egui::Button::new(egui::RichText::new(tr("Use Windows dialog")).size(12.5).color(DIM())).frame(false).min_size(vec2(0.0, 32.0));
            let windows_r = Rect::from_min_size(row.left_top() - vec2(4.0, 0.0), vec2(measure(ui, tr("Use Windows dialog")) - 2.0, 32.0));
            let windows = ui.put(windows_r, quiet).on_hover_text(tr("Close this and show the normal Windows dialog instead"));
            p.in_tab_order(ui, &windows);
            if windows.clicked() {
                p.answer(PickReply::UseWindows);
            }
            let esc = ctx.input(|i| i.key_pressed(Key::Escape));
            let elsewhere = self.popup.is_some() || self.editing_address.is_some() || t.filter_open || !t.query.is_empty() || egui::Popup::is_any_open(ctx);
            if esc && !elsewhere && p.confirm_overwrite.is_some() {
                // Esc takes back the last question, not the whole dialog
                p.confirm_overwrite = None;
            } else if cancel.clicked() || (esc && !elsewhere) {
                p.answer_cancel();
            }
        });
        p.move_focus(ctx);
        if let Some(folder) = go_to {
            self.acts.push(Act::Navigate(Loc::Dir(folder)));
        }
        self.picker = Some(p);
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    /// A file opened in the list, which in a picker means choosing it. True if the picker took it.
    pub(super) fn picker_takes_open(&mut self, paths: &[PathBuf], t: &mut Tab) -> bool {
        let Some(mut p) = self.picker.take() else { return false };
        let Some(dir) = t.dir().map(|d| d.to_path_buf()) else {
            self.picker = Some(p);
            return true;
        };
        let go_to = p.choose(paths.to_vec(), &dir);
        self.picker = Some(p);
        if let Some(folder) = go_to {
            self.navigate(t, Loc::Dir(folder));
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn several_names_go_both_ways() {
        let names = vec!["a b.txt".to_string(), "c.txt".to_string()];
        assert_eq!(split_names(&quoted_names(&names)), names);
        assert_eq!(split_names("plain name.txt"), ["plain name.txt"]);
        assert!(split_names("   ").is_empty());
    }
}
