//! Every text the person can see is translated, in every language: these read the source to make sure.
//!
//! `every_marked_text_is_translated` collects what's marked with `tr`, `tx`, `trf!` and `trn!` (`three` carries its own
//! translations) and checks the catalogs have each, with the same `{values}`, and nothing they don't need.
//! `no_text_is_left_unmarked` looks at every other string in the code: one that reads like words to a person has to
//! be marked, or say why not on its line with `// i18n-ignore: <why>` (a name Windows knows a thing by, and so on).
//! DALERIS_I18N_ONLY=<path>,<path> narrows that to some files, for working on them one at a time.

use super::*;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Where text the person sees can come from: the app, and the launcher with its message boxes. The other crates show
/// nothing: the file dialog DLL and the integration library only log and write the registry, and the dialog test
/// program isn't shipped.
const ROOTS: [&str; 2] = ["src", "crates/open/src"];

/// Calls whose strings are for developers or the system, never shown: everything inside them is skipped.
const NOT_SHOWN: [&str; 17] = [
    "log::trace!(",
    "log::debug!(",
    "log::info!(",
    "log::warn!(",
    "log::error!(",
    "panic!(",
    "unreachable!(",
    "assert!(",
    "assert_eq!(",
    "assert_ne!(",
    "debug_assert!(",
    ".expect(",
    "include_str!(",
    "include_bytes!(",
    "env!(",
    "#[",
    "#![",
];

/// A string literal in the source: its value, the line it's on, and what was written just before it.
struct Lit {
    value: String,
    line: usize,
    marked: bool,
    /// marked with `three`, which carries its own translations: none are looked up
    in_place: bool,
}

fn files() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let base = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    for r in ROOTS {
        walk(&base.join(r), &mut out);
    }
    // the catalogs are the translations, and these tests talk about text
    out.retain(|p| !p.components().any(|c| c.as_os_str() == "i18n"));
    out.sort();
    out
}

/// The literals in Rust source, skipping comments, char literals, `NOT_SHOWN` calls and `#[cfg(test)]` modules.
/// `marked` says the literal is an argument of `tr`, `tx`, `trf!` or `trn!`.
fn literals(src: &str) -> Vec<Lit> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut line = 1;
    // (depth where a skipped call ends) while inside one, and the same for a marking call
    let mut skip_until: Option<usize> = None;
    let mut mark_until: Option<usize> = None;
    let mut in_place = false;
    let mut depth = 0usize;
    let starts = |i: usize, s: &str| b[i..].starts_with(s.as_bytes());
    // a word boundary before a call's name, so `str(` isn't `tr(`
    let call_at = |i: usize, name: &str| starts(i, name) && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_'));
    while i < b.len() {
        let c = b[i];
        if c == b'\n' {
            line += 1;
            i += 1;
            continue;
        }
        if starts(i, "//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if starts(i, "/*") {
            while i < b.len() && !starts(i, "*/") {
                if b[i] == b'\n' {
                    line += 1;
                }
                i += 1;
            }
            i += 2;
            continue;
        }
        // test modules are the code's own business
        if starts(i, "#[cfg(test)]") && src[i..].trim_start_matches("#[cfg(test)]").trim_start().starts_with("mod ") {
            break;
        }
        if skip_until.is_none() {
            if let Some(n) = NOT_SHOWN.iter().find(|n| starts(i, n)) {
                skip_until = Some(depth);
                if n.starts_with('#') {
                    depth += 1;
                    i += n.len();
                    continue;
                }
            }
        }
        if mark_until.is_none() && skip_until.is_none() && ["tr(", "tx(", "trf!(", "trn!(", "three("].iter().any(|n| call_at(i, n)) {
            mark_until = Some(depth);
            in_place = call_at(i, "three(");
        }
        match c {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth = depth.saturating_sub(1);
                if skip_until == Some(depth) {
                    skip_until = None;
                }
                if mark_until == Some(depth) {
                    mark_until = None;
                }
            }
            b'\'' => {
                // a char literal ('x', '\n', '\u{..}'), not a lifetime ('a)
                let close = if b.get(i + 1) == Some(&b'\\') { src[i + 2..].find('\'').map(|e| i + 2 + e) } else if b.get(i + 2) == Some(&b'\'') { Some(i + 2) } else { None };
                if let Some(e) = close {
                    i = e + 1;
                    continue;
                }
            }
            b'"' | b'r' if c == b'"' || (starts(i, "r\"") || starts(i, "r#\"")) && (i == 0 || !b[i - 1].is_ascii_alphanumeric() && b[i - 1] != b'_') => {
                let start_line = line;
                let (value, end) = read_literal(src, i);
                line += src[i..end].matches('\n').count();
                if skip_until.is_none() {
                    out.push(Lit { value, line: start_line, marked: mark_until.is_some(), in_place: in_place && mark_until.is_some() });
                }
                i = end;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// The value of the string literal starting at `i`, and where it ends.
fn read_literal(src: &str, i: usize) -> (String, usize) {
    let b = src.as_bytes();
    if b[i] == b'r' {
        let hashes = src[i + 1..].chars().take_while(|c| *c == '#').count();
        let open = i + 1 + hashes + 1;
        let close = format!("\"{}", "#".repeat(hashes));
        let end = open + src[open..].find(&close).expect("a raw string ends");
        return (src[open..end].to_string(), end + close.len());
    }
    let mut out = String::new();
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'"' => return (out, j + 1),
            b'\\' => {
                j += 1;
                match b[j] {
                    b'n' => out.push('\n'),
                    b't' => out.push('\t'),
                    b'r' => out.push('\r'),
                    b'0' => out.push('\0'),
                    b'\\' => out.push('\\'),
                    b'"' => out.push('"'),
                    b'\'' => out.push('\''),
                    b'u' => {
                        let end = j + src[j..].find('}').expect("\\u{..}");
                        let code = u32::from_str_radix(&src[j + 2..end], 16).expect("hex");
                        out.push(char::from_u32(code).expect("a char"));
                        j = end;
                    }
                    b'\n' | b'\r' => {
                        // a line continued: the next line's leading spaces aren't part of it
                        while j + 1 < b.len() && (b[j + 1] as char).is_whitespace() {
                            j += 1;
                        }
                    }
                    x => out.push(x as char),
                }
                j += 1;
            }
            _ => {
                let ch = src[j..].chars().next().expect("a char");
                out.push(ch);
                j += ch.len_utf8();
            }
        }
    }
    panic!("a string never ends");
}

