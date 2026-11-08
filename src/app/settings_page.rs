//! The Settings page (opens as its own tab): appearance (theme, accent, icon pack,
//! density, title bar), the sidebar and ribbon bar editors, the right-click menu editor, file options, and About & updates.

use super::sidebar::side_key;
use super::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Appearance,
    Sidebar,
    Ribbon,
    ContextMenu,
    Files,
    Explorer,
    About,
}

const PAGES: [(Page, &str, &str); 7] = [
    (Page::Appearance, I_PALETTE, "Appearance"),
    (Page::Sidebar, I_NAVPANE, "Sidebar"),
    (Page::Ribbon, I_GRID, "Ribbon bar"),
    (Page::ContextMenu, I_MORE, "Right-click menu"),
    (Page::Files, I_FOLDER, "Files & folders"),
    (Page::Explorer, I_NAVPANE, "Windows Explorer"),
    (Page::About, I_INFO, "About & updates"),
];

/// `icon packs` in the settings folder: each sub-folder is an icon pack of PNGs.
pub fn icon_pack_dir() -> PathBuf {
    config_dir().unwrap_or_default().join("icon packs")
}


pub fn list_icon_packs() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(icon_pack_dir())
        .map(|rd| rd.filter_map(|e| e.ok()).filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().to_string()).collect())
        .unwrap_or_default();
    v.sort_by_key(|s| s.to_lowercase());
    v
}

pub fn load_png(path: &Path) -> Option<egui::ColorImage> {
    let f = std::fs::File::open(path).ok()?;
    let mut dec = png::Decoder::new(std::io::BufReader::new(f));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let px = &buf[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => px.to_vec(),
        png::ColorType::Rgb => px.chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => px.chunks_exact(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Grayscale => px.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        _ => return None,
    };
    (rgba.len() == w * h * 4).then(|| egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba))
}

pub(super) fn card() -> egui::Frame {
    egui::Frame::new().fill(PANEL()).stroke(Stroke::new(1.0_f32, LINE())).corner_radius(10).inner_margin(egui::Margin::same(16))
}

fn heading(ui: &mut Ui, title: &str, sub: &str) {
    ui.label(egui::RichText::new(title).size(15.0).strong());
    if !sub.is_empty() {
        ui.label(egui::RichText::new(sub).color(DIM()));
    }
    ui.add_space(8.0);
}

/// A small switch, like Windows 11 settings.
pub(super) fn toggle(ui: &mut Ui, on: &mut bool, label: &str) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let (r, resp) = ui.allocate_exact_size(vec2(40.0, 20.0), Sense::click());
        if resp.clicked() {
            *on = !*on;
            changed = true;
        }
        let t = ui.ctx().animate_bool(resp.id, *on);
        let bg = if *on { ACCENT() } else { RAISED() };
        ui.painter().rect_filled(r, CornerRadius::same(10), bg);
        ui.painter().rect_stroke(r, CornerRadius::same(10), Stroke::new(1.0_f32, if *on { ACCENT() } else { FAINT() }), StrokeKind::Inside);
        let x = egui::lerp(r.left() + 10.0..=r.right() - 10.0, t);
        ui.painter().circle_filled(pos2(x, r.center().y), 6.0, if *on { Color32::WHITE } else { DIM() });
        let l = ui.add(egui::Label::new(label).sense(Sense::click()));
        if l.clicked() {
            *on = !*on;
            changed = true;
        }
    });
    changed
}

/// Tooltip and note on anything a policy fixes.
pub(super) const MANAGED: &str = "Managed by your organization";

/// A small lock and "Managed by your organization", for a grayed-out setting or section.
pub(super) fn managed_note(ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(egui::RichText::new(ic(I_LOCK)).font(icon_font(11.0)).color(DIM()));
        ui.label(egui::RichText::new(MANAGED).small().color(DIM()));
    });
}

/// A switch for one settings value: grayed out with a note when a policy fixes that value.
pub(super) fn managed_toggle(ui: &mut Ui, on: &mut bool, key: &str, label: &str) -> bool {
    let fixed = policy::is_fixed(key);
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.add_enabled_ui(!fixed, |ui| changed = toggle(ui, on, label));
        if fixed {
            managed_note(ui);
        }
    });
    changed
}

/// A tick box for one settings value, greyed out with the note when a policy fixes it. The arrow's room is kept
/// free, so it lines up with the groups above it.
fn managed_check(ui: &mut Ui, on: &mut bool, key: &str, label: &str) -> bool {
    let fixed = policy::is_fixed(key);
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.add_space(22.0);
        changed = tick_box(ui, !fixed, egui::Checkbox::new(on, label)).changed();
        if fixed {
            managed_note(ui);
        }
    });
    changed
}

/// A tick box with an outline, so an empty one still shows against the card.
fn tick_box(ui: &mut Ui, enabled: bool, check: egui::Checkbox) -> egui::Response {
    ui.scope(|ui| {
        ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::new(1.0_f32, FAINT());
        ui.add_enabled(enabled, check)
    })
    .inner
}

/// A section title for one settings value, with the note when a policy fixes it.
fn managed_heading(ui: &mut Ui, title: &str, sub: &str, key: &str) {
    heading(ui, title, sub);
    if policy::is_fixed(key) {
        managed_note(ui);
        ui.add_space(4.0);
    }
}

