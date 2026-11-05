//! Background work that may hang on a path nobody is answering for.
//!
//! A look at a dead server can take the system 20 seconds or more, and nothing can interrupt it. So work is sorted
//! into lanes, one per server or drive. A lane only runs a few jobs at once and queues the rest, however often someone
//! tries again, so a dead drive holds up its own lane and nothing else. A job that panics is logged and costs nothing
//! but itself.

use std::collections::{HashMap, VecDeque};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::mpsc::channel;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

/// How many jobs of one lane run at the same time.
const PER_LANE: usize = 3;
/// How many wait. The oldest go first: nobody is waiting for those any more.
const QUEUED: usize = 64;

type Job = Box<dyn FnOnce() + Send + 'static>;

#[derive(Default)]
struct Lane {
    running: usize,
    queue: VecDeque<Job>,
}

static LANES: Mutex<Option<HashMap<String, Lane>>> = Mutex::new(None);

/// The lane of a path: everything on one server or drive shares a fate.
pub fn lane_of(path: &Path) -> String {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\").or_else(|| s.strip_prefix("//")) {
        let server = rest.split(['\\', '/']).next().unwrap_or("");
        return format!(r"\\{}", server.to_lowercase());
    }
    if s.as_bytes().get(1) == Some(&b':') {
        return s[..2].to_lowercase();
    }
    // Linux and macOS: the mount it's on, when that's a network one
    crate::fscommon::network_root_of(path).map_or_else(|| "/".to_string(), |r| r.to_string_lossy().into_owned())
}

/// Runs `job` on a thread of its own, which never takes the app with it. False if the system has no thread to give.
pub fn spawn(name: &str, job: impl FnOnce() + Send + 'static) -> bool {
    let done = std::thread::Builder::new().name(name.to_string()).spawn(move || run(job));
    if let Err(e) = &done {
        log::error!("couldn't start a thread for {name}: {e}");
    }
    done.is_ok()
}

fn run(job: impl FnOnce()) {
    // the panic hook has already written it to the log
    let _ = catch_unwind(AssertUnwindSafe(job));
}

/// Queues `job` on the lane of `path`.
pub fn submit(path: &Path, name: &str, job: impl FnOnce() + Send + 'static) {
    submit_on(lane_of(path), name, Box::new(job));
}

fn submit_on(lane: String, name: &str, job: Job) {
    {
        let mut all = LANES.lock().unwrap_or_else(PoisonError::into_inner);
        let l = all.get_or_insert_with(HashMap::new).entry(lane.clone()).or_default();
        if l.running >= PER_LANE {
            if l.queue.len() >= QUEUED {
                l.queue.pop_front();
            }
            l.queue.push_back(job);
            return;
        }
        l.running += 1;
    }
    let key = lane.clone();
    let started = spawn(name, move || {
        run(job);
        // then whatever waited for this lane
        loop {
            let next = {
                let mut all = LANES.lock().unwrap_or_else(PoisonError::into_inner);
                let l = all.get_or_insert_with(HashMap::new).entry(key.clone()).or_default();
                let next = l.queue.pop_back();
                if next.is_none() {
                    l.running -= 1;
                }
                next
            };
            match next {
                Some(job) => run(job),
                None => return,
            }
        }
    });
    if !started {
        let mut all = LANES.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(l) = all.get_or_insert_with(HashMap::new).get_mut(&lane) {
            l.running -= 1;
        }
    }
}

/// The result of `job`, if it's done within `wait`. It goes on regardless, and its result is dropped.
/// For the few things the window has to know before it can go on: at worst it loses `wait`.
pub fn within<T: Send + 'static>(path: &Path, wait: Duration, job: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, rx) = channel();
    submit(path, "within", move || {
        let _ = tx.send(job());
    });
    rx.recv_timeout(wait).ok()
}

/// How many jobs of a lane are running and waiting.
#[cfg(test)]
fn load(lane: &str) -> (usize, usize) {
    let all = LANES.lock().unwrap_or_else(PoisonError::into_inner);
    all.as_ref().and_then(|m| m.get(lane)).map_or((0, 0), |l| (l.running, l.queue.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Condvar};
    use std::time::Instant;

    #[test]
    fn everything_on_one_server_shares_a_lane() {
        assert_eq!(lane_of(Path::new(r"\\Server\share\a")), lane_of(Path::new(r"\\server\other")));
        assert_ne!(lane_of(Path::new(r"\\server\share")), lane_of(Path::new(r"\\other\share")));
        assert_eq!(lane_of(Path::new(r"Z:\a\b")), "z:");
    }

    #[test]
    fn a_stuck_lane_stays_bounded_and_holds_up_nothing_else() {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let ran = Arc::new(AtomicUsize::new(0));
        let stuck = Path::new(r"\\stuck-test\share");
        // far more attempts than anyone would make: they must not become threads
        for _ in 0..500 {
            let (gate, ran) = (gate.clone(), ran.clone());
            submit(stuck, "test", move || {
                let mut open = gate.0.lock().unwrap();
                while !*open {
                    open = gate.1.wait(open).unwrap();
                }
                ran.fetch_add(1, Ordering::SeqCst);
            });
        }
        let (running, queued) = load(r"\\stuck-test");
        assert!(running <= PER_LANE, "{running} running");
        assert!(queued <= QUEUED, "{queued} queued");
        // another drive doesn't notice
        let other = within(Path::new(r"\\fine-test\share"), Duration::from_secs(5), || 7);
        assert_eq!(other, Some(7));
        // and within gives up on the stuck one in time
        let start = Instant::now();
        assert_eq!(within(stuck, Duration::from_millis(50), || 1), None);
        assert!(start.elapsed() < Duration::from_secs(2));
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        let start = Instant::now();
        while load(r"\\stuck-test").0 > 0 && start.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(load(r"\\stuck-test"), (0, 0));
        assert!(ran.load(Ordering::SeqCst) > 0);
    }

    #[test]
    fn a_job_that_panics_costs_only_itself() {
        let lane = Path::new(r"\\panic-test\share");
        for _ in 0..(PER_LANE * 3) {
            submit(lane, "test", || panic!("on purpose"));
        }
        assert_eq!(within(lane, Duration::from_secs(5), || 3), Some(3));
    }
}
