//! Who has a file open, shown with the file: up to three pictures at the end of its name, and everyone in the tooltip.

use super::*;
use crate::presence::Opener;

/// How often the folder in front is looked at again for open files.
const LOOK_EVERY: Duration = Duration::from_secs(6);

impl App {
    /// Looks at the folder in front for who has what open, now and then, off the window's thread.
    pub(super) fn look_for_openers(&mut self) {
        let share = self.settings.share_marks.then(crate::marks::open_file_name);
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        let Some(t) = self.tabs.get_mut(self.active) else { return };
        let Loc::Dir(dir) = &t.loc else { return };
        if t.loading || t.entries.is_empty() || t.presence_at.is_some_and(|at| at.elapsed() < LOOK_EVERY) {
            return;
        }
        t.presence_at = Some(Instant::now());
        let items: Vec<(String, bool)> = t.entries.iter().map(|e| (e.name.clone(), e.is_dir)).collect();
        let (dir, id) = (dir.clone(), t.id);
        bg::submit(&dir.clone(), "presence", move || {
            let open = crate::presence::scan(&dir, &items, share.as_deref());
            let _ = tx.send(Msg::Presence { tab: id, open });
            ctx.request_repaint();
        });
        self.ctx.request_repaint_after(LOOK_EVERY);
    }

    /// Everyone with the item at `path` open: you first, then the others, each once.
    pub(super) fn openers(&self, t: &Tab, path: &Path) -> Vec<Opener> {
        let Some(key) = path.file_name().map(|n| n.to_string_lossy().to_lowercase()) else { return Vec::new() };
        let mut out: Vec<Opener> = t.open_here.get(&key).cloned().unwrap_or_default();
        let me = crate::marks::who();
        for m in t.open_notes.get(&key).into_iter().flatten() {
            // your own note, read back, says nothing new
            if m.who == me {
                continue;
            }
            out.push(Opener { who: m.who.clone(), app: m.app.clone(), since: Some(m.started), face: self.faces.get(&m.who).cloned(), me: false, from: tx("their Daleris") });
        }
        out.sort_by_key(|o| !o.me);
        let mut seen = HashSet::new();
        out.retain(|o| seen.insert(o.who.to_lowercase()));
        out
    }

    /// The pictures of who has it open, right-aligned against `right` on the row at height `cy`, `d` across; how much
    /// room they took. Pointing at them lists everyone.
    pub(super) fn draw_openers(&mut self, ui: &Ui, openers: &[Opener], right: f32, cy: f32, d: f32) -> f32 {
        if openers.is_empty() {
            return 0.0;
        }
        // three at most; past that, two and "+n"
        let shown = if openers.len() > 3 { 2 } else { openers.len() };
        let more = openers.len() - shown;
        let step = d * 0.7;
        let width = openers_width(openers.len(), d);
        let area = Rect::from_min_size(pos2(right - width, cy - d / 2.0), vec2(width, d));
        // right to left, so the first, you when it's you, lies on top
        if more > 0 {
            let c = pos2(area.left() + d / 2.0 + step * shown as f32, cy);
            ui.painter().circle_filled(c, d / 2.0 + 1.5, BG());
            ui.painter().circle_filled(c, d / 2.0, RAISED());
            ui.painter().text(c, Align2::CENTER_CENTER, format!("+{more}"), FontId::proportional(d * 0.45), TEXT());
        }
        for (k, o) in openers.iter().take(shown).enumerate().rev() {
            let c = pos2(area.left() + d / 2.0 + step * k as f32, cy);
            self.draw_face(ui, o, c, d);
        }
        // shown whenever the pointer is over them: the row under them takes the pointer for clicks and drags
        if ui.rect_contains_pointer(area) {
            let faces: Vec<Option<egui::TextureId>> = openers.iter().map(|o| self.face_tex(o)).collect();
            let id = ui.scope_id().with(("openers", right as i64, cy as i64));
            egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), id, egui::PopupAnchor::Pointer).gap(12.0).show(|ui| openers_tip(ui, openers, &faces));
        }
        width
    }

    /// One person's round picture, or their initials on a colour of their own.
    fn draw_face(&mut self, ui: &Ui, o: &Opener, c: egui::Pos2, d: f32) {
        let tex = self.face_tex(o);
        let p = ui.painter();
        p.circle_filled(c, d / 2.0 + 1.5, BG());
        match tex {
            Some(tex) => {
                // the square picture, rounded off by a ring the colour of the list around it
                let r = Rect::from_center_size(c, vec2(d + 2.0, d + 2.0));
                p.add(egui::Shape::image(tex, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE));
                p.circle_stroke(c, d / 2.0 + 2.0, Stroke::new(3.0_f32, BG()));
            }
            None => {
                // your own initials, as other people see them
                let name = if o.me { crate::marks::who() } else { o.who.clone() };
                p.circle_filled(c, d / 2.0, person_colour(&name));
                p.text(c, Align2::CENTER_CENTER, crate::presence::initials(&name), FontId::proportional(d * 0.42), Color32::WHITE);
            }
        }
        if o.me {
            p.circle_stroke(c, d / 2.0 + 0.5, Stroke::new(1.0_f32, ACCENT()));
        }
    }

    /// A picture as a texture, made once per person.
    fn face_tex(&mut self, o: &Opener) -> Option<egui::TextureId> {
        let face = o.face.as_ref()?;
        let key = format!("face:{}", o.who);
        if let Some(Slot::Ready(t)) = self.icons.get(&key) {
            return Some(t.id());
        }
        let img = egui::ColorImage::from_rgba_unmultiplied([face.size, face.size], &face.rgba);
        let tex = self.ctx.load_texture(&key, img, egui::TextureOptions::LINEAR);
        let id = tex.id();
        self.icons.insert(key, Slot::Ready(tex));
        Some(id)
    }
}

