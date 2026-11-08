//! Our own Properties window: name, type, default app, location, size (folders are
//! measured in the background), dates, attributes, cloud status and drive space.
//! Settings › Files & folders can switch back to the Windows dialog.

use super::*;

pub(super) struct PropsState {
    pub generation: u64,
    paths: Vec<PathBuf>,
    entry: Option<Entry>,
    name: String,
    opens_with: Option<String>,
    readonly: bool,
    hidden: bool,
    /// (bytes, files, folders, finished)
    pub size: (u64, usize, usize, bool),
    cancel: Arc<AtomicBool>,
    opened: f64,
    /// where the window first opened (kept so it doesn't follow the main window)
    pos: Option<egui::Pos2>,
}

impl Drop for PropsState {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl App {
    pub(super) fn open_props(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let generation = self.preview.generation.wrapping_add(self.props.as_ref().map(|p| p.generation + 1).unwrap_or(1)).wrapping_add(7);
        let entry = if paths.len() == 1 {
            let p = paths[0].clone();
            bg::within(&paths[0], Duration::from_millis(400), move || fsx::stat(&p)).flatten()
        } else {
            None
        };
        let cancel = Arc::new(AtomicBool::new(false));
        // measure everything in the background, reporting progress as it goes
        {
            let (tx, ctx, paths, cancel) = (self.tx.clone(), self.ctx.clone(), paths.clone(), cancel.clone());
            let lane = paths[0].clone();
            bg::submit(&lane, "props", move || {
                let (mut bytes, mut files, mut dirs) = (0u64, 0usize, 0usize);
                let mut stack: Vec<PathBuf> = Vec::new();
                for p in &paths {
                    match fsx::stat(p) {
                        Some(e) if e.is_dir && e.attrs & fsx::A_REPARSE == 0 => stack.push(p.clone()),
                        Some(e) if !e.is_dir => {
                            files += 1;
                            bytes += e.size;
                        }
                        _ => {}
                    }
                }
                let mut last = Instant::now();
                while let Some(dir) = stack.pop() {
                    if cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    fsx::for_each_raw(&dir, |d| {
                        if d.attrs & fsx::A_DIR != 0 {
                            dirs += 1;
                            if d.attrs & fsx::A_REPARSE == 0 {
                                stack.push(dir.join(d.name.as_ref()));
                            }
                        } else {
                            files += 1;
                            bytes += d.size;
                        }
                    });
                    if last.elapsed() > Duration::from_millis(80) {
                        last = Instant::now();
                        let _ = tx.send(Msg::PropSize { generation, bytes, files, dirs, done: false });
                        ctx.request_repaint();
                    }
                }
                let _ = tx.send(Msg::PropSize { generation, bytes, files, dirs, done: true });
                ctx.request_repaint();
            });
        }
        let opens_with = entry.as_ref().filter(|e| !e.is_dir).and_then(|e| shell::default_app(&e.ext));
        self.props = Some(PropsState {
            generation,
            name: entry.as_ref().map(|e| e.name.clone()).unwrap_or_default(),
            readonly: entry.as_ref().is_some_and(|e| e.attrs & fsx::A_READONLY != 0),
            hidden: entry.as_ref().is_some_and(|e| e.attrs & fsx::A_HIDDEN != 0),
            entry,
            opens_with,
            paths,
            size: (0, 0, 0, false),
            cancel,
            opened: self.ctx.input(|i| i.time),
            pos: None,
        });
    }