impl App {
    pub(super) fn set_titlebar(&mut self) {
        self.ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(!self.settings.title_custom));
        // re-apply the size so Windows recomputes the client area around the new frame
        if let Some(r) = self.ctx.input(|i| i.viewport().inner_rect) {
            self.ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(r.size()));
        }
        self.save_settings();
    }

    /// With "Follow Windows" / the Windows accent, pick up changes made in Windows Settings
    /// while the app is running (checked every two seconds; two registry reads).
    pub(super) fn follow_windows_theme(&mut self, ctx: &egui::Context) {
        if self.settings.theme != 0 && self.settings.accent < ACCENTS.len() && !self.settings.explorer_colours() {
            return;
        }
        let now = ctx.input(|i| i.time);
        if now - self.sys_check < 2.0 {
            return;
        }
        self.sys_check = now;
        ctx.request_repaint_after(Duration::from_secs(2));
        let p = self.settings.palette();
        let cur = pal();
        if p.accent != cur.accent || p.dark != cur.dark {
            if p.dark != cur.dark {
                self.forget_system_icons();
            }
            apply(ctx, p);
            self.set_backdrop();
            let icon = egui::IconData { rgba: logo_rgba(64, p.accent), width: 64, height: 64 };
            ctx.send_viewport_cmd(egui::ViewportCommand::Icon(Some(Arc::new(icon))));
        }
    }

    /// When Explorer's colours decide the theme and accent, says so with a way to that switch; true then.
    fn explorer_decides(&mut self, ui: &mut Ui) -> bool {
        if !self.settings.explorer_colours() {
            return false;
        }
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Set by \"Explorer's colours\" under Windows Explorer.").small().color(DIM()));
            if ui.link(egui::RichText::new("Change").small()).clicked() {
                self.acts.push(Act::OpenSettings(Some(Page::Explorer)));
            }
        });
        ui.add_space(4.0);
        true
    }

    /// Windows draws some of its icons for the light or the dark theme: when that changes, they're fetched again.
    pub(super) fn forget_system_icons(&mut self) {
        self.icons.retain(|k, _| !super::is_system_icon_key(k));
        self.icon_retry.clear();
        self.icon_asked.clear();
    }

    /// Mica behind the window, tinted for the theme, when the window was made for it and it's wanted; none otherwise.
    pub(super) fn set_backdrop(&self) {
        #[cfg(windows)]
        if self.hwnd != 0 && crate::theme::mica_window() {
            let on = self.settings.wants_mica();
            shell::set_backdrop(self.hwnd, on, pal().dark);
        }
    }

    pub(super) fn apply_theme(&mut self) {
        let p = self.settings.palette();
        if p.dark != pal().dark {
            self.forget_system_icons();
        }
        apply(&self.ctx, p);
        self.set_backdrop();
        let icon = egui::IconData { rgba: logo_rgba(64, p.accent), width: 64, height: 64 };
        self.ctx.send_viewport_cmd(egui::ViewportCommand::Icon(Some(Arc::new(icon))));
        self.save_settings();
    }

    pub(super) fn settings_ui(&mut self, ui: &mut Ui) {
        egui::Panel::left("settings_nav")
            .resizable(false)
            .exact_size(210.0)
            .frame(egui::Frame::new().fill(BG()).inner_margin(egui::Margin { left: 16, right: 8, top: 18, bottom: 12 }))
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.label(egui::RichText::new("Settings").size(20.0).strong());
                ui.add_space(12.0);
                for (p, g, label) in PAGES {
                    // standing in for File Explorer only means something on Windows
                    if p == Page::Explorer && !cfg!(windows) {
                        continue;
                    }
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
                    let active = self.settings_page == p;
                    if active {
                        ui.painter().rect_filled(r, CornerRadius::same(6), SEL());
                        ui.painter().rect_filled(Rect::from_min_size(r.left_top() + vec2(0.0, 9.0), vec2(3.0, 16.0)), CornerRadius::same(2), ACCENT());
                    } else if resp.hovered() {
                        ui.painter().rect_filled(r, CornerRadius::same(6), HOVER());
                    }
                    ui.painter().text(r.left_center() + vec2(18.0, 0.0), Align2::CENTER_CENTER, ic(g), icon_font(14.0), if active { ACCENT() } else { DIM() });
                    ui.painter().text(r.left_center() + vec2(38.0, 0.0), Align2::LEFT_CENTER, label, FontId::proportional(14.0), TEXT());
                    if resp.clicked() {
                        self.settings_page = p;
                    }
                }
            });
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG()).inner_margin(egui::Margin { left: 24, right: 24, top: 18, bottom: 12 })).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                // cap the width on very wide windows; never wider than what's visible
                let w = ui.available_width().min(1040.0);
                ui.set_max_width(w);
                if policy::current().any_fixed() {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(ic(I_LOCK)).font(icon_font(13.0)).color(ACCENT()));
                        ui.label(egui::RichText::new("Some settings are managed by your organization. They are grayed out and can't be changed here.").color(DIM()));
                    });
                    ui.add_space(10.0);
                }
                match self.settings_page {
                    Page::Appearance => self.page_appearance(ui),
                    Page::Sidebar => self.page_sidebar(ui),
                    Page::Ribbon => self.page_ribbon(ui),
                    Page::ContextMenu => {
                        card().show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            self.menu_editor(ui);
                        });
                    }
                    Page::Files => self.page_files(ui),
                    Page::Explorer => self.page_explorer(ui),
                    Page::About => {
                        self.about_card(ui);
                        ui.add_space(14.0);
                        self.updates_card(ui);
                        ui.add_space(14.0);
                        self.storage_card(ui);
                    }
                }
                ui.add_space(20.0);
            });
        });
    }

    // ------------------------------------------------------------------ appearance

    fn page_appearance(&mut self, ui: &mut Ui) {
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            managed_heading(ui, "Theme", "", "theme");
            let explorer = self.explorer_decides(ui);
            ui.add_enabled_ui(!policy::is_fixed("theme") && !explorer, |ui| ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(12.0, 12.0);
                let accent = self.settings.palette().accent;
                for (i, name) in [(0u8, if cfg!(windows) { "Follow Windows" } else { "Follow system" }), (1, "Dark"), (2, "Light")] {
                    let dark = match i {
                        0 => system_dark(),
                        1 => true,
                        _ => false,
                    };
                    // what's in effect: Explorer's look follows Windows
                    let chosen = if explorer { 0 } else { self.settings.theme };
                    if theme_card(ui, name, Palette::new(dark, accent), chosen == i) {
                        self.settings.theme = i;
                        self.apply_theme();
                    }
                }
            }));
            ui.add_space(16.0);
            managed_heading(ui, "Accent colour", "", "accent2");
            let explorer = self.explorer_decides(ui);
            ui.add_enabled_ui(!policy::is_fixed("accent2") && !explorer, |ui| ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
                let dark = pal().dark;
                // Windows' own accent first: it's the default
                let mut opts: Vec<(usize, String, Color32)> = vec![(ACCENTS.len(), if cfg!(windows) { "Windows accent colour (follows your Windows settings)" } else { "System accent colour" }.into(), system_accent().unwrap_or(ACCENTS[0].1))];
                opts.extend(ACCENTS.iter().enumerate().map(|(i, a)| (i, a.0.to_string(), if dark { a.1 } else { a.2 })));
                for (n, (i, name, c)) in opts.into_iter().enumerate() {
                    let (r, resp) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::click());
                    ui.painter().circle_filled(r.center(), 13.0, c);
                    if n == 0 {
                        // mark the Windows option with the Windows logo squares
                        for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                            let q = Rect::from_center_size(r.center() + vec2(dx * 2.6, dy * 2.6), vec2(4.2, 4.2));
                            ui.painter().rect_filled(q, CornerRadius::ZERO, Color32::from_white_alpha(230));
                        }
                    }
                    let chosen = if explorer { n == 0 } else { self.settings.accent == i };
                    if chosen {
                        ui.painter().circle_stroke(r.center(), 16.0, Stroke::new(2.0_f32, TEXT()));
                    } else if resp.hovered() {
                        ui.painter().circle_stroke(r.center(), 16.0, Stroke::new(1.0_f32, FAINT()));
                    }
                    if resp.on_hover_text(name).clicked() {
                        self.settings.accent = i;
                        self.apply_theme();
                    }
                }
            }));
            ui.add_space(16.0);
            managed_heading(ui, "Row height", "How tall each row in the file list is.", "density");
            let explorer = look_fixed(ui);
            ui.add_enabled_ui(!policy::is_fixed("density") && !explorer, |ui| ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(12.0, 12.0);
                for (i, l) in ["Compact", "Normal", "Comfortable"].iter().enumerate() {
                    if self.density_card(ui, i as u8, l) {
                        self.settings.density = i as u8;
                        self.save_settings();
                    }
                }
            }));
        });
        ui.add_space(14.0);
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            heading(ui, "Title bar", "The bar with the tabs at the top of the window.");
            if managed_toggle(ui, &mut self.settings.title_custom, "title_custom", if cfg!(windows) { "Use the Daleris title bar (off = standard Windows title bar)" } else { "Use the Daleris title bar (off = the system's own title bar)" }) {
                self.set_titlebar();
                self.save_settings();
            }
            ui.add_enabled_ui(self.settings.title_custom, |ui| {
                let mut ch = managed_toggle(ui, &mut self.settings.title_logo, "title_logo", "Show the logo");
                ch |= managed_toggle(ui, &mut self.settings.title_name, "title_name2", "Show the app name");
                if ch {
                    self.save_settings();
                }
            });
            if managed_toggle(ui, &mut self.settings.title_gear, "title_gear", "Show the settings button (Settings also opens with Ctrl+,)") {
                self.save_settings();
            }
        });
        ui.add_space(14.0);
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            managed_heading(ui, "Icon pack", "How files and folders are drawn everywhere in the app.", "iconpack");
            let explorer = look_fixed(ui);
            let mut packs: Vec<(String, String)> = vec![("flat".into(), "Daleris".into()), ("phosphor".into(), "Phosphor".into())];
            if !prefers_bundled_icons() {
                packs.insert(0, ("system".into(), "Windows (system icons)".into()));
            }

            packs.extend(list_icon_packs().into_iter().map(|p| (p.clone(), p)));
            let pack_free = !policy::is_fixed("iconpack") && !explorer;
            for (id, name) in packs {
                let selected = self.settings.pack() == id;
                let fill = if selected { SEL() } else { RAISED() };
                let resp = ui
                    .add_enabled_ui(pack_free, |ui| {
                        egui::Frame::new()
                            .fill(fill)
                            .stroke(Stroke::new(1.0_f32, if selected { ACCENT() } else { LINE() }))
                            .corner_radius(8)
                            .inner_margin(egui::Margin::symmetric(12, 8))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    ui.add_sized(vec2(190.0, 28.0), egui::Label::new(egui::RichText::new(&name).strong()).selectable(false));
                                    self.pack_preview(ui, &id);
                                });
                            })
                            .response
                            .interact(Sense::click())
                    })
                    .inner;
                if resp.clicked() && !selected && pack_free {
                    self.settings.icon_pack = id.clone();
                    self.save_settings();
                }
                ui.add_space(6.0);
            }
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(
                    "Add your own pack: make a folder in the icon packs folder and put PNG files in it named folder.png, file.png, drive.png, \
                     desktop.png, documents.png, downloads.png…, or by file type (pdf.png, jpg.png) or kind (image.png, video.png, audio.png, code.png, archive.png). \
                     Anything missing falls back to the standard icon.",
                )
                .color(DIM()),
            );
            ui.horizontal(|ui| {
                if ui.button(egui::RichText::new(format!("{}  Open icon packs folder", ic(I_FOLDER))).font(icon_font(13.0))).clicked() {
                    let d = icon_pack_dir();
                    let _ = std::fs::create_dir_all(&d);
                    self.acts.push(Act::NavigateNewTab(Loc::Dir(d)));
                }
                if ui.button("Reload packs").clicked() {
                    self.icons.retain(|k, _| !k.starts_with("pack:"));
                }
            });
        });
    }

    /// A miniature file list drawn at one row height, so the choice is visible.
    fn density_card(&mut self, ui: &mut Ui, density: u8, label: &str) -> bool {
        let row_h = [24.0, 28.0, 34.0][density as usize];
        let rows = 4;
        let (r, resp) = ui.allocate_exact_size(vec2(230.0, rows as f32 * row_h + 34.0), Sense::click());
        let selected = self.settings.density() == density;
        let list = Rect::from_min_size(r.min, vec2(r.width(), rows as f32 * row_h + 8.0));
        ui.painter().rect_filled(list, CornerRadius::same(8), BG());
        let samples = [("Documents", "", true), ("Holiday.jpg", "jpg", false), ("Report.pdf", "pdf", false), ("notes.txt", "txt", false)];
        let docs = self.places.iter().find(|p| p.name == "Documents").map(|p| p.path.clone()).unwrap_or_default();
        for (k, (name, ext, dir)) in samples.iter().enumerate() {
            let rr = Rect::from_min_size(list.min + vec2(4.0, 4.0 + k as f32 * row_h), vec2(list.width() - 8.0, row_h));
            if k == 1 {
                ui.painter().rect_filled(rr.shrink2(vec2(0.0, 1.0)), CornerRadius::same(4), SEL());
            }
            let ic_r = Rect::from_center_size(pos2(rr.left() + 14.0, rr.center().y), vec2(16.0, 16.0));
            let p = if *dir { docs.clone() } else { PathBuf::from(format!("x.{ext}")) };
            self.draw_icon(ui, ic_r, &p, ext, *dir, if *dir { fsx::A_READONLY } else { 0 });
            ui.painter().text(pos2(rr.left() + 30.0, rr.center().y), Align2::LEFT_CENTER, *name, FontId::proportional(12.5), TEXT());
            ui.painter().text(pos2(rr.right() - 6.0, rr.center().y), Align2::RIGHT_CENTER, "2026-11-02", FontId::proportional(11.0), FAINT());
        }
        let stroke = if selected { Stroke::new(2.0_f32, ACCENT()) } else if resp.hovered() { Stroke::new(1.0_f32, DIM()) } else { Stroke::new(1.0_f32, LINE()) };
        ui.painter().rect_stroke(list, CornerRadius::same(8), stroke, StrokeKind::Outside);
        ui.painter().text(pos2(r.left() + 2.0, r.bottom() - 12.0), Align2::LEFT_CENTER, label, FontId::proportional(13.5), if selected { TEXT() } else { DIM() });
        resp.clicked()
    }

    /// A row of sample icons drawn with icon pack `id`.
    fn pack_preview(&mut self, ui: &mut Ui, id: &str) {
        let saved = std::mem::replace(&mut self.settings.icon_pack, id.to_string());
        let docs = self.places.iter().find(|p| p.name == "Documents").map(|p| p.path.clone());
        let desk = self.places.iter().find(|p| p.name == "Desktop").map(|p| p.path.clone());
        let mut samples: Vec<(PathBuf, String, bool)> = Vec::new();
        samples.push((PathBuf::from(if cfg!(windows) { r"C:\" } else { "/" }), String::new(), true));
        if let Some(d) = desk {
            samples.push((d, String::new(), true));
        }
        if let Some(d) = docs {
            samples.push((d, String::new(), true));
        }
        samples.push((PathBuf::from(r"C:\_sample_folder"), String::new(), true));
        for e in ["txt", "pdf", "jpg", "mp3", "mp4", "zip", "rs", "exe"] {
            samples.push((PathBuf::from(format!("sample.{e}")), e.into(), false));
        }
        for (p, ext, dir) in samples {
            let (r, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
            let attrs = if dir && p.to_string_lossy().contains("_sample") { 0 } else { fsx::A_READONLY };
            let path = if ext == "exe" { PathBuf::from(r"C:\Windows\explorer.exe") } else { p };
            self.draw_icon(ui, r.shrink(2.0), &path, &ext, dir, attrs);
        }
        self.settings.icon_pack = saved;
    }

    // ------------------------------------------------------------------ sidebar editor

    fn page_sidebar(&mut self, ui: &mut Ui) {
        ui.label(egui::RichText::new("Sidebar").heading());
        ui.label(
            egui::RichText::new(
                "Show, hide and reorder sections and the places in them. In the sidebar itself you can also drag places to reorder them, \
                 drag folders from the file list onto a section title to pin them, and right-click anything for more.",
            )
            .color(DIM()),
        );
        let locked = policy::current().sidebar_locked();
        if locked {
            managed_note(ui);
        }
        ui.add_space(10.0);
        ui.add_enabled_ui(!locked, |ui| self.sidebar_editor(ui));
    }

    fn sidebar_editor(&mut self, ui: &mut Ui) {
        let model = self.sidebar_model();
        let n = model.len();
        for (si, sec) in model.iter().enumerate() {
            card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let mut vis = sec.visible;
                    if eye(ui, &mut vis, &sec.id) {
                        if let Some(s) = self.settings.section_mut(&sec.id) {
                            s.visible = vis;
                        }
                        self.save_settings();
                    }
                    if sec.custom {
                        let mut title = sec.title.clone();
                        if ui.add(egui::TextEdit::singleline(&mut title).desired_width(220.0).font(FontId::proportional(14.0))).changed() {
                            if let Some(s) = self.settings.section_mut(&sec.id) {
                                s.title = title;
                            }
                            self.save_settings();
                        }
                    } else {
                        ui.label(egui::RichText::new(&sec.title).size(14.0).strong().color(if sec.visible { TEXT() } else { FAINT() }));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if sec.custom && small_btn(ui, I_DELETE, "Delete section") {
                            self.settings.sections.retain(|s| s.id != sec.id);
                            self.save_settings();
                        }
                        if sec.accepts_pins && ui.button(egui::RichText::new(format!("{}  Add folder…", ic(I_ADD))).font(icon_font(12.5))).clicked() {
                            if let Some(p) = shell::pick_folder() {
                                self.pin_to(&sec.id, p);
                            }
                        }
                        if si + 1 < n && small_btn(ui, I_CHEV_D, "Move section down") {
                            self.move_section(&sec.id, 1);
                        }
                        if si > 0 && small_btn(ui, I_CHEV_U, "Move section up") {
                            self.move_section(&sec.id, -1);
                        }
                    });
                });
                ui.add_space(4.0);
                if sec.items.is_empty() {
                    ui.label(egui::RichText::new(if sec.accepts_pins { "No folders yet. Add one, or drag a folder onto this section in the sidebar." } else { "Nothing to show on this PC right now." }).color(FAINT()));
                }
                let m = sec.items.len();
                for (ii, item) in sec.items.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.add_space(24.0);
                        let mut vis = item.visible;
                        if eye(ui, &mut vis, &format!("{}{}", sec.id, item.key)) {
                            self.toggle_side_item(&sec.id, &item.key);
                        }
                        let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                        match (&item.loc, item.glyph) {
                            (_, Some(g)) => {
                                ui.painter().text(r.center(), Align2::CENTER_CENTER, ic(g), icon_font(14.0), ACCENT());
                            }
                            (Some(Loc::Dir(p)), None) => {
                                let p = p.clone();
                                self.draw_icon(ui, r, &p, "", true, fsx::A_READONLY);
                            }
                            _ => {}
                        }
                        ui.label(egui::RichText::new(&item.label).color(if item.visible && sec.visible { TEXT() } else { FAINT() }));
                        if item.pinned {
                            ui.label(egui::RichText::new("pinned").small().color(FAINT()));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if item.pinned && small_btn(ui, I_CLOSE, "Unpin") {
                                if let Some(s) = self.settings.section_mut(&sec.id) {
                                    s.pins.retain(|p| side_key(p) != item.key);
                                }
                                self.save_settings();
                            }
                            if ii + 1 < m && small_btn(ui, I_CHEV_D, "Move down") {
                                self.move_side_item(&sec.id, &item.key, None, 1);
                            }
                            if ii > 0 && small_btn(ui, I_CHEV_U, "Move up") {
                                self.move_side_item(&sec.id, &item.key, None, -1);
                            }
                        });
                    });
                }
            });
            ui.add_space(10.0);
        }
        ui.horizontal(|ui| {
            if ui.button(egui::RichText::new(format!("{}  New section", ic(I_ADD))).font(icon_font(13.0))).clicked() {
                self.new_section(None);
                self.renaming_section = None; // rename right here instead
            }
            if ui.button("Reset sidebar").clicked() {
                let pins: Vec<(String, Vec<PathBuf>)> = self.settings.sections.iter().map(|s| (s.id.clone(), s.pins.clone())).collect();
                self.settings.sections = default_sections();
                for (id, p) in pins {
                    if let Some(s) = self.settings.section_mut(&id) {
                        s.pins = p;
                    }
                }
                self.settings.side_hidden.clear();
                self.settings.side_order.clear();
                self.save_settings();
            }
        });
    }

    // ------------------------------------------------------------------ ribbon bar editor

    fn page_ribbon(&mut self, ui: &mut Ui) {
        ui.label(egui::RichText::new("Ribbon bar").heading());
        ui.label(egui::RichText::new("Show, hide and reorder what's in the bar above the file list, and choose which side each item sits on.").color(DIM()));
        let locked = policy::current().ribbon_locked();
        if locked {
            managed_note(ui);
        }
        ui.add_space(10.0);
        self.ribbon_preview(ui);
        ui.add_space(10.0);
        ui.add_enabled_ui(!locked, |ui| {
            self.ribbon_side(ui, true);
            ui.add_space(10.0);
            self.ribbon_side(ui, false);
            ui.add_space(10.0);
            if ui.button("Reset ribbon bar").clicked() {
                let d = Settings::baseline();
                self.settings.ribbon_left = d.ribbon_left;
                self.settings.ribbon_right = d.ribbon_right;
                self.settings.ribbon_hidden = d.ribbon_hidden;
                self.save_settings();
            }
        });
    }

    /// The bar as it will look, so a change can be seen without leaving Settings.
    fn ribbon_preview(&self, ui: &mut Ui) {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::same(8), PANEL());
        painter.rect_stroke(rect, CornerRadius::same(8), Stroke::new(1.0_f32, LINE()), StrokeKind::Inside);
        let item_w = |id: &str| -> (f32, Option<Arc<egui::Galley>>) {
            let word = match id {
                "new" => format!("{}  New", ic(I_ADD)),
                "sort" => format!("{}  Sort: Name", ic(I_SORT)),
                "view" => format!("{}  View", ic(I_LIST)),
                _ => return (if id.starts_with("sep") { 22.0 } else { 34.0 }, None),
            };
            let g = painter.layout_no_wrap(word, icon_font(13.5), TEXT());
            (g.size().x + 16.0, Some(g))
        };
        let draw = |x: f32, id: &str, w: f32, galley: Option<Arc<egui::Galley>>| {
            let cy = rect.center().y;
            if let Some(g) = galley {
                painter.galley(pos2(x + 8.0, cy - g.size().y / 2.0), g, TEXT());
            } else if id.starts_with("sep") {
                painter.line_segment([pos2(x + w / 2.0, cy - 11.0), pos2(x + w / 2.0, cy + 11.0)], Stroke::new(1.0_f32, LINE()));
            } else if let Some((_, g, _)) = RIBBON_ITEMS.iter().find(|(k, _, _)| *k == id) {
                painter.text(pos2(x + w / 2.0, cy), Align2::CENTER_CENTER, ic(g), icon_font(15.0), TEXT());
            }
        };
        let mut x = rect.left() + 8.0;
        for id in self.settings.ribbon_shown(&self.settings.ribbon_left) {
            let (w, g) = item_w(&id);
            draw(x, &id, w, g);
            x += w;
        }
        let mut x = rect.right() - 8.0;
        for id in self.settings.ribbon_shown(&self.settings.ribbon_right).iter().rev() {
            let (w, g) = item_w(id);
            x -= w;
            draw(x, id, w, g);
        }
    }

    /// One side of the bar: the same rows as the sidebar editor, plus a button to change sides.
    fn ribbon_side(&mut self, ui: &mut Ui, left: bool) {
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(if left { "Left side" } else { "Right side" }).size(14.0).strong());
            ui.add_space(4.0);
            let ids = if left { self.settings.ribbon_left.clone() } else { self.settings.ribbon_right.clone() };
            if ids.is_empty() {
                ui.label(egui::RichText::new("Nothing here. Move an item over from the other side.").color(FAINT()));
            }
            let m = ids.len();
            for (i, id) in ids.iter().enumerate() {
                let Some((_, glyph, label)) = RIBBON_ITEMS.iter().find(|(k, _, _)| k == id) else { continue };
                if id == "more" && !cfg!(windows) {
                    continue;
                }
                ui.horizontal(|ui| {
                    let mut vis = !self.settings.ribbon_hidden.contains(id);
                    if eye(ui, &mut vis, id) {
                        if vis {
                            self.settings.ribbon_hidden.remove(id);
                        } else {
                            self.settings.ribbon_hidden.insert(id.clone());
                        }
                        self.save_settings();
                    }
                    let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, ic(glyph), icon_font(14.0), if vis { ACCENT() } else { FAINT() });
                    ui.label(egui::RichText::new(*label).color(if vis { TEXT() } else { FAINT() }));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if small_btn(ui, if left { I_FWD } else { I_BACK }, if left { "Move to the right side" } else { "Move to the left side" }) {
                            self.ribbon_switch_side(id, left);
                        }
                        if i + 1 < m && small_btn(ui, I_CHEV_D, "Move down (further right in the bar)") {
                            self.ribbon_move(left, i, i + 1);
                        }
                        if i > 0 && small_btn(ui, I_CHEV_U, "Move up (further left in the bar)") {
                            self.ribbon_move(left, i, i - 1);
                        }
                    });
                });
            }
        });
    }

    fn ribbon_move(&mut self, left: bool, from: usize, to: usize) {
        let side = if left { &mut self.settings.ribbon_left } else { &mut self.settings.ribbon_right };
        if from < side.len() && to < side.len() {
            side.swap(from, to);
            self.save_settings();
        }
    }

    /// Move an item to the other side, to the end nearest the side it came from.
    fn ribbon_switch_side(&mut self, id: &str, from_left: bool) {
        if from_left {
            self.settings.ribbon_left.retain(|k| k != id);
            self.settings.ribbon_right.insert(0, id.to_string());
        } else {
            self.settings.ribbon_right.retain(|k| k != id);
            self.settings.ribbon_left.push(id.to_string());
        }
        self.save_settings();
    }

    // ------------------------------------------------------------------ files

    /// What this is, which version, and where it comes from.
    fn about_card(&mut self, ui: &mut Ui) {
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
                paint_logo(ui.painter(), r, ACCENT());
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("Daleris").size(20.0).strong());
                    ui.label(format!("Version {}", crate::update::CURRENT));
                    ui.label(egui::RichText::new("A file explorer built for speed. Free software under the GPL-3.0 licence.").color(DIM()));
                });
            });
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                for (label, url) in [
                    ("Project", crate::update::PROJECT.to_string()),
                    ("What's new", crate::update::RELEASES.to_string()),
                    ("Report a problem", format!("{}/issues", crate::update::PROJECT)),
                    ("Licence", format!("{}/blob/main/LICENSE", crate::update::PROJECT)),
                ] {
                    if ui.button(label).on_hover_text(&url).clicked() {
                        shell::open_url(&url);
                    }
                }
            });
        });
    }

    /// Whether there's a newer version, and whether to install it without asking.
    fn updates_card(&mut self, ui: &mut Ui) {
        use crate::update::{Install, Status};
        let status = self.updater.as_ref().map_or(Status::Idle, |u| u.status());
        let install = self.updater.as_ref().map_or(Install::Unmanaged, |u| u.install.clone());
        let mut clicked_check = false;
        let mut clicked_install = false;
        let machine = matches!(install, Install::Machine { .. });
        // the whole PC shares this one: show what it's really set to
        if machine && !policy::is_fixed("auto_update") {
            if let Some(on) = crate::update::machine_switch() {
                self.settings.auto_update = on;
            }
        }
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            heading(ui, "Updates", "");
            if policy::current().no_updates {
                ui.label(egui::RichText::new("Your organization installs new versions of Daleris, so it doesn't look for updates itself.").color(DIM()));
                ui.add_space(4.0);
                managed_note(ui);
                return;
            }
            let how = match &install {
                Install::PerUser { .. } => "Installed for this account. Updates install silently when you close Daleris.",
                Install::Machine { .. } => "Installed for this PC. A background task installs updates once Daleris is closed, within about an hour.",
                Install::Portable(_) | Install::AppImage(_) => "A single program file. Updates replace it, and take effect the next time you start it.",
                Install::MacApp(_) => "Updates replace the app, and take effect the next time you start it.",
                Install::LinuxPackage(_) => "Installed by your package manager, so updates ask for your password.",
                Install::Unmanaged => "This copy can't update itself. New versions are on the releases page.",
            };
            ui.label(egui::RichText::new(how).color(DIM()));
            ui.add_space(6.0);
            if managed_toggle(ui, &mut self.settings.auto_update, "auto_update", "Install updates automatically") {
                if machine {
                    if let Err(e) = crate::update::set_machine_switch(self.settings.auto_update) {
                        self.settings.auto_update = !self.settings.auto_update;
                        if let Some(updater) = &self.updater {
                            updater.fail(e);
                        }
                    }
                }
                self.save_settings();
            }
            ui.add_space(6.0);
            let text = match &status {
                Status::Idle => "Not checked yet.".to_string(),
                Status::Checking => "Checking for updates…".to_string(),
                Status::UpToDate => "Daleris is up to date.".to_string(),
                Status::Available(v) if machine => format!("Version {v} is available. It installs when automatic updates are on."),
                Status::Available(v) => format!("Version {v} is available."),
                Status::Downloading(v) => format!("Downloading version {v}…"),
                Status::InstallsOnClose(v) => format!("Version {v} installs when you close Daleris."),
                Status::RestartToFinish(v) => format!("Version {v} is ready. Restart Daleris to use it."),
                Status::Failed(e) => e.clone(),
            };
            ui.label(if matches!(status, Status::Failed(_)) { egui::RichText::new(text).color(WARN()) } else { egui::RichText::new(text) });
            ui.add_space(6.0);
            let busy = matches!(status, Status::Checking | Status::Downloading(_));
            ui.horizontal(|ui| {
                clicked_check = ui.add_enabled(!busy && self.updater.is_some(), egui::Button::new("Check for updates")).clicked();
                if matches!(status, Status::Available(_)) && !machine && !matches!(install, Install::Unmanaged) {
                    clicked_install = ui.button("Install update").clicked();
                }
            });
        });
        if let Some(updater) = &self.updater {
            if clicked_check {
                updater.check_now(self.settings.auto_update);
            }
            if clicked_install {
                updater.install_now();
            }
        }
    }

    /// Where the settings file and the log are, for editing by hand or troubleshooting.
    fn storage_card(&mut self, ui: &mut Ui) {
        let settings = config_dir().map(|d| d.join("settings.txt"));
        let log = crate::logging::path().cloned();
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            heading(ui, "Settings file & log", "Settings are a plain text file. If Daleris crashes, the details are in the log.");
            for (label, file, button) in [("Settings", settings, "Open settings folder"), ("Log", log, "Open log folder")] {
                ui.horizontal(|ui| {
                    ui.add_sized(vec2(60.0, 24.0), egui::Label::new(label));
                    let shown = file.as_ref().map_or("Not available".to_string(), |f| f.display().to_string());
                    ui.add(egui::Label::new(egui::RichText::new(shown).monospace().color(DIM())).truncate());
                });
                if let Some(dir) = file.as_ref().and_then(|f| f.parent()) {
                    if ui.button(egui::RichText::new(format!("{}  {button}", ic(I_FOLDER))).font(icon_font(13.0))).clicked() {
                        let _ = std::fs::create_dir_all(dir);
                        self.acts.push(Act::NavigateNewTab(Loc::Dir(dir.to_path_buf())));
                    }
                }
                ui.add_space(6.0);
            }
        });
    }

    fn page_files(&mut self, ui: &mut Ui) {

        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            heading(ui, "Files & folders", "");
            let mut ch = false;
            let hidden_label = if cfg!(windows) { "Show hidden and system items" } else { "Show hidden items (names starting with a dot)" };
            if managed_toggle(ui, &mut self.settings.show_hidden, "hidden", hidden_label) {
                self.refilter_tabs();
                ch = true;
            }
            // on Linux and macOS a leading dot is what makes a name hidden, so the switch above covers them
            if cfg!(windows) {
                if managed_toggle(ui, &mut self.settings.hide_dotfiles, "hide_dots", "Hide dotfiles and dotfolders") {
                    self.refilter_tabs();
                    ch = true;
                }
                ui.label(egui::RichText::new("Names starting with a dot, such as .git or .env. This works on top of the setting above: an item stays hidden if either one hides it. Ctrl+Shift+. switches it for now.").size(12.0).color(DIM()));
            } else {
                ui.label(egui::RichText::new("Ctrl+Shift+. switches it for now.").size(12.0).color(DIM()));
            }
            ui.add_space(4.0);
            ch |= managed_toggle(ui, &mut self.settings.show_ext, "ext", "Show file name extensions");
            ch |= managed_toggle(ui, &mut self.settings.show_preview, "preview", "Show the preview pane");
            if managed_toggle(ui, &mut self.settings.folder_sizes, "folder_sizes", "Show folder sizes (everything inside each folder, counted in the background)") {
                self.tabs.iter_mut().for_each(|t| t.dirty = true);
                ch = true;
            }
            ch |= managed_toggle(ui, &mut self.settings.restore_tabs, "restore_tabs2", "Reopen my tabs when Daleris starts");
            if shell::HAS_NATIVE_PROPERTIES {
                ch |= managed_toggle(ui, &mut self.settings.native_props, "native_props", "Use the Windows Properties dialog instead of Daleris's");
            }
            // Windows opens a drive that was just connected as a folder, which comes here with "Open folders in Daleris"
            if cfg!(windows) {
                ch |= managed_toggle(ui, &mut self.settings.open_new_drives, "open_new_drives", "Open a tab for a drive when it's connected, such as a USB stick");
                ch |= managed_toggle(ui, &mut self.settings.start_new_window, "start_new_window", "Starting Daleris again opens a new window (off: a new tab in the open window)");
            }
            ch |= managed_toggle(ui, &mut self.settings.share_marks, "share_marks", "On network shares, let other people's Daleris see what you copy, move, delete and have open");
            ui.label(egui::RichText::new("Daleris leaves a small hidden note in the folder, with your name and your Windows account picture, and removes it when it's done. Notes from others show the same way.").size(12.0).color(DIM()));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let fixed = policy::is_fixed("view");
                ui.add_enabled_ui(!fixed, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Default view");
                        if ui.selectable_label(self.settings.view == View::Details, "Details").clicked() {
                            self.settings.view = View::Details;
                            ch = true;
                        }
                        if ui.selectable_label(self.settings.view == View::Icons, "Large icons").clicked() {
                            self.settings.view = View::Icons;
                            ch = true;
                        }
                    });
                });
                if fixed {
                    managed_note(ui);
                }
            });
            ui.add_space(8.0);
            let mut sub = self.settings.nested_search;
            if managed_toggle(ui, &mut sub, "nested_search", "Search subfolders too") {
                self.settings.nested_search = sub;
                let scope = if sub { Scope::Subfolders } else { Scope::ThisFolder };
                for t in self.tabs.iter_mut().filter(|t| t.scope != Scope::AllDrives) {
                    t.scope = scope;
                }
                self.refilter_tabs();
                ch = true;
            }
            if managed_toggle(ui, &mut self.settings.regex_search, "regex_search", "Treat searches as regular expressions") {
                self.refilter_tabs();
                ch = true;
            }
            if ch {
                self.save_settings();
            }
        });
        ui.add_space(12.0);
        self.search_skip_card(ui);
        ui.add_space(12.0);
        self.sort_card(ui);
    }

    /// What search never finds: ready-made groups, hidden and system items, and lists of the person's own.
    fn search_skip_card(&mut self, ui: &mut Ui) {
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            heading(ui, "Leave out of search", "Search never finds what's ticked here, and doesn't look inside ticked folders, which also makes it faster.");
            let mut ch = false;
            for (g, (key, name, _, _)) in search::SKIP_GROUPS.iter().enumerate() {
                let fixed = policy::is_fixed(key);
                let items = search::group_items(g);
                let ticks = &mut self.settings.skip_groups[g];
                let (all, some) = (ticks.iter().all(|t| *t), ticks.iter().any(|t| *t));
                let open_id = ui.make_persistent_id(("skip-group", g));
                let mut open = ui.data(|d| d.get_temp::<bool>(open_id).unwrap_or(false));
                ui.horizontal(|ui| {
                    let arrow = egui::RichText::new(ic(if open { I_CHEV_D } else { I_CHEV_R })).font(icon_font(10.0)).color(DIM());
                    if ui.add(egui::Button::new(arrow).frame(false)).on_hover_text(if open { "Hide what's in it" } else { "Choose what's in it one by one" }).clicked() {
                        open = !open;
                    }
                    ui.add_enabled_ui(!fixed, |ui| {
                        // a partly ticked group shows a dash; clicking it ticks all of it
                        let mut checked = all;
                        if tick_box(ui, true, egui::Checkbox::new(&mut checked, *name).indeterminate(some && !all)).changed() {
                            ticks.iter_mut().for_each(|t| *t = checked);
                            ch = true;
                        }
                    });
                    if fixed {
                        managed_note(ui);
                    } else if !open {
                        ui.add(egui::Label::new(egui::RichText::new(items.join("  ")).small().color(FAINT())).truncate());
                    }
                });
                if open {
                    for (i, item) in items.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.add_space(44.0);
                            ch |= tick_box(ui, !fixed, egui::Checkbox::new(&mut ticks[i], *item)).changed();
                        });
                    }
                }
                ui.data_mut(|d| d.insert_temp(open_id, open));
            }
            ui.add_space(4.0);
            ch |= managed_check(ui, &mut self.settings.skip_hidden, "search_skip_hidden", "Hidden files and folders, even when hidden items are shown");
            // only Windows marks files as system files
            if cfg!(windows) {
                ch |= managed_check(ui, &mut self.settings.skip_system, "search_skip_system", "System files, such as desktop.ini");
            }
            let folder_example = if cfg!(windows) { r"%USERPROFILE%\Archive" } else { "~/Archive" };
            for (n, (key, title, sub, example)) in [
                (policy::SKIP_FOLDERS, "Folders", "These folders and everything in them.", folder_example),
                (policy::SKIP_NAMES, "Folder names", "Folders with these names, wherever they are.", "Backup"),
                (policy::SKIP_TYPES, "File types", "Files ending in these.", ".iso"),
                (policy::SKIP_PATTERNS, "Patterns", "Regular expressions, matched against the whole path. Case doesn't matter.", r"\.(key|pem)$"),
            ]
            .into_iter()
            .enumerate()
            {
                ui.add_space(10.0);
                ch |= self.skip_list(ui, n, key, title, sub, example);
            }
            if ch {
                self.save_settings();
                // the next search builds its index again, without what's left out now
                self.refilter_tabs();
            }
        });
    }

    /// One list: what the administrator adds, with a lock, then the person's own, each removable, and a field to add one.
    fn skip_list(&mut self, ui: &mut Ui, n: usize, key: &str, title: &str, sub: &str, example: &str) -> bool {
        let mut fixed = policy::current().fixed_items(key);
        if key == policy::SKIP_TYPES {
            // shown the way the person's own are written
            fixed = fixed.iter().filter_map(|t| search::file_type(t)).collect();
        }
        let invalid = |item: &str| if key == policy::SKIP_PATTERNS { search::pattern(item).err() } else { None };
        let mut changed = false;
        ui.label(egui::RichText::new(title).strong());
        ui.label(egui::RichText::new(sub).small().color(DIM()));
        let own = own_list(&mut self.settings, key);
        if !fixed.is_empty() || !own.is_empty() {
            let mut remove = None;
            ui.horizontal_wrapped(|ui| {
                for item in &fixed {
                    rule_chip(ui, item, true, invalid(item).is_some());
                }
                for (i, item) in own.iter().enumerate() {
                    if rule_chip(ui, item, false, invalid(item).is_some()) {
                        remove = Some(i);
                    }
                }
            });
            if let Some(i) = remove {
                own.remove(i);
                changed = true;
            }
            for item in fixed.iter().chain(own.iter()) {
                if let Some(e) = invalid(item) {
                    ui.label(egui::RichText::new(format!("{item}  isn't used: {e}")).small().color(WARN()));
                }
            }
        }
        let mut add = false;
        ui.horizontal(|ui| {
            let field = ui.add(egui::TextEdit::singleline(&mut self.skip_new[n]).hint_text(format!("Add one, such as {example}")).desired_width(320.0));
            add = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            add |= ui.add_enabled(!self.skip_new[n].trim().is_empty(), egui::Button::new(egui::RichText::new(format!("{}  Add", ic(I_ADD))).font(icon_font(13.0)))).clicked();
            if key == policy::SKIP_FOLDERS && ui.button(egui::RichText::new(format!("{}  Choose folder…", ic(I_FOLDER))).font(icon_font(13.0))).clicked() {
                if let Some(p) = shell::pick_folder() {
                    self.skip_new[n] = p.display().to_string();
                    add = true;
                }
            }
        });
        let text = self.skip_new[n].trim().to_string();
        // a list other than patterns keeps its items apart with |, so none can hold one
        let item = if key == policy::SKIP_TYPES { search::file_type(&text) } else { Some(text).filter(|t| !t.is_empty() && (key == policy::SKIP_PATTERNS || !t.contains('|'))) };
        if let (true, Some(item)) = (add, item) {
            let own = own_list(&mut self.settings, key);
            if !own.iter().chain(fixed.iter()).any(|x| x.eq_ignore_ascii_case(&item)) {
                own.push(item);
                changed = true;
            }
            self.skip_new[n].clear();
        }
        changed
    }

    /// How folders are sorted until someone chooses otherwise, in plain words.
    fn sort_card(&mut self, ui: &mut Ui) {
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            heading(ui, "Sorting", "A folder keeps the sort you choose for it, wherever it opens. Every other folder starts like this:");
            let mut ch = false;
            let fixed_key = policy::is_fixed("sort_key");
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!fixed_key, |ui| {
                    ui.label("Sort by");
                    let items: Vec<(String, bool)> = SortKey::ALL.iter().map(|k| (k.label().to_string(), true)).collect();
                    let at = SortKey::ALL.iter().position(|k| *k == self.settings.sort_key);
                    let (_, picked) = field_dropdown(ui, "default_sort", 190.0, self.settings.sort_key.label(), &items, at, !fixed_key);
                    if let Some(k) = picked.and_then(|i| SortKey::ALL.get(i).copied()) {
                        self.settings.sort_key = k;
                        if !policy::is_fixed("sort_asc") {
                            self.settings.sort_asc = k.starts_ascending();
                        }
                        ch = true;
                    }
                });
                let fixed_way = policy::is_fixed("sort_asc");
                ui.add_enabled_ui(!fixed_way, |ui| {
                    let spec = SortSpec { key: self.settings.sort_key, asc: true, dirs_first: true };
                    if ui.selectable_label(self.settings.sort_asc, spec.direction()).clicked() && !self.settings.sort_asc {
                        self.settings.sort_asc = true;
                        ch = true;
                    }
                    if ui.selectable_label(!self.settings.sort_asc, SortSpec { asc: false, ..spec }.direction()).clicked() && self.settings.sort_asc {
                        self.settings.sort_asc = false;
                        ch = true;
                    }
                });
                if fixed_key || fixed_way {
                    managed_note(ui);
                }
            });
            ui.add_space(6.0);
            ch |= managed_toggle(ui, &mut self.settings.dirs_first, "dirs_first", "Keep folders above files");
            ui.add_space(4.0);
            ch |= managed_toggle(ui, &mut self.settings.downloads_newest, "downloads_newest", "Show Downloads newest first, with folders mixed in");
            let n = self.settings.folder_sorts.len();
            if n > 0 {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("{n} folder{} sorted your own way", if n == 1 { " is" } else { "s are" })).color(DIM()));
                    if ui.button("Forget them all").clicked() {
                        self.settings.folder_sorts.clear();
                        ch = true;
                    }
                });
            }
            if ch {
                self.save_settings();
                // open folders nobody chose anything for follow the new default
                let mut tabs = std::mem::take(&mut self.tabs);
                for t in &mut tabs {
                    if let Some(d) = t.dir().map(Path::to_path_buf) {
                        t.set_spec(self.spec_for(&d));
                        t.dirty = true;
                    }
                }
                self.tabs = tabs;
            }
        });
    }
}

