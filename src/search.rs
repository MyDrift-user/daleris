//! Search: a parallel crawler builds a compact in-memory index (names in one arena,
//! parent links instead of full paths) and queries scan it on all cores. The first
//! search of a folder streams matches while crawling; every later keystroke re-queries
//! the finished index in milliseconds.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::{Duration, Instant};

use crate::fsx::{for_each_raw, A_DIR, A_HIDDEN, A_REPARSE, A_SYSTEM};
use crate::util::*;

pub const NO_PARENT: u32 = u32::MAX;

#[derive(Clone, Copy)]
pub struct Item {
    pub name_off: u32,
    pub lower_off: u32,
    pub name_len: u16,
    pub lower_len: u16,
    pub parent: u32,
    pub attrs: u32,
    pub size: u64,
    pub mtime: u64,
}

#[derive(Default)]
pub struct Data {
    pub names: String,
    pub lower: String,
    pub items: Vec<Item>,
}

impl Data {
    pub fn name(&self, i: u32) -> &str {
        let it = &self.items[i as usize];
        &self.names[it.name_off as usize..it.name_off as usize + it.name_len as usize]
    }
    fn lower(&self, it: &Item) -> &[u8] {
        &self.lower.as_bytes()[it.lower_off as usize..it.lower_off as usize + it.lower_len as usize]
    }
    /// Is it somewhere inside a folder whose name starts with a dot? The roots of a search don't count: if that's
    /// where the search was started, that's where the person wants to look.
    pub fn under_dot_folder(&self, i: u32) -> bool {
        let mut cur = self.items[i as usize].parent;
        while cur != NO_PARENT {
            let it = &self.items[cur as usize];
            if it.parent != NO_PARENT && self.lower(it).first() == Some(&b'.') {
                return true;
            }
            cur = it.parent;
        }
        false
    }

    pub fn full_path(&self, i: u32) -> PathBuf {
        let mut chain = Vec::with_capacity(16);
        let mut cur = i;
        while cur != NO_PARENT {
            chain.push(cur);
            cur = self.items[cur as usize].parent;
        }
        let mut p = PathBuf::new();
        for &c in chain.iter().rev() {
            p.push(self.name(c));
        }
        p
    }
    fn push(&mut self, name: &str, parent: u32, attrs: u32, size: u64, mtime: u64) -> u32 {
        let name_off = self.names.len() as u32;
        self.names.push_str(name);
        let lower_off = self.lower.len() as u32;
        let before = self.lower.len();
        for c in name.chars() {
            if c.is_ascii() {
                self.lower.push(c.to_ascii_lowercase());
            } else {
                self.lower.extend(c.to_lowercase());
            }
        }
        let idx = self.items.len() as u32;
        self.items.push(Item {
            name_off,
            lower_off,
            name_len: name.len().min(u16::MAX as usize) as u16,
            lower_len: (self.lower.len() - before).min(u16::MAX as usize) as u16,
            parent,
            attrs,
            size,
            mtime,
        });
        idx
    }
}

pub struct Index {
    pub roots: Vec<PathBuf>,
    pub data: RwLock<Data>,
    pub len: AtomicUsize,
    pub done: AtomicBool,
    pub cancel: AtomicBool,
    pub started: Instant,
    pub took: Mutex<Option<Duration>>,
    pub dirs: AtomicUsize,
}

impl Index {
    pub fn build(roots: Vec<PathBuf>, ctx: eframe::egui::Context) -> Arc<Index> {
        let idx = Arc::new(Index {
            roots: roots.clone(),
            data: RwLock::new(Data::default()),
            len: AtomicUsize::new(0),
            done: AtomicBool::new(false),
            cancel: AtomicBool::new(false),
            started: Instant::now(),
            took: Mutex::new(None),
            dirs: AtomicUsize::new(0),
        });
        let mut queue = Vec::new();
        {
            let mut d = idx.data.write().unwrap();
            for r in &roots {
                let name = r.to_string_lossy().trim_end_matches('\\').to_string();
                let name = if name.ends_with(':') { format!("{name}\\") } else { name };
                let i = d.push(&name, NO_PARENT, A_DIR, 0, 0);
                queue.push((i, r.clone()));
            }
            idx.len.store(d.items.len(), Ordering::Release);
        }
        let shared = Arc::new((Mutex::new((queue, 0usize)), Condvar::new()));
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 16);
        let finished = Arc::new(AtomicUsize::new(0));
        for t in 0..threads {
            let idx = idx.clone();
            let shared = shared.clone();
            let finished = finished.clone();
            let ctx = ctx.clone();
            std::thread::Builder::new()
                .name(format!("crawl{t}"))
                .spawn(move || {
                    crawl_worker(&idx, &shared);
                    if finished.fetch_add(1, Ordering::AcqRel) + 1 == threads {
                        *idx.took.lock().unwrap() = Some(idx.started.elapsed());
                        idx.done.store(true, Ordering::Release);
                        ctx.request_repaint();
                    }
                })
                .ok();
        }
        idx
    }

    pub fn is_done(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }
    pub fn count(&self) -> usize {
        self.len.load(Ordering::Acquire)
    }
}

