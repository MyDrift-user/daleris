//! Window chrome: tab strip, navigation + address bar, search box, command bar,
//! status bar, sidebar and the "This PC" page.

use super::*;

/// How long dragged files are held over a tab before it opens.
const TAB_OPENS_AFTER: Duration = Duration::from_millis(600);

impl App {
    pub(super) fn tab_bar(&mut self, root: &mut Ui, t: &mut Tab) {
        let ctx = &root.ctx().clone();
        let custom = self.settings.title_custom;
        let frame = egui::Frame::new().fill(CHROME()).inner_margin(egui::Margin { left: 8, right: if custom { 0 } else { 8 }, top: if custom { 0 } else { 6 }, bottom: 0 });
        egui::Panel::top("tabs").exact_size(if custom { 40.0 } else { 38.0 }).frame(frame).show_separator_line(false).show(root, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                // title-bar parts (each can be switched off in Settings › Appearance)
                let mut left_w = 0.0;
                if custom && (self.settings.shows_logo() || self.settings.shows_name()) {
                    let name_w = if self.settings.shows_name() { ui.fonts_mut(|f| f.layout_no_wrap("Daleris".into(), FontId::proportional(13.0), TEXT()).size().x) + 10.0 } else { 0.0 };
                    let logo_w = if self.settings.shows_logo() { 26.0 } else { 0.0 };
                    let (r, resp) = ui.allocate_exact_size(vec2(logo_w + name_w + 8.0, 40.0), Sense::click_and_drag());
                    if self.settings.shows_logo() {
                        paint_logo(ui.painter(), Rect::from_center_size(pos2(r.left() + 11.0, r.center().y), vec2(18.0, 18.0)), ACCENT());
                    }
                    if self.settings.shows_name() {
                        ui.painter().text(pos2(r.left() + logo_w, r.center().y), Align2::LEFT_CENTER, "Daleris", FontId::proportional(13.0), DIM());
                    }
                    self.title_drag(ui, &resp);
                    left_w = r.width();
                }
                let right_w = if custom { 46.0 * 3.0 } else { 0.0 } + if self.settings.title_gear { 40.0 } else { 0.0 };
                let tab_y = if custom { 8.0 } else { 0.0 };
                let count = self.tabs.len();
                let avail = ui.available_width() - 40.0 - right_w - 24.0;
                let _ = left_w;
                let w = (avail / count as f32).clamp(80.0, 230.0);
                let mut tab_under_drag = None;
                for i in 0..count {
                    let (loc, loading) = if i == self.active { (t.loc.clone(), t.loading) } else { (self.tabs[i].loc.clone(), self.tabs[i].loading) };
                    let active = i == self.active;
                    let (slot, resp) = ui.allocate_exact_size(vec2(w, 32.0 + tab_y), Sense::click());
                    let rect = Rect::from_min_max(slot.min + vec2(0.0, tab_y), slot.max);
                    let resp = resp.with_new_rect(rect);
                    if active {
                        ui.painter().rect_filled(rect, CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 }, PANEL());
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rect.shrink2(vec2(0.0, 2.0)), CornerRadius::same(6), HOVER().gamma_multiply(0.6));
                    }
                    let icon_r = Rect::from_center_size(rect.left_center() + vec2(18.0, 0.0), vec2(16.0, 16.0));
                    match &loc {
                        Loc::ThisPc => {
                            ui.painter().text(icon_r.center(), Align2::CENTER_CENTER, ic(I_PC), icon_font(14.0), ACCENT());
                        }
                        Loc::Settings => {
                            ui.painter().text(icon_r.center(), Align2::CENTER_CENTER, ic(I_SETTINGS), icon_font(14.0), ACCENT());
                        }
                        Loc::Dir(path) => {
                            let path = path.clone();
                            self.draw_icon(ui, icon_r, &path, "", true, 0);
                        }
                    }
                    let title = if active && t.searching() { format!("Search: {}", t.query) } else { loc.title() };
                    let gal = truncated(ui, &title, FontId::proportional(13.0), if active { TEXT() } else { DIM() }, w - 64.0);
                    ui.painter().galley(pos2(rect.left() + 32.0, rect.center().y - gal.size().y / 2.0), gal, TEXT());
                    if loading {
                        let c = rect.right_center() - vec2(36.0, 0.0);
                        let a = ui.input(|i| i.time) as f32 * 6.0;
                        ui.painter().circle_filled(c + vec2(a.cos(), a.sin()) * 3.0, 1.6, ACCENT());
                        ctx.request_repaint_after(Duration::from_millis(30));
                    }
                    let close_r = Rect::from_center_size(rect.right_center() - vec2(16.0, 0.0), vec2(20.0, 20.0));
                    let close = ui.interact(close_r, ui.scope_id().with(("close", i)), Sense::click());
                    if resp.hovered() || active || close.hovered() {
                        if close.hovered() {
                            ui.painter().rect_filled(close_r, CornerRadius::same(4), HOVER());
                        }
                        ui.painter().text(close_r.center(), Align2::CENTER_CENTER, ic(I_CLOSE), icon_font(10.0), DIM());
                    }
                    if close.clicked() || resp.middle_clicked() {
                        self.acts.push(Act::CloseTab(i));
                    } else if resp.clicked() {
                        self.acts.push(Act::SwitchTab(i));
                    }
                    // files dropped on a tab go into its folder; held over it, it opens, so they can go deeper
                    if resp.dnd_hover_payload::<DragFiles>().is_some() {
                        tab_under_drag = Some(i);
                        if let Loc::Dir(d) = &loc {
                            let ok = self.drop_zone(ui, &resp, d).unwrap_or(true);
                            list::drop_mark(ui.painter(), rect.shrink2(vec2(0.0, 2.0)), 6, ok);
                        }
                        match self.drag_tab_hover {
                            Some((at, since)) if at == i => {
                                if !active && since.elapsed() >= TAB_OPENS_AFTER {
                                    self.acts.push(Act::SwitchTab(i));
                                }
                            }
                            _ => self.drag_tab_hover = Some((i, Instant::now())),
                        }
                        ui.ctx().request_repaint_after(Duration::from_millis(100));
                    } else if let Loc::Dir(d) = &loc {
                        self.drop_zone(ui, &resp, d);
                    }
                    resp.on_hover_text(match &loc {
                        Loc::Dir(p) => p.to_string_lossy().to_string(),
                        other => other.title(),
                    });
                }
                if tab_under_drag.is_none() {
                    self.drag_tab_hover = None;
                }
                ui.add_space(2.0);
                ui.vertical(|ui| {
                    ui.add_space(tab_y + 1.0);
                    if icon_button(ui, I_ADD, "New tab (Ctrl+T)", true).clicked() {
                        self.acts.push(Act::NewTab);
                    }
                });
                // empty title-bar space: drag to move the window, double-click to maximise
                let free = (ui.available_width() - right_w).max(0.0);
                let (_, rest) = ui.allocate_exact_size(vec2(free, if custom { 40.0 } else { 32.0 }), Sense::click_and_drag());
                if custom {
                    self.title_drag(ui, &rest);
                } else if rest.double_clicked() {
                    self.acts.push(Act::NewTab);
                }
                if self.settings.title_gear {
                    ui.vertical(|ui| {
                        ui.add_space(if custom { 5.0 } else { 1.0 });
                        if icon_button(ui, I_SETTINGS, "Settings", true).clicked() {
                            self.acts.push(Act::OpenSettings(None));
                        }
                    });
                    ui.add_space(6.0);
                }
                if custom {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    self.maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                    let max_glyph = if self.maximized { I_RESTORE } else { I_MAX };
                    for (g, tip, close) in [(I_MIN, "Minimize", false), (max_glyph, if self.maximized { "Restore" } else { "Maximize" }, false), (I_WCLOSE, "Close", true)] {
                        let (r, resp) = ui.allocate_exact_size(vec2(46.0, 40.0), Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(r, CornerRadius::ZERO, if close { Color32::from_rgb(0xc4, 0x2b, 0x1c) } else { HOVER() });
                        }
                        let col = if close && resp.hovered() { Color32::WHITE } else { TEXT() };
                        ui.painter().text(r.center(), Align2::CENTER_CENTER, ic(g), icon_font(10.0), col);
                        if resp.clicked() {
                            ctx.send_viewport_cmd(match g {
                                I_MIN => egui::ViewportCommand::Minimized(true),
                                I_WCLOSE => egui::ViewportCommand::Close,
                                _ => egui::ViewportCommand::Maximized(!self.maximized),
                            });
                        }
                        let _ = resp.on_hover_text(tip);
                    }
                }
            });
        });
    }

    fn title_drag(&mut self, ui: &Ui, resp: &Response) {
        if resp.drag_started_by(egui::PointerButton::Primary) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        if resp.double_clicked() {
            let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
        }
    }

    /// Without the Windows frame we provide our own resize edges.
    pub(super) fn resize_grips(&mut self, ctx: &egui::Context) {
        if !self.settings.title_custom || self.maximized {
            return;
        }
        let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else { return };
        let r = ctx.content_rect();
        let m = 5.0;
        let (l, rt, tp, bt) = (pos.x <= r.left() + m, pos.x >= r.right() - m, pos.y <= r.top() + m, pos.y >= r.bottom() - m);
        use egui::viewport::ResizeDirection as D;
        let dir = match (l, rt, tp, bt) {
            (true, _, true, _) => D::NorthWest,
            (_, true, true, _) => D::NorthEast,
            (true, _, _, true) => D::SouthWest,
            (_, true, _, true) => D::SouthEast,
            (true, ..) => D::West,
            (_, true, ..) => D::East,
            (_, _, true, _) => D::North,
            (.., true) => D::South,
            _ => return,
        };
        ctx.set_cursor_icon(match dir {
            D::West | D::East => egui::CursorIcon::ResizeHorizontal,
            D::North | D::South => egui::CursorIcon::ResizeVertical,
            D::NorthWest | D::SouthEast => egui::CursorIcon::ResizeNwSe,
            _ => egui::CursorIcon::ResizeNeSw,
        });
        if ctx.input(|i| i.pointer.primary_pressed()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
        }
    }

    pub(super) fn toolbar(&mut self, root: &mut Ui, t: &mut Tab) {
        let frame = egui::Frame::new().fill(PANEL()).inner_margin(egui::Margin::symmetric(8, 7));
        egui::Panel::top("toolbar").exact_size(46.0).frame(frame).show_separator_line(false).show(root, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if icon_button(ui, I_BACK, "Back (Alt+Left)", !t.back.is_empty()).clicked() {
                    self.acts.push(Act::Back);
                }
                if icon_button(ui, I_FWD, "Forward (Alt+Right)", !t.fwd.is_empty()).clicked() {
                    self.acts.push(Act::Forward);
                }
                if icon_button(ui, I_UP, "Up (Alt+Up)", t.dir().is_some()).clicked() {
                    self.acts.push(Act::Up);
                }
                if icon_button(ui, I_REFRESH, "Refresh (F5)", true).clicked() {
                    self.acts.push(Act::Refresh);
                }
                ui.add_space(6.0);
                let search_w = (ui.available_width() * 0.32).clamp(220.0, 420.0);
                let addr_w = ui.available_width() - search_w - 8.0;
                self.address_bar(ui, t, addr_w);
                ui.add_space(8.0);
                self.search_box(ui, t, search_w);
            });
        });
    }

    fn address_bar(&mut self, ui: &mut Ui, t: &mut Tab, width: f32) {
        let (rect, resp) = ui.allocate_exact_size(vec2(width, 32.0), Sense::click());
        text_field(ui, rect);
        if self.editing_address.is_some() {
            ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0_f32, ACCENT()), StrokeKind::Inside);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(10.0, 6.0))));
            let buf = self.editing_address.as_mut().unwrap();
            let te = egui::TextEdit::singleline(buf)
                .frame(egui::Frame::NONE)
                .desired_width(rect.width() - 20.0)
                .hint_text("Type a path, e.g. C:\\Users, %APPDATA% or \\\\server\\share")
                .font(FontId::proportional(13.5));
            let r = child.add(te);
            if self.focus_address {
                r.request_focus();
                let len = buf.chars().count();
                if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), r.id) {
                    st.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(len))));
                    st.store(ui.ctx(), r.id);
                }
                self.focus_address = false;
            }
            if r.lost_focus() {
                let text = expand_env(buf.trim().trim_matches('"'));
                self.editing_address = None;
                if ui.input(|i| i.key_pressed(Key::Enter)) {
                    self.go_to_typed(&text);
                }
            }
            return;
        }
        // breadcrumbs
        let mut crumbs: Vec<(String, Loc)> = if t.loc == Loc::Settings { vec![("Settings".into(), Loc::Settings)] } else { vec![("This PC".into(), Loc::ThisPc)] };
        if let Loc::Dir(p) = &t.loc {
            let mut acc = PathBuf::new();
            for c in p.components() {
                acc.push(c.as_os_str());
                let label = match c {
                    std::path::Component::RootDir => continue,
                    std::path::Component::Prefix(pre) => {
                        let s = pre.as_os_str().to_string_lossy().to_string();
                        acc.push("\\");
                        self.drives.iter().find(|d| s.eq_ignore_ascii_case(&format!("{}:", d.letter))).map(|d| d.title()).unwrap_or(s)
                    }
                    other => other.as_os_str().to_string_lossy().to_string(),
                };
                crumbs.push((label, Loc::Dir(acc.clone())));
            }
        }
        let inner = rect.shrink2(vec2(6.0, 2.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)));
        child.spacing_mut().item_spacing.x = 0.0;
        child.set_clip_rect(rect);
        // when the path is too long, show its tail
        let widths: Vec<f32> = crumbs.iter().map(|(l, _)| child.fonts_mut(|f| f.layout_no_wrap(l.clone(), FontId::proportional(13.5), TEXT()).size().x) + 30.0).collect();
        let mut first = 0;
        while first + 1 < crumbs.len() && widths[first..].iter().sum::<f32>() > rect.width() - 40.0 {
            first += 1;
        }
        if first > 0 {
            child.label(egui::RichText::new("…  ").color(DIM()));
        }
        for (i, (label, loc)) in crumbs.iter().enumerate().skip(first) {
            if i > first {
                child.label(egui::RichText::new(ic(I_CHEV_R)).font(icon_font(9.0)).color(FAINT()));
            }
            let b = child.add(egui::Button::new(egui::RichText::new(label).size(13.5)).fill(Color32::TRANSPARENT).min_size(vec2(0.0, 26.0)));
            if b.clicked() {
                self.acts.push(Act::Navigate(loc.clone()));
            }
            if b.middle_clicked() {
                self.acts.push(Act::NavigateNewTab(loc.clone()));
            }
            // each part of the path takes dropped files into that folder
            if let Loc::Dir(d) = loc {
                if let Some(ok) = self.drop_zone(ui, &b, d) {
                    list::drop_mark(child.painter(), b.rect, 4, ok);
                }
            }
        }
        if resp.clicked() {
            self.start_address_edit(t);
        }
    }

    fn go_to_typed(&mut self, text: &str) {
        let lower = text.to_lowercase();
        // smb://, sftp://, ftp://, dav(s)://, nfs://, afp://: connect (Linux/macOS)
        if !cfg!(windows) && ["smb://", "sftp://", "ssh://", "ftp://", "ftps://", "dav://", "davs://", "nfs://", "afp://", "webdav://"].iter().any(|s| lower.starts_with(s)) {
            self.acts.push(Act::ConnectServer(text.to_string(), String::new(), String::new()));
            return;
        }
        if lower == "settings" {
            self.acts.push(Act::Navigate(Loc::Settings));
            return;
        }
        if lower == "this pc" || lower.is_empty() {
            self.acts.push(Act::Navigate(Loc::ThisPc));
            return;
        }
        let p = if text.len() == 2 && text.ends_with(':') { PathBuf::from(format!("{text}\\")) } else { PathBuf::from(text) };
        // no answer yet may be a server that's down: going there says what's wrong, without stopping here to find out
        let kind = if fsx::is_server_root(&p).is_some() { target::PathKind::Dir } else { target::kind_within(&p, Duration::from_millis(200)) };
        match kind {
            target::PathKind::Dir | target::PathKind::Unknown => self.acts.push(Act::Navigate(Loc::Dir(p))),
            target::PathKind::File => self.acts.push(Act::Open(p)),
            target::PathKind::Missing => self.toast(format!("Can't find '{text}'"), true),
        }
    }

    fn search_box(&mut self, ui: &mut Ui, t: &mut Tab, width: f32) {
        let (rect, _) = ui.allocate_exact_size(vec2(width, 32.0), Sense::hover());
        text_field(ui, rect);
        let cy = rect.center().y;
        ui.painter().text(pos2(rect.left() + 16.0, cy), Align2::CENTER_CENTER, ic(I_SEARCH), icon_font(13.0), DIM());

        // right side, from the edge inwards: help (?), the Subfolders switch, the clear (×) button
        // right side, from the edge inwards: help (?), regex (.*), subfolders, clear (×)
        let help_r = Rect::from_center_size(pos2(rect.right() - 15.0, cy), vec2(24.0, 24.0));
        let regex_r = Rect::from_center_size(pos2(help_r.left() - 13.0, cy), vec2(24.0, 24.0));
        let chip_r = Rect::from_center_size(pos2(regex_r.left() - 14.0, cy), vec2(24.0, 24.0));
        let clear_r = Rect::from_center_size(pos2(chip_r.left() - 14.0, cy), vec2(20.0, 20.0));
        let in_dir = matches!(t.loc, Loc::Dir(_));
        let nested = self.settings.nested_search;
        let text_right = if t.query.is_empty() { chip_r.left() - 6.0 } else { clear_r.left() - 2.0 };

        // the text field fills the bar's height and centres its text, so it lines up with the icon
        let edit_r = Rect::from_min_max(pos2(rect.left() + 30.0, rect.top() + 2.0), pos2(text_right, rect.bottom() - 2.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(edit_r));
        let place = match (&t.loc, t.scope) {
            (_, Scope::AllDrives) | (Loc::ThisPc, _) => "Search all drives".to_string(),
            (Loc::Dir(_), Scope::ThisFolder) => format!("Filter {}", t.loc.title()),
            _ => format!("Search {}", t.loc.title()),
        };
        let te = egui::TextEdit::singleline(&mut t.query)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .align(Align2::LEFT_CENTER)
            .min_size(edit_r.size())
            .desired_width(edit_r.width())
            .font(FontId::proportional(13.5))
            .hint_text(egui::RichText::new(place).color(FAINT()))
            .id_salt("search");
        let r = child.add(te);
        if self.focus_search {
            r.request_focus();
            self.focus_search = false;
        }
        if r.has_focus() {
            ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0_f32, ACCENT()), StrokeKind::Inside);
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                t.query.clear();
                self.query_changed(t);
            }
            if ui.input(|i| i.key_pressed(Key::ArrowDown)) {
                r.surrender_focus();
            }
        }
        if r.changed() {
            self.query_changed(t);
        }
        if !t.query.is_empty() {
            let x = ui.interact(clear_r, ui.scope_id().with("clear-search"), Sense::click());
            if x.hovered() {
                ui.painter().rect_filled(clear_r, CornerRadius::same(4), HOVER());
            }
            ui.painter().text(clear_r.center(), Align2::CENTER_CENTER, ic(I_CLOSE), icon_font(10.0), DIM());
            if x.clicked() {
                t.query.clear();
                self.query_changed(t);
            }
        }

        // square switch: search subfolders too (on = this folder and every folder below it)
        let tip = if !in_dir {
            "Searching every drive"
        } else if nested {
            "Subfolders: on. Searches this folder and every folder below it. Click to search only this folder."
        } else {
            "Subfolders: off. Only filters what's in this folder. Click to also search every folder below it."
        };
        let sub_fixed = policy::is_fixed("nested_search");
        let tip = if sub_fixed { format!("{tip}\n\n{}", settings_page::MANAGED) } else { tip.to_string() };
        // one setting for every tab, like the regex switch next to it; only "all drives" is a tab's own
        let all_drives = t.scope == Scope::AllDrives;
        let sub_clicked = square_toggle(ui, chip_r, "nested-search", self.settings.nested_search || all_drives, !sub_fixed && !all_drives, &tip, |p, r, col| {
            // a folder with a branch below it: "this folder and the ones inside"
            p.text(r.center() + vec2(1.5, -2.5), Align2::CENTER_CENTER, ic(I_FOLDER), icon_font(11.0), col);
            let s = Stroke::new(1.3_f32, col);
            let a = r.center() + vec2(-6.0, -1.0);
            p.line_segment([a, a + vec2(0.0, 7.0)], s);
            p.line_segment([a + vec2(0.0, 7.0), a + vec2(4.0, 7.0)], s);
        });
        // square switch: regular expressions
        let rx_tip = match &t.query_error {
            Some(e) => format!("Regular expression: {}\n\nInvalid: {e}", if self.settings.regex_search { "on" } else { "off" }),
            None if self.settings.regex_search => "Regular expression: on. The whole search is a regex (case-insensitive), e.g. ^img_\\d+\\.jpg$".into(),
            None => "Regular expression: off. Click to treat the search as a regex. You can also write re:pattern or /pattern/ in a normal search.".into(),
        };
        let rx_fixed = policy::is_fixed("regex_search");
        let rx_tip = if rx_fixed { format!("{rx_tip}\n\n{}", settings_page::MANAGED) } else { rx_tip };
        if square_toggle(ui, regex_r, "regex-search", self.settings.regex_search, !rx_fixed, &rx_tip, |p, r, col| {
            p.text(r.center() + vec2(0.0, -1.0), Align2::CENTER_CENTER, ".*", FontId::monospace(15.0), col);
        }) {
            self.settings.regex_search = !self.settings.regex_search;
            self.save_settings();
            self.query_changed(t);
        }
        if let Some(e) = t.query_error.clone().filter(|_| !t.query.trim().is_empty()) {
            ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0_f32, WARN()), StrokeKind::Inside);
            let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("search-err")));
            let g = painter.layout_no_wrap(format!("Invalid regex: {e}"), FontId::proportional(12.0), Color32::WHITE);
            let r = Rect::from_min_size(rect.left_bottom() + vec2(0.0, 4.0), g.size() + vec2(16.0, 8.0));
            painter.rect_filled(r, CornerRadius::same(5), WARN());
            painter.galley(r.min + vec2(8.0, 4.0), g, Color32::WHITE);
        }
        if sub_clicked {
            self.settings.nested_search = !self.settings.nested_search;
            self.save_settings();
            self.follow_subfolder_setting(t);
        }

        // help: query syntax, all drives, rescan
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(help_r).layout(egui::Layout::left_to_right(egui::Align::Center)));
        let before = t.scope;
        let own_scope = if self.settings.nested_search { Scope::Subfolders } else { Scope::ThisFolder };
        let mut rescan = false;
        child
            .menu_button(egui::RichText::new("?").size(12.5).color(DIM()), |ui| {
                ui.set_min_width(320.0);
                if in_dir {
                    let mut all = t.scope == Scope::AllDrives;
                    if ui.checkbox(&mut all, "Search all drives instead of this folder").changed() {
                        t.scope = if all { Scope::AllDrives } else { own_scope };
                    }
                    ui.separator();
                }
                ui.label(egui::RichText::new("Search syntax").color(DIM()).small());
                for (q, d) in [
                    ("report 2024", "name contains both words"),
                    ("*.rs   img_??.png", "wildcards"),
                    ("\"exact name.txt\"", "exact name"),
                    ("-draft", "exclude a word"),
                    ("ext:pdf,docx", "by extension"),
                    ("kind:image", "video audio doc code archive app folder"),
                    ("size:>100mb", "also <1k or 1mb..1gb"),
                    ("modified:today", "yesterday week month year 2025 >2026-01-01"),
                    (r"re:^img_\d+   /^img_\d+/", "regular expression (or switch on .* for the whole search)"),
                ] {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(q).monospace().color(ACCENT()));
                        ui.label(egui::RichText::new(d).color(DIM()).small());
                    });
                }
                if t.search.is_some() {
                    ui.separator();
                    if ui.button("Rescan the index").clicked() {
                        rescan = true;
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text("Search syntax and options");
        if rescan {
            if let Some(s) = &t.search {
                s.index.cancel.store(true, Ordering::Relaxed);
                let roots = s.index.roots.clone();
                self.indexes.retain(|i| !search::roots_equal(&i.roots, &roots));
            }
            self.query_changed(t);
        } else if before != t.scope {
            self.query_changed(t);
        }
    }

    /// Puts a tab's search in line with "Search subfolders too", unless the tab searches every drive.
    pub(super) fn follow_subfolder_setting(&mut self, t: &mut Tab) {
        let wanted = if self.settings.nested_search { Scope::Subfolders } else { Scope::ThisFolder };
        if t.scope != Scope::AllDrives && t.scope != wanted {
            t.scope = wanted;
            self.query_changed(t);
        }
    }

    pub(super) fn query_changed(&mut self, t: &mut Tab) {
        let q = t.query.trim().to_string();
        t.query_error = if q.is_empty() {
            None
        } else if self.settings.regex_search {
            Query::regex_whole(&q).error
        } else {
            Query::new(&q).error
        };
        t.sel.clear();
        t.sel_info = (0, 0);
        t.cursor = None;
        t.anchor = None;
        t.scroll_to = Some(0);
        if t.loc == Loc::Settings {
            t.search = None;
            return;
        }
        if q.is_empty() || (t.scope == Scope::ThisFolder && t.loc != Loc::ThisPc) {
            t.search = None;
            t.dirty = true;
            return;
        }
        let roots: Vec<PathBuf> = match (&t.loc, t.scope) {
            (Loc::Dir(p), Scope::Subfolders) => vec![p.clone()],
            // Linux/macOS: one tree from / (removable media live inside it)
            _ if !cfg!(windows) => vec![PathBuf::from("/")],
            _ => self.drives.iter().filter(|d| matches!(d.kind, DriveKind::Fixed | DriveKind::Removable)).map(|d| d.root.clone()).collect(),
        };
        // crawling a folder that can't be opened would only keep threads waiting for it
        if t.error.is_some() {
            t.search = None;
            return;
        }
        let index = self.index_for(roots);
        let panel = t.filter.is_active().then(|| t.filter.compile_now());
        let mut s = Search::new(index, &q, self.settings.show_hidden, self.settings.regex_search, self.settings.hides_dotfiles(), panel);
        s.pump();
        t.search = Some(s);
    }

    fn index_for(&mut self, roots: Vec<PathBuf>) -> Arc<Index> {
        let skip = self.settings.search_skip(&policy::current());
        // an index built before what search never finds changed would still find it
        let fresh = |i: &Arc<Index>| search::roots_equal(&i.roots, &roots) && i.skip.key == skip.key && (!i.is_done() || i.started.elapsed() < Duration::from_secs(600));
        if let Some(i) = self.indexes.iter().find(|i| fresh(i)) {
            return i.clone();
        }
        self.indexes.retain(|i| !search::roots_equal(&i.roots, &roots));
        let i = Index::build(roots, skip, self.ctx.clone());
        self.indexes.push(i.clone());
        while self.indexes.len() > 6 {
            let old = self.indexes.remove(0);
            old.cancel.store(true, Ordering::Relaxed);
        }
        i
    }

    pub(super) fn command_bar(&mut self, root: &mut Ui, t: &mut Tab) {
        let left = self.settings.ribbon_shown(&self.settings.ribbon_left);
        let right = self.settings.ribbon_shown(&self.settings.ribbon_right);
        if left.is_empty() && right.is_empty() {
            return;
        }
        let frame = egui::Frame::new().fill(PANEL()).inner_margin(egui::Margin { left: 8, right: 8, top: 0, bottom: 6 });
        egui::Panel::top("commands").exact_size(40.0).frame(frame).show(root, |ui| {
            // right-click the empty part of the bar to customize it
            ui.interact(ui.max_rect(), ui.scope_id().with("ribbon_bg"), Sense::click()).context_menu(|ui| {
                if menu_item(ui, I_SETTINGS, "Customize ribbon bar…", "") {
                    self.acts.push(Act::OpenSettings(Some(settings_page::Page::Ribbon)));
                }
            });
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for id in &left {
                    self.ribbon_item(ui, t, id);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for id in right.iter().rev() {
                        self.ribbon_item(ui, t, id);
                    }
                });
            });
        });
    }

    /// One item of the ribbon bar (see `RIBBON_ITEMS`).
    fn ribbon_item(&mut self, ui: &mut Ui, t: &mut Tab, id: &str) {
        let in_dir = t.dir().is_some() && !t.searching();
        let has_sel = !t.sel.is_empty();
        match id {
            "new" => {
                ui.add_enabled_ui(in_dir, |ui| {
                    // the arrow says it opens a list, as Explorer's does
                    let label = (egui::RichText::new(format!("{}  New", ic(I_ADD))).font(icon_font(13.5)), egui::RichText::new(ic(I_CHEV_D)).font(icon_font(9.0)).color(DIM()));
                    ui.menu_button(label, |ui| {
                        ui.set_max_width(280.0);
                        if menu_item(ui, I_NEWFOLDER, "Folder", "Ctrl+Shift+N") {
                            self.acts.push(Act::NewFolder);
                        }
                        if menu_item(ui, I_RENAME, "Text document", "") {
                            self.acts.push(Act::NewTextFile);
                        }
                        // what the system offers too: Explorer's list on Windows, the Templates folder on Linux
                        let more = crate::newitems::templates();
                        if !more.is_empty() {
                            ui.separator();
                        }
                        for (i, m) in more.iter().enumerate() {
                            let icon = match &m.how {
                                crate::newitems::How::Shortcut => I_FWD,
                                _ if m.name.to_lowercase().ends_with(".zip") => I_FOLDER,
                                _ => I_ADD,
                            };
                            if menu_item(ui, icon, &m.label, "") {
                                self.acts.push(Act::NewFrom(i));
                            }
                        }
                    });
                });
            }
            "cut" => {
                if icon_button(ui, I_CUT, "Cut (Ctrl+X)", has_sel).clicked() {
                    self.acts.push(Act::Cut);
                }
            }
            "copy" => {
                if icon_button(ui, I_COPY, "Copy (Ctrl+C)", has_sel).clicked() {
                    self.acts.push(Act::Copy);
                }
            }
            "paste" => {
                if icon_button(ui, I_PASTE, "Paste (Ctrl+V)", in_dir).clicked() {
                    self.acts.push(Act::Paste);
                }
            }
            "rename" => {
                if icon_button(ui, I_RENAME, "Rename (F2)", t.sel.len() == 1).clicked() {
                    self.acts.push(Act::StartRename);
                }
            }
            "delete" => {
                if icon_button(ui, I_DELETE, "Delete (Del). Shift+Del deletes permanently", has_sel).clicked() {
                    self.acts.push(Act::Delete(ui.input(|i| i.modifiers.shift)));
                }
            }
            "sort" => self.sort_button(ui, t),
            "filter" => self.filter_button(ui, t),
            "view" => {
                let view_glyph = if self.settings.view == View::Details { I_LIST } else { I_GRID };
                ui.menu_button(egui::RichText::new(format!("{}  View", ic(view_glyph))).font(icon_font(13.5)), |ui| {
                    let free = |key: &str| !policy::is_fixed(key);
                    if ui.add_enabled(free("view"), egui::RadioButton::new(self.settings.view == View::Details, "Details")).clicked() {
                        self.settings.view = View::Details;
                    }
                    if ui.add_enabled(free("view"), egui::RadioButton::new(self.settings.view == View::Icons, "Large icons")).clicked() {
                        self.settings.view = View::Icons;
                    }
                    ui.separator();
                    ui.add_enabled(free("preview"), egui::Checkbox::new(&mut self.settings.show_preview, "Preview pane  (Alt+P)"));
                    if ui.add_enabled(free("hidden"), egui::Checkbox::new(&mut self.settings.show_hidden, "Hidden items")).changed() {
                        t.dirty = true;
                        self.tabs.iter_mut().for_each(|o| o.dirty = true);
                        if t.searching() {
                            self.query_changed(t);
                        }
                    }
                    let mut show_dots = !self.settings.hide_dotfiles;
                    if cfg!(windows) && ui.add_enabled(free("hide_dots"), egui::Checkbox::new(&mut show_dots, "Dotfiles  (Ctrl+Shift+.)")).changed() {
                        self.acts.push(Act::ToggleDotfiles);
                    }
                    ui.add_enabled(free("ext"), egui::Checkbox::new(&mut self.settings.show_ext, "File name extensions"));
                });
            }
            "more" => {
                if icon_button(ui, I_MORE, "Windows menu for this folder", t.dir().is_some()).clicked() {
                    self.acts.push(Act::NativeMenu(false));
                }
            }
            "terminal" => {
                if icon_button(ui, I_TERMINAL, "Open in Terminal", t.dir().is_some()).clicked() {
                    self.acts.push(Act::Terminal);
                }
            }
            "hidden" => {
                let fixed = policy::is_fixed("hidden");
                let hid = icon_button(ui, I_EYE, if fixed { settings_page::MANAGED } else { "Show hidden items" }, !fixed);
                if self.settings.show_hidden {
                    ui.painter().rect_stroke(hid.rect, CornerRadius::same(5), Stroke::new(1.0_f32, ACCENT()), StrokeKind::Inside);
                }
                if hid.clicked() {
                    self.settings.show_hidden = !self.settings.show_hidden;
                    self.tabs.iter_mut().for_each(|o| o.dirty = true);
                    t.dirty = true;
                    if t.searching() {
                        self.query_changed(t);
                    }
                }
            }
            "preview" => {
                let fixed = policy::is_fixed("preview");
                let pv = icon_button(ui, I_PANE, if fixed { settings_page::MANAGED } else { "Preview pane (Alt+P)" }, !fixed);
                if self.settings.show_preview {
                    ui.painter().rect_stroke(pv.rect, CornerRadius::same(5), Stroke::new(1.0_f32, ACCENT()), StrokeKind::Inside);
                }
                if pv.clicked() {
                    self.settings.show_preview = !self.settings.show_preview;
                }
            }
            _ => {
                ui.add(egui::Separator::default().vertical().spacing(10.0));
            }
        }
    }

    /// The Sort menu: what by, which way round, whether folders stay on top, and whether this folder has its own.
    fn sort_button(&mut self, ui: &mut Ui, t: &mut Tab) {
        let arrow = if t.asc { "↑" } else { "↓" };
        let label = format!("{}  {}  {arrow}", ic(I_SORT), t.sort.label());
        ui.menu_button(egui::RichText::new(label).font(icon_font(13.5)), |ui| {
            ui.set_min_width(250.0);
            ui.set_max_width(280.0);
            ui.label(egui::RichText::new("Sort by").size(12.0).color(DIM()));
            for k in SortKey::ALL {
                if ui.radio(t.sort == k, k.label()).clicked() {
                    self.acts.push(Act::Sort(k, None));
                }
            }
            ui.separator();
            let words = |asc: bool| SortSpec { key: t.sort, asc, dirs_first: true }.direction();
            if ui.radio(t.asc, words(true)).clicked() {
                self.acts.push(Act::Sort(t.sort, Some(true)));
            }
            if ui.radio(!t.asc, words(false)).clicked() {
                self.acts.push(Act::Sort(t.sort, Some(false)));
            }
            ui.separator();
            let mut first = t.dirs_first;
            if ui.checkbox(&mut first, "Folders first").changed() {
                self.acts.push(Act::FoldersFirst(first));
            }
            if t.dir().is_some() && !t.searching() {
                ui.separator();
                let own = self.folder_has_own_sort(t);
                let note = if own {
                    "This folder keeps this sort."
                } else if t.dir().is_some_and(|d| self.places.iter().any(|p| p.name == "Downloads" && fsx::same_path(&p.path, d))) && self.settings.downloads_newest {
                    "Downloads opens newest first. Changing it here keeps your choice."
                } else {
                    "This is your default. Changing it here keeps your choice for this folder."
                };
                ui.add(egui::Label::new(egui::RichText::new(note).size(12.0).color(DIM())).wrap());
                ui.add_space(2.0);
                if ui.add_enabled(own, egui::Button::new("Reset this folder to default")).clicked() {
                    self.acts.push(Act::ResetSort);
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text("How this folder is sorted");
    }

    /// The Filter button, and while something's filtered, how much of the folder is showing.
    fn filter_button(&mut self, ui: &mut Ui, t: &mut Tab) {
        let n = t.filter.count();
        let label = if n > 0 { format!("{}  Filter ({n})", ic(I_FILTER)) } else { format!("{}  Filter", ic(I_FILTER)) };
        let b = ui.add_enabled(t.dir().is_some(), egui::Button::new(egui::RichText::new(label).font(icon_font(13.5))).selected(t.filter_open || n > 0));
        if b.clicked() {
            t.filter_open = !t.filter_open;
            // the panel offers what's in this folder
            t.dirty = true;
        }
        t.filter_anchor = Some(b.rect);
        let _ = b.on_hover_text("Show only some of what's in this folder");
        if t.filter.is_active() {
            ui.add_space(4.0);
            if t.filtered() {
                ui.label(egui::RichText::new(format!("{} of {} shown", fmt_count(t.view.len()), fmt_count(t.shown_of))).size(12.5).color(ACCENT()));
            }
            let x = ui.add(egui::Button::new(egui::RichText::new(ic(I_CLOSE)).font(icon_font(10.0))).frame(false).min_size(vec2(22.0, 22.0))).on_hover_text("Clear the filter");
            if x.clicked() {
                self.acts.push(Act::ClearFilter);
            }
        }
    }

    /// The filter panel: every choice a click away, and the list follows as they're made.
    pub(super) fn filter_panel(&mut self, ctx: &egui::Context, t: &mut Tab) {
        if !t.filter_open {
            return;
        }
        if t.dir().is_none() {
            t.filter_open = false;
            return;
        }
        let at = t.filter_anchor.map_or(pos2(200.0, 100.0), |r| r.left_bottom() + vec2(0.0, 6.0));
        let screen = ctx.content_rect();
        let width = 400.0_f32.min(screen.width() - 24.0);
        let at = pos2(at.x.min(screen.right() - width - 12.0).max(screen.left() + 12.0), at.y);
        let before = t.filter.clone();
        let area = egui::Area::new(egui::Id::unique(("filter_panel", t.id))).order(egui::Order::Foreground).fixed_pos(at).show(ctx, |ui| {
            egui::Frame::new().fill(PANEL()).stroke(Stroke::new(1.0_f32, LINE())).corner_radius(10).inner_margin(egui::Margin::same(14)).shadow(ui.visuals().popup_shadow).show(ui, |ui| {
                ui.set_width(width - 28.0);
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                egui::ScrollArea::vertical().max_height((screen.height() - at.y - 80.0).max(160.0)).auto_shrink([false, true]).show(ui, |ui| {
                    let f = &mut t.filter;
                    section(ui, "Kind");
                    ui.horizontal_wrapped(|ui| {
                        for (k, label) in listing::FILTER_KINDS {
                            if chip(ui, label, f.kinds.contains(&k)) {
                                f.toggle_kind(k);
                            }
                        }
                    });
                    section(ui, "Extension");
                    // what's in this folder, most common first, and whatever was chosen elsewhere
                    let mut seen: Vec<(String, usize)> = t.type_stats_all.clone();
                    seen.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                    seen.truncate(14);
                    for e in &f.exts {
                        if !seen.iter().any(|(x, _)| x == e) {
                            seen.push((e.clone(), 0));
                        }
                    }
                    ui.horizontal_wrapped(|ui| {
                        for (e, _) in &seen {
                            if chip(ui, &format!(".{e}"), f.exts.contains(e)) {
                                f.toggle_ext(e);
                            }
                        }
                        let typed = ui.add(egui::TextEdit::singleline(&mut t.filter_ext).desired_width(70.0).hint_text("other"));
                        if typed.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                            f.toggle_ext(&t.filter_ext);
                            t.filter_ext.clear();
                        }
                    });
                    section(ui, "Date modified");
                    ui.horizontal_wrapped(|ui| {
                        for (d, label) in [(DateRange::Any, "Any time"), (DateRange::Today, "Today"), (DateRange::Week, "This week"), (DateRange::Month, "This month"), (DateRange::Custom, "Between…")] {
                            if chip(ui, label, f.date == d) {
                                f.date = d;
                            }
                        }
                    });
                    if f.date == DateRange::Custom {
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut f.date_from).desired_width(100.0).hint_text("2026-01-01"));
                            ui.label("and");
                            ui.add(egui::TextEdit::singleline(&mut f.date_to).desired_width(100.0).hint_text("2026-12-31"));
                        });
                        if let Some(e) = f.date_error() {
                            ui.label(egui::RichText::new(e).size(12.0).color(WARN()));
                        }
                    }
                    section(ui, "Size");
                    ui.horizontal_wrapped(|ui| {
                        for (s, label) in [(SizeRange::Any, "Any size"), (SizeRange::Under1Mb, "Under 1 MB"), (SizeRange::Mb1To100, "1 to 100 MB"), (SizeRange::Mb100To1Gb, "100 MB to 1 GB"), (SizeRange::Over1Gb, "Over 1 GB"), (SizeRange::Custom, "Between…")] {
                            if chip(ui, label, f.size == s) {
                                f.size = s;
                            }
                        }
                    });
                    if f.size == SizeRange::Custom {
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut f.size_min).desired_width(90.0).hint_text("10 MB"));
                            ui.label("and");
                            ui.add(egui::TextEdit::singleline(&mut f.size_max).desired_width(90.0).hint_text("2 GB"));
                        });
                        if let Some(e) = f.size_error() {
                            ui.label(egui::RichText::new(e).size(12.0).color(WARN()));
                        }
                    }
                    section(ui, "Name contains");
                    ui.add(egui::TextEdit::singleline(&mut f.name).desired_width(f32::INFINITY).margin(egui::Margin::symmetric(8, 5)));
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let text = if t.search.is_some() {
                        "Applies to the search results".to_string()
                    } else {
                        format!("{} of {} shown", fmt_count(t.view.len()), fmt_count(t.shown_of))
                    };
                    ui.label(egui::RichText::new(text).size(12.5).color(if t.filter.is_active() { ACCENT() } else { DIM() }));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if action_button(ui, "Done", false).clicked() {
                            t.filter_open = false;
                        }
                        if ui.add_enabled(t.filter.is_active(), egui::Button::new("Clear").min_size(vec2(64.0, 30.0))).clicked() {
                            t.filter = ListFilter::default();
                        }
                    });
                });
            });
        });
        if t.filter != before {
            t.dirty = true;
            if t.searching() {
                self.query_changed(t);
            }
        }
        // anywhere else closes it, as a menu would
        let clicked_away = ctx.input(|i| i.pointer.primary_clicked()) && ctx.input(|i| i.pointer.interact_pos()).is_some_and(|p| !area.response.rect.contains(p) && t.filter_anchor.is_none_or(|a| !a.contains(p)));
        if clicked_away || ctx.input(|i| i.key_pressed(Key::Escape)) {
            t.filter_open = false;
        }
    }

    /// The top of a dialog: where it is, and how the list is arranged.
    pub(super) fn picker_top(&mut self, root: &mut Ui, t: &mut Tab) {
        let frame = egui::Frame::new().fill(PANEL()).inner_margin(egui::Margin::symmetric(10, 8));
        egui::Panel::top("picker_top").exact_size(48.0).frame(frame).show_separator_line(true).show(root, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if icon_button(ui, I_BACK, "Back (Alt+Left)", !t.back.is_empty()).clicked() {
                    self.acts.push(Act::Back);
                }
                if icon_button(ui, I_FWD, "Forward (Alt+Right)", !t.fwd.is_empty()).clicked() {
                    self.acts.push(Act::Forward);
                }
                if icon_button(ui, I_UP, "Up (Alt+Up)", t.dir().is_some()).clicked() {
                    self.acts.push(Act::Up);
                }
                if icon_button(ui, I_REFRESH, "Refresh (F5)", true).clicked() {
                    self.acts.push(Act::Refresh);
                }
                ui.add_space(6.0);
                // narrow windows lose the search box first: the address matters more
                let room = ui.available_width();
                let search_w = if room > 620.0 { (room * 0.26).clamp(150.0, 260.0) } else { 0.0 };
                let tools = 190.0;
                let addr_w = (room - search_w - tools - 16.0).max(120.0);
                self.address_bar(ui, t, addr_w);
                ui.add_space(6.0);
                self.sort_button(ui, t);
                self.filter_button(ui, t);
                let glyph = if self.settings.view == View::Details { I_GRID } else { I_LIST };
                if icon_button(ui, glyph, "Switch between details and large icons", !policy::is_fixed("view")).clicked() {
                    let next = if self.settings.view == View::Details { View::Icons } else { View::Details };
                    self.acts.push(Act::SetView(next));
                }
                if search_w > 0.0 {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            let free = ui.available_width();
                            ui.add_space((free - search_w).max(0.0));
                            self.search_box(ui, t, search_w.min(free));
                        });
                    });
                }
            });
        });
    }

    pub(super) fn sort_search(&self, t: &mut Tab) {
        let (sort, asc, first) = (t.sort, t.asc, t.dirs_first);
        if let Some(s) = &mut t.search {
            let data = s.index.data.read().unwrap();
            s.results.sort_by(|&a, &b| {
                let (x, y) = (&data.items[a as usize], &data.items[b as usize]);
                let (dx, dy) = (x.attrs & fsx::A_DIR != 0, y.attrs & fsx::A_DIR != 0);
                if first && dx != dy {
                    return dy.cmp(&dx);
                }
                let o = match sort {
                    SortKey::Name => natural_cmp(&data.name(a).to_lowercase(), &data.name(b).to_lowercase()),
                    // the search index only has the one date
                    SortKey::Modified | SortKey::Created => x.mtime.cmp(&y.mtime),
                    SortKey::Size => x.size.cmp(&y.size),
                    SortKey::Type | SortKey::Extension => ext_of(data.name(a), false).cmp(&ext_of(data.name(b), false)),
                };
                if asc {
                    o
                } else {
                    o.reverse()
                }
            });
            s.sorted = true;
        }
    }

    pub(super) fn status_bar(&mut self, root: &mut Ui, t: &Tab) {
        let frame = egui::Frame::new().fill(STATUS()).inner_margin(egui::Margin::symmetric(12, 4));
        egui::Panel::bottom("status").exact_size(26.0).frame(frame).show_separator_line(false).show(root, |ui| {
            ui.horizontal_centered(|ui| {
                let small = |s: String| egui::RichText::new(s).size(12.0).color(DIM());
                if let Some(s) = &t.search {
                    let idx = &s.index;
                    let more = if s.results.len() >= search::MAX_RESULTS { "+" } else { "" };
                    ui.label(small(format!("{}{} results", fmt_count(s.results.len()), more)));
                    ui.label(small("·".into()));
                    if idx.is_done() {
                        let took = idx.took.lock().unwrap().unwrap_or_default();
                        ui.label(small(format!("{} items indexed in {}", fmt_count(idx.count()), fmt_duration(took))));
                        ui.label(small("·".into()));
                        ui.label(egui::RichText::new(format!("query {}", fmt_duration(s.query_time))).size(12.0).color(GREEN()));
                    } else {
                        ui.add(egui::Spinner::new().size(12.0).color(ACCENT()));
                        ui.label(small(format!(
                            "Indexing… {} items in {} folders ({:.1} s)",
                            fmt_count(idx.count()),
                            fmt_count(idx.dirs.load(Ordering::Relaxed)),
                            idx.started.elapsed().as_secs_f32()
                        )));
                    }
                } else if t.loc == Loc::ThisPc {
                    ui.label(small(format!("{} drives", self.drives.len())));
                } else {
                    if t.error.is_some() {
                        ui.label(small("Not available".into()));
                    } else if t.loading && t.entries.is_empty() {
                        ui.label(small("Opening".into()));
                    } else if t.filtered() {
                        ui.label(egui::RichText::new(format!("{} of {} shown", fmt_count(t.view.len()), fmt_count(t.shown_of))).size(12.0).color(ACCENT()));
                    } else {
                        ui.label(small(format!("{} items", fmt_count(t.view.len()))));
                    }
                    if t.sel_info.0 > 0 {
                        ui.label(small("·".into()));
                        let size = if t.sel_info.1 > 0 { format!("  {}", fmt_size(t.sel_info.1)) } else { String::new() };
                        ui.label(small(format!("{} selected{size}", fmt_count(t.sel_info.0))));
                    }
                    if t.loading {
                        ui.add(egui::Spinner::new().size(12.0).color(ACCENT()));
                    }
                    if t.in_cloud {
                        ui.label(small("·".into()));
                        ui.label(egui::RichText::new(format!("{}  Cloud folder", ic(I_CLOUD))).font(icon_font(12.0)).color(CLOUD_BLUE()));
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let drive = t.dir().and_then(|d| self.drive_for(d));
                    if let Some(d) = drive.filter(|d| d.ready) {
                        ui.label(small(format!("{}  ·  {} free of {}", d.title(), fmt_size(d.free), fmt_size(d.total))));
                    }
                });
            });
        });
    }

    pub(super) fn this_pc(&mut self, ui: &mut Ui) {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(14.0);
            egui::Frame::new().inner_margin(egui::Margin::symmetric(22, 0)).show(ui, |ui| {
                ui.heading("Folders");
                ui.add_space(6.0);
                let places = self.places.clone();
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
                    for p in places.iter().filter(|p| p.name != "Home") {
                        self.place_tile(ui, &p.name, &p.path);
                    }
                });
                ui.add_space(18.0);
                ui.heading("Drives and devices");
                ui.add_space(6.0);
                let drives = self.drives.clone();
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
                    for d in drives.iter().filter(|d| d.kind != DriveKind::Network) {
                        self.drive_card(ui, d);
                    }
                });
                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    ui.heading("Network locations");
                    ui.add_space(10.0);
                    if ui.button(egui::RichText::new(format!("{}  {}", ic(I_NETDRIVE), network_label().trim_end_matches('…'))).font(icon_font(13.0))).clicked() {
                        self.acts.push(Act::MapDrive);
                    }
                });
                ui.add_space(6.0);
                let net: Vec<&Drive> = drives.iter().filter(|d| d.kind == DriveKind::Network).collect();
                if net.is_empty() {
                    ui.label(egui::RichText::new("No network drives mapped yet. Type \\\\server or \\\\server\\share in the address bar to browse a network computer.").color(DIM()));
                }
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
                    for d in net {
                        self.drive_card(ui, d);
                    }
                });
                if !self.clouds.is_empty() {
                    ui.add_space(18.0);
                    ui.heading("Cloud drives");
                    ui.add_space(6.0);
                    let clouds = self.clouds.clone();
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
                        for c in &clouds {
                            self.place_tile(ui, &c.name, &c.path);
                        }
                    });
                }
            });
        });
    }

    fn place_tile(&mut self, ui: &mut Ui, name: &str, path: &Path) {
        let (rect, resp) = ui.allocate_exact_size(vec2(200.0, 56.0), Sense::click());
        let path = path.to_path_buf();
        let drop = self.drop_zone(ui, &resp, &path);
        let drop_here = drop.is_some();
        ui.painter().rect_filled(rect, CornerRadius::same(8), if resp.hovered() || drop_here { ROW_HOVER() } else { TILE() });
        if let Some(ok) = drop {
            list::drop_mark(ui.painter(), rect, 8, ok);
        }
        self.draw_icon(ui, Rect::from_center_size(rect.left_center() + vec2(28.0, 0.0), vec2(32.0, 32.0)), &path, "", true, fsx::A_READONLY);
        let g = truncated(ui, name, FontId::proportional(14.0), TEXT(), rect.width() - 64.0);
        ui.painter().galley(pos2(rect.left() + 54.0, rect.center().y - g.size().y / 2.0), g, TEXT());
        if resp.clicked() {
            self.acts.push(Act::Navigate(Loc::Dir(path.clone())));
        }
        if resp.middle_clicked() {
            self.acts.push(Act::NavigateNewTab(Loc::Dir(path)));
        }
    }

    fn drive_card(&mut self, ui: &mut Ui, d: &Drive) {
        let (rect, resp) = ui.allocate_exact_size(vec2(290.0, 72.0), Sense::click());
        // an offline drive takes nothing, and says why
        let drop = self.drop_zone(ui, &resp, &d.root);
        let drop_here = drop.is_some();
        ui.painter().rect_filled(rect, CornerRadius::same(8), if resp.hovered() || drop_here { ROW_HOVER() } else { TILE() });
        if let Some(ok) = drop {
            list::drop_mark(ui.painter(), rect, 8, ok);
        }
        self.draw_icon(ui, Rect::from_center_size(rect.left_center() + vec2(30.0, 0.0), vec2(36.0, 36.0)), &d.root, "", true, 0);
        let x = rect.left() + 60.0;
        let g = truncated(ui, &d.title(), FontId::proportional(14.0), TEXT(), rect.width() - 72.0);
        ui.painter().galley(pos2(x, rect.top() + 10.0), g, TEXT());
        let bar = Rect::from_min_size(pos2(x, rect.top() + 34.0), vec2(rect.width() - 76.0, 6.0));
        if self.drive_offline(d) {
            ui.painter().circle_filled(pos2(x + 4.0, rect.top() + 43.0), 3.5, WARN());
            let sub = format!("Not available{}", d.remote.as_ref().map_or(String::new(), |r| format!("  ·  {r}")));
            let g = truncated(ui, &sub, FontId::proportional(12.0), DIM(), rect.width() - 84.0);
            ui.painter().galley(pos2(x + 14.0, rect.top() + 36.0), g, DIM());
        } else if d.ready && d.total > 0 {
            let u = 1.0 - d.free as f32 / d.total as f32;
            ui.painter().rect_filled(bar, CornerRadius::same(3), LINE());
            ui.painter().rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * u, 6.0)), CornerRadius::same(3), if u > 0.9 { WARN() } else { ACCENT() });
            let sub = format!("{} free of {}  ·  {}", fmt_size(d.free), fmt_size(d.total), d.fs);
            ui.painter().text(pos2(x, rect.top() + 48.0), Align2::LEFT_TOP, sub, FontId::proportional(12.0), DIM());
        } else {
            let sub = d.remote.clone().unwrap_or_else(|| "Not ready".into());
            ui.painter().text(pos2(x, rect.top() + 36.0), Align2::LEFT_TOP, sub, FontId::proportional(12.0), DIM());
        }
        if resp.clicked() {
            self.acts.push(Act::Navigate(Loc::Dir(d.root.clone())));
        }
        let (letter, net, root) = (d.letter, d.kind == DriveKind::Network, d.root.clone());
        resp.context_menu(|ui| {
            if menu_item(ui, I_ADD, "Open in new tab", "") {
                self.acts.push(Act::NavigateNewTab(Loc::Dir(root.clone())));
            }
            if net && menu_item(ui, I_CLOSE, "Disconnect", "") {
                self.acts.push(Act::Disconnect(letter));
            }
            if menu_item(ui, I_INFO, "Properties", "") {
                self.acts.push(Act::Properties(root.clone()));
            }
        });
    }
}

