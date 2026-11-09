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
        // on a share, a note in each folder it touches tells other people's Daleris what's going on
        let shared = job.sources.iter().chain(job.dest.iter()).any(|p| fsx::is_network_path(p));
        if shared && self.settings.share_marks {
            let j = job.clone();
            bg::spawn("share-notes", move || note_job(&j));
        }
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
            self.toast(tr("Stopped. What was done already stays done."), false);
        } else if let [one] = p.failed.as_slice() {
            let text = match j.kind {
                Kind::Copy => trf!("Couldn't copy {name}", name = one),
                Kind::Move => trf!("Couldn't move {name}", name = one),
                Kind::Delete => trf!("Couldn't delete {name}", name = one),
            };
            self.toast(text, true);
        } else if !p.failed.is_empty() {
            let (n, first) = (p.failed.len(), &p.failed[0]);
            let text = match j.kind {
                Kind::Copy => trn!(n, "{n} item couldn't be copied: {first}", "{n} items couldn't be copied. The first: {first}", first = first),
                Kind::Move => trn!(n, "{n} item couldn't be moved: {first}", "{n} items couldn't be moved. The first: {first}", first = first),
                Kind::Delete => trn!(n, "{n} item couldn't be deleted: {first}", "{n} items couldn't be deleted. The first: {first}", first = first),
            };
            self.toast(text, true);
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
                            if ui.add(egui::Button::new(egui::RichText::new(ic(I_CLOSE)).font(icon_font(12.0))).frame(false)).on_hover_text(tr("Stop")).clicked() {
                                j.cancel();
                            }
                        });
                    });
                    let r = readout(j.kind, &p);
                    // what's being worked on, and which of how many
                    ui.horizontal(|ui| {
                        let name = if j.question().is_some() { tr("Waiting for your answer").to_string() } else { p.current.clone() };
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
                    ui.label(egui::RichText::new(if there.is_dir { tr("There's already a folder with this name") } else { tr("There's already a file with this name") }).size(16.0).strong());
                    ui.add_space(6.0);
                    ui.label(trf!("\"{name}\" is in \"{folder}\" already.", name = name, folder = folder));
                    ui.add_space(10.0);
                    let newer = |a: &Side, b: &Side| a.modified.zip(b.modified).is_some_and(|(x, y)| x > y);
                    side_row(ui, if job.kind == Kind::Move { tr("Being moved") } else { tr("Being copied") }, from, newer(from, there));
                    side_row(ui, tr("Already there"), there, newer(there, from));
                    if *others > 0 {
                        ui.add_space(8.0);
                        tick_box(ui, egui::Checkbox::new(&mut all, trn!(*others, "Do the same for the other one", "Do the same for the other {n}")));
                    }
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if *can_replace && ui.button(tr("Replace")).clicked() {
                            answer = Some(Answer::Conflict(Choice::Replace, all));
                        }
                        if ui.button(tr("Keep both")).on_hover_text(tr("The new one gets a number after its name")).clicked() {
                            answer = Some(Answer::Conflict(Choice::KeepBoth, all));
                        }
                        if ui.button(tr("Skip")).clicked() {
                            answer = Some(Answer::Conflict(Choice::Skip, all));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(tr("Stop")).clicked() {
                                answer = Some(Answer::Cancel);
                            }
                        });
                    });
                }
                Question::Error { path, message, needs_admin } => {
                    let name = file_name(path);
                    let title = match job.kind {
                        Kind::Copy => trf!("Couldn't copy \"{name}\"", name = name),
                        Kind::Move => trf!("Couldn't move \"{name}\"", name = name),
                        Kind::Delete => trf!("Couldn't delete \"{name}\"", name = name),
                    };
                    ui.label(egui::RichText::new(title).size(16.0).strong());
                    ui.add_space(6.0);
                    ui.label(message);
                    if *needs_admin {
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new(tr("This place may need administrator rights. Windows can do the rest and ask for them.")).small().color(DIM()));
                    }
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.button(tr("Try again")).clicked() {
                            answer = Some(Answer::Retry);
                        }
                        if ui.button(tr("Skip")).clicked() {
                            answer = Some(Answer::Skip);
                        }
                        if ui.button(tr("Skip all that fail")).clicked() {
                            answer = Some(Answer::SkipAll);
                        }
                        if *needs_admin && ui.button(tr("Let Windows do it")).clicked() {
                            answer = Some(Answer::HandOff);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(tr("Stop")).clicked() {
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
            ui.label(egui::RichText::new(tr("Delete for good?")).size(16.0).strong());
            ui.add_space(6.0);
            let text = match (paths.as_slice(), cfg!(windows)) {
                ([one], true) => trf!("Daleris deletes \"{name}\" without putting it in the Recycle Bin. This can't be undone.", name = file_name(one)),
                ([one], false) => trf!("Daleris deletes \"{name}\" without putting it in the Trash. This can't be undone.", name = file_name(one)),
                (more, true) => trf!("Daleris deletes these {count} items without putting them in the Recycle Bin. This can't be undone.", count = more.len()),
                (more, false) => trf!("Daleris deletes these {count} items without putting them in the Trash. This can't be undone.", count = more.len()),
            };
            ui.label(text);
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let delete = egui::Button::new(egui::RichText::new(tr("Delete")).color(Color32::WHITE)).fill(Color32::from_rgb(0xc4, 0x2b, 0x1c));
                go = ui.add(delete).clicked() || ui.input(|i| i.key_pressed(Key::Enter));
                cancel = ui.button(tr("Cancel")).clicked();
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
    // as wide as the widest of the three labels in the language shown, so both rows line up
    let font = egui::TextStyle::Body.resolve(ui.style());
    let w = [tr("Being moved"), tr("Being copied"), tr("Already there")].iter().map(|l| ui.fonts_mut(|f| f.layout_no_wrap(l.to_string(), font.clone(), DIM()).size().x)).fold(0.0, f32::max) + 12.0;
    ui.horizontal(|ui| {
        ui.add_sized(vec2(w.max(110.0), 18.0), egui::Label::new(egui::RichText::new(label).color(DIM())));
        let mut parts = Vec::new();
        if !s.is_dir {
            parts.push(crate::util::fmt_size(s.size));
        }
        if let Some(m) = s.modified {
            parts.push(trf!("changed {date}", date = crate::util::fmt_filetime(m)));
        }
        ui.label(parts.join(", "));
        if newer {
            ui.label(egui::RichText::new(tr("newer")).small().color(ACCENT()));
        }
    });
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

fn job_title(j: &Job) -> String {
    let to = j.dest.as_deref().map(file_name).unwrap_or_default();
    match (j.kind, j.sources.as_slice()) {
        (Kind::Copy, [one]) => trf!("Copying \"{name}\" to {folder}", name = file_name(one), folder = to),
        (Kind::Copy, more) => trf!("Copying {count} items to {folder}", count = more.len(), folder = to),
        (Kind::Move, [one]) => trf!("Moving \"{name}\" to {folder}", name = file_name(one), folder = to),
        (Kind::Move, more) => trf!("Moving {count} items to {folder}", count = more.len(), folder = to),
        (Kind::Delete, [one]) => trf!("Deleting \"{name}\"", name = file_name(one)),
        (Kind::Delete, more) => trf!("Deleting {count} items", count = more.len()),
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
    let count = if p.files_total > 1 { trf!("file {done} of {total}", done = n((p.files_done + 1).min(p.files_total)), total = n(p.files_total)) } else { String::new() };
    if p.stage == Stage::Counting {
        return Readout { frac: None, amount: tr("Finding out how much there is").into(), count: String::new(), speed: None, left: None };
    }
    if kind == Kind::Delete || p.bytes_total == 0 {
        let frac = if p.files_total == 0 { 1.0 } else { p.files_done as f32 / p.files_total as f32 };
        let amount = if p.files_total == 1 { trf!("{done} of {total} file", done = n(p.files_done), total = n(p.files_total)) } else { trf!("{done} of {total} files", done = n(p.files_done), total = n(p.files_total)) };
        return Readout { frac: Some(frac), amount, count: String::new(), speed: None, left: None };
    }
    let frac = (p.bytes_done as f32 / p.bytes_total as f32).clamp(0.0, 1.0);
    let amount = trf!("{done} of {total}", done = crate::util::fmt_size(p.bytes_done), total = crate::util::fmt_size(p.bytes_total));
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
        0..=4 => tr("almost done").into(),
        s @ 5..=59 => trf!("about {s} s left", s = s),
        s @ 60..=3599 => trf!("about {m} min left", m = s.div_ceil(60)),
        s => trf!("about {h} h {m} min left", h = s / 3600, m = (s % 3600) / 60),
    }
}

/// What a copy, move or delete is doing to an item the list shows: this window's own, another person's, or a write
/// from anywhere, guessed from the item changing size.
#[derive(Clone, PartialEq, Debug)]
pub(super) struct Transit {
    pub way: Way,
    pub kind: Kind,
    /// how far along this item is, when it's the one being worked on
    pub frac: Option<f32>,
    /// someone else, by the note they left; None for this window's own
    pub who: Option<String>,
    /// the folder at the other end: where it comes from or goes to, when this window is doing it
    pub there: Option<PathBuf>,
    pub info: Info,
}

/// The details the tooltip shows, as far as they're known.
#[derive(Clone, Default, PartialEq, Debug)]
pub(super) struct Info {
    /// this item's bytes done and in all; for one being written, its size now
    pub done: Option<u64>,
    pub total: Option<u64>,
    /// bytes a second
    pub speed: Option<u64>,
    pub left: Option<Duration>,
    /// how long ago it started, or was first seen changing
    pub since: Option<Duration>,
    /// how long ago the other person's note was written, or the item last changed
    pub heard: Option<Duration>,
    /// which of how many items chosen together
    pub place: Option<(usize, usize)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Way {
    /// coming into the folder it's shown in
    Arriving,
    /// going away from it, moved or deleted
    Leaving,
    /// being read for a copy; it stays where it is
    Source,
    /// changing as it's looked at, by someone or some app; nothing more is known
    Writing,
}

impl App {
    /// Whether a copy, move or delete is working on the item at `path`, and how: this window's own first, then what
    /// other people's notes in `t`'s folder say, then whether it has been changing.
    pub(super) fn transit(&self, t: &Tab, path: &Path) -> Option<Transit> {
        for j in &self.ops {
            let hand = j.hand();
            let p = j.progress();
            let secs = p.started.elapsed().as_secs_f64();
            let speed = (secs > 1.0 && p.bytes_done > 0).then(|| (p.bytes_done as f64 / secs) as u64);
            // what's known when it's the item being worked on
            let info = |h: Option<&crate::fileops::Hand>| Info {
                done: h.map(|h| h.done),
                total: h.map(|h| h.total),
                speed,
                left: h.zip(speed).filter(|(_, s)| *s > 0).map(|(h, s)| Duration::from_secs_f64((h.total - h.done) as f64 / s as f64)),
                since: Some(p.started.elapsed()),
                heard: None,
                place: h.filter(|h| h.count > 1).map(|h| (h.index + 1, h.count)),
            };
            if let Some(h) = hand.as_ref().filter(|h| h.dst.as_deref().is_some_and(|d| fsx::same_path(d, path))) {
                let there = h.src.parent().map(Path::to_path_buf);
                return Some(Transit { way: Way::Arriving, kind: j.kind, frac: Some(h.frac()), who: None, there, info: info(Some(h)) });
            }
            if j.sources.iter().any(|s| fsx::same_path(s, path)) {
                let mine = hand.as_ref().filter(|h| fsx::same_path(&h.src, path));
                let way = if j.kind == Kind::Copy { Way::Source } else { Way::Leaving };
                return Some(Transit { way, kind: j.kind, frac: mine.map(|h| h.frac()), who: None, there: j.dest.clone(), info: info(mine) });
            }
        }
        if t.dir().is_some_and(|d| path.parent().is_some_and(|p| fsx::same_path(p, d))) {
            let key = path.file_name()?.to_string_lossy().to_lowercase();
            if let Some(m) = t.marks.get(&key) {
                let way = match m.way {
                    crate::marks::Way::Arriving => Way::Arriving,
                    crate::marks::Way::Leaving => Way::Leaving,
                    // open items are kept apart, so never here
                    crate::marks::Way::Source | crate::marks::Way::Open => Way::Source,
                };
                let kind = match m.kind {
                    'm' => Kind::Move,
                    'd' => Kind::Delete,
                    _ => Kind::Copy,
                };
                let now = crate::marks::now();
                let heard = now.saturating_sub(m.at);
                let info = Info {
                    done: m.done,
                    total: m.total,
                    speed: None,
                    left: m.left.map(|l| Duration::from_secs(l.saturating_sub(heard))),
                    since: Some(Duration::from_secs(now.saturating_sub(m.started))),
                    heard: Some(Duration::from_secs(heard)),
                    place: None,
                };
                return Some(Transit { way, kind, frac: m.frac, who: Some(m.who.clone()), there: None, info });
            }
            if let Some(g) = t.growing.get(&key).filter(|g| g.last.elapsed() < GROWING_FOR) {
                let span = g.last.duration_since(g.first).as_secs_f64();
                let info = Info {
                    done: Some(g.size),
                    speed: (span > 0.5 && g.size > g.first_size).then(|| ((g.size - g.first_size) as f64 / span) as u64),
                    since: Some(g.first.elapsed()),
                    heard: Some(g.last.elapsed()),
                    ..Info::default()
                };
                return Some(Transit { way: Way::Writing, kind: Kind::Copy, frac: None, who: None, there: None, info });
            }
        }
        None
    }
}

/// How long an item that changed size counts as being written.
const GROWING_FOR: Duration = Duration::from_secs(8);

impl Transit {
    /// What's happening, in a few words, as the tooltip's title.
    fn title(&self) -> &'static str {
        match (self.way, self.kind) {
            (Way::Writing, _) => tr("Being written"),
            (Way::Arriving, Kind::Move) => tr("Moving here"),
            (Way::Arriving, _) => tr("Copying here"),
            (Way::Leaving, Kind::Delete) => tr("Deleting"),
            (Way::Leaving, _) => tr("Moving away"),
            (Way::Source, _) => tr("Copying from here"),
        }
    }

    /// What it means for the item, below the details.
    fn note(&self) -> &'static str {
        match (self.way, self.kind) {
            (Way::Writing, _) => tr("Its size changed while you were looking. Wait before opening or changing it."),
            (Way::Arriving, Kind::Move) => tr("It's all here once it's done."),
            (Way::Arriving, _) => tr("Wait until it's done before opening it."),
            (Way::Leaving, Kind::Delete) => tr("It's gone for good once it's done."),
            (Way::Leaving, _) => tr("It's gone from here once it's done."),
            (Way::Source, _) => tr("It stays here and can still be used."),
        }
    }

    /// The tooltip: a title, then progress, the other end and who, each on its own row, then what it means.
    pub(super) fn tip_ui(&self, ui: &mut Ui) {
        ui.set_max_width(320.0);
        ui.label(egui::RichText::new(self.title()).strong().color(self.colour()));
        ui.add_space(4.0);
        let key = |ui: &mut Ui, k: &str| {
            ui.label(egui::RichText::new(k).color(DIM()));
        };
        egui::Grid::new("transit_tip").num_columns(2).spacing([14.0, 4.0]).show(ui, |ui| {
            if let Some(f) = self.frac {
                key(ui, tr("Progress"));
                ui.horizontal(|ui| {
                    ui.add(egui::ProgressBar::new(f).desired_width(120.0).desired_height(6.0));
                    ui.label(format!("{:.0} %", f * 100.0));
                });
                ui.end_row();
            }
            if let Some(p) = &self.there {
                key(ui, if self.way == Way::Arriving { tr("From") } else { tr("To") });
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string());
                ui.label(name).on_hover_text(p.display().to_string());
                ui.end_row();
            }
            let i = &self.info;
            let size = crate::util::fmt_size;
            match (i.done, i.total) {
                (Some(d), Some(t)) if self.way != Way::Writing => {
                    key(ui, tr("Size"));
                    ui.label(trf!("{done} of {total}", done = size(d), total = size(t)));
                    ui.end_row();
                }
                (Some(d), _) if self.way == Way::Writing => {
                    key(ui, tr("Size"));
                    ui.label(trf!("{size} so far", size = size(d)));
                    ui.end_row();
                }
                _ => {}
            }
            if let Some(s) = i.speed {
                key(ui, if self.way == Way::Writing { tr("Growing") } else { tr("Speed") });
                ui.label(format!("{}/s", size(s)));
                ui.end_row();
            }
            if let Some(l) = i.left {
                key(ui, tr("Time left"));
                ui.label(time_left(l.as_secs_f64()));
                ui.end_row();
            }
            if let Some((n, of)) = i.place {
                key(ui, tr("Item"));
                ui.label(trf!("{index} of {count} chosen together", index = n, count = of));
                ui.end_row();
            }
            key(ui, tr("By"));
            ui.label(match (&self.who, self.way) {
                (Some(who), _) => who.clone(),
                (None, Way::Writing) => tr("Someone, or an app").into(),
                (None, _) => tr("You, in this window").into(),
            });
            ui.end_row();
            if let Some(d) = i.since {
                key(ui, if self.way == Way::Writing { tr("Noticed") } else { tr("Started") });
                ui.label(ago(d));
                ui.end_row();
            }
            if let Some(d) = i.heard {
                key(ui, if self.way == Way::Writing { tr("Last change") } else { tr("Last heard") });
                ui.label(ago(d));
                ui.end_row();
            }
        });
        ui.add_space(4.0);
        ui.label(egui::RichText::new(self.note()).small().color(DIM()));
    }

    /// The colour of its name: greyed while it arrives or is written, fainter while it leaves, as usual while it's only
    /// read.
    pub(super) fn text(&self) -> Color32 {
        match self.way {
            Way::Arriving | Way::Writing => DIM(),
            Way::Leaving => FAINT(),
            Way::Source => TEXT(),
        }
    }

    /// The accent for this window's own work; orange for other people's, so a clash is seen coming.
    fn colour(&self) -> Color32 {
        if self.who.is_some() || self.way == Way::Writing {
            return CAUTION();
        }
        match self.way {
            Way::Arriving => ACCENT(),
            Way::Leaving => FAINT(),
            _ => DIM(),
        }
    }

    /// Around the item's own icon: a ring that fills with its progress (turning while that isn't known), dotted for a
    /// copy's source; and in the corner a badge, an arrow in, an arrow out, or two sheets for a copy's source.
    pub(super) fn draw(&self, ui: &Ui, icon: Rect) {
        let p = ui.painter();
        let c = icon.center();
        let radius = icon.width().max(icon.height()) * 0.5 + 3.0;
        let colour = self.colour();
        let arc = |from: f32, to: f32, stroke: Stroke| {
            let n = (((to - from).abs() / std::f32::consts::TAU) * 48.0).ceil().max(2.0) as usize;
            let pts: Vec<egui::Pos2> = (0..=n).map(|k| from + (to - from) * k as f32 / n as f32).map(|a| c + radius * vec2(a.cos(), a.sin())).collect();
            p.add(egui::Shape::line(pts, stroke));
        };
        let top = -std::f32::consts::FRAC_PI_2;
        let t = ui.input(|i| i.time) as f32;
        if self.way == Way::Source {
            // dotted, turning slowly
            for k in 0..12 {
                let a = top + t * 0.8 + k as f32 / 12.0 * std::f32::consts::TAU;
                p.circle_filled(c + radius * vec2(a.cos(), a.sin()), 1.1, colour);
            }
        } else {
            p.circle_stroke(c, radius, Stroke::new(1.5_f32, LINE()));
            match self.frac {
                Some(f) if f > 0.0 => arc(top, top + f * std::f32::consts::TAU, Stroke::new(2.0_f32, colour)),
                _ => arc(top + t * 4.0, top + t * 4.0 + 1.4, Stroke::new(2.0_f32, colour)),
            }
        }
        // the badge, on the icon's bottom right
        // a little inside the corner, so it stays within a compact row
        let b = icon.right_bottom() + vec2(1.0, -1.5);
        p.circle_filled(b, 5.5, BG());
        p.circle_stroke(b, 5.5, Stroke::new(1.0_f32, colour));
        let s = Stroke::new(1.3_f32, colour);
        match self.way {
            Way::Arriving | Way::Leaving => {
                // down-right into the corner for arriving, up-right out of it for leaving
                let (from, to) = if self.way == Way::Arriving { (b + vec2(-2.4, -2.4), b + vec2(2.0, 2.0)) } else { (b + vec2(-2.0, 2.0), b + vec2(2.4, -2.4)) };
                p.line_segment([from, to], s);
                let back = (from - to).normalized() * 2.4;
                let side = vec2(-back.y, back.x);
                p.line_segment([to, to + back + side * 0.9], s);
                p.line_segment([to, to + back - side * 0.9], s);
            }
            Way::Source => {
                p.rect_stroke(Rect::from_min_size(b + vec2(-2.8, -2.8), vec2(3.6, 3.6)), CornerRadius::ZERO, s, StrokeKind::Middle);
                p.rect_stroke(Rect::from_min_size(b + vec2(-0.8, -0.8), vec2(3.6, 3.6)), CornerRadius::ZERO, s, StrokeKind::Middle);
            }
            Way::Writing => {
                // the pencil from the icon font, which reads at this size where a drawn one doesn't
                p.circle_filled(b, 7.0, BG());
                p.circle_stroke(b, 7.0, Stroke::new(1.0_f32, colour));
                p.text(b, Align2::CENTER_CENTER, ic(I_EDIT), icon_font(9.0), colour);
            }
        }
        ui.ctx().request_repaint_after(Duration::from_millis(33));
    }

}