    /// The Properties window: a real window of its own (own title bar and taskbar entry).
    pub(super) fn props_window(&mut self, ctx: &egui::Context, t: &Tab) {
        let Some(mut p) = self.props.take() else { return };
        let title = match &p.entry {
            Some(e) => format!("{} Properties", e.name),
            None => format!("Properties of {} items", p.paths.len()),
        };
        let size = vec2(460.0, 620.0);
        if p.pos.is_none() {
            // centre on the main window the first time
            let main = ctx.input(|i| i.viewport().outer_rect).unwrap_or(ctx.content_rect());
            p.pos = Some(main.center() - size / 2.0);
        }
        let icon = egui::IconData { rgba: logo_rgba(64, ACCENT()), width: 64, height: 64 };
        let builder = egui::ViewportBuilder::default()
            .with_title(title.clone())
            .with_inner_size(size)
            .with_min_inner_size(vec2(400.0, 420.0))
            .with_position(p.pos.unwrap())
            .with_icon(Arc::new(icon));
        let vid = egui::ViewportId::from_hash_of(("props", p.generation));
        let mut result = PropsResult::Keep;
        if std::env::var("DALERIS_SHOT_PROPS").is_ok() {
            // test hook: the same contents drawn inside the main window so a screenshot can show them
            egui::Window::new(title.clone()).collapsible(false).resizable(false).fixed_size(size - vec2(0.0, 40.0)).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).frame(egui::Frame::window(&ctx.global_style()).fill(BG()).inner_margin(0)).show(ctx, |ui| {
                egui::Panel::bottom("props_footer_t")
                    .frame(egui::Frame::new().fill(PANEL()).inner_margin(egui::Margin::symmetric(16, 12)))
                    .show_separator_line(false)
                    .show(ui, |ui| result = result.or(self.props_footer(ui, &p)));
                egui::CentralPanel::default().frame(egui::Frame::new().fill(BG()).inner_margin(egui::Margin::same(16))).show(ui, |ui| {
                    result = result.or(self.props_body(ui, &mut p, t));
                });
            });
            let _ = result; // the test preview never closes
            self.props = Some(p);
            let _ = (vid, builder);
            return;
        }
        ctx.show_viewport_immediate(vid, builder, |vctx, class| {
            if class == egui::ViewportClass::EmbeddedWindow {
                // fallback when separate windows aren't available: egui has already put this in a window of its own
                result = self.props_body(vctx, &mut p, t);
            } else {
                egui::Panel::bottom("props_footer")
                    .frame(egui::Frame::new().fill(PANEL()).inner_margin(egui::Margin::symmetric(16, 12)))
                    .show_separator_line(false)
                    .show(vctx, |ui| result = result.or(self.props_footer(ui, &p)));
                egui::CentralPanel::default().frame(egui::Frame::new().fill(BG()).inner_margin(egui::Margin::same(16))).show(vctx, |ui| {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        result = result.or(self.props_body(ui, &mut p, t));
                    });
                });
            }
            if vctx.input(|i| i.viewport().close_requested() || i.key_pressed(Key::Escape)) {
                result = PropsResult::Cancel;
            }
            let fresh = vctx.input(|i| i.time) - p.opened < 0.3;
            if !fresh && vctx.input(|i| i.key_pressed(Key::Enter)) && !vctx.ctx().egui_wants_keyboard_input() {
                result = PropsResult::Ok;
            }
        });
        match result {
            PropsResult::Ok => self.apply_props(&mut p),
            PropsResult::Apply => {
                self.apply_props(&mut p);
                self.props = Some(p);
            }
            PropsResult::Cancel => {}
            PropsResult::Keep => self.props = Some(p),
        }
    }

    fn props_footer(&mut self, ui: &mut Ui, p: &PropsState) -> PropsResult {
        let mut r = PropsResult::Keep;
        ui.horizontal(|ui| {
            if shell::HAS_NATIVE_PROPERTIES && windows_props_button(ui) {
                self.acts.push(Act::NativeProperties(p.paths[0].clone()));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let btn = |ui: &mut Ui, text: &str, primary: bool| {
                    let b = egui::Button::new(egui::RichText::new(text).color(if primary { Color32::WHITE } else { TEXT() }))
                        .min_size(vec2(84.0, 30.0))
                        .corner_radius(6)
                        .fill(if primary { ACCENT() } else { RAISED() })
                        .stroke(if primary { Stroke::NONE } else { Stroke::new(1.0_f32, LINE()) });
                    ui.add(b).clicked()
                };
                if btn(ui, "Apply", false) {
                    r = PropsResult::Apply;
                }
                if btn(ui, "Cancel", false) {
                    r = PropsResult::Cancel;
                }
                if btn(ui, "OK", true) {
                    r = PropsResult::Ok;
                }
            });
        });
        r
    }

    fn props_body(&mut self, ui: &mut Ui, p: &mut PropsState, t: &Tab) -> PropsResult {
        let single = p.paths.len() == 1;
        // ---- header: big icon, editable name, type
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
            match &p.entry {
                Some(e) => {
                    let path = p.paths[0].clone();
                    self.draw_thumb(ui, r, &path, &e.ext, e.is_dir, e.attrs, e.mtime);
                }
                None => {
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, ic(I_COPY), icon_font(34.0), ACCENT());
                }
            }
            ui.add_space(10.0);
            ui.vertical(|ui| {
                ui.add_space(8.0);
                if single {
                    egui::Frame::new().fill(RAISED()).stroke(Stroke::new(1.0_f32, LINE())).corner_radius(6).inner_margin(egui::Margin::symmetric(8, 5)).show(ui, |ui| {
                        ui.add(egui::TextEdit::singleline(&mut p.name).frame(egui::Frame::NONE).desired_width(ui.available_width()).font(FontId::proportional(15.0)));
                    });
                } else {
                    ui.label(egui::RichText::new(format!("{} items", p.paths.len())).size(16.0).strong());
                }
                let kind = match &p.entry {
                    Some(e) => self.type_name(&e.ext, e.is_dir),
                    None => "Multiple items".into(),
                };
                ui.add_space(2.0);
                ui.label(egui::RichText::new(kind).color(DIM()));
            });
        });
        ui.add_space(14.0);

        let e = p.entry.clone();
        // ---- where it is and what opens it
        if let Some(e) = &e {
            prop_card(ui, |ui| {
                if !e.is_dir {
                    prop_row(ui, "Opens with", |ui| {
                        ui.label(p.opens_with.clone().unwrap_or_else(|| "Not set".into()));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(egui::Button::new("Change…").corner_radius(5)).clicked() {
                                self.acts.push(Act::OpenWith(p.paths[0].clone()));
                            }
                        });
                    });
                }
                let loc = p.paths[0].parent().map(|x| x.display().to_string()).unwrap_or_else(|| p.paths[0].display().to_string());
                prop_row(ui, "Location", |ui| {
                    ui.add(egui::Label::new(&loc).truncate()).on_hover_text(&loc);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let b = ui.add(egui::Button::new(egui::RichText::new(ic(I_COPY)).font(icon_font(12.0))).corner_radius(5).min_size(vec2(28.0, 24.0)));
                        if b.on_hover_text("Copy full path").clicked() {
                            shell::clipboard_set_text(&p.paths[0].display().to_string());
                        }
                    });
                });
            });
            ui.add_space(10.0);
        }

        // ---- size
        let (bytes, files, dirs, done) = p.size;
        prop_card(ui, |ui| {
            prop_row(ui, "Size", |ui| {
                ui.label(egui::RichText::new(fmt_size(bytes)).strong());
                ui.label(egui::RichText::new(format!("{} bytes", fmt_count(bytes as usize))).color(DIM()));
                if !done {
                    ui.add(egui::Spinner::new().size(12.0).color(ACCENT()));
                }
            });
            if e.as_ref().is_none_or(|e| e.is_dir) {
                prop_row(ui, "Contains", |ui| {
                    ui.label(format!("{} files, {} folders", fmt_count(files), fmt_count(dirs)));
                });
            }
            // a drive: show its space
            if let Some(d) = p.paths.first().filter(|_| single).and_then(|x| self.drives.iter().find(|d| fsx::same_path(&d.root, x))) {
                if d.ready && d.total > 0 {
                    let used = d.total - d.free;
                    prop_row(ui, "Drive", |ui| {
                        ui.vertical(|ui| {
                            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 8.0), Sense::hover());
                            ui.painter().rect_filled(r, CornerRadius::same(4), LINE());
                            ui.painter().rect_filled(Rect::from_min_size(r.min, vec2(r.width() * used as f32 / d.total as f32, 8.0)), CornerRadius::same(4), ACCENT());
                            ui.label(egui::RichText::new(format!("{} used · {} free of {} · {}", fmt_size(used), fmt_size(d.free), fmt_size(d.total), d.fs)).color(DIM()).size(12.0));
                        });
                    });
                }
            }
        });
        ui.add_space(10.0);

        // ---- dates and status
        if let Some(e) = &e {
            prop_card(ui, |ui| {
                prop_row(ui, "Created", |ui| {
                    ui.label(fmt_filetime(e.ctime));
                });
                prop_row(ui, "Modified", |ui| {
                    ui.label(fmt_filetime(e.mtime));
                });
                if e.atime != 0 {
                    prop_row(ui, "Accessed", |ui| {
                        ui.label(fmt_filetime(e.atime));
                    });
                }
                let st = self.cloud_of(&p.paths[0], e.attrs, t);
                if st != Cloud::None {
                    prop_row(ui, "Status", |ui| {
                        ui.label(cloud_text(st));
                    });
                }
            });
            ui.add_space(10.0);
            // ---- attributes
            prop_card(ui, |ui| {
                prop_row(ui, "Attributes", |ui| {
                    settings_page::toggle(ui, &mut p.readonly, "Read-only");
                    ui.add_space(14.0);
                    settings_page::toggle(ui, &mut p.hidden, "Hidden");
                });
            });
        }
        PropsResult::Keep
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PropsResult {
    Keep,
    Ok,
    Apply,
    Cancel,
}