/// A heading inside the filter panel.
fn section(ui: &mut Ui, title: &str) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(title).size(12.0).color(DIM()));
}

/// An on/off choice in the filter panel.
fn chip(ui: &mut Ui, label: &str, on: bool) -> bool {
    let fill = if on { ACCENT().gamma_multiply(0.28) } else { RAISED() };
    let stroke = if on { Stroke::new(1.0_f32, ACCENT()) } else { Stroke::new(1.0_f32, LINE()) };
    ui.add(egui::Button::new(egui::RichText::new(label).size(12.5).color(TEXT())).fill(fill).stroke(stroke).corner_radius(13).min_size(vec2(0.0, 26.0))).clicked()
}

/// A button with some presence: the main thing to do here, or one of the others.
pub(super) fn action_button(ui: &mut Ui, label: &str, primary: bool) -> Response {
    if primary {
        ui.add(primary_button(label, true))
    } else {
        ui.add(secondary_button(label))
    }
}

pub(super) fn primary_button(label: &str, enabled: bool) -> egui::Button<'static> {
    let fill = if enabled { ACCENT() } else { ACCENT().gamma_multiply(0.45) };
    egui::Button::new(egui::RichText::new(label.to_string()).size(13.5).color(Color32::WHITE)).fill(fill).stroke(Stroke::NONE).corner_radius(6).min_size(vec2(96.0, 32.0))
}