/// The person's own list in "Never found in search" for a settings file key.
fn own_list<'a>(s: &'a mut Settings, key: &str) -> &'a mut Vec<String> {
    match key {
        policy::SKIP_FOLDERS => &mut s.skip_folders,
        policy::SKIP_NAMES => &mut s.skip_names,
        policy::SKIP_TYPES => &mut s.skip_types,
        _ => &mut s.skip_patterns,
    }
}

/// One item of a "Never found in search" list: removable, or with a lock when the administrator added it.
fn rule_chip(ui: &mut Ui, text: &str, locked: bool, invalid: bool) -> bool {
    let mut removed = false;
    let stroke = Stroke::new(1.0_f32, if invalid { WARN() } else { LINE() });
    egui::Frame::new().fill(RAISED()).stroke(stroke).corner_radius(13).inner_margin(egui::Margin { left: 10, right: 2, top: 0, bottom: 0 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.label(egui::RichText::new(text).size(12.5).color(if locked { DIM() } else { TEXT() }));
            if locked {
                let (r, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
                ui.painter().text(r.center(), Align2::CENTER_CENTER, ic(I_LOCK), icon_font(11.0), DIM());
                resp.on_hover_text(MANAGED);
            } else {
                removed = small_btn(ui, I_CLOSE, "Remove");
            }
        });
    });
    removed
}