impl PropsResult {
    /// Keep the first real decision made this frame.
    fn or(self, other: PropsResult) -> PropsResult {
        if self == PropsResult::Keep {
            other
        } else {
            self
        }
    }
}

/// A square button with the Windows logo that opens the Windows Properties dialog.
fn windows_props_button(ui: &mut Ui) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(34.0, 30.0), Sense::click());
    let hot = resp.hovered();
    ui.painter().rect_filled(r, CornerRadius::same(6), if hot { HOVER() } else { RAISED() });
    ui.painter().rect_stroke(r, CornerRadius::same(6), Stroke::new(1.0_f32, if hot { ACCENT() } else { LINE() }), StrokeKind::Inside);
    // the four-pane logo
    let col = if hot { ACCENT() } else { DIM() };
    let (s, gap) = (6.0, 1.5);
    let o = r.center() - vec2(s + gap / 2.0, s + gap / 2.0);
    for (dx, dy) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
        let q = Rect::from_min_size(o + vec2(dx * (s + gap), dy * (s + gap)), vec2(s, s));
        ui.painter().rect_filled(q, CornerRadius::same(1), col);
    }
    resp.on_hover_text("Open the Windows Properties dialog (Security, Details, Previous versions…)").clicked()
}

/// A rounded group of rows.
fn prop_card(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new().fill(PANEL()).stroke(Stroke::new(1.0_f32, LINE())).corner_radius(8).inner_margin(egui::Margin::symmetric(14, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 2.0;
        add(ui);
    });
}

