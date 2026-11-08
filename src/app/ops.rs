//! Copying, moving and deleting in Daleris's own look: a card with progress for each job in the corner, a question when
//! one needs an answer, and a check before anything is deleted for good.

use super::*;
use crate::fileops::{Answer, Choice, Job, Kind, Progress, Question, Side, Stage};

impl App {
    /// Starts a copy, move or delete on its own thread; a move to the Recycle Bin is left to the system.
    pub(super) fn start_job(&mut self, kind: Kind, files: Vec<PathBuf>, dest: Option<PathBuf>) {
        let job = Arc::new(Job::new(kind, files, dest));
        let (j, ctx) = (job.clone(), self.ctx.clone());
        bg::spawn("file-op", move || {
            j.run(&|q| {
                ctx.request_repaint();
                j.wait_for(q)
            });
            ctx.request_repaint();
        });
        self.ops.push(job);
    }

    /// True while a question or check is up, so the list's shortcuts stay out of its way.
    pub(super) fn op_dialog_open(&self) -> bool {
        self.confirm_delete.is_some() || self.ops.iter().any(|j| j.question().is_some())
    }

    pub(super) fn file_ops(&mut self, ctx: &egui::Context) {
        let mut finished = Vec::new();
        self.ops.retain(|j| {
            let done = j.finished();
            if done {
                finished.push(j.clone());
            }
            !done
        });
        for j in finished {
            self.job_finished(&j);
        }
        if !self.ops.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(150));
            self.ops_cards(ctx);
            self.op_question(ctx);
        }
        self.delete_check(ctx);
    }

    fn job_finished(&mut self, j: &Job) {
        // copies, moves and deletes change folder sizes
        self.sizer.clear();
        let p = j.progress();
        if !p.handed_off.is_empty() {
            let op = if j.kind == Kind::Move { Op::Move } else { Op::Copy };
            let tx = self.tx.clone();
            shell::file_op(op, p.handed_off.clone(), j.dest.clone(), move |r| {
                let _ = tx.send(Msg::OpDone(r));
            });
        }
        if p.cancelled {
            self.toast("Stopped. What was done already stays done.", false);
        } else if let [one] = p.failed.as_slice() {
            self.toast(format!("Couldn't {} {one}", verb(j.kind)), true);
        } else if !p.failed.is_empty() {
            self.toast(format!("{} items couldn't be {}. The first: {}", p.failed.len(), done_word(j.kind), p.failed[0]), true);
        }
    }

    fn ops_cards(&mut self, ctx: &egui::Context) {
        egui::Area::new(egui::Id::unique("file_ops")).order(egui::Order::Foreground).anchor(Align2::RIGHT_BOTTOM, vec2(-16.0, -44.0)).show(ctx, |ui| {
            for j in &self.ops {
                let p = j.progress();
                egui::Frame::new().fill(RAISED()).stroke(Stroke::new(1.0_f32, LINE())).corner_radius(8).inner_margin(12).show(ui, |ui| {
                    ui.set_width(340.0);
                    ui.horizontal(|ui| {
                        ui.add(egui::Label::new(egui::RichText::new(job_title(j)).strong()).truncate());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(egui::Button::new(egui::RichText::new(ic(I_CLOSE)).font(icon_font(12.0))).frame(false)).on_hover_text("Stop").clicked() {
                                j.cancel();
                            }
                        });
                    });
                    let r = readout(j.kind, &p);
                    // what's being worked on, and which of how many
                    ui.horizontal(|ui| {
                        let name = if j.question().is_some() { "Waiting for your answer".to_string() } else { p.current.clone() };
                        ui.add(egui::Label::new(egui::RichText::new(name).color(DIM())).truncate());
                        if !r.count.is_empty() {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(egui::RichText::new(&r.count).small().color(FAINT()));
                            });
                        }
                    });
                    ui.add_space(4.0);
                    ui.add(egui::ProgressBar::new(r.frac.unwrap_or(0.0)).desired_height(6.0).animate(r.frac.is_none()));
                    ui.add_space(6.0);
                    // two columns: how much on the left, how far and how long on the right
                    let pair = |ui: &mut Ui, left: &str, right: &str, strong: bool| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(left).color(if strong { TEXT() } else { DIM() }));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(egui::RichText::new(right).color(if strong { TEXT() } else { DIM() }));
                            });
                        });
                    };
                    let percent = r.frac.map(|f| format!("{:.0} %", f * 100.0)).unwrap_or_default();
                    pair(ui, &r.amount, &percent, true);
                    if r.speed.is_some() || r.left.is_some() {
                        pair(ui, r.speed.as_deref().unwrap_or(""), r.left.as_deref().unwrap_or(""), false);
                    }
                });
                ui.add_space(8.0);
            }
        });
    }

    /// The first question any job has, in front of everything else.
    fn op_question(&mut self, ctx: &egui::Context) {
        let Some((job, q)) = self.ops.iter().find_map(|j| j.question().map(|q| (j.clone(), q))) else { return };
        let all_id = egui::Id::unique(("op_all", Arc::as_ptr(&job) as usize));
        let mut all = ctx.data(|d| d.get_temp::<bool>(all_id).unwrap_or(false));
        let mut answer = None;
        let modal = egui::Modal::new(egui::Id::unique("op_question")).frame(dialog_frame(ctx)).show(ctx, |ui| {
            ui.set_width(440.0);
            match &q {
                Question::Conflict { dst, from, there, can_replace, others } => {
                    let name = file_name(dst);
                    let folder = dst.parent().map(|p| file_name(p)).unwrap_or_default();
                    ui.label(egui::RichText::new(if there.is_dir { "There's already a folder with this name" } else { "There's already a file with this name" }).size(16.0).strong());
                    ui.add_space(6.0);
                    ui.label(format!("\"{name}\" is in \"{folder}\" already."));
                    ui.add_space(10.0);
                    let newer = |a: &Side, b: &Side| a.modified.zip(b.modified).is_some_and(|(x, y)| x > y);
                    side_row(ui, if job.kind == Kind::Move { "Being moved" } else { "Being copied" }, from, newer(from, there));
                    side_row(ui, "Already there", there, newer(there, from));
                    if *others > 0 {
                        ui.add_space(8.0);
                        tick_box(ui, egui::Checkbox::new(&mut all, format!("Do the same for the other {others}")));
                    }
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if *can_replace && ui.button("Replace").clicked() {
                            answer = Some(Answer::Conflict(Choice::Replace, all));
                        }
                        if ui.button("Keep both").on_hover_text("The new one gets a number after its name").clicked() {
                            answer = Some(Answer::Conflict(Choice::KeepBoth, all));
                        }
                        if ui.button("Skip").clicked() {
                            answer = Some(Answer::Conflict(Choice::Skip, all));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Stop").clicked() {
                                answer = Some(Answer::Cancel);
                            }
                        });
                    });
                }
                Question::Error { path, message, needs_admin } => {
                    ui.label(egui::RichText::new(format!("Couldn't {} \"{}\"", verb(job.kind), file_name(path))).size(16.0).strong());
                    ui.add_space(6.0);
                    ui.label(message);
                    if *needs_admin {
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new("This place may need administrator rights. Windows can do the rest and ask for them.").small().color(DIM()));
                    }
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.button("Try again").clicked() {
                            answer = Some(Answer::Retry);
                        }
                        if ui.button("Skip").clicked() {
                            answer = Some(Answer::Skip);
                        }
                        if ui.button("Skip all that fail").clicked() {
                            answer = Some(Answer::SkipAll);
                        }
                        if *needs_admin && ui.button("Let Windows do it").clicked() {
                            answer = Some(Answer::HandOff);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Stop").clicked() {
                                answer = Some(Answer::Cancel);
                            }
                        });
                    });
                }
            }
        });
        // Escape, or a click beside it, leaves this one out rather than stopping everything
        if answer.is_none() && modal.should_close() {
            answer = Some(if matches!(q, Question::Conflict { .. }) { Answer::Conflict(Choice::Skip, false) } else { Answer::Skip });
        }
        ctx.data_mut(|d| d.insert_temp(all_id, all));
        if let Some(a) = answer {
            job.answer(a);
        }
    }

    pub(super) fn ask_delete(&mut self, paths: Vec<PathBuf>) {
        if !paths.is_empty() {
            self.confirm_delete = Some(paths);
        }
    }

    /// Deleting for good can't be undone, so it's asked first.
    fn delete_check(&mut self, ctx: &egui::Context) {
        let Some(paths) = self.confirm_delete.clone() else { return };
        let (mut go, mut cancel) = (false, false);
        let modal = egui::Modal::new(egui::Id::unique("delete_check")).frame(dialog_frame(ctx)).show(ctx, |ui| {
            ui.set_width(420.0);
            let what = match paths.as_slice() {
                [one] => format!("\"{}\"", file_name(one)),
                more => format!("these {} items", more.len()),
            };
            ui.label(egui::RichText::new("Delete for good?").size(16.0).strong());
            ui.add_space(6.0);
            let bin = if cfg!(windows) { "the Recycle Bin" } else { "the Trash" };
            ui.label(format!("Daleris deletes {what} without putting {} in {bin}. This can't be undone.", if paths.len() == 1 { "it" } else { "them" }));
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let delete = egui::Button::new(egui::RichText::new("Delete").color(Color32::WHITE)).fill(Color32::from_rgb(0xc4, 0x2b, 0x1c));
                go = ui.add(delete).clicked() || ui.input(|i| i.key_pressed(Key::Enter));
                cancel = ui.button("Cancel").clicked();
            });
        });
        if go {
            self.confirm_delete = None;
            self.start_job(Kind::Delete, paths, None);
        } else if cancel || modal.should_close() {
            self.confirm_delete = None;
        }
    }
}

