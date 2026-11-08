//! Copying, moving and deleting for good, with Daleris's own progress, questions and cancel instead of the system's dialogs.
//! A job runs on a worker thread. When it needs a decision, such as a name that's taken, it puts up a question and waits
//! for the window to answer it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use crate::fsx;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Copy,
    Move,
    Delete,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    /// adding up what there is to do
    Counting,
    Working,
    Done,
}

#[derive(Clone, Debug)]
pub struct Progress {
    pub stage: Stage,
    pub files_total: u64,
    pub files_done: u64,
    pub bytes_total: u64,
    pub bytes_done: u64,
    /// the name being worked on
    pub current: String,
    pub started: Instant,
    /// items left alone because of a question's answer
    pub skipped: u64,
    /// what went wrong with items that were skipped after all
    pub failed: Vec<String>,
    /// what's left for the system to do, when the person asked for that
    pub handed_off: Vec<PathBuf>,
    pub cancelled: bool,
}

/// One side of a name that's taken: what's being copied or moved, or what's there already.
#[derive(Clone, Debug)]
pub struct Side {
    pub size: u64,
    /// as a FILETIME, for `util::fmt_filetime`
    pub modified: Option<u64>,
    pub is_dir: bool,
}

#[derive(Clone, Debug)]
pub enum Question {
    /// `dst` exists already. A folder is never replaced, so `can_replace` is false when either side is one.
    /// `others` is how many more names are taken, as far as it's known.
    Conflict { dst: PathBuf, from: Side, there: Side, can_replace: bool, others: u64 },
    /// `needs_admin` when the system could do it with administrator rights
    Error { path: PathBuf, message: String, needs_admin: bool },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Choice {
    Replace,
    Skip,
    KeepBoth,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Answer {
    /// what to do with this name, and whether to do the same for the others
    Conflict(Choice, bool),
    Retry,
    Skip,
    SkipAll,
    /// let the system do the rest, which can ask for administrator rights
    HandOff,
    Cancel,
}

pub struct Job {
    pub kind: Kind,
    pub sources: Vec<PathBuf>,
    pub dest: Option<PathBuf>,
    progress: Mutex<Progress>,
    question: Mutex<Option<Question>>,
    answers: (Mutex<Sender<Answer>>, Mutex<Receiver<Answer>>),
    /// the top-level item being worked on, as the list shows it
    in_hand: Mutex<Option<InHand>>,
    cancel: AtomicBool,
}

impl Job {
    pub fn new(kind: Kind, sources: Vec<PathBuf>, dest: Option<PathBuf>) -> Job {
        let (tx, rx) = channel();
        Job {
            kind,
            sources,
            dest,
            progress: Mutex::new(Progress {
                stage: Stage::Counting,
                files_total: 0,
                files_done: 0,
                bytes_total: 0,
                bytes_done: 0,
                current: String::new(),
                started: Instant::now(),
                skipped: 0,
                failed: Vec::new(),
                handed_off: Vec::new(),
                cancelled: false,
            }),
            question: Mutex::new(None),
            answers: (Mutex::new(tx), Mutex::new(rx)),
            in_hand: Mutex::new(None),
            cancel: AtomicBool::new(false),
        }
    }

    pub fn progress(&self) -> Progress {
        self.progress.lock().unwrap().clone()
    }

    pub fn question(&self) -> Option<Question> {
        self.question.lock().unwrap().clone()
    }

    pub fn answer(&self, a: Answer) {
        *self.question.lock().unwrap() = None;
        let _ = self.answers.0.lock().unwrap().send(a);
    }

    /// The top-level item being worked on right now: where from, where to under its final name (none for a delete), and
    /// how far along it is.
    pub fn in_hand(&self) -> Option<(PathBuf, Option<PathBuf>, f32)> {
        let h = self.in_hand.lock().unwrap().clone()?;
        let done = self.bytes_done().saturating_sub(h.start_bytes);
        let frac = if h.bytes == 0 { 0.0 } else { (done as f32 / h.bytes as f32).clamp(0.0, 1.0) };
        Some((h.src, h.dst, frac))
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn finished(&self) -> bool {
        self.progress.lock().unwrap().stage == Stage::Done
    }

    /// Puts up the question and waits for the window to answer it; Cancel if the job is cancelled meanwhile.
    pub fn wait_for(&self, q: Question) -> Answer {
        *self.question.lock().unwrap() = Some(q);
        let rx = self.answers.1.lock().unwrap();
        loop {
            if let Ok(a) = rx.recv_timeout(Duration::from_millis(100)) {
                return a;
            }
            if self.cancelled() {
                *self.question.lock().unwrap() = None;
                return Answer::Cancel;
            }
        }
    }

    fn update(&self, f: impl FnOnce(&mut Progress)) {
        f(&mut self.progress.lock().unwrap());
    }

    fn bytes_done(&self) -> u64 {
        self.progress.lock().unwrap().bytes_done
    }

    /// Does the whole job, asking `ask` whenever it needs a decision.
    pub fn run(&self, ask: &dyn Fn(Question) -> Answer) {
        let mut r = Run { job: self, ask, rule: None, skip_errors: false, conflicts_left: 0, item_bytes: 0 };
        let outcome = if self.kind == Kind::Delete { r.delete_all() } else { r.transfer_all() };
        self.update(|p| {
            if let Err(Stop::HandOff(rest)) = outcome {
                p.handed_off = rest;
            }
            p.cancelled = self.cancelled();
            p.stage = Stage::Done;
            *self.in_hand.lock().unwrap() = None;
            p.current.clear();
        });
    }
}

/// One of the items the person chose, while it's worked on.
#[derive(Clone)]
struct InHand {
    src: PathBuf,
    dst: Option<PathBuf>,
    /// the job's bytes done when it started, and its own size
    start_bytes: u64,
    bytes: u64,
}

enum Stop {
    Cancel,
    /// the top-level items that are left, for the system
    HandOff(Vec<PathBuf>),
}

/// What to do with a step that failed and the person chose to leave.
enum Failed {
    Skip,
    HandOff,
}

struct Run<'a> {
    job: &'a Job,
    ask: &'a dyn Fn(Question) -> Answer,
    /// the answer to every taken name, once the person said to do the same for the others
    rule: Option<Choice>,
    skip_errors: bool,
    conflicts_left: u64,
    /// the size of the top-level item about to be worked on
    item_bytes: u64,
}

#[derive(Default)]
struct Tally {
    files: u64,
    bytes: u64,
    conflicts: u64,
}

impl Run<'_> {
    fn transfer_all(&mut self) -> Result<(), Stop> {
        let job = self.job;
        let moving = job.kind == Kind::Move;
        let Some(dest) = job.dest.clone() else { return Ok(()) };
        let mut todo = Vec::new();
        for src in &job.sources {
            let Some(name) = src.file_name() else { continue };
            if fsx::same_path(src, &dest) || fsx::is_under(&dest, src) {
                job.update(|p| p.failed.push(format!("{}: a folder can't go into itself", name.to_string_lossy())));
                continue;
            }
            let here = src.parent().is_some_and(|p| fsx::same_path(p, &dest));
            if here && moving {
                continue;
            }
            // a copy into its own folder is a second one next to it, as in Explorer
            let target = if here { free_name(&dest, name) } else { dest.join(name) };
            // within one drive a move is a rename, which takes no time however much is inside
            if moving && std::fs::symlink_metadata(&target).is_err() && std::fs::rename(src, &target).is_ok() {
                continue;
            }
            todo.push((src.clone(), target));
        }
        let mut tally = Tally::default();
        let mut sizes = Vec::with_capacity(todo.len());
        for (src, target) in &todo {
            if job.cancelled() {
                return Err(Stop::Cancel);
            }
            let before = tally.bytes;
            count(src, Some(target), true, &mut tally);
            sizes.push(tally.bytes - before);
        }
        self.conflicts_left = tally.conflicts;
        job.update(|p| {
            p.files_total = tally.files;
            p.bytes_total = tally.bytes;
            p.stage = Stage::Working;
        });
        for (i, (src, target)) in todo.iter().enumerate() {
            self.item_bytes = sizes[i];
            match self.transfer(src, target, moving, true) {
                Err(Stop::HandOff(_)) => return Err(Stop::HandOff(todo[i..].iter().map(|(s, _)| s.clone()).collect())),
                other => other?,
            }
        }
        Ok(())
    }

    /// `top`: one of the items the person chose, rather than something inside one.
    fn transfer(&mut self, src: &Path, dst: &Path, moving: bool, top: bool) -> Result<(), Stop> {
        let job = self.job;
        if job.cancelled() {
            return Err(Stop::Cancel);
        }
        let Some(md) = self.attempt(src, || std::fs::symlink_metadata(src))? else { return Ok(()) };
        let is_dir = md.is_dir() && !md.file_type().is_symlink();
        let mut dst = dst.to_path_buf();
        if let Ok(there) = std::fs::symlink_metadata(&dst) {
            let there_dir = there.is_dir() && !there.file_type().is_symlink();
            // a folder goes into a folder of the same name, as in Explorer: only the files inside can clash
            if !(is_dir && there_dir) {
                match self.decide(&dst, &md, &there)? {
                    Choice::Skip => {
                        let mut t = Tally::default();
                        count(src, None, false, &mut t);
                        job.update(|p| {
                            p.skipped += 1;
                            p.files_done += t.files;
                            p.bytes_done += t.bytes;
                        });
                        return Ok(());
                    }
                    Choice::KeepBoth => dst = free_name(dst.parent().unwrap_or(&dst), dst.file_name().unwrap_or_default()),
                    Choice::Replace => {
                        // a read-only file can't be written over
                        let mut perm = there.permissions();
                        if perm.readonly() {
                            #[allow(clippy::permissions_set_readonly_false)]
                            perm.set_readonly(false);
                            let _ = std::fs::set_permissions(&dst, perm);
                        }
                        if moving {
                            let _ = std::fs::remove_file(&dst);
                        }
                    }
                }
            }
        }
        if top {
            *job.in_hand.lock().unwrap() = Some(InHand { src: src.to_path_buf(), dst: Some(dst.clone()), start_bytes: job.bytes_done(), bytes: self.item_bytes });
        }
        if is_dir {
            if std::fs::symlink_metadata(&dst).is_err() {
                // the whole folder in one go when it can be renamed into place
                if moving && std::fs::rename(src, &dst).is_ok() {
                    let mut t = Tally::default();
                    count(&dst, None, false, &mut t);
                    job.update(|p| {
                        p.files_done += t.files;
                        p.bytes_done += t.bytes;
                    });
                    return Ok(());
                }
                if self.attempt(&dst, || std::fs::create_dir(&dst))?.is_none() {
                    return Ok(());
                }
            }
            let Some(entries) = self.attempt(src, || std::fs::read_dir(src).and_then(|d| d.map(|e| e.map(|e| e.file_name())).collect::<std::io::Result<Vec<_>>>()))? else {
                return Ok(());
            };
            for name in entries {
                self.transfer(&src.join(&name), &dst.join(&name), moving, false)?;
            }
            if moving {
                // still there when something inside was skipped, and then it should be
                let _ = std::fs::remove_dir(src);
            }
            return Ok(());
        }
        let name = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        job.update(|p| p.current = name);
        let base = job.bytes_done();
        let link = md.file_type().is_symlink();
        let moved = moving && std::fs::rename(src, &dst).is_ok();
        if !moved {
            let to_dir = link && std::fs::metadata(src).is_ok_and(|m| m.is_dir());
            let copied = self.attempt(src, || copy_file(job, src, &dst, link, to_dir))?;
            if copied.is_some() && moving {
                self.attempt(src, || remove_one(src))?;
            }
        }
        job.update(|p| {
            p.files_done += 1;
            p.bytes_done = base + if link { 0 } else { md.len() };
        });
        Ok(())
    }

    /// What to do about a name that's taken: the rule the person set, or their answer.
    fn decide(&mut self, dst: &Path, from: &std::fs::Metadata, there: &std::fs::Metadata) -> Result<Choice, Stop> {
        let can_replace = !from.is_dir() && !there.is_dir();
        self.conflicts_left = self.conflicts_left.saturating_sub(1);
        if let Some(rule) = self.rule {
            return Ok(if rule == Choice::Replace && !can_replace { Choice::KeepBoth } else { rule });
        }
        let side = |m: &std::fs::Metadata| Side { size: if m.is_dir() { 0 } else { m.len() }, modified: m.modified().ok().map(filetime), is_dir: m.is_dir() };
        let q = Question::Conflict { dst: dst.into(), from: side(from), there: side(there), can_replace, others: self.conflicts_left };
        match (self.ask)(q) {
            Answer::Conflict(c, all) => {
                if all {
                    self.rule = Some(c);
                }
                Ok(if c == Choice::Replace && !can_replace { Choice::KeepBoth } else { c })
            }
            Answer::Cancel => {
                self.job.cancel();
                Err(Stop::Cancel)
            }
            _ => Ok(Choice::Skip),
        }
    }

    /// Runs one step, asking what to do when it fails. None when the person chose to leave it.
    fn attempt<T>(&mut self, path: &Path, mut step: impl FnMut() -> std::io::Result<T>) -> Result<Option<T>, Stop> {
        loop {
            let e = match step() {
                Ok(v) => return Ok(Some(v)),
                Err(e) => e,
            };
            if self.job.cancelled() {
                return Err(Stop::Cancel);
            }
            let message = e.to_string();
            let needs_admin = cfg!(windows) && e.kind() == std::io::ErrorKind::PermissionDenied && self.job.kind != Kind::Delete;
            let answer = if self.skip_errors { Answer::Skip } else { (self.ask)(Question::Error { path: path.into(), message: message.clone(), needs_admin }) };
            let failed = match answer {
                Answer::Retry => continue,
                Answer::SkipAll => {
                    self.skip_errors = true;
                    Failed::Skip
                }
                Answer::HandOff if needs_admin => Failed::HandOff,
                Answer::Cancel => {
                    self.job.cancel();
                    return Err(Stop::Cancel);
                }
                _ => Failed::Skip,
            };
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
            self.job.update(|p| p.failed.push(format!("{name}: {message}")));
            return match failed {
                Failed::Skip => Ok(None),
                Failed::HandOff => Err(Stop::HandOff(Vec::new())),
            };
        }
    }

    fn delete_all(&mut self) -> Result<(), Stop> {
        let job = self.job;
        let mut tally = Tally::default();
        let mut sizes = Vec::with_capacity(job.sources.len());
        for src in &job.sources {
            let before = tally.bytes;
            count(src, None, false, &mut tally);
            sizes.push(tally.bytes - before);
        }
        job.update(|p| {
            p.files_total = tally.files;
            p.bytes_total = tally.bytes;
            p.stage = Stage::Working;
        });
        for (src, bytes) in job.sources.iter().zip(sizes) {
            *job.in_hand.lock().unwrap() = Some(InHand { src: src.clone(), dst: None, start_bytes: job.bytes_done(), bytes });
            self.delete(src)?;
        }
        Ok(())
    }

    fn delete(&mut self, path: &Path) -> Result<(), Stop> {
        let job = self.job;
        if job.cancelled() {
            return Err(Stop::Cancel);
        }
        let Some(md) = self.attempt(path, || std::fs::symlink_metadata(path))? else { return Ok(()) };
        if md.is_dir() && !md.file_type().is_symlink() {
            let Some(entries) = self.attempt(path, || std::fs::read_dir(path).and_then(|d| d.map(|e| e.map(|e| e.path())).collect::<std::io::Result<Vec<_>>>()))? else {
                return Ok(());
            };
            for e in entries {
                self.delete(&e)?;
            }
            // left when something inside was skipped
            if std::fs::read_dir(path).is_ok_and(|mut d| d.next().is_none()) {
                self.attempt(path, || std::fs::remove_dir(path))?;
            }
            return Ok(());
        }
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        job.update(|p| p.current = name);
        self.attempt(path, || remove_one(path))?;
        job.update(|p| {
            p.files_done += 1;
            p.bytes_done += md.len();
        });
        Ok(())
    }
}

/// Adds up the files and bytes under `path`, and, when it goes to `dst`, how many of those names are taken there.
/// `dst_known` is false once a folder on the way doesn't exist there, so nothing below it can clash.
fn count(path: &Path, dst: Option<&Path>, dst_known: bool, t: &mut Tally) {
    let Ok(md) = std::fs::symlink_metadata(path) else { return };
    let there = dst.filter(|_| dst_known).and_then(|d| std::fs::symlink_metadata(d).ok());
    if md.is_dir() && !md.file_type().is_symlink() {
        let merge = there.as_ref().is_some_and(|m| m.is_dir());
        if there.is_some() && !merge {
            t.conflicts += 1;
        }
        if let Ok(rd) = std::fs::read_dir(path) {
            for e in rd.flatten() {
                let child = dst.map(|d| d.join(e.file_name()));
                count(&e.path(), child.as_deref(), merge, t);
            }
        }
        return;
    }
    t.files += 1;
    if !md.file_type().is_symlink() {
        t.bytes += md.len();
    }
    if there.is_some() {
        t.conflicts += 1;
    }
}

/// "name (2).ext" in `dir`, or the next number that's free, as Explorer names a second one.
pub fn free_name(dir: &Path, name: &std::ffi::OsStr) -> PathBuf {
    let first = dir.join(name);
    if std::fs::symlink_metadata(&first).is_err() {
        return first;
    }
    let is_dir = std::fs::metadata(&first).is_ok_and(|m| m.is_dir());
    let p = Path::new(name);
    let (stem, ext) = match (p.file_stem(), p.extension()) {
        (Some(s), Some(e)) if !is_dir => (s.to_string_lossy().into_owned(), format!(".{}", e.to_string_lossy())),
        _ => (name.to_string_lossy().into_owned(), String::new()),
    };
    (2..).map(|k| dir.join(format!("{stem} ({k}){ext}"))).find(|c| std::fs::symlink_metadata(c).is_err()).unwrap()
}

fn filetime(t: SystemTime) -> u64 {
    let d = t.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
    crate::fscommon::unix_secs_to_filetime(d.as_secs() as i64, d.subsec_nanos())
}

/// Removes one file or link, also when it's read-only. A link to a folder goes, never what it points to.
fn remove_one(path: &Path) -> std::io::Result<()> {
    let first = std::fs::remove_file(path).or_else(|e| if cfg!(windows) && path.is_dir() { std::fs::remove_dir(path) } else { Err(e) });
    match first {
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            let mut perm = std::fs::symlink_metadata(path)?.permissions();
            if !perm.readonly() {
                return Err(e);
            }
            #[allow(clippy::permissions_set_readonly_false)]
            perm.set_readonly(false);
            std::fs::set_permissions(path, perm)?;
            std::fs::remove_file(path)
        }
        other => other,
    }
}

