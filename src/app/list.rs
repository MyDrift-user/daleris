//! The file list (virtualised details view and icon grid), item and background
//! context menus, drag & drop, and the preview pane.

use super::*;

/// One column of the details view.
#[derive(Clone)]
pub(super) struct ColSpec {
    id: &'static str,
    label: &'static str,
    sort: Option<SortKey>,
    w: f32,
}

/// Space left of the first column, and right of the last.
const COL_LEFT: f32 = 12.0;
const COL_RIGHT: f32 = 16.0;

impl App {
    /// The columns and their widths: what you set by dragging, otherwise sensible defaults.
    /// Name fills the space that's left; Type shows every type name in full when there is
    /// room for that, and otherwise is as wide as the average type name.
    fn columns(&self, ui: &Ui, t: &Tab, full: f32) -> Vec<ColSpec> {
        let search = t.searching();
        let user = |id: &str| self.settings.col_w.get(id).copied();
        let status_w = if t.in_cloud || (search && !self.clouds.is_empty()) { 60.0 } else { 0.0 };
        let modified = user("modified").unwrap_or(140.0);
        let size = user("size").unwrap_or(90.0);
        let mut fixed = modified + size + status_w;
        let ty = if search {
            0.0
        } else {
            let w = user("type").unwrap_or_else(|| {
                let (max_w, avg_w) = self.type_widths(ui, t);
                let room = full - COL_LEFT - COL_RIGHT - fixed - 260.0; // keep ~260 px for names
                if room >= max_w { max_w } else { avg_w.max(70.0) }
            });
            fixed += w;
            w
        };
        let folder = if search { user("folder").unwrap_or(((full - COL_LEFT - COL_RIGHT - fixed) * 0.42).max(120.0)) } else { 0.0 };
        let name = user("name").unwrap_or((full - COL_LEFT - COL_RIGHT - fixed - folder).max(200.0));

        let mut v = vec![ColSpec { id: "name", label: "Name", sort: Some(SortKey::Name), w: name }];
        if search {
            v.push(ColSpec { id: "folder", label: "Folder", sort: None, w: folder });
        }
        if status_w > 0.0 {
            v.push(ColSpec { id: "status", label: "Status", sort: None, w: status_w });
        }
        v.push(ColSpec { id: "modified", label: "Date modified", sort: Some(SortKey::Modified), w: modified });
        if !search {
            v.push(ColSpec { id: "type", label: "Type", sort: Some(SortKey::Type), w: ty });
        }
        v.push(ColSpec { id: "size", label: "Size", sort: Some(SortKey::Size), w: size });
        v
    }

    /// (widest, average) width the Type column needs for this folder, padding included.
    fn type_widths(&self, ui: &Ui, t: &Tab) -> (f32, f32) {
        let (mut max_w, mut sum, mut count) = (0.0f32, 0.0f32, 0usize);
        for (ext, is_dir, n) in &t.type_stats {
            let w = text_w(ui, &self.type_name(ext, *is_dir), 12.5) + 24.0;
            max_w = max_w.max(w);
            sum += w * *n as f32;
            count += n;
        }
        if count == 0 {
            return (150.0, 150.0);
        }
        (max_w.min(420.0), (sum / count as f32).min(420.0))
    }

    /// The width that shows every entry of column `id` in full (double-click on its right edge).
    fn fit_width(&self, ui: &Ui, t: &Tab, id: &str) -> f32 {
        let n = self.row_count(t);
        // long lists: measure a large sample (the longest names are found by length first)
        let rows = self.rows(t, 0..n.min(if t.searching() { 5000 } else { 200_000 }));
        let widest = |mut texts: Vec<String>, size: f32| -> f32 {
            texts.sort_by_key(|s| std::cmp::Reverse(s.chars().count()));
            texts.truncate(300);
            texts.iter().map(|s| text_w(ui, s, size)).fold(0.0, f32::max)
        };
        match id {
            "name" => 40.0 + widest(rows.iter().map(|r| display_name(r, self.settings.show_ext)).collect(), 13.5) + 8.0,
            "folder" => 8.0 + widest(rows.iter().filter_map(|r| r.folder.clone()).collect(), 12.5) + 18.0,
            "modified" => widest(rows.iter().map(|r| fmt_filetime(r.mtime)).collect(), 12.5) + 24.0,
            "type" => widest(rows.iter().map(|r| self.type_name(&r.ext, r.is_dir)).collect(), 12.5) + 24.0,
            "size" => {
                let sizes = rows
                    .iter()
                    .map(|r| if r.is_dir { self.sizer.known(&r.path).map(fmt_size).unwrap_or_default() } else { fmt_size(r.size) })
                    .collect();
                widest(sizes, 12.5) + 26.0
            }
            _ => 60.0,
        }
        .max(48.0)
    }