/// One "label   value" row with an aligned label column.
fn prop_row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.set_min_height(30.0);
        ui.allocate_ui_with_layout(vec2(96.0, 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(96.0);
            ui.label(egui::RichText::new(label).color(DIM()));
        });
        add(ui);
    });
}

impl App {

    fn apply_props(&mut self, p: &mut PropsState) {
        let Some(e) = p.entry.clone() else { return };
        let path = p.paths[0].clone();
        let (ro, hid) = (e.attrs & fsx::A_READONLY != 0, e.attrs & fsx::A_HIDDEN != 0);
        if ro != p.readonly || hid != p.hidden {
            if let Err(err) = fsx::set_attrs(&path, (ro != p.readonly).then_some(p.readonly), (hid != p.hidden).then_some(p.hidden)) {
                self.toast(format!("Couldn't change attributes: {err}"), true);
            }
        }
        let new = p.name.trim().to_string();
        if !new.is_empty() && new != e.name {
            if new.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|']) {
                self.toast("A file name can't contain any of these characters: \\ / : * ? \" < > |", true);
            } else {
                let dest = path.with_file_name(&new);
                match std::fs::rename(&path, &dest) {
                    Ok(()) => p.paths[0] = dest,
                    Err(err) => self.toast(format!("Rename failed: {err}"), true),
                }
            }
        }
        p.entry = fsx::stat(&p.paths[0]);
    }
}