/// Copies one file with its times and attributes, reporting bytes as they go. A link is copied as a link.
#[cfg(windows)]
fn copy_file(job: &Job, src: &Path, dst: &Path, link: bool, link_to_dir: bool) -> std::io::Result<()> {
    use std::ffi::c_void;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::*;
    use windows::core::PCWSTR;

    if link && link_to_dir {
        return std::os::windows::fs::symlink_dir(std::fs::read_link(src)?, dst);
    }
    struct Tick<'a> {
        job: &'a Job,
        base: u64,
    }
    unsafe extern "system" fn tick(_: i64, done: i64, _: i64, _: i64, _: u32, _: LPPROGRESS_ROUTINE_CALLBACK_REASON, _: HANDLE, _: HANDLE, data: *const c_void) -> COPYPROGRESSROUTINE_PROGRESS {
        let t = unsafe { &*(data as *const Tick) };
        t.job.update(|p| p.bytes_done = t.base + done as u64);
        if t.job.cancelled() { PROGRESS_CANCEL } else { PROGRESS_CONTINUE }
    }
    let t = Tick { job, base: job.bytes_done() };
    let (s, d) = (crate::util::wide(src), crate::util::wide(dst));
    let flags = if link { COPY_FILE_COPY_SYMLINK } else { COPYFILE_FLAGS(0) };
    unsafe { CopyFileExW(PCWSTR(s.as_ptr()), PCWSTR(d.as_ptr()), Some(tick), Some(&t as *const Tick as *const c_void), None, flags) }
        .map_err(|e| std::io::Error::from_raw_os_error((e.code().0 as u32 & 0xFFFF) as i32))
}