fn dialog_frame(ctx: &egui::Context) -> egui::Frame {
    egui::Frame::window(&ctx.global_style()).fill(RAISED()).stroke(Stroke::new(1.0_f32, LINE())).corner_radius(10).inner_margin(20)
}

fn tick_box(ui: &mut Ui, check: egui::Checkbox) -> egui::Response {
    ui.scope(|ui| {
        ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::new(1.0_f32, FAINT());
        ui.add(check)
    })
    .inner
}

fn side_row(ui: &mut Ui, label: &str, s: &Side, newer: bool) {
    ui.horizontal(|ui| {
        ui.add_sized(vec2(110.0, 18.0), egui::Label::new(egui::RichText::new(label).color(DIM())));
        let mut parts = Vec::new();
        if !s.is_dir {
            parts.push(crate::util::fmt_size(s.size));
        }
        if let Some(m) = s.modified {
            parts.push(format!("changed {}", crate::util::fmt_filetime(m)));
        }
        ui.label(parts.join(" · "));
        if newer {
            ui.label(egui::RichText::new("newer").small().color(ACCENT()));
        }
    });
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

fn verb(k: Kind) -> &'static str {
    match k {
        Kind::Copy => "copy",
        Kind::Move => "move",
        Kind::Delete => "delete",
    }
}