/// Does this read like words to a person: a capitalised word, or words with a space between?
fn reads_like_words(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    let capitalised = chars.windows(2).any(|w| w[0].is_uppercase() && w[1].is_lowercase());
    // one word with a capital inside, ShowHiddenItems or DllGetClassObject, is a name in code
    let identifier = !s.contains(' ') && chars.iter().skip(1).any(|c| c.is_uppercase());
    let words = chars.windows(3).any(|w| w[0].is_alphabetic() && w[1] == ' ' && w[2].is_alphabetic());
    // paths, registry keys, addresses and code aren't words for a person
    let machine = s.contains('\\') || s.contains("://") || s.starts_with('.') || s.starts_with('%') || s.contains("{path}");
    (capitalised || words) && !machine && !identifier
}

fn ignored(src_line: &str) -> bool {
    src_line.contains("// i18n-ignore")
}

/// (text, where it's marked) for everything marked in the source
fn marked() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for f in files() {
        let src = std::fs::read_to_string(&f).expect("read");
        for l in literals(&src).into_iter().filter(|l| l.marked && !l.in_place) {
            out.entry(l.value).or_insert_with(|| format!("{}:{}", f.display(), l.line));
        }
    }
    out
}

/// `{name}`s in a text
fn values(s: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = s;
    while let Some(a) = rest.find('{') {
        rest = &rest[a + 1..];
        if let Some(e) = rest.find('}') {
            let name = &rest[..e];
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                out.insert(name.to_string());
            }
        }
    }
    out
}

#[test]
fn every_marked_text_is_translated() {
    let keys = marked();
    assert!(keys.len() > 20, "the source scan found next to nothing: {}", keys.len());
    let mut problems = Vec::new();
    for (lang, parts) in [("de", de::ALL), ("it", it::ALL)] {
        let mut seen: BTreeMap<&str, &str> = BTreeMap::new();
        for (en, t) in parts.iter().flat_map(|p| p.iter()) {
            if let Some(before) = seen.insert(en, t) {
                if before != *t {
                    problems.push(format!("{lang}: \"{en}\" is translated twice, differently: \"{before}\" and \"{t}\""));
                }
            }
            if !keys.contains_key(*en) {
                problems.push(format!("{lang}: \"{en}\" isn't used anywhere any more"));
            }
            if t.trim().is_empty() {
                problems.push(format!("{lang}: \"{en}\" has an empty translation"));
            }
            if values(en) != values(t) {
                problems.push(format!("{lang}: \"{en}\" and \"{t}\" don't have the same {{values}}"));
            }
        }
        for (en, at) in &keys {
            if !seen.contains_key(en.as_str()) {
                problems.push(format!("{lang}: no translation for \"{en}\" ({at})"));
            }
        }
    }
    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.join("\n"));
}

