//! How a folder's list is ordered and narrowed down: the sort a folder uses, and the filter panel's choices.
//! No drawing and no disk in here.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::Path;

use crate::util::{filetime_from_ymd, filetime_local, kind_of, natural_cmp, now_filetime, Kind, FT_DAY};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Name,
    Modified,
    Created,
    Type,
    Extension,
    Size,
}

impl SortKey {
    pub const ALL: [SortKey; 6] = [SortKey::Name, SortKey::Modified, SortKey::Created, SortKey::Type, SortKey::Extension, SortKey::Size];

    pub fn word(self) -> &'static str {
        match self {
            SortKey::Name => "name",
            SortKey::Modified => "modified",
            SortKey::Created => "created",
            SortKey::Type => "type",
            SortKey::Extension => "extension",
            SortKey::Size => "size",
        }
    }

    pub fn from_word(w: &str) -> Option<SortKey> {
        SortKey::ALL.into_iter().find(|k| k.word() == w)
    }

    pub fn label(self) -> &'static str {
        match self {
            SortKey::Name => "Name",
            SortKey::Modified => "Date modified",
            SortKey::Created => "Date created",
            SortKey::Type => "Type",
            SortKey::Extension => "Extension",
            SortKey::Size => "Size",
        }
    }

    /// The direction people expect when they first pick this: names from A, dates and sizes from the top.
    pub fn starts_ascending(self) -> bool {
        matches!(self, SortKey::Name | SortKey::Type | SortKey::Extension)
    }
}

/// How one folder is sorted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SortSpec {
    pub key: SortKey,
    pub asc: bool,
    pub dirs_first: bool,
}

impl SortSpec {
    /// What Downloads gets out of the box: what just arrived on top, folders in among it.
    pub const fn downloads() -> SortSpec {
        SortSpec { key: SortKey::Modified, asc: false, dirs_first: false }
    }

    pub fn encode(self) -> String {
        format!("{}\t{}\t{}", self.key.word(), self.asc as u8, self.dirs_first as u8)
    }

    pub fn decode(s: &str) -> Option<SortSpec> {
        let mut f = s.split('\t');
        Some(SortSpec { key: SortKey::from_word(f.next()?)?, asc: f.next()? == "1", dirs_first: f.next()? == "1" })
    }

    /// Which way round it runs, as a person would say it.
    pub fn direction(self) -> &'static str {
        match (self.key, self.asc) {
            (SortKey::Modified | SortKey::Created, false) => "Newest first",
            (SortKey::Modified | SortKey::Created, true) => "Oldest first",
            (SortKey::Size, false) => "Largest first",
            (SortKey::Size, true) => "Smallest first",
            (_, true) => "A to Z",
            (_, false) => "Z to A",
        }
    }

}

/// Hidden items and dotfiles are separate settings, and either can hide an item. Showing hidden items doesn't bring
/// back a dotfile, and showing dotfiles doesn't bring back something with the hidden attribute.
pub fn is_visible(hidden_attr: bool, dotfile: bool, show_hidden: bool, hide_dotfiles: bool) -> bool {
    (show_hidden || !hidden_attr) && !(hide_dotfiles && dotfile)
}

/// The form a folder's path is remembered under.
pub fn folder_key(p: &Path) -> String {
    let s = p.to_string_lossy();
    let sep = std::path::MAIN_SEPARATOR;
    let t = s.trim_end_matches(sep);
    let t = if t.is_empty() { s.as_ref() } else { t };
    if cfg!(any(windows, target_os = "macos")) {
        t.to_lowercase()
    } else {
        t.to_string()
    }
}

/// What the person last chose for this folder, else the built-in order of Downloads, else the default.
pub fn sort_for(folder: Option<&Path>, remembered: &BTreeMap<String, SortSpec>, downloads: Option<&Path>, downloads_newest: bool, default: SortSpec) -> SortSpec {
    let Some(folder) = folder else { return default };
    let key = folder_key(folder);
    if let Some(s) = remembered.get(&key) {
        return *s;
    }
    if downloads_newest && downloads.is_some_and(|d| folder_key(d) == key) {
        return SortSpec::downloads();
    }
    default
}

/// What a row is sorted by. A copy, so that a big folder can be sorted away from the window.
pub struct SortRow {
    pub index: u32,
    pub is_dir: bool,
    pub lower: String,
    pub ext: String,
    pub size: u64,
    pub mtime: u64,
    pub ctime: u64,
}