fn done_word(k: Kind) -> &'static str {
    match k {
        Kind::Copy => "copied",
        Kind::Move => "moved",
        Kind::Delete => "deleted",
    }
}

fn job_title(j: &Job) -> String {
    let what = match j.sources.as_slice() {
        [one] => format!("\"{}\"", file_name(one)),
        more => format!("{} items", more.len()),
    };
    let to = j.dest.as_deref().map(file_name).unwrap_or_default();
    match j.kind {
        Kind::Copy => format!("Copying {what} to {to}"),
        Kind::Move => format!("Moving {what} to {to}"),
        Kind::Delete => format!("Deleting {what}"),
    }
}

/// What the card shows about how far along a job is, each in its own place.
struct Readout {
    /// None while that isn't known yet
    frac: Option<f32>,
    /// "1.2 GB of 4.8 GB", or "3 of 12 files"
    amount: String,
    /// "file 3 of 12", beside the name being worked on
    count: String,
    speed: Option<String>,
    left: Option<String>,
}

fn readout(kind: Kind, p: &Progress) -> Readout {
    let n = |v: u64| crate::util::fmt_count(v as usize);
    // only worth saying when there's more than one
    let count = if p.files_total > 1 { format!("file {} of {}", n((p.files_done + 1).min(p.files_total)), n(p.files_total)) } else { String::new() };
    if p.stage == Stage::Counting {
        return Readout { frac: None, amount: "Finding out how much there is".into(), count: String::new(), speed: None, left: None };
    }
    if kind == Kind::Delete || p.bytes_total == 0 {
        let frac = if p.files_total == 0 { 1.0 } else { p.files_done as f32 / p.files_total as f32 };
        return Readout { frac: Some(frac), amount: format!("{} of {} files", n(p.files_done), n(p.files_total)), count: String::new(), speed: None, left: None };
    }
    let frac = (p.bytes_done as f32 / p.bytes_total as f32).clamp(0.0, 1.0);
    let amount = format!("{} of {}", crate::util::fmt_size(p.bytes_done), crate::util::fmt_size(p.bytes_total));
    let secs = p.started.elapsed().as_secs_f64();
    let (speed, left) = if secs > 1.0 && p.bytes_done > 0 {
        let speed = p.bytes_done as f64 / secs;
        (Some(format!("{}/s", crate::util::fmt_size(speed as u64))), Some(time_left(p.bytes_total.saturating_sub(p.bytes_done) as f64 / speed)))
    } else {
        (None, None)
    };
    Readout { frac: Some(frac), amount, count, speed, left }
}