type Shared = Arc<(Mutex<(Vec<(u32, PathBuf)>, usize)>, Condvar)>;

fn crawl_worker(idx: &Index, shared: &Shared) {
    struct Raw {
        name: String,
        attrs: u32,
        size: u64,
        mtime: u64,
    }
    let mut local: Vec<Raw> = Vec::with_capacity(256);
    loop {
        let job = {
            let (lock, cv) = &**shared;
            let mut g = lock.lock().unwrap();
            loop {
                if idx.cancel.load(Ordering::Relaxed) {
                    g.0.clear();
                }
                if let Some(j) = g.0.pop() {
                    g.1 += 1;
                    break Some(j);
                }
                if g.1 == 0 {
                    cv.notify_all();
                    break None;
                }
                g = cv.wait(g).unwrap();
            }
        };
        let Some((dir_idx, path)) = job else { return };
        local.clear();
        for_each_raw(&path, |d| {
            local.push(Raw { name: d.name.into_owned(), attrs: d.attrs, size: d.size, mtime: d.mtime });
        });
        idx.dirs.fetch_add(1, Ordering::Relaxed);
        let mut subdirs = Vec::new();
        if !local.is_empty() {
            let mut d = idx.data.write().unwrap();
            for r in &local {
                let i = d.push(&r.name, dir_idx, r.attrs, r.size, r.mtime);
                // don't follow junctions/symlinks (loops, duplicate trees)
                if r.attrs & A_DIR != 0 && r.attrs & A_REPARSE == 0 {
                    subdirs.push((i, path.join(&r.name)));
                }
            }
            idx.len.store(d.items.len(), Ordering::Release);
        }
        let (lock, cv) = &**shared;
        let mut g = lock.lock().unwrap();
        g.0.extend(subdirs);
        g.1 -= 1;
        cv.notify_all();
    }
}

// ---------------------------------------------------------------- query language

#[derive(Clone, Debug)]
enum Term {
    Sub(Vec<u8>, bool),
    Glob(Vec<u8>, bool),
    Exact(Vec<u8>),
    Ext(Vec<String>),
    Kind(Kind),
    SizeMin(u64),
    SizeMax(u64),
    After(u64),
    Before(u64),
    Hidden(bool),
    Regex(regex::bytes::Regex, bool),
    /// an invalid regex: matches nothing (the error is shown to the person)
    Never,
}

/// Parsed query.
///
/// `report final`: names containing both words · `*.rs` / `img_??.png`: wildcards ·
/// `"exact name.txt"` · `-draft`: exclude · `ext:pdf,docx` · `kind:image|video|audio|doc|code|archive|app|folder` ·
/// `size:>10mb` `size:<1k` `size:1mb..1gb` · `modified:today|yesterday|week|month|year|>2026-01-01|2025` ·
/// `hidden:no` · `re:^img_\d+` or `/^img_\d+/`: regular expression (case-insensitive)
#[derive(Clone, Debug, Default)]
pub struct Query {
    terms: Vec<Term>,
    pub highlight: Vec<String>,
    /// why a regular expression couldn't be used
    pub error: Option<String>,
    /// leave out names that start with a dot, and what's inside folders that do
    pub hide_dots: bool,
    /// what the filter panel asks for
    pub extra: Option<crate::listing::CompiledFilter>,
}

