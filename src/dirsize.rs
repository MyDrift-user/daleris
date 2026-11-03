//! Folder sizes: the total of everything inside a folder, counted in the background.
//! Requests are served newest first (the folders on screen), progress is reported
//! while a big folder is still being counted, and results are remembered for a while.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::fsx::{for_each_raw, A_DIR, A_REPARSE};

/// How long a counted size is trusted before it is counted again.
const FRESH_FOR: Duration = Duration::from_secs(120);

#[derive(Clone, Copy, Debug)]
pub enum DirSize {
    /// waiting to be counted
    Queued,
    /// still counting: what has been found so far
    Counting(u64),
    Done(u64),
}

struct Slot {
    size: DirSize,
    at: Instant,
}

struct Inner {
    cache: Mutex<HashMap<String, Slot>>,
    queue: Mutex<Vec<PathBuf>>,
    wake: Condvar,
    /// set when any size changed, so the list can re-sort
    changed: AtomicBool,
    ctx: eframe::egui::Context,
}

#[derive(Clone)]
pub struct DirSizer {
    inner: Arc<Inner>,
}

fn key(p: &Path) -> String {
    let s = p.to_string_lossy().trim_end_matches(std::path::MAIN_SEPARATOR).to_string();
    if cfg!(any(windows, target_os = "macos")) {
        s.to_lowercase()
    } else {
        s
    }
}

impl DirSizer {
    pub fn new(ctx: eframe::egui::Context) -> DirSizer {
        let inner = Arc::new(Inner { cache: Mutex::new(HashMap::new()), queue: Mutex::new(Vec::new()), wake: Condvar::new(), changed: AtomicBool::new(false), ctx });
        for i in 0..3 {
            let inner = inner.clone();
            std::thread::Builder::new().name(format!("dirsize{i}")).spawn(move || worker(&inner)).ok();
        }
        DirSizer { inner }
    }

    /// The size of `dir` if known (or being counted); asks for it to be counted otherwise.
    pub fn get(&self, dir: &Path) -> DirSize {
        let k = key(dir);
        {
            let cache = self.inner.cache.lock().unwrap();
            if let Some(s) = cache.get(&k) {
                let stale = matches!(s.size, DirSize::Done(_)) && s.at.elapsed() > FRESH_FOR;
                if !stale {
                    return s.size;
                }
            }
        }
        self.inner.cache.lock().unwrap().insert(k, Slot { size: DirSize::Queued, at: Instant::now() });
        self.inner.queue.lock().unwrap().push(dir.to_path_buf());
        self.inner.wake.notify_one();
        DirSize::Queued
    }

    /// The finished size, without asking for a count (for sorting and totals).
    pub fn known(&self, dir: &Path) -> Option<u64> {
        match self.inner.cache.lock().unwrap().get(&key(dir))?.size {
            DirSize::Done(b) => Some(b),
            _ => None,
        }
    }

    /// Forget sizes at or inside `dir` (after a refresh or a file operation).
    pub fn invalidate_under(&self, dir: &Path) {
        let k = key(dir);
        let sep = std::path::MAIN_SEPARATOR;
        self.inner.cache.lock().unwrap().retain(|p, s| {
            let inside = *p == k || p.starts_with(&format!("{k}{sep}")) || k.starts_with(&format!("{p}{sep}"));
            !inside || !matches!(s.size, DirSize::Done(_))
        });
    }

    /// Forget everything.
    pub fn clear(&self) {
        self.inner.cache.lock().unwrap().retain(|_, s| !matches!(s.size, DirSize::Done(_)));
    }

    /// Remember that something changed (to be picked up later).
    pub fn mark_changed(&self) {
        self.inner.changed.store(true, Ordering::Release);
    }

    /// True once after any size changed.
    pub fn take_changed(&self) -> bool {
        self.inner.changed.swap(false, Ordering::AcqRel)
    }
}

fn worker(inner: &Inner) {
    loop {
        let dir = {
            let mut q = inner.queue.lock().unwrap();
            while q.is_empty() {
                q = inner.wake.wait(q).unwrap();
            }
            q.pop().unwrap() // newest first: what is on screen now
        };
        let k = key(&dir);
        // another worker may have taken it already
        {
            let mut cache = inner.cache.lock().unwrap();
            match cache.get(&k).map(|s| s.size) {
                Some(DirSize::Queued) => {
                    cache.insert(k.clone(), Slot { size: DirSize::Counting(0), at: Instant::now() });
                }
                _ => continue,
            }
        }
        let mut bytes = 0u64;
        let mut stack = vec![dir];
        let mut last = Instant::now();
        while let Some(d) = stack.pop() {
            for_each_raw(&d, |e| {
                if e.attrs & A_DIR != 0 {
                    // never follow links/junctions: no loops, no double counting
                    if e.attrs & A_REPARSE == 0 {
                        stack.push(d.join(e.name.as_ref()));
                    }
                } else {
                    bytes += e.size;
                }
            });
            if last.elapsed() > Duration::from_millis(250) {
                last = Instant::now();
                let mut cache = inner.cache.lock().unwrap();
                match cache.get_mut(&k) {
                    Some(s) => s.size = DirSize::Counting(bytes),
                    None => break, // forgotten meanwhile: stop counting
                }
                drop(cache);
                inner.ctx.request_repaint();
            }
        }
        if let Some(s) = inner.cache.lock().unwrap().get_mut(&k) {
            *s = Slot { size: DirSize::Done(bytes), at: Instant::now() };
        }
        inner.changed.store(true, Ordering::Release);
        inner.ctx.request_repaint();
    }
}