fn time_left(secs: f64) -> String {
    match secs as u64 {
        0..=4 => "almost done".into(),
        s @ 5..=59 => format!("about {s} s left"),
        s @ 60..=3599 => format!("about {} min left", s.div_ceil(60)),
        s => format!("about {} h {} min left", s / 3600, (s % 3600) / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_how_long_is_left_in_plain_words() {
        assert_eq!(time_left(2.0), "almost done");
        assert_eq!(time_left(40.0), "about 40 s left");
        assert_eq!(time_left(61.0), "about 2 min left");
        assert_eq!(time_left(3700.0), "about 1 h 1 min left");
    }
}

/// An item that's on its way: arriving in the folder it's shown in, or leaving it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Transit {
    Arriving(Kind),
    Leaving(Kind),
}

impl App {
    /// Whether a copy, move or delete is working on the item at `path` right now.
    pub(super) fn transit(&self, path: &Path) -> Option<Transit> {
        for j in &self.ops {
            if j.arriving().is_some_and(|a| fsx::same_path(&a, path)) {
                return Some(Transit::Arriving(j.kind));
            }
            if matches!(j.kind, Kind::Move | Kind::Delete) && j.sources.iter().any(|s| fsx::same_path(s, path)) {
                return Some(Transit::Leaving(j.kind));
            }
        }
        None
    }
}

impl Transit {
    pub(super) fn tip(self) -> &'static str {
        match self {
            Transit::Arriving(Kind::Move) => "Being moved here",
            Transit::Arriving(_) => "Being copied here",
            Transit::Leaving(Kind::Delete) => "Being deleted",
            Transit::Leaving(_) => "Being moved away from here",
        }
    }

    /// The colour of its name: greyed while it arrives, fainter still while it leaves.
    pub(super) fn text(self) -> Color32 {
        if matches!(self, Transit::Arriving(_)) { DIM() } else { FAINT() }
    }

    /// A ring turning in place of the icon: the accent, clockwise, for what arrives; faint and the other way for
    /// what leaves.
    pub(super) fn draw(self, ui: &Ui, r: Rect) {
        let arriving = matches!(self, Transit::Arriving(_));
        let t = ui.input(|i| i.time) as f32 * 4.0;
        let start = if arriving { t } else { -t };
        let radius = r.width().min(r.height()) * 0.38;
        let c = r.center();
        ui.painter().circle_stroke(c, radius, Stroke::new(2.0_f32, LINE()));
        let points: Vec<egui::Pos2> = (0..=24).map(|k| start + k as f32 / 24.0 * std::f32::consts::PI * 1.3).map(|a| c + radius * vec2(a.cos(), a.sin())).collect();
        ui.painter().add(egui::Shape::line(points, Stroke::new(2.0_f32, if arriving { ACCENT() } else { FAINT() })));
        ui.ctx().request_repaint_after(Duration::from_millis(33));
    }
}
