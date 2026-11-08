//! The words Daleris shows, in English, German or Italian.
//!
//! The English text in the code is the key: `tr("Open")` gives "Öffnen" in German. Text with values in it names them,
//! `trf!("{n} selected", n = count)`, and text that changes with a count has both forms, `trn!(count, "{n} item",
//! "{n} items")`. Text kept in tables is marked with `tx("…")` where it's written and looked up with `tr` where it's
//! shown. The catalogs are in `de/` and `it/`, one file per part of the app; the tests in `tests.rs` make sure every
//! marked text has a translation, and that no text the person sees is left unmarked.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

mod de;
mod it;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    En,
    De,
    It,
}

/// The choices for the `language` setting, as (settings file value, name in its own language). "system" follows the
/// language Windows (or the desktop) is shown in.
pub const CHOICES: [(&str, &str); 4] = [("system", ""), ("en", "English"), ("de", "Deutsch"), ("it", "Italiano")];

static CURRENT: AtomicU8 = AtomicU8::new(0);

impl Lang {
    fn from_code(code: &str) -> Option<Lang> {
        match code.get(..2)?.to_ascii_lowercase().as_str() {
            "en" => Some(Lang::En),
            "de" => Some(Lang::De),
            "it" => Some(Lang::It),
            _ => None,
        }
    }

    /// "en", "de" or "it"
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::De => "de",
            Lang::It => "it",
        }
    }

    /// The language a `language` setting stands for: its own, or for "system" (or anything else) the system's.
    pub fn from_setting(value: &str) -> Lang {
        if value == "system" { system() } else { Lang::from_code(value).unwrap_or_else(system) }
    }
}

/// The language text is shown in now.
pub fn current() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Lang::De,
        2 => Lang::It,
        _ => Lang::En,
    }
}

/// Show text in `lang` from now on. Cheap: the window can call it every frame.
pub fn set(lang: Lang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

/// The language the system shows itself in, if Daleris has it; English otherwise. Test hook: DALERIS_TEST_LANG.
pub fn system() -> Lang {
    static SYSTEM: OnceLock<Lang> = OnceLock::new();
    if let Some(l) = std::env::var("DALERIS_TEST_LANG").ok().and_then(|v| Lang::from_code(&v)) {
        return l;
    }
    *SYSTEM.get_or_init(|| system_code().and_then(|c| Lang::from_code(&c)).unwrap_or(Lang::En))
}

#[cfg(windows)]
fn system_code() -> Option<String> {
    // the display language chosen in Windows's settings, as "de-DE"
    let mut buf = [0u16; 85];
    let n = unsafe { windows::Win32::Globalization::GetUserDefaultLocaleName(&mut buf) };
    let lang = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
    // the UI language decides, the locale only when it's one Daleris has
    let ui = match lang & 0x3ff {
        0x07 => Some("de".to_string()),
        0x10 => Some("it".to_string()),
        0x09 => Some("en".to_string()),
        _ => None,
    };
    ui.or_else(|| (n > 1).then(|| String::from_utf16_lossy(&buf[..n as usize - 1])))
}

#[cfg(not(windows))]
fn system_code() -> Option<String> {
    // LANGUAGE can list several, "de:en"; LC_ALL and LC_MESSAGES win over LANG
    for var in ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(var) {
            let first = v.split(':').next().unwrap_or("").trim().to_string();
            if !first.is_empty() && first != "C" && first != "POSIX" {
                return Some(first);
            }
        }
    }
    if cfg!(target_os = "macos") {
        // ( "de-DE", "en-US" ): the first one listed
        let out = std::process::Command::new("defaults").args(["read", "-g", "AppleLanguages"]).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        return text.split('"').nth(1).map(str::to_string);
    }
    None
}

fn table(lang: Lang) -> &'static HashMap<&'static str, &'static str> {
    static DE: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    static IT: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    let (slot, parts) = match lang {
        Lang::It => (&IT, it::ALL),
        _ => (&DE, de::ALL),
    };
    slot.get_or_init(|| parts.iter().flat_map(|p| p.iter().copied()).collect())
}

/// `en` in the language text is shown in now. Text without a translation (which the tests don't let happen) stays
/// English.
pub fn tr(en: &'static str) -> &'static str {
    tr_in(current(), en)
}

/// `en` in `lang`, whatever is shown now.
pub fn tr_in(lang: Lang, en: &'static str) -> &'static str {
    match lang {
        Lang::En => en,
        lang => table(lang).get(en).copied().unwrap_or(en),
    }
}

/// Text kept in a table, marked for translation where it's written; `tr` gives it in the language shown.
pub const fn tx(en: &'static str) -> &'static str {
    en
}

/// Text that isn't known when the code is written, such as a table's entry passed along as a `String`: its
/// translation if it has one.
pub fn tr_dyn(en: &str) -> String {
    match current() {
        Lang::En => en.to_string(),
        lang => table(lang).get(en).map_or_else(|| en.to_string(), |t| t.to_string()),
    }
}

/// `text` with each `{name}` replaced by the value given for it.
pub fn fill(text: &str, args: &[(&str, &dyn Display)]) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let mut rest = text;
    while let Some(at) = rest.find('{') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        match after.find('}').map(|end| (&after[..end], end)).filter(|(name, _)| args.iter().any(|(k, _)| k == name)) {
            Some((name, end)) => {
                let (_, v) = args.iter().find(|(k, _)| *k == name).expect("found above");
                out.push_str(&v.to_string());
                rest = &after[end + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Translated text with values in it: `trf!("Copying {name}", name = file)`.
#[macro_export]
macro_rules! trf {
    ($en:literal $(, $k:ident = $v:expr)* $(,)?) => {
        $crate::i18n::fill($crate::i18n::tr($en), &[$((stringify!($k), &$v as &dyn ::std::fmt::Display)),*])
    };
}

/// Translated text for a count: `trn!(n, "{n} item", "{n} items")`, the first when there's one. German and Italian
/// have the same two forms. Further values as with `trf!`.
#[macro_export]
macro_rules! trn {
    ($n:expr, $one:literal, $other:literal $(, $k:ident = $v:expr)* $(,)?) => {{
        let n = $n;
        let text = if n == 1 { $crate::i18n::tr($one) } else { $crate::i18n::tr($other) };
        $crate::i18n::fill(text, &[("n", &$crate::util::fmt_count(n as usize) as &dyn ::std::fmt::Display) $(, (stringify!($k), &$v as &dyn ::std::fmt::Display))*])
    }};
}

/// The decimal separator: "2,5" in German and Italian.
pub fn decimal() -> char {
    if current() == Lang::En { '.' } else { ',' }
}

/// The thousands separator: "1.234" in German and Italian.
pub fn thousands() -> char {
    if current() == Lang::En { ',' } else { '.' }
}
