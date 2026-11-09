//! The navigation sidebar: built-in sections (Quick access, Cloud drives, This PC,
//! Network) and the person's own sections. Every section and item can be hidden,
//! reordered (drag or right-click) and folders can be pinned by dragging them in.

use super::*;

pub(super) struct SideItem {
    pub key: String,
    pub label: String,
    /// None = an action row (Map network drive…)
    pub loc: Option<Loc>,
    pub glyph: Option<&'static str>,
    pub usage: Option<f32>,
    pub visible: bool,
    pub pinned: bool,
    pub net: Option<char>,
    pub tip: Option<String>,
}

pub(super) struct SideSection {
    pub id: String,
    pub title: String,
    pub custom: bool,
    pub visible: bool,
    pub collapsed: bool,
    pub accepts_pins: bool,
    pub items: Vec<SideItem>,
}

/// Payload when dragging a sidebar entry to reorder it.
#[derive(Clone)]
struct SideDrag {
    section: String,
    key: String,
}

/// Hover text for a drive: where it comes from, and whether it's a FUSE mount.
fn drive_tip(d: &Drive) -> Option<String> {
    if d.locked {
        return Some(tr("Locked by BitLocker. Click to unlock it").into());
    }
    let fuse = d.is_fuse().then(|| trf!("FUSE mount ({fs})", fs = d.fs));
    match (d.remote.clone(), fuse) {
        (Some(r), Some(f)) => Some(format!("{r}\n{f}")),
        (r, f) => r.or(f),
    }
}

pub(super) fn side_key(p: &Path) -> String {
    p.to_string_lossy().trim_end_matches('\\').to_lowercase()
}

/// Reorder `items` so the ones named in `order` follow that order, while items
/// the order doesn't mention keep their natural place.
pub(super) fn apply_order<T>(items: &mut Vec<T>, order: &[String], key: impl Fn(&T) -> &str) {
    if order.is_empty() {
        return;
    }
    let rank = |k: &str| order.iter().position(|o| o == k);
    let slots: Vec<usize> = (0..items.len()).filter(|&i| rank(key(&items[i])).is_some()).collect();
    let mut taken: Vec<(usize, T)> = Vec::new();
    for &i in slots.iter().rev() {
        let item = items.remove(i);
        taken.push((rank(key(&item)).unwrap(), item));
    }
    taken.sort_by_key(|(r, _)| *r);
    for (slot, (_, item)) in slots.into_iter().zip(taken) {
        items.insert(slot, item);
    }
}

impl App {
    pub(super) fn sidebar_model(&self) -> Vec<SideSection> {
        let mut out = Vec::new();
        for sec in &self.settings.sections {
            let mut items: Vec<SideItem> = Vec::new();
            let mut push = |key: String, label: String, loc: Option<Loc>, glyph, usage, pinned, net, tip| {
                items.push(SideItem { visible: !self.settings.side_hidden.contains(&format!("{}|{key}", sec.id)), key, label, loc, glyph, usage, pinned, net, tip });
            };
            match sec.id.as_str() {
                "quick" => {
                    for p in &self.places {
                        push(side_key(&p.path), tr_dyn(&p.name), Some(Loc::Dir(p.path.clone())), None, None, false, None, None);
                    }
                }
                "cloud" => {
                    for c in &self.clouds {
                        push(side_key(&c.path), tr_dyn(&c.name), Some(Loc::Dir(c.path.clone())), None, None, false, None, None);
                    }
                }
                "pc" => {
                    push("thispc".into(), tr("This PC").into(), Some(Loc::ThisPc), Some(I_PC), None, false, None, None);
                    for d in self.drives.iter().filter(|d| d.kind != DriveKind::Network) {
                        let usage = (d.ready && d.total > 0).then(|| 1.0 - d.free as f32 / d.total.max(1) as f32);
                        push(side_key(&d.root), d.title(), Some(Loc::Dir(d.root.clone())), None, usage, false, None, drive_tip(d));
                    }
                }
                "network" => {
                    for d in self.drives.iter().filter(|d| d.kind == DriveKind::Network) {
                        let tip = if self.drive_offline(d) {
                            Some(trf!("Not available right now. Click to try again.\n{remote}", remote = d.remote.clone().unwrap_or_default()))
                        } else {
                            drive_tip(d)
                        };
                        push(side_key(&d.root), d.title(), Some(Loc::Dir(d.root.clone())), None, None, false, Some(d.letter), tip);
                    }
                }
                _ => {}
            }
            for p in &sec.pins {
                let name = Loc::Dir(p.clone()).title();
                if self.settings.hides_dotfiles() && fsx::is_dot_name(&name) {
                    continue;
                }
                push(side_key(p), name, Some(Loc::Dir(p.clone())), None, None, true, None, Some(p.display().to_string()));
            }
            if let Some(order) = self.settings.side_order.get(&sec.id) {
                apply_order(&mut items, order, |i| i.key.as_str());
            }
            out.push(SideSection {
                id: sec.id.clone(),
                title: sec.title.clone(),
                custom: !sec.builtin(),
                visible: sec.visible && !policy::current().section_hidden(&sec.id),
                collapsed: sec.collapsed,
                accepts_pins: sec.accepts_pins(),
                items,
            });
        }
        out
    }