fn small_btn(ui: &mut Ui, glyph: &'static str, tip: &str) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(5), HOVER());
    }
    ui.painter().text(r.center(), Align2::CENTER_CENTER, ic(glyph), icon_font(12.0), DIM());
    resp.on_hover_text(tip).clicked()
}

fn eye(ui: &mut Ui, on: &mut bool, id: &str) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    let _ = id;
    if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(5), HOVER());
    }
    ui.painter().text(r.center(), Align2::CENTER_CENTER, ic(if *on { I_EYE } else { I_HIDE }), icon_font(13.0), if *on { ACCENT() } else { FAINT() });
    let clicked = resp.on_hover_text(if *on { "Shown. Click to hide" } else { "Hidden. Click to show" }).clicked();
    if clicked {
        *on = !*on;
    }
    clicked
}

/// A clickable miniature of the window in a given theme.
fn theme_card(ui: &mut Ui, name: &str, p: Palette, selected: bool) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(178.0, 128.0), Sense::click());
    let painter = ui.painter();
    let shot = Rect::from_min_size(r.min, vec2(r.width(), 96.0));
    painter.rect_filled(shot, CornerRadius::same(8), p.bg);
    painter.rect_filled(Rect::from_min_size(shot.min, vec2(shot.width(), 16.0)), CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 }, p.chrome);
    painter.rect_filled(Rect::from_min_size(shot.min + vec2(0.0, 16.0), vec2(46.0, 80.0)), CornerRadius { nw: 0, ne: 0, sw: 8, se: 0 }, p.panel);
    for i in 0..4 {
        let y = shot.top() + 26.0 + i as f32 * 16.0;
        let row = Rect::from_min_size(pos2(shot.left() + 54.0, y), vec2(shot.width() - 64.0, 10.0));
        painter.rect_filled(row, CornerRadius::same(3), if i == 1 { p.sel_strong } else { p.raised });
        painter.rect_filled(Rect::from_min_size(pos2(shot.left() + 8.0, y), vec2(30.0, 6.0)), CornerRadius::same(3), if i == 0 { p.accent } else { p.line });
    }
    painter.rect_stroke(shot, CornerRadius::same(8), Stroke::new(if selected { 2.0_f32 } else { 1.0_f32 }, if selected { ACCENT() } else { LINE() }), StrokeKind::Outside);
    painter.text(pos2(r.left() + 2.0, r.bottom() - 12.0), Align2::LEFT_CENTER, name, FontId::proportional(13.5), if selected { TEXT() } else { DIM() });
    resp.clicked()
}

/// A policy fixing "Look like Windows Explorer" fixes the settings it's made of too: says so; true then.
fn look_fixed(ui: &mut Ui) -> bool {
    let fixed = policy::is_fixed("explorer_look");
    if fixed {
        managed_note(ui);
    }
    fixed
}