impl Query {
    pub fn parse(q: &str) -> Query {
        let mut out = Query::default();
        for tok in tokenize(q) {
            let (neg, t) = match tok.strip_prefix('-') {
                Some(r) if !r.is_empty() => (true, r.to_string()),
                _ => (false, tok.clone()),
            };
            if let Some(p) = regex_token(&t) {
                out.push_regex(p, neg);
                continue;
            }
            let lower = t.to_lowercase();
            if let Some((k, v)) = lower.split_once(':') {
                if let Some(terms) = parse_filter(k, v) {
                    out.terms.extend(terms);
                    continue;
                }
            }
            if tok.starts_with('"') {
                let inner = lower.trim_matches('"').to_string();
                out.highlight.push(inner.clone());
                out.terms.push(Term::Exact(inner.into_bytes()));
            } else if lower.contains('*') || lower.contains('?') {
                out.terms.push(Term::Glob(lower.into_bytes(), neg));
            } else if !lower.is_empty() {
                if !neg {
                    out.highlight.push(lower.clone());
                }
                out.terms.push(Term::Sub(lower.into_bytes(), neg));
            }
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// Regex mode: the whole search text is one regular expression.
    pub fn regex_whole(q: &str) -> Query {
        let mut out = Query::default();
        let p = q.trim();
        if !p.is_empty() {
            out.push_regex(p, false);
        }
        out
    }

    fn push_regex(&mut self, pattern: &str, neg: bool) {
        match regex::bytes::RegexBuilder::new(pattern).case_insensitive(true).size_limit(1 << 22).build() {
            Ok(r) => self.terms.push(Term::Regex(r, neg)),
            Err(e) => {
                let msg = e.to_string();
                let last = msg.lines().filter(|l| !l.trim().is_empty()).last().unwrap_or("invalid pattern").trim().trim_start_matches("error: ").to_string();
                self.error = Some(last);
                self.terms.push(Term::Never);
            }
        }
    }

    pub fn matches(&self, lower_name: &[u8], attrs: u32, size: u64, mtime: u64) -> bool {
        let is_dir = attrs & A_DIR != 0;
        for t in &self.terms {
            let ok = match t {
                Term::Sub(s, neg) => contains(lower_name, s) != *neg,
                Term::Glob(g, neg) => glob_match(g, lower_name) != *neg,
                Term::Exact(s) => lower_name == s.as_slice(),
                Term::Ext(list) => {
                    !is_dir
                        && match lower_name.iter().rposition(|&c| c == b'.') {
                            Some(p) => list.iter().any(|e| e.as_bytes() == &lower_name[p + 1..]),
                            None => false,
                        }
                }
                Term::Kind(k) => {
                    let ext = match lower_name.iter().rposition(|&c| c == b'.') {
                        Some(p) if !is_dir => std::str::from_utf8(&lower_name[p + 1..]).unwrap_or(""),
                        _ => "",
                    };
                    kind_of(ext, is_dir) == *k
                }
                Term::SizeMin(n) => !is_dir && size >= *n,
                Term::SizeMax(n) => !is_dir && size <= *n,
                Term::After(t) => mtime >= *t,
                Term::Before(t) => mtime < *t,
                Term::Hidden(want) => (attrs & (A_HIDDEN | A_SYSTEM) != 0) == *want,
                Term::Regex(r, neg) => r.is_match(lower_name) != *neg,
                Term::Never => false,
            };
            if !ok {
                return false;
            }
        }
        if self.hide_dots && lower_name.first() == Some(&b'.') {
            return false;
        }
        if let Some(f) = &self.extra {
            let name = std::str::from_utf8(lower_name).unwrap_or("");
            let ext = if is_dir { "" } else { name.rsplit_once('.').map_or("", |(_, e)| e) };
            return f.matches(name, ext, is_dir, size, mtime);
        }
        true
    }
}

/// `re:pattern`, `regex:pattern` (quotes allowed) or `/pattern/`.
fn regex_token(t: &str) -> Option<&str> {
    let lower = t.to_ascii_lowercase();
    for p in ["re:", "regex:"] {
        if lower.starts_with(p) {
            return Some(t[p.len()..].trim_matches('"')).filter(|s| !s.is_empty());
        }
    }
    (t.len() >= 3 && t.starts_with('/') && t.ends_with('/')).then(|| &t[1..t.len() - 1])
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    if needle.len() > hay.len() {
        return false;
    }
    if needle.len() == 1 {
        return hay.contains(&needle[0]);
    }
    let first = needle[0];
    let last = hay.len() - needle.len();
    let mut i = 0;
    while i <= last {
        match hay[i..=last].iter().position(|&c| c == first) {
            None => return false,
            Some(p) => {
                i += p;
                if &hay[i..i + needle.len()] == needle {
                    return true;
                }
                i += 1;
            }
        }
    }
    false
}

fn tokenize(q: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    for c in q.chars() {
        match c {
            '"' => {
                cur.push(c);
                in_q = !in_q;
            }
            c if c.is_whitespace() && !in_q => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn parse_filter(k: &str, v: &str) -> Option<Vec<Term>> {
    Some(match k {
        "ext" | "type" => vec![Term::Ext(
            v.split([',', '|', ';']).map(|s| s.trim_start_matches('.').to_string()).filter(|s| !s.is_empty()).collect(),
        )],
        "kind" | "is" => vec![Term::Kind(kind_from_word(v)?)],
        "hidden" => vec![Term::Hidden(matches!(v, "yes" | "true" | "1"))],
        "size" => {
            if let Some((a, b)) = v.split_once("..") {
                vec![Term::SizeMin(parse_size(a)?), Term::SizeMax(parse_size(b)?)]
            } else if let Some(r) = v.strip_prefix(">=").or_else(|| v.strip_prefix('>')) {
                vec![Term::SizeMin(parse_size(r)?)]
            } else if let Some(r) = v.strip_prefix("<=").or_else(|| v.strip_prefix('<')) {
                vec![Term::SizeMax(parse_size(r)?)]
            } else {
                vec![Term::SizeMin(parse_size(v)?)]
            }
        }
        "modified" | "date" | "dm" | "mod" => parse_date(v)?,
        _ => return None,
    })
}

impl Query {
    pub fn new(q: &str) -> Query {
        Query::parse(q)
    }

    /// Hide hidden/system items unless the query asks about them explicitly.
    pub fn without_hidden(mut self) -> Query {
        if !self.terms.iter().any(|t| matches!(t, Term::Hidden(_))) {
            self.terms.push(Term::Hidden(false));
        }
        self
    }
}

fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(s.len());
    let n: f64 = s[..split].parse().ok()?;
    let mul = match s[split..].trim() {
        "" | "b" => 1.0,
        "k" | "kb" => 1024.0,
        "m" | "mb" => 1024.0 * 1024.0,
        "g" | "gb" => 1024.0 * 1024.0 * 1024.0,
        "t" | "tb" => 1024.0f64.powi(4),
        _ => return None,
    };
    Some((n * mul) as u64)
}

fn parse_date(v: &str) -> Option<Vec<Term>> {
    let now = now_filetime();
    let st = filetime_local(now);
    let today = filetime_from_ymd(st.year as i64, st.month as i64, st.day as i64)?;
    Some(match v {
        "today" => vec![Term::After(today)],
        "yesterday" => vec![Term::After(today - FT_DAY), Term::Before(today)],
        "week" | "thisweek" => vec![Term::After(now - 7 * FT_DAY)],
        "month" | "thismonth" => vec![Term::After(now - 30 * FT_DAY)],
        "year" | "thisyear" => vec![Term::After(now - 365 * FT_DAY)],
        _ => {
            if let Some(r) = v.strip_prefix('>') {
                vec![Term::After(parse_ymd(r)?)]
            } else if let Some(r) = v.strip_prefix('<') {
                vec![Term::Before(parse_ymd(r)?)]
            } else if v.len() == 4 {
                let y: i64 = v.parse().ok()?;
                vec![Term::After(filetime_from_ymd(y, 1, 1)?), Term::Before(filetime_from_ymd(y + 1, 1, 1).unwrap_or(u64::MAX))]
            } else {
                let d = parse_ymd(v)?;
                vec![Term::After(d), Term::Before(d + FT_DAY)]
            }
        }
    })
}

fn parse_ymd(s: &str) -> Option<u64> {
    let mut it = s.trim_start_matches('=').split(['-', '.', '/']);
    let y = it.next()?.parse().ok()?;
    let m = it.next().map(|x| x.parse().ok()).unwrap_or(Some(1))?;
    let d = it.next().map(|x| x.parse().ok()).unwrap_or(Some(1))?;
    filetime_from_ymd(y, m, d)
}

// ---------------------------------------------------------------- running a query

pub const MAX_RESULTS: usize = 250_000;

pub struct Search {
    pub index: Arc<Index>,
    pub text: String,
    query: Arc<Query>,
    pub results: Vec<u32>,
    scanned: usize,
    rx: Option<Receiver<(Vec<u32>, usize, Duration)>>,
    pub query_time: Duration,
    pub sorted: bool,
}

impl Search {
    pub fn new(index: Arc<Index>, text: &str, show_hidden: bool, regex: bool, hide_dots: bool, extra: Option<crate::listing::CompiledFilter>) -> Search {
        let q = if regex { Query::regex_whole(text) } else { Query::new(text) };
        let mut q = if show_hidden { q } else { q.without_hidden() };
        q.hide_dots = hide_dots;
        q.extra = extra;
        Search {
            index,
            text: text.to_string(),
            query: Arc::new(q),
            results: Vec::new(),
            scanned: 0,
            rx: None,
            query_time: Duration::ZERO,
            sorted: false,
        }
    }

    pub fn highlight(&self) -> &[String] {
        &self.query.highlight
    }

    pub fn busy(&self) -> bool {
        self.rx.is_some() || !self.index.is_done() || self.scanned < self.index.count()
    }

    /// Advance: collect finished scans, start a scan over newly indexed items.
    pub fn pump(&mut self) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.rx {
            match rx.try_recv() {
                Ok((mut found, upto, took)) => {
                    let room = MAX_RESULTS.saturating_sub(self.results.len());
                    found.truncate(room);
                    self.results.extend(found);
                    self.scanned = upto;
                    self.rx = None;
                    // time spent matching only (not crawling)
                    self.query_time += took;
                    changed = true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return false,
                Err(_) => self.rx = None,
            }
        }
        let total = self.index.count();
        if self.rx.is_none() && self.scanned < total && self.results.len() < MAX_RESULTS {
            let (tx, rx) = channel();
            let idx = self.index.clone();
            let q = self.query.clone();
            let from = self.scanned;
            crate::bg::spawn("scan", move || {
                let t0 = Instant::now();
                let r = scan(&idx, &q, from, total);
                let _ = tx.send((r, total, t0.elapsed()));
            });
            self.rx = Some(rx);
        }
        changed
    }
}

fn scan(idx: &Index, q: &Query, from: usize, to: usize) -> Vec<u32> {
    let data = idx.data.read().unwrap();
    let to = to.min(data.items.len());
    if from >= to || q.is_empty() {
        return Vec::new();
    }
    let n = to - from;
    let threads = if n < 50_000 { 1 } else { std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(16) };
    let chunk = n.div_ceil(threads);
    let data = &*data;
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                s.spawn(move || {
                    let a = from + t * chunk;
                    let b = (a + chunk).min(to);
                    let mut out = Vec::new();
                    for i in a..b {
                        let it = &data.items[i];
                        if it.parent == NO_PARENT {
                            continue;
                        }
                        if q.matches(data.lower(it), it.attrs, it.size, it.mtime) && !(q.hide_dots && data.under_dot_folder(i as u32)) {
                            out.push(i as u32);
                            if out.len() >= MAX_RESULTS {
                                break;
                            }
                        }
                    }
                    out
                })
            })
            .collect();
        let mut all = Vec::new();
        for h in handles {
            all.extend(h.join().unwrap_or_default());
        }
        all
    })
}