#[test]
fn no_text_is_left_unmarked() {
    let only: Option<Vec<String>> = std::env::var("DALERIS_I18N_ONLY").ok().map(|v| v.split(',').map(|s| s.replace('\\', "/")).collect());
    let mut problems = Vec::new();
    for f in files() {
        let shown = f.display().to_string().replace('\\', "/");
        if only.as_ref().is_some_and(|o| !o.iter().any(|x| shown.ends_with(x.as_str()))) {
            continue;
        }
        let src = std::fs::read_to_string(&f).expect("read");
        let lines: Vec<&str> = src.lines().collect();
        for l in literals(&src) {
            if l.marked || !reads_like_words(&l.value) || ignored(lines.get(l.line - 1).unwrap_or(&"")) {
                continue;
            }
            problems.push(format!("{shown}:{}: \"{}\"", l.line, l.value.replace('\n', "\\n")));
        }
    }
    assert!(problems.is_empty(), "{} texts aren't marked for translation:\n{}", problems.len(), problems.join("\n"));
}

/// Middle dots and arrow characters aren't used in Daleris, on screen or in the code: items are separated with commas or
/// room, paths in Settings with ">", and arrows that are drawn come from the icon font.
#[test]
fn no_middle_dots_or_arrow_characters() {
    const NOT_USED: [char; 13] = ['\u{B7}', '\u{2022}', '\u{2190}', '\u{2191}', '\u{2192}', '\u{2193}', '\u{2039}', '\u{203A}', '\u{AB}', '\u{BB}', '\u{25B2}', '\u{25BC}', '\u{21D2}'];
    let base = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut all = Vec::new();
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| ["rs", "md", "txt", "adml", "admx", "wxl", "iss"].contains(&x.to_str().unwrap_or(""))) {
                out.push(p);
            }
        }
    }
    for dir in ["src", "crates", "PolicyDefinitions", "installer"] {
        walk(&base.join(dir), &mut all);
    }
    all.push(base.join("README.md"));
    let mut found = Vec::new();
    for f in all {
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        for (i, line) in text.lines().enumerate() {
            if line.chars().any(|c| NOT_USED.contains(&c)) {
                found.push(format!("{}:{}: {}", f.display(), i + 1, line.trim()));
            }
        }
    }
    assert!(found.is_empty(), "middle dots or arrow characters:\n{}", found.join("\n"));
}

#[test]
fn the_scanner_reads_rust_as_written() {
    let src = "let a = tr(\"Open\");\nlog::warn!(\"Not \\\"shown\\\"\");\nlet b = \"Shown to\\n a person\";\nlet c = 'x';\nfn f<'a>(s: &'a str) {}\n// \"In a comment\"\nlet d = trn!(n, \"{n} item\", \"{n} items\");\nlet e = r\"C:\\Raw\";\nlet g = trf!(\"{a} of {b}\", a = x, b = \"Not marked\");\n";
    let lits = literals(src);
    let got: Vec<(&str, bool)> = lits.iter().map(|l| (l.value.as_str(), l.marked)).collect();
    assert_eq!(got, [("Open", true), ("Shown to\n a person", false), ("{n} item", true), ("{n} items", true), ("C:\\Raw", false), ("{a} of {b}", true), ("Not marked", true)]);
    assert!(reads_like_words("Open") && reads_like_words("name contains both words") && !reads_like_words("explorer_look") && !reads_like_words("KB"));
    assert_eq!(fill("{a} of {b}, {c}", &[("a", &1), ("b", &"two")]), "1 of two, {c}");
}

#[test]
fn text_follows_the_language() {
    // without switching what's shown, as other tests run alongside
    assert_eq!(tr_in(Lang::En, "Settings"), "Settings");
    assert_ne!(tr_in(Lang::De, "Settings"), "Settings", "German has its own word for it");
    assert_ne!(tr_in(Lang::It, "Settings"), "Settings", "and Italian");
    assert_eq!(Lang::from_setting("it"), Lang::It);
    assert_eq!(Lang::from_setting("system"), system());
}