fn kind_rank(r: &SortRow) -> u8 {
    match kind_of(&r.ext, r.is_dir) {
        Kind::Folder => 0,
        Kind::Document => 1,
        Kind::Image => 2,
        Kind::Video => 3,
        Kind::Audio => 4,
        Kind::Archive => 5,
        Kind::Code => 6,
        Kind::App => 7,
        Kind::Other => 8,
    }
}

pub fn compare(x: &SortRow, y: &SortRow, spec: SortSpec) -> Ordering {
    if spec.dirs_first && x.is_dir != y.is_dir {
        // folders stay on top whichever way the rest runs
        return y.is_dir.cmp(&x.is_dir);
    }
    let o = match spec.key {
        SortKey::Name => Ordering::Equal,
        SortKey::Modified => x.mtime.cmp(&y.mtime),
        SortKey::Created => x.ctime.cmp(&y.ctime),
        SortKey::Size => x.size.cmp(&y.size),
        SortKey::Extension => x.ext.cmp(&y.ext),
        SortKey::Type => kind_rank(x).cmp(&kind_rank(y)).then_with(|| x.ext.cmp(&y.ext)),
    }
    .then_with(|| natural_cmp(&x.lower, &y.lower));
    if spec.asc {
        o
    } else {
        o.reverse()
    }
}

pub fn sorted_indexes(mut rows: Vec<SortRow>, spec: SortSpec) -> Vec<u32> {
    rows.sort_unstable_by(|a, b| compare(a, b, spec));
    rows.into_iter().map(|r| r.index).collect()
}

// ---------------------------------------------------------------- the filter panel

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum DateRange {
    #[default]
    Any,
    Today,
    Week,
    Month,
    Custom,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SizeRange {
    #[default]
    Any,
    Under1Mb,
    Mb1To100,
    Mb100To1Gb,
    Over1Gb,
    Custom,
}

/// The kinds the panel offers.
pub const FILTER_KINDS: [(Kind, &str); 7] = [
    (Kind::Folder, "Folders"),
    (Kind::Image, "Images"),
    (Kind::Document, "Documents"),
    (Kind::Video, "Video"),
    (Kind::Audio, "Audio"),
    (Kind::Archive, "Archives"),
    (Kind::Code, "Code"),
];

/// What the person chose in the filter panel, as they chose it.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct ListFilter {
    pub kinds: Vec<Kind>,
    /// lower case, no dot
    pub exts: Vec<String>,
    pub date: DateRange,
    pub date_from: String,
    pub date_to: String,
    pub size: SizeRange,
    pub size_min: String,
    pub size_max: String,
    pub name: String,
}

impl ListFilter {
    pub fn is_active(&self) -> bool {
        !self.kinds.is_empty() || !self.exts.is_empty() || self.date != DateRange::Any || self.size != SizeRange::Any || !self.name.trim().is_empty()
    }

    /// How many things it's narrowing down by.
    pub fn count(&self) -> usize {
        usize::from(!self.kinds.is_empty() || !self.exts.is_empty()) + usize::from(self.date != DateRange::Any) + usize::from(self.size != SizeRange::Any) + usize::from(!self.name.trim().is_empty())
    }

    pub fn toggle_kind(&mut self, k: Kind) {
        match self.kinds.iter().position(|x| *x == k) {
            Some(i) => {
                self.kinds.remove(i);
            }
            None => self.kinds.push(k),
        }
    }

    pub fn toggle_ext(&mut self, e: &str) {
        let e = e.trim().trim_start_matches('.').to_lowercase();
        if e.is_empty() {
            return;
        }
        match self.exts.iter().position(|x| *x == e) {
            Some(i) => {
                self.exts.remove(i);
            }
            None => self.exts.push(e),
        }
    }

    pub fn date_error(&self) -> Option<&'static str> {
        (self.date == DateRange::Custom && [&self.date_from, &self.date_to].iter().any(|d| !d.trim().is_empty() && parse_day(d).is_none())).then_some("Write dates as 2026-11-05")
    }

    pub fn size_error(&self) -> Option<&'static str> {
        (self.size == SizeRange::Custom && [&self.size_min, &self.size_max].iter().any(|s| !s.trim().is_empty() && parse_size(s).is_none())).then_some("Write sizes as 500 KB, 10 MB or 2 GB")
    }

    /// Ready to test against many entries, as of `now` (FILETIME ticks).
    pub fn compile(&self, now: u64) -> CompiledFilter {
        let (after, before) = date_bounds(self, now);
        let (min, max) = match self.size {
            SizeRange::Any => (None, None),
            SizeRange::Under1Mb => (None, Some(MB - 1)),
            SizeRange::Mb1To100 => (Some(MB), Some(100 * MB - 1)),
            SizeRange::Mb100To1Gb => (Some(100 * MB), Some(1024 * MB - 1)),
            SizeRange::Over1Gb => (Some(1024 * MB), None),
            SizeRange::Custom => (parse_size(&self.size_min), parse_size(&self.size_max)),
        };
        CompiledFilter {
            kinds: self.kinds.clone(),
            exts: self.exts.clone(),
            after,
            before,
            sized: self.size != SizeRange::Any,
            min,
            max,
            name: self.name.trim().to_lowercase(),
        }
    }

    pub fn compile_now(&self) -> CompiledFilter {
        self.compile(now_filetime())
    }
}