    /// Move `key` within a section: before `target`, or by `delta` places.
    pub(super) fn move_side_item(&mut self, section: &str, key: &str, target: Option<&str>, delta: i32) {
        if policy::current().sidebar_locked() {
            return;
        }
        let model = self.sidebar_model();
        let Some(sec) = model.iter().find(|s| s.id == section) else { return };
        let mut keys: Vec<String> = sec.items.iter().map(|i| i.key.clone()).collect();
        let Some(from) = keys.iter().position(|k| k == key) else { return };
        let k = keys.remove(from);
        let to = match target {
            Some(t) => keys.iter().position(|x| x == t).unwrap_or(keys.len()),
            None => (from as i32 + delta).clamp(0, keys.len() as i32) as usize,
        };
        keys.insert(to, k);
        self.settings.side_order.insert(section.to_string(), keys);
        self.save_settings();
    }

    pub(super) fn move_section(&mut self, id: &str, delta: i32) {
        if policy::current().sidebar_locked() {
            return;
        }
        let secs = &mut self.settings.sections;
        if let Some(i) = secs.iter().position(|s| s.id == id) {
            let j = (i as i32 + delta).clamp(0, secs.len() as i32 - 1) as usize;
            let s = secs.remove(i);
            secs.insert(j, s);
            self.save_settings();
        }
    }

    pub(super) fn toggle_side_item(&mut self, section: &str, key: &str) {
        if policy::current().sidebar_locked() {
            return;
        }
        let k = format!("{section}|{key}");
        if !self.settings.side_hidden.remove(&k) {
            self.settings.side_hidden.insert(k);
        }
        self.save_settings();
    }