    /// The column titles. Click a title to sort; drag the line at a column's right edge to
    /// resize it, or double-click that line to fit the column to its longest entry.
    fn header(&mut self, ui: &mut Ui, t: &mut Tab, cols: &[ColSpec], hoff: f32) {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
        let painter = ui.painter_at(rect);
        painter.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0_f32, LINE()));
        let mut x = rect.left() + COL_LEFT - hoff;
        let mut edges = Vec::new();
        // a press on (or right next to) a column edge resizes; it must never also sort
        let edge_xs: Vec<f32> = cols.iter().scan(x, |ex, c| { *ex += c.w; Some(*ex) }).collect();
        let near_edge = ui.input(|i| i.pointer.press_origin().or(i.pointer.interact_pos())).is_some_and(|p| edge_xs.iter().any(|ex| (p.x - ex).abs() <= 7.0));
        for (i, c) in cols.iter().enumerate() {
            let r = Rect::from_min_size(pos2(x, rect.top()), vec2(c.w, rect.height()));
            let visible = r.shrink2(vec2(6.0, 0.0)).intersect(rect);
            if visible.width() > 1.0 {
                let resp = ui.interact(visible, ui.id().with(("hdr", c.id)), if c.sort.is_some() { Sense::click() } else { Sense::hover() });
                if resp.hovered() && c.sort.is_some() {
                    painter.rect_filled(r.shrink(2.0), CornerRadius::same(4), HOVER());
                }
                let label = truncated(ui, c.label, FontId::proportional(12.5), DIM(), c.w - 24.0);
                painter.galley(r.left_center() + vec2(8.0, -label.size().y / 2.0), label, DIM());
                if let Some(k) = c.sort {
                    if t.sort == k {
                        let arrow = if t.asc { "▲" } else { "▼" };
                        painter.text(r.right_center() - vec2(10.0, 0.0), Align2::RIGHT_CENTER, arrow, FontId::proportional(8.0), DIM());
                    }
                    if resp.clicked() && !near_edge {
                        if t.sort == k {
                            t.asc = !t.asc;
                        } else {
                            t.sort = k;
                            t.asc = k == SortKey::Name || k == SortKey::Type;
                        }
                        t.dirty = true;
                        self.sort_search(t);
                    }
                }
            }
            if i > 0 {
                painter.line_segment([pos2(x, rect.top() + 8.0), pos2(x, rect.bottom() - 8.0)], Stroke::new(1.0_f32, LINE()));
            }
            x += c.w;
            edges.push((c.id, c.w, x));
        }
        // resize handles on each column's right edge (registered last, so they win over the titles)
        for (id, w, ex) in edges {
            let h = Rect::from_center_size(pos2(ex, rect.center().y), vec2(10.0, rect.height()));
            if !rect.contains(h.center()) {
                continue;
            }
            let resp = ui.interact(h, ui.id().with(("coledge", id)), Sense::click_and_drag());
            let active = resp.hovered() || resp.dragged();
            if active {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeColumn);
                painter.line_segment([pos2(ex, rect.top() + 4.0), pos2(ex, rect.bottom() - 4.0)], Stroke::new(2.0_f32, ACCENT()));
            }
            let min = if id == "name" { 90.0 } else { 40.0 };
            if resp.dragged() {
                let nw = (w + resp.drag_delta().x).max(min);
                self.settings.col_w.insert(id.to_string(), nw);
            }
            if resp.drag_stopped() {
                self.save_settings();
            }
            if resp.double_clicked() {
                let fit = self.fit_width(ui, t, id);
                self.settings.col_w.insert(id.to_string(), fit);
                self.save_settings();
            }
            let tip = format!("Drag to resize · double-click to fit {}", if id == "name" { "the longest name" } else { "the longest entry" });
            let _ = resp.on_hover_text(tip);
        }
    }

    fn empty_or_error(&self, ui: &mut Ui, t: &Tab) -> bool {
        if let Some(e) = &t.error {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new("This folder can't be opened").size(16.0));
                ui.label(egui::RichText::new(e).color(DIM()));
            });
            return true;
        }
        let busy = t.loading || t.search.as_ref().is_some_and(|s| s.busy());
        if self.row_count(t) == 0 && !busy {
            let msg = if t.searching() || !t.query.is_empty() { "No items match your search." } else { "This folder is empty." };
            ui.add_space(40.0);
            ui.vertical_centered(|ui| ui.label(egui::RichText::new(msg).color(DIM())));
        }
        false
    }

    pub(super) fn details(&mut self, ui: &mut Ui, t: &mut Tab) {
        let full = ui.available_width();
        let cols = self.columns(ui, t, full);
        let total = COL_LEFT + cols.iter().map(|c| c.w).sum::<f32>() + COL_RIGHT;
        let content_w = total.max(full);
        // the header follows the list when it is scrolled sideways
        let hkey = egui::Id::new(("hscroll", t.id));
        let hoff = ui.ctx().memory(|m| m.data.get_temp::<f32>(hkey)).unwrap_or(0.0).clamp(0.0, (content_w - full).max(0.0));
        self.header(ui, t, &cols, hoff);
        if self.empty_or_error(ui, t) {
            return;
        }
        let bg = self.background_claim(ui, t);
        let n = self.row_count(t);
        let row_h = self.settings.row_h();
        // no drag-to-scroll: it would cover the empty space once the list can scroll, and swallow its clicks
        let mut sa = egui::ScrollArea::both().auto_shrink([false, false]).drag_to_scroll(false).id_salt(("list", t.id));
        let mem_id = egui::Id::new(("scroll", t.id));
        if let Some(i) = t.scroll_to.take() {
            let offset = ui.ctx().memory(|m| m.data.get_temp::<f32>(mem_id)).unwrap_or(0.0);
            let view_h = ui.available_height();
            let y = i as f32 * row_h;
            if i == 0 {
                sa = sa.vertical_scroll_offset(0.0);
            } else if y < offset {
                sa = sa.vertical_scroll_offset(y);
            } else if y + row_h > offset + view_h - 4.0 {
                sa = sa.vertical_scroll_offset(y + row_h - view_h + 4.0);
            }
        }
        let highlight: Vec<String> = t.search.as_ref().map(|s| s.highlight().to_vec()).unwrap_or_default();
        // one spare line at the end, so even a full folder has empty space to right-click
        let out = sa.show_rows(ui, row_h, n + 1, |ui, range| {
            let start = range.start.min(n);
            for (k, row) in self.rows(t, start..range.end.min(n)).into_iter().enumerate() {
                let i = start + k;
                let (rect, resp) = ui.allocate_exact_size(vec2(content_w, row_h), Sense::click_and_drag());
                let selected = t.sel.contains(&row.key);
                let drop_target = row.is_dir && resp.dnd_hover_payload::<DragFiles>().is_some();
                let inner = rect.shrink2(vec2(6.0, 1.0));
                let bg = if selected {
                    Some(if t.cursor == Some(i) { SEL_STRONG() } else { SEL() })
                } else if resp.hovered() || drop_target {
                    Some(HOVER())
                } else {
                    None
                };
                if let Some(c) = bg {
                    ui.painter().rect_filled(inner, CornerRadius::same(4), c);
                }
                if drop_target {
                    ui.painter().rect_stroke(inner, CornerRadius::same(4), Stroke::new(1.0_f32, ACCENT()), StrokeKind::Inside);
                }
                let cy = rect.center().y;
                let mut x = rect.left() + COL_LEFT;
                for c in &cols {
                    let w = c.w;
                    match c.id {
                        "name" => {
                            self.draw_icon(ui, Rect::from_center_size(pos2(x + 17.0, cy), vec2(18.0, 18.0)), &row.path, &row.ext, row.is_dir, row.attrs);
                            let tx = x + 36.0;
                            let name_col = if row.attrs & fsx::A_HIDDEN != 0 { DIM() } else { TEXT() };
                            let shown = display_name(&row, self.settings.show_ext);
                            if t.rename.as_ref().is_some_and(|r| r.key == row.key) {
                                let r = t.rename.as_mut().unwrap();
                                let er = Rect::from_min_size(pos2(tx - 4.0, rect.top() + 3.0), vec2((w - 40.0).max(120.0), row_h - 6.0));
                                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(er));
                                let te = child.add(egui::TextEdit::singleline(&mut r.buf).desired_width(er.width()).id_salt("rename"));
                                if r.focus {
                                    te.request_focus();
                                    // select the name without its extension, like Explorer
                                    let all = r.buf.chars().count();
                                    let stem = if row.is_dir { all } else { r.buf.rfind('.').map(|p| r.buf[..p].chars().count()).filter(|&s| s > 0).unwrap_or(all) };
                                    if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), te.id) {
                                        st.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(stem))));
                                        st.store(ui.ctx(), te.id);
                                    }
                                    r.focus = false;
                                }
                                if te.lost_focus() {
                                    if ui.input(|i| i.key_pressed(Key::Escape)) {
                                        t.rename = None;
                                    } else {
                                        self.acts.push(Act::CommitRename);
                                    }
                                }
                            } else {
                                let g = if highlight.is_empty() {
                                    truncated(ui, &shown, FontId::proportional(13.5), name_col, w - 44.0)
                                } else {
                                    highlighted(ui, &shown, &highlight, w - 44.0)
                                };
                                ui.painter().galley(pos2(tx, cy - g.size().y / 2.0), g, name_col);
                            }
                        }
                        "folder" => {
                            if let Some(f) = &row.folder {
                                let g = truncated(ui, f, FontId::proportional(12.5), DIM(), w - 16.0);
                                ui.painter().galley(pos2(x + 8.0, cy - g.size().y / 2.0), g, DIM());
                            }
                        }
                        "status" => {
                            let st = self.cloud_of(&row.path, row.attrs, t);
                            let c = pos2(x + w / 2.0 - 10.0, cy);
                            cloud_badge(ui.painter(), c, st);
                            if st != Cloud::None && ui.rect_contains_pointer(Rect::from_center_size(c, vec2(22.0, 22.0))) {
                                egui::show_tooltip_at_pointer(ui.ctx(), ui.layer_id(), ui.id().with(("cloud", i)), |ui| {
                                    ui.label(cloud_text(st));
                                });
                            }
                        }
                        id => {
                            let text = match id {
                                "modified" => fmt_filetime(row.mtime),
                                "type" => self.type_name(&row.ext, row.is_dir),
                                "size" if !row.is_dir => fmt_size(row.size),
                                // folders: the total of everything inside, counted in the background
                                "size" if self.settings.folder_sizes => match self.sizer.get(&row.path) {
                                    DirSize::Done(b) => fmt_size(b),
                                    DirSize::Counting(b) if b > 0 => format!("{}…", fmt_size(b)),
                                    _ => "…".into(),
                                },
                                _ => String::new(),
                            };
                            // a folder still being counted is shown fainter
                            let col = if id == "size" && row.is_dir && text.ends_with('…') { FAINT() } else { DIM() };
                            let g = truncated(ui, &text, FontId::proportional(12.5), col, w - 16.0);
                            let gx = if id == "size" { x + w - 12.0 - g.size().x } else { x + 8.0 };
                            ui.painter().galley(pos2(gx, cy - g.size().y / 2.0), g, col);
                        }
                    }
                    x += w;
                }
                self.row_interact(ui, t, &resp, &row, i);
            }
        });
        ui.ctx().memory_mut(|m| {
            m.data.insert_temp(mem_id, out.state.offset.y);
            m.data.insert_temp(hkey, out.state.offset.x);
        });
        self.background_area(ui, t, bg);
    }

    pub(super) fn grid(&mut self, ui: &mut Ui, t: &mut Tab) {
        if self.empty_or_error(ui, t) {
            return;
        }
        let bg = self.background_claim(ui, t);
        let full = ui.available_width();
        let (tile_w, tile_h) = (116.0, 132.0);
        let cols = ((full - 16.0) / tile_w).floor().max(1.0) as usize;
        let n = self.row_count(t);
        let lines = n.div_ceil(cols);
        let mut sa = egui::ScrollArea::vertical().auto_shrink([false, false]).drag_to_scroll(false).id_salt(("grid", t.id));
        let mem_id = egui::Id::new(("gscroll", t.id));
        if let Some(i) = t.scroll_to.take() {
            let offset = ui.ctx().memory(|m| m.data.get_temp::<f32>(mem_id)).unwrap_or(0.0);
            let y = (i / cols) as f32 * tile_h;
            let vh = ui.available_height();
            if y < offset {
                sa = sa.vertical_scroll_offset(y);
            } else if y + tile_h > offset + vh {
                sa = sa.vertical_scroll_offset(y + tile_h - vh);
            }
        }
        let out = sa.show_rows(ui, tile_h, lines, |ui, range| {
            for line in range {
                let rows = self.rows(t, line * cols..((line + 1) * cols).min(n));
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for (k, row) in rows.into_iter().enumerate() {
                        let i = line * cols + k;
                        let (rect, resp) = ui.allocate_exact_size(vec2(tile_w, tile_h), Sense::click_and_drag());
                        let inner = rect.shrink(4.0);
                        if t.sel.contains(&row.key) {
                            ui.painter().rect_filled(inner, CornerRadius::same(6), SEL());
                        } else if resp.hovered() || (row.is_dir && resp.dnd_hover_payload::<DragFiles>().is_some()) {
                            ui.painter().rect_filled(inner, CornerRadius::same(6), HOVER());
                        }
                        let img = Rect::from_min_size(inner.min + vec2(10.0, 6.0), vec2(inner.width() - 20.0, 76.0));
                        self.draw_thumb(ui, img, &row.path, &row.ext, row.is_dir, row.attrs, row.mtime);
                        cloud_badge(ui.painter(), pos2(img.left() + 6.0, img.bottom() - 6.0), self.cloud_of(&row.path, row.attrs, t));
                        let shown = display_name(&row, self.settings.show_ext);
                        let mut job = egui::text::LayoutJob::simple(shown, FontId::proportional(12.5), TEXT(), inner.width() - 8.0);
                        job.halign = egui::Align::Center;
                        job.wrap = egui::text::TextWrapping { max_width: inner.width() - 8.0, max_rows: 2, break_anywhere: true, overflow_character: Some('…') };
                        let g = ui.fonts(|f| f.layout_job(job));
                        ui.painter().galley(pos2(inner.center().x, img.bottom() + 6.0), g, TEXT());
                        self.row_interact(ui, t, &resp, &row, i);
                    }
                });
            }
        });
        ui.ctx().memory_mut(|m| m.data.insert_temp(mem_id, out.state.offset.y));
        if let Some(r) = t.rename.as_mut() {
            // the grid renames in a small floating box
            let (mut commit, mut cancel) = (false, false);
            egui::Window::new("Rename").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(ui.ctx(), |ui| {
                let te = ui.add(egui::TextEdit::singleline(&mut r.buf).desired_width(320.0));
                if r.focus {
                    te.request_focus();
                    r.focus = false;
                }
                if te.lost_focus() {
                    if ui.input(|i| i.key_pressed(Key::Escape)) {
                        cancel = true;
                    } else {
                        commit = true;
                    }
                }
            });
            if cancel {
                t.rename = None;
            } else if commit {
                self.acts.push(Act::CommitRename);
            }
        }
        self.background_area(ui, t, bg);
    }

    fn row_interact(&mut self, ui: &Ui, t: &mut Tab, resp: &Response, row: &Row, i: usize) {
        let mods = ui.input(|i| i.modifiers);
        if resp.clicked() {
            if mods.command {
                if !t.sel.remove(&row.key) {
                    t.sel.insert(row.key.clone());
                }
                t.anchor = Some(i);
            } else if mods.shift {
                let a = t.anchor.unwrap_or(i);
                let (lo, hi) = (a.min(i), a.max(i));
                t.sel = self.rows(t, lo..hi + 1).into_iter().map(|r| r.key).collect();
            } else {
                t.sel.clear();
                t.sel.insert(row.key.clone());
                t.anchor = Some(i);
            }
            t.cursor = Some(i);
            self.update_sel_info(t);
        }
        if resp.double_clicked() {
            self.acts.push(if row.is_dir { Act::Navigate(Loc::Dir(row.path.clone())) } else { Act::Open(row.path.clone()) });
        }
        if resp.middle_clicked() && row.is_dir {
            self.acts.push(Act::NavigateNewTab(Loc::Dir(row.path.clone())));
        }
        if resp.secondary_clicked() && !t.sel.contains(&row.key) {
            t.sel.clear();
            t.sel.insert(row.key.clone());
            t.cursor = Some(i);
            t.anchor = Some(i);
            self.update_sel_info(t);
        }
        // drag files onto folders, tabs, breadcrumbs or the sidebar
        if resp.drag_started() {
            if !t.sel.contains(&row.key) {
                t.sel.clear();
                t.sel.insert(row.key.clone());
                self.update_sel_info(t);
            }
            resp.dnd_set_drag_payload(DragFiles(self.selected_paths(t)));
        }
        if row.is_dir {
            if let Some(drag) = resp.dnd_release_payload::<DragFiles>() {
                self.acts.push(Act::Drop { srcs: drag.0.clone(), dest: row.path.clone(), copy: mods.ctrl });
            }
        }
        // right-click: load the Windows menu for the selection, then show ours with it merged in
        if resp.secondary_clicked() {
            let paths = self.selected_paths(t);
            let base = paths.first().and_then(|p| p.parent()).map(|p| p.to_path_buf()).or_else(|| t.dir().map(|d| d.to_path_buf()));
            if let Some(base) = base {
                let extended = ui.input(|i| i.modifiers.shift);
                self.open_shell_menu(&paths, &base, extended);
                let m = self.menu_ctx(t, Some(row));
                self.open_popup(m, false, (paths, base));
            }
        }
    }

    /// Claim the empty list area. Call this BEFORE the rows are added: egui gives a
    /// click to the last widget registered under the pointer, so the rows win.
    fn background_claim(&self, ui: &mut Ui, t: &Tab) -> Response {
        let rect = ui.available_rect_before_wrap();
        ui.interact(rect, ui.id().with(("bg", t.id)), Sense::click())
    }

    fn background_area(&mut self, ui: &mut Ui, t: &mut Tab, resp: Response) {
        // empty space: click clears the selection, right-click opens the folder menu
        if resp.clicked() && !ui.input(|i| i.modifiers.command || i.modifiers.shift) {
            t.sel.clear();
            self.update_sel_info(t);
        }
        let Some(dir) = t.dir().map(|d| d.to_path_buf()) else { return };
        if t.searching() {
            return;
        }
        if let Some(drag) = resp.dnd_release_payload::<DragFiles>() {
            self.acts.push(Act::Drop { srcs: drag.0.clone(), dest: dir.clone(), copy: ui.input(|i| i.modifiers.ctrl) });
        }
        if resp.secondary_clicked() {
            t.sel.clear();
            self.update_sel_info(t);
            let extended = ui.input(|i| i.modifiers.shift);
            self.open_shell_menu(&[], &dir, extended);
            let m = self.menu_ctx(t, None);
            self.open_popup(m, true, (Vec::new(), dir));
        }
    }

    pub(super) fn drag_overlay(&self, ctx: &egui::Context) {
        let (Some(p), Some(pos)) = (egui::DragAndDrop::payload::<DragFiles>(ctx), ctx.pointer_interact_pos()) else { return };
        let copy = ctx.input(|i| i.modifiers.ctrl);
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("dragtip")));
        let n = p.0.len();
        let text = format!("{} {} item{}", if copy { "Copy" } else { "Move" }, n, if n == 1 { "" } else { "s" });
        let g = painter.layout_no_wrap(text, FontId::proportional(13.0), TEXT());
        let r = Rect::from_min_size(pos + vec2(14.0, 14.0), g.size() + vec2(16.0, 10.0));
        painter.rect_filled(r, CornerRadius::same(5), SEL_STRONG());
        painter.galley(r.min + vec2(8.0, 5.0), g, TEXT());
    }

    pub(super) fn toast_overlay(&mut self, ctx: &egui::Context) {
        let Some((msg, at, err)) = &self.toast else { return };
        if at.elapsed() > Duration::from_secs(4) {
            self.toast = None;
            return;
        }
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("toast")));
        let screen = ctx.screen_rect();
        let g = painter.layout_no_wrap(msg.clone(), FontId::proportional(13.5), TEXT());
        let r = Rect::from_center_size(pos2(screen.center().x, screen.bottom() - 60.0), g.size() + vec2(28.0, 16.0));
        painter.rect_filled(r, CornerRadius::same(8), if *err { WARN().gamma_multiply(0.5) } else { RAISED() });
        painter.galley(r.min + vec2(14.0, 8.0), g, TEXT());
        ctx.request_repaint_after(Duration::from_millis(200));
    }

    // ------------------------------------------------------------------ preview

    pub(super) fn preview_panel(&mut self, ctx: &egui::Context, t: &Tab) {
        let paths = if t.sel.is_empty() { Vec::new() } else { self.selected_paths(t) };
        let target: Option<PathBuf> = if paths.len() == 1 { paths.first().cloned() } else { None };
        if target != self.preview.path {
            self.preview.gen += 1;
            self.preview.path = target.clone();
            self.preview.entry = target.as_ref().and_then(|p| fsx::stat(p));
            self.preview.content = PvContent::Nothing;
            if let (Some(p), Some(e)) = (&target, &self.preview.entry) {
                self.preview.content = PvContent::Loading;
                let (tx, gen, ctx2, p, e) = (self.tx.clone(), self.preview.gen, ctx.clone(), p.clone(), e.clone());
                std::thread::spawn(move || {
                    let content = load_preview(&p, &e);
                    let _ = tx.send(Msg::Preview { gen, content });
                    ctx2.request_repaint();
                });
            }
        }
        let frame = egui::Frame::new().fill(PANEL()).inner_margin(egui::Margin::same(14));
        egui::SidePanel::right("preview").resizable(true).default_width(300.0).width_range(220.0..=640.0).frame(frame).show_separator_line(false).show(ctx, |ui| {
            let Some(e) = self.preview.entry.clone() else {
                self.preview_summary(ui, t, paths.len());
                return;
            };
            let path = self.preview.path.clone().unwrap();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let w = ui.available_width();
                if !matches!(self.preview.content, PvContent::Text { .. }) {
                    let side = w.min(320.0);
                    let r = Rect::from_min_size(ui.cursor().min + vec2((w - side) / 2.0, 0.0), vec2(side, side * 0.78));
                    ui.painter().rect_filled(r, CornerRadius::same(8), pal().code_bg);
                    self.draw_thumb(ui, r.shrink(12.0), &path, &e.ext, e.is_dir, e.attrs, e.mtime);
                    ui.add_space(side * 0.78 + 12.0);
                }
                ui.add(egui::Label::new(egui::RichText::new(&e.name).size(16.0).strong()).wrap());
                ui.label(egui::RichText::new(self.type_name(&e.ext, e.is_dir)).color(DIM()));
                ui.add_space(8.0);
                match &self.preview.content {
                    PvContent::Text { text, truncated, lines } => {
                        egui::Frame::new().fill(pal().code_bg).corner_radius(6).inner_margin(10).show(ui, |ui| {
                            ui.set_width(w - 20.0);
                            egui::ScrollArea::both().id_salt("pvtext").max_height(440.0).auto_shrink([false, true]).show(ui, |ui| {
                                ui.add(egui::Label::new(egui::RichText::new(text).monospace().size(12.0).color(TEXT())).extend());
                            });
                        });
                        let note = format!("{} lines{}", fmt_count(*lines), if *truncated { " · preview truncated" } else { "" });
                        ui.label(egui::RichText::new(note).size(11.5).color(FAINT()));
                    }
                    PvContent::CloudOnly => {
                        let msg = format!("{}  Online-only file. Open it, or choose \"Always keep on this device\", to download it.", ic(I_CLOUD));
                        ui.add(egui::Label::new(egui::RichText::new(msg).font(icon_font(12.5)).color(CLOUD_BLUE())).wrap());
                    }
                    PvContent::Loading => {
                        ui.add(egui::Spinner::new().color(ACCENT()));
                    }
                    _ => {}
                }
                ui.add_space(10.0);
                let mut rows: Vec<(&str, String)> = Vec::new();
                if let PvContent::Folder { files, dirs, size } = &self.preview.content {
                    rows.push(("Contains", format!("{} files, {} folders", fmt_count(*files), fmt_count(*dirs))));
                    rows.push(("Files size", fmt_size(*size)));
                } else if !e.is_dir {
                    rows.push(("Size", format!("{}  ({} bytes)", fmt_size(e.size), fmt_count(e.size as usize))));
                }
                rows.push(("Modified", fmt_filetime(e.mtime)));
                rows.push(("Created", fmt_filetime(e.ctime)));
                let attrs: Vec<&str> = [(fsx::A_READONLY, "Read-only"), (fsx::A_HIDDEN, "Hidden"), (fsx::A_SYSTEM, "System"), (fsx::A_COMPRESSED, "Compressed"), (fsx::A_ENCRYPTED, "Encrypted"), (fsx::A_REPARSE, "Link")]
                    .iter()
                    .filter(|(b, _)| e.attrs & b != 0)
                    .map(|(_, n)| *n)
                    .collect();
                if !attrs.is_empty() {
                    rows.push(("Attributes", attrs.join(", ")));
                }
                let st = self.cloud_of(&path, e.attrs, t);
                if st != Cloud::None {
                    rows.push(("Status", cloud_text(st).to_string()));
                }
                rows.push(("Location", path.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default()));
                egui::Grid::new("pvinfo").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
                    for (k, v) in rows {
                        ui.label(egui::RichText::new(k).color(DIM()).size(12.5));
                        ui.add(egui::Label::new(egui::RichText::new(v).size(12.5)).wrap());
                        ui.end_row();
                    }
                });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("Open").clicked() {
                        self.acts.push(if e.is_dir { Act::Navigate(Loc::Dir(path.clone())) } else { Act::Open(path.clone()) });
                    }
                    if ui.button("Properties").clicked() {
                        self.acts.push(Act::Properties(path.clone()));
                    }
                });
            });
        });
    }

    fn preview_summary(&mut self, ui: &mut Ui, t: &Tab, n_sel: usize) {
        ui.add_space(8.0);
        let (title, sub) = if n_sel > 1 {
            (format!("{} items selected", fmt_count(n_sel)), if t.sel_info.1 > 0 { fmt_size(t.sel_info.1) } else { String::new() })
        } else if let Some(s) = &t.search {
            (format!("Search: {}", s.text), format!("{} results", fmt_count(s.results.len())))
        } else {
            match &t.loc {
                Loc::ThisPc => ("This PC".into(), format!("{} drives", self.drives.len())),
                Loc::Settings => ("Settings".into(), String::new()),
                Loc::Dir(_) => (t.loc.title(), format!("{} items", fmt_count(t.view.len()))),
            }
        };
        if let (Loc::Dir(d), false, true) = (&t.loc, t.searching(), n_sel <= 1) {
            let d = d.clone();
            let r = Rect::from_min_size(ui.cursor().min + vec2((ui.available_width() - 96.0) / 2.0, 12.0), vec2(96.0, 96.0));
            self.draw_thumb(ui, r, &d, "", true, fsx::A_READONLY, 0);
            ui.add_space(120.0);
        }
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new(title).size(16.0));
            ui.label(egui::RichText::new(sub).color(DIM()));
            ui.add_space(14.0);
            ui.label(egui::RichText::new("Select a file to preview it.").color(FAINT()));
        });
    }
}