/// Filter for the current folder only (no index needed).
pub fn matches_entry(q: &Query, e: &crate::fsx::Entry) -> bool {
    q.matches(e.lower.as_bytes(), e.attrs, e.size, e.mtime)
}

pub fn roots_equal(a: &[PathBuf], b: &[PathBuf]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| crate::fsx::same_path(x, y))
}

pub fn display_folder(p: &Path) -> String {
    p.parent().map(|x| x.to_string_lossy().to_string()).unwrap_or_default()
}

#[cfg(test)]
mod dot_tests {
    use super::*;

    #[test]
    fn hiding_dotfiles_reaches_search() {
        let mut q = Query::new("env");
        assert!(q.matches(b".env", 0, 1, 1));
        q.hide_dots = true;
        assert!(!q.matches(b".env", 0, 1, 1));
        assert!(q.matches(b"my.env", 0, 1, 1));
    }

    #[test]
    fn what_is_inside_a_dot_folder_counts_too() {
        let mut d = Data::default();
        let root = d.push("C:\\", NO_PARENT, A_DIR, 0, 0);
        let dot = d.push(".git", root, A_DIR, 0, 0);
        let inside = d.push("config", dot, 0, 1, 1);
        let plain = d.push("src", root, A_DIR, 0, 0);
        let fine = d.push("main.rs", plain, 0, 1, 1);
        assert!(d.under_dot_folder(inside));
        assert!(!d.under_dot_folder(fine));
        assert!(!d.under_dot_folder(dot), "that one is caught by its own name");
        // searching from inside a dot folder is asking to look there
        let mut e = Data::default();
        let start = e.push("C:\\.config", NO_PARENT, A_DIR, 0, 0);
        let item = e.push("settings", start, 0, 1, 1);
        assert!(!e.under_dot_folder(item));
    }

    #[test]
    fn the_filter_panel_reaches_search() {
        let mut q = Query::new("a");
        let f = crate::listing::ListFilter { kinds: vec![crate::util::Kind::Image], ..Default::default() };
        q.extra = Some(f.compile(1));
        assert!(q.matches(b"a.png", 0, 1, 1));
        assert!(!q.matches(b"a.txt", 0, 1, 1));
    }
}