/// A colour for someone without a picture, the same every time for the same name.
fn person_colour(who: &str) -> Color32 {
    let h = who.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ b as u32);
    let (r, g, b) = [(0x3b, 0x82, 0xc4), (0x2e, 0x9e, 0x8f), (0x8e, 0x5c, 0xc8), (0xc2, 0x4f, 0x8a), (0xc4, 0x78, 0x2a), (0x4f, 0x8f, 0x3a)][(h % 6) as usize];
    Color32::from_rgb(r, g, b)
}

/// The tooltip: how many have it open, then each with their picture, the program, since when and how it's known.
fn openers_tip(ui: &mut Ui, list: &[Opener], faces: &[Option<egui::TextureId>]) {
    ui.set_max_width(340.0);
    let title = match list.len() {
        1 if list[0].me => tr("Open on this PC").to_string(),
        1 => tr("Open by someone").to_string(),
        n => trf!("Open by {n} people", n = fmt_count(n)),
    };
    ui.label(egui::RichText::new(title).strong());
    ui.add_space(6.0);
    for (o, tex) in list.iter().zip(faces) {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
            match tex {
                Some(t) => {
                    ui.painter().add(egui::Shape::image(*t, r.expand(1.0), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE));
                    ui.painter().circle_stroke(r.center(), 16.0, Stroke::new(3.0_f32, ui.visuals().window_fill));
                }
                None => {
                    let name = if o.me { crate::marks::who() } else { o.who.clone() };
                    ui.painter().circle_filled(r.center(), 14.0, person_colour(&name));
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, crate::presence::initials(&name), FontId::proportional(12.0), Color32::WHITE);
                }
            }
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(&o.who).color(TEXT()));
                let ago = o.since.map(|s| super::ops::ago(Duration::from_secs(crate::marks::now().saturating_sub(s))));
                let source = tr(o.from);
                let line = match (&o.app, ago) {
                    (Some(app), Some(ago)) => trf!("in {app} · opened {ago} · known from {source}", app = app, ago = ago, source = source),
                    (Some(app), None) => trf!("in {app} · known from {source}", app = app, source = source),
                    (None, Some(ago)) => trf!("opened {ago} · known from {source}", ago = ago, source = source),
                    (None, None) => trf!("known from {source}", source = source),
                };
                ui.label(egui::RichText::new(line).small().color(DIM()));
            });
        });
        ui.add_space(2.0);
    }
}

/// How wide the pictures of `n` people are, `d` across: three at most, overlapping.
pub(super) fn openers_width(n: usize, d: f32) -> f32 {
    let slots = n.min(3);
    if slots == 0 { 0.0 } else { d + d * 0.7 * (slots - 1) as f32 }
}