/// Keeps this job's notes in the shared folders it touches up to date until it's done, then takes them away. A note is
/// written again only when its progress moved on by 5 %, or every 15 seconds, so other people's folders don't reload
/// all the time.
fn note_job(job: &Job) {
    use crate::marks::{self, Mark};
    static JOBS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let file = marks::file_name(JOBS.fetch_add(1, Ordering::Relaxed));
    let who = marks::who();
    let kind = match job.kind {
        Kind::Copy => 'c',
        Kind::Move => 'm',
        Kind::Delete => 'd',
    };
    let mut written: HashMap<PathBuf, (Instant, Option<f32>)> = HashMap::new();
    loop {
        let done = job.finished();
        let mut by_dir: HashMap<PathBuf, Vec<Mark>> = HashMap::new();
        let hand = job.in_hand();
        let frac_now = hand.as_ref().map(|h| h.2);
        // the details other people's tooltips show: when it started, and this item's bytes and time left
        let p = job.progress();
        let started = marks::now().saturating_sub(p.started.elapsed().as_secs());
        let speed = (p.started.elapsed().as_secs_f64() > 1.0).then(|| p.bytes_done as f64 / p.started.elapsed().as_secs_f64()).filter(|s| *s > 0.0);
        let detail = job.hand();
        if !done {
            let at = marks::now();
            let mut note = |path: &Path, way: marks::Way, frac: Option<f32>| {
                if let (Some(dir), Some(name)) = (path.parent(), path.file_name()) {
                    if fsx::is_network_path(path) {
                        // bytes and time left only for the item being worked on
                        let mine = detail.as_ref().filter(|h| fsx::same_path(&h.src, path) || h.dst.as_deref().is_some_and(|d| fsx::same_path(d, path)));
                        let (done, total) = (mine.map(|h| h.done), mine.map(|h| h.total));
                        let left = mine.zip(speed).map(|(h, s)| ((h.total - h.done) as f64 / s) as u64);
                        let m = Mark { name: name.to_string_lossy().into_owned(), way, kind, frac, who: who.clone(), at, started, done, total, left, app: None };
                        by_dir.entry(dir.to_path_buf()).or_default().push(m);
                    }
                }
            };
            if let Some((_, Some(dst), frac)) = &hand {
                note(dst, marks::Way::Arriving, Some(*frac));
            }
            for s in &job.sources {
                let frac = hand.as_ref().filter(|h| fsx::same_path(&h.0, s)).map(|h| h.2);
                note(s, if job.kind == Kind::Copy { marks::Way::Source } else { marks::Way::Leaving }, frac);
            }
        }
        for (dir, list) in &by_dir {
            let due = written.get(dir).is_none_or(|(when, f)| when.elapsed() >= Duration::from_secs(15) || (frac_now.unwrap_or(0.0) - f.unwrap_or(0.0)).abs() >= 0.05);
            if due && marks::write(dir, &file, list, None).is_ok() {
                written.insert(dir.clone(), (Instant::now(), frac_now));
            }
        }
        // a folder nothing is happening in any more loses its note
        let gone: Vec<PathBuf> = written.keys().filter(|d| !by_dir.contains_key(*d)).cloned().collect();
        for d in gone {
            marks::remove(&d, &file);
            written.remove(&d);
        }
        if done {
            return;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// "just now", "12 s ago", "3 min ago", "2 h ago", in the language shown.
pub(super) fn ago(d: Duration) -> String {
    match d.as_secs() {
        0..=2 => tr("just now").into(),
        s @ 3..=59 => trf!("{s} s ago", s = s),
        s @ 60..=3599 => trf!("{m} min ago", m = s / 60),
        s => trf!("{h} h ago", h = s / 3600),
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