#[cfg(not(windows))]
fn copy_file(job: &Job, src: &Path, dst: &Path, link: bool, _link_to_dir: bool) -> std::io::Result<()> {
    use std::io::{Read, Write};
    if link {
        return std::os::unix::fs::symlink(std::fs::read_link(src)?, dst);
    }
    let mut from = std::fs::File::open(src)?;
    let md = from.metadata()?;
    let mut to = std::fs::File::create(dst)?;
    let base = job.bytes_done();
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    loop {
        if job.cancelled() {
            drop(to);
            let _ = std::fs::remove_file(dst);
            return Err(std::io::ErrorKind::Interrupted.into());
        }
        let n = from.read(&mut buf)?;
        if n == 0 {
            break;
        }
        to.write_all(&buf[..n])?;
        done += n as u64;
        job.update(|p| p.bytes_done = base + done);
    }
    to.set_permissions(md.permissions())?;
    if let Ok(m) = md.modified() {
        let _ = to.set_modified(m);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("daleris-fileops-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("from/sub")).unwrap();
        std::fs::create_dir_all(d.join("to")).unwrap();
        std::fs::write(d.join("from/a.txt"), "new a").unwrap();
        std::fs::write(d.join("from/sub/b.txt"), "new b").unwrap();
        d
    }

    fn read(p: PathBuf) -> String {
        std::fs::read_to_string(p).unwrap_or_default()
    }

    /// Runs a job, answering every question with the next of `answers`, and returns the questions it asked.
    fn run(kind: Kind, sources: Vec<PathBuf>, dest: Option<PathBuf>, answers: &[Answer]) -> (Progress, Vec<Question>) {
        let job = Job::new(kind, sources, dest);
        let asked = RefCell::new(Vec::new());
        job.run(&|q| {
            let mut a = asked.borrow_mut();
            a.push(q);
            answers.get(a.len() - 1).copied().unwrap_or(Answer::Cancel)
        });
        (job.progress(), asked.into_inner())
    }

    #[test]
    fn copies_a_folder_with_everything_in_it() {
        let d = scratch("copy");
        let (p, asked) = run(Kind::Copy, vec![d.join("from")], Some(d.join("to")), &[]);
        assert!(asked.is_empty());
        assert_eq!((p.stage, p.files_total, p.files_done, p.bytes_done), (Stage::Done, 2, 2, 10));
        assert_eq!(read(d.join("to/from/sub/b.txt")), "new b");
        assert_eq!(read(d.join("from/a.txt")), "new a", "a copy leaves the original");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_taken_name_is_asked_about() {
        for (choice, a, kept) in [(Choice::Replace, "new a", None), (Choice::Skip, "old a", None), (Choice::KeepBoth, "old a", Some("a (2).txt"))] {
            let d = scratch("conflict");
            std::fs::write(d.join("to/a.txt"), "old a").unwrap();
            let (p, asked) = run(Kind::Copy, vec![d.join("from/a.txt")], Some(d.join("to")), &[Answer::Conflict(choice, false)]);
            assert!(matches!(&asked[..], [Question::Conflict { can_replace: true, others: 0, .. }]), "{asked:?}");
            assert_eq!(read(d.join("to/a.txt")), a, "{choice:?}");
            if let Some(k) = kept {
                assert_eq!(read(d.join("to").join(k)), "new a");
            }
            assert_eq!(p.skipped, (choice == Choice::Skip) as u64);
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn folders_merge_and_one_answer_can_cover_the_rest() {
        let d = scratch("merge");
        std::fs::create_dir_all(d.join("to/from/sub")).unwrap();
        std::fs::write(d.join("to/from/a.txt"), "old a").unwrap();
        std::fs::write(d.join("to/from/sub/b.txt"), "old b").unwrap();
        std::fs::write(d.join("to/from/keep.txt"), "mine").unwrap();
        let (_, asked) = run(Kind::Copy, vec![d.join("from")], Some(d.join("to")), &[Answer::Conflict(Choice::Replace, true)]);
        assert!(matches!(&asked[..], [Question::Conflict { others: 1, .. }]), "asked once, knowing one more is coming: {asked:?}");
        assert_eq!((read(d.join("to/from/a.txt")), read(d.join("to/from/sub/b.txt"))), ("new a".into(), "new b".into()));
        assert_eq!(read(d.join("to/from/keep.txt")), "mine", "a merge leaves what was there");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_copy_into_its_own_folder_is_a_second_one() {
        let d = scratch("same");
        let (_, asked) = run(Kind::Copy, vec![d.join("from/a.txt"), d.join("from/sub")], Some(d.join("from")), &[]);
        assert!(asked.is_empty());
        assert_eq!(read(d.join("from/a (2).txt")), "new a");
        assert_eq!(read(d.join("from/sub (2)/b.txt")), "new b");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_move_takes_everything_but_what_was_skipped() {
        let d = scratch("move");
        std::fs::create_dir_all(d.join("to/from")).unwrap();
        std::fs::write(d.join("to/from/a.txt"), "old a").unwrap();
        let (p, _) = run(Kind::Move, vec![d.join("from")], Some(d.join("to")), &[Answer::Conflict(Choice::Skip, false)]);
        assert_eq!(read(d.join("to/from/sub/b.txt")), "new b");
        assert_eq!(read(d.join("to/from/a.txt")), "old a");
        assert_eq!(read(d.join("from/a.txt")), "new a", "the skipped file stays where it was");
        assert!(!d.join("from/sub").exists(), "a folder that's moved entirely goes");
        assert_eq!(p.skipped, 1);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_folder_never_goes_into_itself() {
        let d = scratch("itself");
        let (p, asked) = run(Kind::Copy, vec![d.join("from")], Some(d.join("from").join("sub")), &[]);
        assert!(asked.is_empty());
        assert_eq!(p.failed.len(), 1);
        assert!(!d.join("from/sub/from").exists());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn deletes_for_good() {
        let d = scratch("delete");
        let (p, _) = run(Kind::Delete, vec![d.join("from")], None, &[]);
        assert!(!d.join("from").exists());
        assert_eq!((p.files_total, p.files_done), (2, 2));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_cancelled_job_stops_and_says_so() {
        let d = scratch("cancel");
        std::fs::write(d.join("to/a.txt"), "old a").unwrap();
        let (p, _) = run(Kind::Copy, vec![d.join("from/a.txt"), d.join("from/sub")], Some(d.join("to")), &[Answer::Cancel]);
        assert!(p.cancelled);
        assert!(!d.join("to/sub").exists(), "nothing after the cancel is copied");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_failed_step_can_be_skipped() {
        let d = scratch("fail");
        let (p, asked) = run(Kind::Copy, vec![d.join("from/missing.txt"), d.join("from/a.txt")], Some(d.join("to")), &[Answer::Skip]);
        assert!(matches!(&asked[..], [Question::Error { .. }]), "{asked:?}");
        assert_eq!(p.failed.len(), 1);
        assert_eq!(read(d.join("to/a.txt")), "new a", "the rest still happens");
        let _ = std::fs::remove_dir_all(d);
    }
}