const MB: u64 = 1024 * 1024;

/// "10 mb", "500k", "2 GB" or a plain number of bytes.
pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim().to_lowercase();
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ',')).unwrap_or(s.len());
    let n: f64 = s[..split].replace(',', ".").parse().ok()?;
    let mul = match s[split..].trim() {
        "" | "b" => 1.0,
        "k" | "kb" => 1024.0,
        "m" | "mb" => 1024.0 * 1024.0,
        "g" | "gb" => 1024.0 * 1024.0 * 1024.0,
        "t" | "tb" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    Some((n * mul) as u64)
}

/// The start of a day written 2026-11-05.
pub fn parse_day(s: &str) -> Option<u64> {
    let mut f = s.trim().split(['-', '.', '/']);
    let (y, m, d) = (f.next()?.parse().ok()?, f.next()?.parse().ok()?, f.next()?.parse().ok()?);
    if f.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    filetime_from_ymd(y, m, d)
}

fn date_bounds(f: &ListFilter, now: u64) -> (Option<u64>, Option<u64>) {
    let st = filetime_local(now);
    let today = filetime_from_ymd(st.year as i64, st.month as i64, st.day as i64).unwrap_or(now);
    match f.date {
        DateRange::Any => (None, None),
        DateRange::Today => (Some(today), None),
        // weeks start on Monday. 1601-01-01, tick 0, was a Monday.
        DateRange::Week => (Some(today - (today / FT_DAY) % 7 * FT_DAY), None),
        DateRange::Month => (Some(filetime_from_ymd(st.year as i64, st.month as i64, 1).unwrap_or(today)), None),
        DateRange::Custom => (parse_day(&f.date_from), parse_day(&f.date_to).map(|d| d + FT_DAY)),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompiledFilter {
    kinds: Vec<Kind>,
    exts: Vec<String>,
    after: Option<u64>,
    before: Option<u64>,
    sized: bool,
    min: Option<u64>,
    max: Option<u64>,
    name: String,
}

impl CompiledFilter {
    /// Different parts of the panel narrow each other down. Inside "type", ticking more shows more: kinds and
    /// extensions add up. A folder has no type or size of its own, so those leave it out unless "Folders" is ticked.
    pub fn matches(&self, lower_name: &str, ext: &str, is_dir: bool, size: u64, mtime: u64) -> bool {
        if !self.name.is_empty() && !lower_name.contains(&self.name) {
            return false;
        }
        if self.after.is_some_and(|t| mtime < t) || self.before.is_some_and(|t| mtime >= t) {
            return false;
        }
        let typed = !self.kinds.is_empty() || !self.exts.is_empty();
        if is_dir {
            return if typed || self.sized { self.kinds.contains(&Kind::Folder) } else { true };
        }
        if typed && !self.kinds.contains(&kind_of(ext, false)) && !self.exts.iter().any(|e| e == ext) {
            return false;
        }
        if self.min.is_some_and(|m| size < m) || self.max.is_some_and(|m| size > m) {
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn row(i: u32, name: &str, is_dir: bool, size: u64, mtime: u64, ctime: u64) -> SortRow {
        let ext = if is_dir { String::new() } else { name.rsplit_once('.').map(|(_, e)| e.to_string()).unwrap_or_default() };
        SortRow { index: i, is_dir, lower: name.to_lowercase(), ext, size, mtime, ctime }
    }

    fn sample() -> Vec<SortRow> {
        vec![
            row(0, "beta.txt", false, 500, 30, 5),
            row(1, "Alpha", true, 0, 20, 6),
            row(2, "image10.png", false, 100, 40, 1),
            row(3, "image2.png", false, 900, 10, 4),
            row(4, "Zulu", true, 0, 50, 2),
            row(5, "archive.zip", false, 300, 25, 3),
        ]
    }

    fn order(key: SortKey, asc: bool, dirs_first: bool) -> Vec<u32> {
        sorted_indexes(sample(), SortSpec { key, asc, dirs_first })
    }

    #[test]
    fn folders_first_holds_whichever_way_the_rest_runs() {
        assert_eq!(order(SortKey::Name, true, true), [1, 4, 5, 0, 3, 2]);
        assert_eq!(order(SortKey::Modified, false, true), [4, 1, 2, 0, 5, 3]);
        assert_eq!(order(SortKey::Modified, true, true), [1, 4, 3, 5, 0, 2]);
    }

    #[test]
    fn folders_mix_in_when_asked() {
        // newest first, the two folders in among the files
        assert_eq!(order(SortKey::Modified, false, false), [4, 2, 0, 5, 1, 3]);
        assert_eq!(order(SortKey::Name, true, false), [1, 5, 0, 3, 2, 4]);
    }

    #[test]
    fn every_key_sorts_by_what_it_says() {
        assert_eq!(order(SortKey::Created, true, false), [2, 4, 5, 3, 0, 1]);
        // folders first, Z to A among the two of them as they have no size, then largest first
        assert_eq!(order(SortKey::Size, false, true), [4, 1, 3, 0, 5, 2]);
        // extension: png, png, txt, zip
        assert_eq!(order(SortKey::Extension, true, true), [1, 4, 3, 2, 0, 5]);
        // type: documents, then pictures, then archives
        assert_eq!(order(SortKey::Type, true, true), [1, 4, 0, 3, 2, 5]);
    }

    #[test]
    fn a_folder_keeps_the_sort_chosen_for_it() {
        let mut memory = BTreeMap::new();
        let plain = SortSpec { key: SortKey::Name, asc: true, dirs_first: true };
        let dl = PathBuf::from(if cfg!(windows) { r"D:\Moved\Downloads" } else { "/moved/Downloads" });
        let other = PathBuf::from(if cfg!(windows) { r"C:\Work" } else { "/work" });
        // Downloads by its built-in order, wherever it was moved to
        assert_eq!(sort_for(Some(&dl), &memory, Some(&dl), true, plain), SortSpec::downloads());
        // turned off in Settings
        assert_eq!(sort_for(Some(&dl), &memory, Some(&dl), false, plain), plain);
        // the default everywhere else
        assert_eq!(sort_for(Some(&other), &memory, Some(&dl), true, plain), plain);
        // an explicit choice beats the built-in one
        let chosen = SortSpec { key: SortKey::Size, asc: false, dirs_first: true };
        memory.insert(folder_key(&dl), chosen);
        assert_eq!(sort_for(Some(&dl), &memory, Some(&dl), true, plain), chosen);
        // and it's the folder that remembers, however its path is written
        let with_sep = PathBuf::from(format!("{}{}", dl.display(), std::path::MAIN_SEPARATOR));
        assert_eq!(sort_for(Some(&with_sep), &memory, Some(&dl), true, plain), chosen);
        // reset
        memory.remove(&folder_key(&dl));
        assert_eq!(sort_for(Some(&dl), &memory, Some(&dl), true, plain), SortSpec::downloads());
    }

    #[test]
    fn a_sort_survives_the_settings_file() {
        for key in SortKey::ALL {
            let s = SortSpec { key, asc: false, dirs_first: true };
            assert_eq!(SortSpec::decode(&s.encode()), Some(s));
        }
        assert_eq!(SortSpec::decode("nonsense"), None);
    }

    fn day(y: i64, m: i64, d: i64) -> u64 {
        filetime_from_ymd(y, m, d).unwrap()
    }

    fn shows(f: &ListFilter, name: &str, is_dir: bool, size: u64, mtime: u64) -> bool {
        let ext = if is_dir { String::new() } else { name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default() };
        f.compile(day(2026, 11, 5) + FT_DAY / 2).matches(&name.to_lowercase(), &ext, is_dir, size, mtime)
    }

    #[test]
    fn hidden_and_dotfiles_each_can_hide_an_item() {
        // hidden attribute, dotfile, show hidden, hide dotfiles => shown
        for (attr, dot, show, hide, want) in [
            (false, false, false, false, true),
            (false, false, false, true, true),
            (true, false, false, false, false),
            (true, false, true, false, true),
            (true, false, true, true, true),
            (false, true, false, false, true),
            (false, true, false, true, false),
            (false, true, true, true, false),
            (true, true, true, false, true),
            (true, true, false, false, false),
            (true, true, true, true, false),
        ] {
            assert_eq!(is_visible(attr, dot, show, hide), want, "{attr} {dot} {show} {hide}");
        }
    }

    #[test]
    fn no_filter_shows_everything() {
        let f = ListFilter::default();
        assert!(!f.is_active());
        assert!(shows(&f, "a.txt", false, 1, 1));
        assert!(shows(&f, "dir", true, 0, 1));
    }

    #[test]
    fn kinds_and_extensions_add_up() {
        let mut f = ListFilter::default();
        f.toggle_kind(Kind::Image);
        assert!(shows(&f, "a.png", false, 1, 1));
        assert!(!shows(&f, "a.pdf", false, 1, 1));
        assert!(!shows(&f, "dir", true, 0, 1), "a folder isn't an image");
        f.toggle_ext(".PDF");
        assert!(shows(&f, "a.pdf", false, 1, 1));
        assert!(shows(&f, "a.png", false, 1, 1));
        f.toggle_kind(Kind::Folder);
        assert!(shows(&f, "dir", true, 0, 1));
        f.toggle_kind(Kind::Image);
        f.toggle_ext("pdf");
        f.toggle_kind(Kind::Folder);
        assert!(!f.is_active());
    }

    #[test]
    fn date_ranges_start_where_people_think_they_do() {
        let at = |d| day(2026, 11, d) + 1;
        let mut f = ListFilter { date: DateRange::Today, ..Default::default() };
        assert!(shows(&f, "a", false, 1, at(5)));
        assert!(!shows(&f, "a", false, 1, at(4)));
        // 2026-11-05 is a Thursday, so the week began on Monday the 2nd
        f.date = DateRange::Week;
        assert!(shows(&f, "a", false, 1, at(2)));
        assert!(!shows(&f, "a", false, 1, day(2026, 11, 1) + 1));
        f.date = DateRange::Month;
        assert!(shows(&f, "a", false, 1, at(1)));
        assert!(!shows(&f, "a", false, 1, day(2026, 10, 31) + 1));
        // both ends are included
        f = ListFilter { date: DateRange::Custom, date_from: "2026-10-30".into(), date_to: "2026-10-31".into(), ..Default::default() };
        assert!(shows(&f, "a", false, 1, day(2026, 10, 30)));
        assert!(shows(&f, "a", false, 1, day(2026, 10, 31) + FT_DAY - 1));
        assert!(!shows(&f, "a", false, 1, day(2026, 11, 1)));
        assert!(!shows(&f, "a", false, 1, day(2026, 10, 29) + FT_DAY - 1));
        // and folders go by their date too
        assert!(shows(&f, "dir", true, 0, day(2026, 10, 30)));
    }

    #[test]
    fn sizes_apply_to_files_and_leave_folders_out() {
        let mut f = ListFilter { size: SizeRange::Mb1To100, ..Default::default() };
        assert!(shows(&f, "a", false, MB, 1));
        assert!(!shows(&f, "a", false, MB - 1, 1));
        assert!(!shows(&f, "a", false, 100 * MB, 1));
        assert!(!shows(&f, "dir", true, 0, 1));
        f.toggle_kind(Kind::Folder);
        assert!(shows(&f, "dir", true, 0, 1));
        f = ListFilter { size: SizeRange::Custom, size_min: "1,5 mb".into(), size_max: "2 GB".into(), ..Default::default() };
        assert!(f.size_error().is_none());
        assert!(shows(&f, "a", false, 2 * MB, 1));
        assert!(!shows(&f, "a", false, MB, 1));
        f.size_max = "lots".into();
        assert!(f.size_error().is_some());
    }

    #[test]
    fn everything_combines() {
        let f = ListFilter { kinds: vec![Kind::Image], date: DateRange::Today, size: SizeRange::Under1Mb, name: "Report".into(), ..Default::default() };
        assert_eq!(f.count(), 4);
        let now = day(2026, 11, 5) + 5;
        assert!(shows(&f, "Annual report.png", false, 100, now));
        assert!(!shows(&f, "Annual.png", false, 100, now));
        assert!(!shows(&f, "report.pdf", false, 100, now));
        assert!(!shows(&f, "report.png", false, 2 * MB, now));
        assert!(!shows(&f, "report.png", false, 100, now - FT_DAY));
    }

    #[test]
    fn a_big_folder_filters_quickly() {
        let names: Vec<String> = (0..300_000).map(|i| format!("file number {i}.{}", ["txt", "png", "zip", "rs"][i % 4])).collect();
        let f = ListFilter { kinds: vec![Kind::Image, Kind::Document], name: "number 1".into(), ..Default::default() }.compile(day(2026, 11, 5));
        let start = std::time::Instant::now();
        let n = names.iter().filter(|n| f.matches(n, n.rsplit('.').next().unwrap_or(""), false, 5, 5)).count();
        assert!(n > 0);
        assert!(start.elapsed().as_millis() < 500, "took {:?}", start.elapsed());
    }
}