pub(super) fn secondary_button(label: &str) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(label.to_string()).size(13.5).color(TEXT())).fill(RAISED()).stroke(Stroke::new(1.0_f32, FAINT())).corner_radius(6).min_size(vec2(96.0, 32.0))
}

/// A centred column for a page that says something instead of a list.
pub(super) fn page_column(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let room = ui.available_rect_before_wrap();
    let width = room.width().min(520.0) - 32.0;
    ui.add_space((room.height() * 0.2).clamp(24.0, 160.0));
    ui.vertical_centered(|ui| {
        ui.set_max_width(width.max(160.0));
        add(ui);
    });
}

/// A small square on/off button with a custom-drawn icon.
fn square_toggle(ui: &mut Ui, r: Rect, id: &str, on: bool, enabled: bool, tip: &str, draw: impl FnOnce(&egui::Painter, Rect, Color32)) -> bool {
    let resp = ui.interact(r, ui.scope_id().with(id), if enabled { Sense::click() } else { Sense::hover() });
    let fill = if on {
        ACCENT().gamma_multiply(if resp.hovered() { 0.42 } else { 0.28 })
    } else if resp.hovered() && enabled {
        HOVER()
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(r, CornerRadius::same(5), fill);
    let col = if on { ACCENT() } else { DIM() };
    draw(ui.painter(), r, col);
    resp.on_hover_text(tip).clicked() && enabled
}

/// The box behind the address bar and the search box.
fn text_field(ui: &Ui, rect: Rect) {
    ui.painter().rect_filled(rect, CornerRadius::same(6), FIELD());
    if FIELD_LINE() != Color32::TRANSPARENT {
        ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0_f32, FIELD_LINE()), StrokeKind::Inside);
    }
}