    pub(super) fn new_section(&mut self, with: Option<PathBuf>) -> String {
        let id = format!("custom{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));
        let title = tr("New section").to_string();
        self.settings.sections.push(SectionCfg { id: id.clone(), title: title.clone(), visible: true, collapsed: false, pins: with.into_iter().collect() });
        self.renaming_section = Some((id.clone(), title));
        self.save_settings();
        id
    }

    pub(super) fn pin_to(&mut self, section: &str, p: PathBuf) {
        if policy::current().sidebar_locked() {
            return;
        }
        if let Some(sec) = self.settings.section_mut(section) {
            if !sec.pins.iter().any(|x| fsx::same_path(x, &p)) {
                sec.pins.push(p);
            }
            sec.visible = true;
        }
        self.save_settings();
    }

    pub(super) fn sidebar(&mut self, root: &mut Ui, t: &Tab) {
        let frame = egui::Frame::new().fill(SIDE()).inner_margin(egui::Margin { left: 8, right: 6, top: 2, bottom: 8 });
        egui::Panel::left("sidebar").resizable(true).default_size(if self.picker.is_some() { 190.0 } else { 236.0 }).size_range(140.0..=420.0).frame(frame).show_separator_line(pal().explorer).show(root, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                let cur = t.loc.clone();
                let model = self.sidebar_model();
                let visible: Vec<&SideSection> = model.iter().filter(|s| s.visible).collect();
                let n = visible.len();
                for (si, sec) in visible.into_iter().enumerate() {
                    if sec.items.iter().all(|i| !i.visible) && !sec.custom && !sec.accepts_pins && sec.id != "network" {
                        continue; // e.g. no cloud drives on this PC
                    }
                    self.side_header(ui, sec, si, n);
                    if sec.collapsed {
                        continue;
                    }
                    let shown: Vec<&SideItem> = sec.items.iter().filter(|i| i.visible).collect();
                    if shown.is_empty() && sec.accepts_pins {
                        ui.label(egui::RichText::new(format!("   {}", tr("Drag folders here to pin them"))).size(11.5).color(FAINT()));
                    }
                    let count = shown.len();
                    for (ii, item) in shown.into_iter().enumerate() {
                        self.side_row(ui, sec, item, &cur, ii, count);
                    }
                }
            });
        });
    }

    fn side_header(&mut self, ui: &mut Ui, sec: &SideSection, index: usize, count: usize) {
        ui.add_space(8.0);
        if let Some((id, buf)) = &mut self.renaming_section {
            if *id == sec.id {
                let r = ui.add(egui::TextEdit::singleline(buf).desired_width(ui.available_width() - 8.0).font(FontId::proportional(12.0)));
                if !r.has_focus() && !r.lost_focus() {
                    r.request_focus();
                }
                if r.lost_focus() {
                    let (id, title) = self.renaming_section.take().unwrap();
                    let cancel = ui.input(|i| i.key_pressed(Key::Escape));
                    if !cancel && !title.trim().is_empty() {
                        if let Some(s) = self.settings.section_mut(&id) {
                            s.title = title.trim().to_string();
                        }
                        self.save_settings();
                    }
                }
                return;
            }
        }
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
        let drop_hover = sec.accepts_pins && resp.dnd_hover_payload::<DragFiles>().is_some();
        if drop_hover {
            ui.painter().rect_stroke(rect, CornerRadius::same(4), Stroke::new(1.0_f32, ACCENT()), StrokeKind::Inside);
        }
        let chev = if sec.collapsed { I_CHEV_R } else { I_CHEV_D };
        let col = if resp.hovered() { DIM() } else { FAINT() };
        ui.painter().text(rect.left_center() + vec2(7.0, 0.0), Align2::CENTER_CENTER, ic(chev), icon_font(8.0), col);
        ui.painter().text(rect.left_center() + vec2(16.0, 0.0), Align2::LEFT_CENTER, tr_dyn(&sec.title), FontId::proportional(11.5), col);
        if resp.clicked() {
            if let Some(s) = self.settings.section_mut(&sec.id) {
                s.collapsed = !s.collapsed;
            }
            self.save_settings();
        }
        if sec.custom && resp.double_clicked() && !policy::current().sidebar_locked() {
            self.renaming_section = Some((sec.id.clone(), sec.title.clone()));
        }
        if sec.accepts_pins {
            if let Some(drag) = resp.dnd_release_payload::<DragFiles>() {
                for p in drag.0.iter().filter(|p| p.is_dir()) {
                    self.pin_to(&sec.id, p.clone());
                }
            }
        }
        let id = sec.id.clone();
        let (custom, title) = (sec.custom, sec.title.clone());
        let locked = policy::current().sidebar_locked();
        // mapping a drive lives on the section, not as an entry in it
        let network = id == "network";
        let resp = if network { resp.on_hover_text(tr("Right-click to map a network drive")) } else { resp };
        resp.context_menu(|ui| {
            if network {
                if menu_item(ui, I_NETDRIVE, network_label(), "") {
                    self.acts.push(Act::MapDrive);
                }
                ui.separator();
            }
            if menu_item(ui, if sec.collapsed { I_CHEV_D } else { I_CHEV_R }, if sec.collapsed { tr("Expand") } else { tr("Collapse") }, "") {
                if let Some(s) = self.settings.section_mut(&id) {
                    s.collapsed = !s.collapsed;
                }
                self.save_settings();
            }
            // a sidebar managed by the organization only opens and closes
            if locked {
                return;
            }
            if custom && menu_item(ui, I_RENAME, tr("Rename section"), "") {
                self.renaming_section = Some((id.clone(), title.clone()));
            }
            if index > 0 && menu_item(ui, I_CHEV_U, tr("Move section up"), "") {
                self.move_section(&id, -1);
            }
            if index + 1 < count && menu_item(ui, I_CHEV_D, tr("Move section down"), "") {
                self.move_section(&id, 1);
            }
            if menu_item(ui, I_HIDE, tr("Hide section"), "") {
                if let Some(s) = self.settings.section_mut(&id) {
                    s.visible = false;
                }
                self.save_settings();
                self.toast(tr("Section hidden. Show it again in Settings > Sidebar"), false);
            }
            if custom && menu_item(ui, I_DELETE, tr("Delete section"), "") {
                self.settings.sections.retain(|s| s.id != id);
                self.save_settings();
            }
            ui.separator();
            if menu_item(ui, I_ADD, tr("New section"), "") {
                self.new_section(None);
            }
            if menu_item(ui, I_SETTINGS, tr("Customize sidebar…"), "") {
                self.acts.push(Act::OpenSettings(Some(settings_page::Page::Sidebar)));
            }
        });
    }

    fn side_row(&mut self, ui: &mut Ui, sec: &SideSection, item: &SideItem, cur: &Loc, index: usize, count: usize) {
        // a drive that's just come and is still being looked at: a place for its bar, so nothing jumps when it's known
        let loading = matches!(&item.loc, Some(Loc::Dir(p)) if self.drives.iter().any(|d| !d.checked && d.kind != DriveKind::Network && fsx::same_path(&d.root, p)));
        let h = if item.usage.is_some() || loading { 38.0 } else { 28.0 };
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click_and_drag());
        let active = item.loc.as_ref() == Some(cur);
        let files_hover = item.loc.as_ref().is_some_and(|l| matches!(l, Loc::Dir(_))) && resp.dnd_hover_payload::<DragFiles>().is_some();
        let reorder_hover = resp.dnd_hover_payload::<SideDrag>().is_some_and(|d| d.section == sec.id && d.key != item.key);
        if active {
            ui.painter().rect_filled(rect, CornerRadius::same(5), ROW_SEL());
            // Explorer marks the open place by its colour alone
            if !pal().explorer {
                ui.painter().rect_filled(Rect::from_min_size(rect.left_top() + vec2(0.0, 7.0), vec2(3.0, 14.0)), CornerRadius::same(2), ACCENT());
            }
        } else if resp.hovered() || files_hover {
            ui.painter().rect_filled(rect, CornerRadius::same(5), ROW_HOVER());
        }
        if let (true, Some(Loc::Dir(d))) = (files_hover, &item.loc) {
            let ok = self.drop_look(ui.ctx(), d);
            list::drop_mark(ui.painter(), rect, 5, ok);
        }
        if reorder_hover {
            ui.painter().line_segment([rect.left_top(), rect.right_top()], Stroke::new(2.0_f32, ACCENT()));
        }
        let ic_c = pos2(rect.left() + 18.0, rect.top() + 14.0);
        match (&item.loc, item.glyph) {
            // This PC with Windows's icon when Windows icons are in use
            (Some(Loc::ThisPc), Some(_)) if self.draw_this_pc(ui, Rect::from_center_size(ic_c, vec2(18.0, 18.0))) => {}
            (_, Some(g)) => {
                ui.painter().text(ic_c, Align2::CENTER_CENTER, ic(g), icon_font(14.0), ACCENT());
            }
            (Some(Loc::Dir(p)), None) => {
                let p = p.clone();
                self.draw_icon(ui, Rect::from_center_size(ic_c, vec2(18.0, 18.0)), &p, "", true, fsx::A_READONLY);
            }
            _ => {}
        }
        let offline = item.net.is_some_and(|l| self.drives.iter().any(|d| d.letter == l && self.drive_offline(d)));
        let col = if offline {
            FAINT()
        } else if active {
            TEXT()
        } else {
            TEXT().gamma_multiply(0.88)
        };
        if offline {
            // over the icon: this one isn't there at the moment
            let at = ic_c + vec2(6.0, 6.0);
            ui.painter().circle_filled(at, 4.5, PANEL());
            ui.painter().circle_filled(at, 3.0, WARN());
        }
        let gal = truncated(ui, &item.label, FontId::proportional(13.5), col, rect.width() - 44.0);
        ui.painter().galley(pos2(rect.left() + 36.0, ic_c.y - gal.size().y / 2.0), gal, col);
        if loading {
            chrome::shimmer(ui, Rect::from_min_size(pos2(rect.left() + 36.0, rect.top() + 27.0), vec2(rect.width() - 48.0, 4.0)));
        }
        if let Some(u) = item.usage {
            let bar = Rect::from_min_size(pos2(rect.left() + 36.0, rect.top() + 27.0), vec2(rect.width() - 48.0, 4.0));
            ui.painter().rect_filled(bar, CornerRadius::same(2), LINE());
            let fill = Rect::from_min_size(bar.min, vec2(bar.width() * u.clamp(0.0, 1.0), 4.0));
            ui.painter().rect_filled(fill, CornerRadius::same(2), if u > 0.9 { WARN() } else { ACCENT() });
        }
        if resp.clicked() {
            self.acts.push(match &item.loc {
                Some(l) => Act::Navigate(l.clone()),
                None => Act::MapDrive,
            });
        }
        if let (true, Some(l)) = (resp.middle_clicked(), &item.loc) {
            self.acts.push(Act::NavigateNewTab(l.clone()));
        }
        // drag to reorder within the section; drop files to move/copy them into the folder
        if resp.drag_started() {
            resp.dnd_set_drag_payload(SideDrag { section: sec.id.clone(), key: item.key.clone() });
        }
        // egui hands over whatever is being dragged to the first one asking, so only ask for what it is
        let reordering = egui::DragAndDrop::has_payload_of_type::<SideDrag>(ui.ctx());
        if let Some(d) = if reordering { resp.dnd_release_payload::<SideDrag>() } else { None } {
            if d.section == sec.id {
                self.move_side_item(&sec.id, &d.key, Some(&item.key), 0);
            }
        }
        if let Some(Loc::Dir(dir)) = &item.loc {
            self.drop_zone(ui, &resp, dir);
        }
        let resp = match &item.tip {
            Some(tip) => resp.on_hover_text(tip),
            None => resp,
        };
        let (sid, key, loc, pinned, net) = (sec.id.clone(), item.key.clone(), item.loc.clone(), item.pinned, item.net);
        let pin_sections: Vec<(String, String)> = self.settings.sections.iter().filter(|s| s.accepts_pins() && s.id != sid).map(|s| (s.id.clone(), s.title.clone())).collect();
        let locked = policy::current().sidebar_locked();
        resp.context_menu(|ui| {
            if let Some(l) = &loc {
                if menu_item(ui, I_ADD, tr("Open in new tab"), "") {
                    self.acts.push(Act::NavigateNewTab(l.clone()));
                }
            }
            if locked {
                // a sidebar managed by the organization can't be rearranged
            } else if pinned {
                if menu_item(ui, I_PIN, tr("Unpin"), "") {
                    if let Some(s) = self.settings.section_mut(&sid) {
                        s.pins.retain(|p| side_key(p) != key);
                    }
                    self.save_settings();
                }
            } else if let Some(Loc::Dir(p)) = &loc {
                if !pin_sections.is_empty() {
                    ui.menu_button(format!("      {}", tr("Pin to")), |ui| {
                        for (id, title) in &pin_sections {
                            if menu_item(ui, I_PIN, &tr_dyn(title), "") {
                                self.pin_to(id, p.clone());
                            }
                        }
                    });
                }
            }
            if !locked && index > 0 && menu_item(ui, I_CHEV_U, tr("Move up"), "") {
                self.move_side_item(&sid, &key, None, -1);
            }
            if !locked && index + 1 < count && menu_item(ui, I_CHEV_D, tr("Move down"), "") {
                self.move_side_item(&sid, &key, None, 1);
            }
            if !locked && menu_item(ui, I_HIDE, tr("Hide from sidebar"), "") {
                self.toggle_side_item(&sid, &key);
                self.toast(tr("Hidden. Show it again in Settings > Sidebar"), false);
            }
            if let Some(l) = net {
                if menu_item(ui, I_CLOSE, tr("Disconnect"), "") {
                    self.acts.push(Act::Disconnect(l));
                }
            }
            if let Some(Loc::Dir(p)) = &loc {
                if menu_item(ui, I_INFO, tr("Properties"), "") {
                    self.acts.push(Act::Properties(p.clone()));
                }
            }
            ui.separator();
            if menu_item(ui, I_SETTINGS, tr("Customize sidebar…"), "") {
                self.acts.push(Act::OpenSettings(Some(settings_page::Page::Sidebar)));
            }
        });
    }
}
