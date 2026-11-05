//! What the file dialog DLL and the picker window say to each other, over pipes only the two of them hold.
//!
//! One line each way per message. Paths can hold what Unicode can't, which `\uXXXX` keeps.

use std::ffi::{OsStr, OsString};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Open,
    Save,
    Folder,
}

/// One dialog, as far as the picker needs to know.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PickRequest {
    pub kind: Kind,
    pub multi: bool,
    pub must_exist: bool,
    pub overwrite_prompt: bool,
    pub title: String,
    pub ok_label: String,
    pub name_label: String,
    pub file_name: String,
    pub folder: OsString,
    pub default_ext: String,
    /// (what the person reads, patterns such as `*.txt;*.md`)
    pub filters: Vec<(String, String)>,
    /// counted from 0
    pub filter_index: usize,
    /// the app's window, as a number
    pub owner: isize,
    /// the app's name, for the title bar
    pub host: String,
    /// what the app added through `IFileDialogCustomize`, in the order it added them
    pub controls: Vec<Control>,
}

/// `CDCS_ENABLED` and `CDCS_VISIBLE`.
pub const STATE_ENABLED: u32 = 1;
pub const STATE_VISIBLE: u32 = 2;
pub const STATE_NORMAL: u32 = STATE_ENABLED | STATE_VISIBLE;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ControlKind {
    #[default]
    Check,
    Combo,
    Radio,
    Edit,
    Text,
    Button,
    Menu,
    /// the choices `EnableOpenDropDown` puts on the OK button
    OpenDropDown,
    Separator,
    /// a heading the controls added until `EndVisualGroup` belong to
    Group,
}

impl ControlKind {
    fn word(self) -> &'static str {
        match self {
            ControlKind::Check => "check",
            ControlKind::Combo => "combo",
            ControlKind::Radio => "radio",
            ControlKind::Edit => "edit",
            ControlKind::Text => "text",
            ControlKind::Button => "button",
            ControlKind::Menu => "menu",
            ControlKind::OpenDropDown => "dropdown",
            ControlKind::Separator => "separator",
            ControlKind::Group => "group",
        }
    }

    fn from_word(w: &str) -> Option<ControlKind> {
        [ControlKind::Check, ControlKind::Combo, ControlKind::Radio, ControlKind::Edit, ControlKind::Text, ControlKind::Button, ControlKind::Menu, ControlKind::OpenDropDown, ControlKind::Separator, ControlKind::Group]
            .into_iter()
            .find(|k| k.word() == w)
    }

    /// Can it hold items, selectable ones or not?
    pub fn has_items(self) -> bool {
        matches!(self, ControlKind::Combo | ControlKind::Radio | ControlKind::Menu | ControlKind::OpenDropDown)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ControlItem {
    pub id: u32,
    pub label: String,
    pub state: u32,
}

/// One control an app added to its dialog.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Control {
    pub id: u32,
    pub kind: ControlKind,
    pub label: String,
    /// an edit box's text
    pub text: String,
    pub checked: bool,
    pub selected: Option<u32>,
    pub state: u32,
    pub items: Vec<ControlItem>,
    /// the `Group` control this is in
    pub group: Option<u32>,
    pub prominent: bool,
}

fn encode_controls(controls: &[Control]) -> String {
    let mut out = String::new();
    let opt = |v: Option<u32>| v.map_or("-".to_string(), |n| n.to_string());
    for c in controls {
        out += &format!("control\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n", c.id, c.kind.word(), c.state, c.checked as u8, opt(c.selected), opt(c.group), c.prominent as u8, esc_text(&c.label), esc_text(&c.text));
        for i in &c.items {
            out += &format!("item\t{}\t{}\t{}\t{}\n", c.id, i.id, i.state, esc_text(&i.label));
        }
    }
    out
}

/// One line of `encode_controls` into the list being built. False if it isn't one, or doesn't make sense.
fn decode_control_line(line: &str, controls: &mut Vec<Control>) -> bool {
    let opt = |v: &str| if v == "-" { Some(None) } else { v.parse().ok().map(Some) };
    let f: Vec<&str> = line.split('\t').collect();
    match f.first().copied() {
        Some("control") if f.len() >= 10 => {
            let (Ok(id), Some(kind), Ok(state), Some(selected), Some(group)) = (f[1].parse(), ControlKind::from_word(f[2]), f[3].parse(), opt(f[5]), opt(f[6])) else { return false };
            controls.push(Control { id, kind, state, checked: f[4] == "1", selected, group, prominent: f[7] == "1", label: unesc_text(f[8]), text: unesc_text(f[9]), items: Vec::new() });
            true
        }
        Some("item") if f.len() >= 5 => {
            let (Ok(control), Ok(id), Ok(state)) = (f[1].parse::<u32>(), f[2].parse(), f[3].parse()) else { return false };
            match controls.iter_mut().rev().find(|c| c.id == control) {
                Some(c) => {
                    c.items.push(ControlItem { id, state, label: unesc_text(f[4]) });
                    true
                }
                None => false,
            }
        }
        _ => false,
    }
}

/// What the app tells the picker while it's up, one or more lines each.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostMessage {
    Controls(Vec<Control>),
    /// The app is handling one of its controls, and may be showing a window of its own for it.
    Busy(bool),
    SetName(String),
    SetFilter(usize),
    SetFolder(OsString),
}

impl HostMessage {
    pub fn encode(&self) -> String {
        match self {
            HostMessage::Controls(list) => format!("controls\n{}controls-end\n", encode_controls(list)),
            HostMessage::Busy(on) => format!("busy\t{}\n", *on as u8),
            HostMessage::SetName(n) => format!("set-name\t{}\n", esc_text(n)),
            HostMessage::SetFilter(i) => format!("set-filter\t{i}\n"),
            HostMessage::SetFolder(p) => format!("set-folder\t{}\n", esc_os(p)),
        }
    }
}

/// Puts `HostMessage`s back together from lines.
#[derive(Default)]
pub struct HostDecoder {
    building: Option<Vec<Control>>,
}

impl HostDecoder {
    pub fn feed(&mut self, line: &str) -> Option<HostMessage> {
        let line = line.trim_end_matches(['\r', '\n']);
        if let Some(list) = &mut self.building {
            if line == "controls-end" {
                return self.building.take().map(HostMessage::Controls);
            }
            decode_control_line(line, list);
            return None;
        }
        let (key, value) = line.split_once('\t').unwrap_or((line, ""));
        match key {
            "controls" => {
                self.building = Some(Vec::new());
                None
            }
            "busy" => Some(HostMessage::Busy(value == "1")),
            "set-name" => Some(HostMessage::SetName(unesc_text(value))),
            "set-filter" => value.parse().ok().map(HostMessage::SetFilter),
            "set-folder" => Some(HostMessage::SetFolder(unesc_os(value))),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickReply {
    /// The window is on the screen.
    Ready,
    Picked { filter_index: usize, paths: Vec<OsString> },
    Cancelled,
    /// The person asked for Windows' dialog instead.
    UseWindows,
    Check { id: u32, on: bool },
    Item { control: u32, item: u32 },
    Button { id: u32 },
    Activating { id: u32 },
    Text { id: u32, text: String },
    /// Where the person is now, sent whenever it changes, for what the app reads or is told about while the picker is up.
    State { filter_index: usize, folder: OsString, name: String, selection: OsString },
}


fn esc_text(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            '\\' => vec!['\\', '\\'],
            '\n' => vec!['\\', 'n'],
            '\r' => vec!['\\', 'r'],
            '\t' => vec!['\\', 't'],
            c => vec![c],
        })
        .collect()
}

#[cfg(windows)]
fn esc_os(s: &OsStr) -> String {
    use std::os::windows::ffi::OsStrExt;
    let mut out = String::new();
    for c in char::decode_utf16(s.encode_wide()) {
        match c {
            Ok(c) => out.push_str(&esc_text(c.encode_utf8(&mut [0; 4]))),
            Err(e) => out.push_str(&format!("\\u{:04X}", e.unpaired_surrogate())),
        }
    }
    out
}

#[cfg(not(windows))]
fn esc_os(s: &OsStr) -> String {
    esc_text(&s.to_string_lossy())
}

fn unesc_wide(s: &str) -> Vec<u16> {
    let mut out: Vec<u16> = Vec::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        let c = if c == '\\' {
            match chars.next() {
                Some('n') => '\n',
                Some('r') => '\r',
                Some('t') => '\t',
                Some('u') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    out.extend(u16::from_str_radix(&hex, 16).ok());
                    continue;
                }
                Some(other) => other,
                None => break,
            }
        } else {
            c
        };
        out.extend(c.encode_utf16(&mut [0; 2]).iter());
    }
    out
}

#[cfg(windows)]
fn unesc_os(s: &str) -> OsString {
    use std::os::windows::ffi::OsStringExt;
    OsString::from_wide(&unesc_wide(s))
}

#[cfg(not(windows))]
fn unesc_os(s: &str) -> OsString {
    OsString::from(unesc_text(s))
}

fn unesc_text(s: &str) -> String {
    String::from_utf16_lossy(&unesc_wide(s))
}

impl PickRequest {
    pub fn encode(&self) -> String {
        let kind = match self.kind {
            Kind::Open => "open",
            Kind::Save => "save",
            Kind::Folder => "folder",
        };
        let mut out = format!("kind\t{kind}\nmulti\t{}\nmust_exist\t{}\noverwrite\t{}\nindex\t{}\nowner\t{}\n", self.multi as u8, self.must_exist as u8, self.overwrite_prompt as u8, self.filter_index, self.owner);
        for (key, value) in [("title", &self.title), ("ok", &self.ok_label), ("label", &self.name_label), ("name", &self.file_name), ("ext", &self.default_ext), ("host", &self.host)] {
            out += &format!("{key}\t{}\n", esc_text(value));
        }
        out += &format!("folder\t{}\n", esc_os(&self.folder));
        out += &encode_controls(&self.controls);
        for (name, spec) in &self.filters {
            out += &format!("filter\t{}\t{}\n", esc_text(name), esc_text(spec));
        }
        out += "end\n";
        out
    }

    /// From the lines of `encode`. Unknown lines are skipped, for a picker newer or older than the DLL in an app.
    pub fn decode(text: &str) -> Option<PickRequest> {
        let mut r = PickRequest::default();
        let mut complete = false;
        for line in text.lines() {
            let mut parts = line.split('\t');
            let key = parts.next()?;
            let value = parts.next().unwrap_or("");
            match key {
                "kind" => {
                    r.kind = match value {
                        "open" => Kind::Open,
                        "save" => Kind::Save,
                        "folder" => Kind::Folder,
                        _ => return None,
                    }
                }
                "multi" => r.multi = value == "1",
                "must_exist" => r.must_exist = value == "1",
                "overwrite" => r.overwrite_prompt = value == "1",
                "index" => r.filter_index = value.parse().ok()?,
                "owner" => r.owner = value.parse().ok()?,
                "title" => r.title = unesc_text(value),
                "ok" => r.ok_label = unesc_text(value),
                "label" => r.name_label = unesc_text(value),
                "name" => r.file_name = unesc_text(value),
                "ext" => r.default_ext = unesc_text(value),
                "host" => r.host = unesc_text(value),
                "folder" => r.folder = unesc_os(value),
                "filter" => r.filters.push((unesc_text(value), unesc_text(parts.next().unwrap_or("")))),
                "control" | "item" => {
                    decode_control_line(line, &mut r.controls);
                }
                "end" => complete = true,
                _ => {}
            }
        }
        complete.then_some(r)
    }
}

impl PickReply {
    pub fn encode(&self) -> String {
        match self {
            PickReply::Ready => "ready\n".into(),
            PickReply::Cancelled => "cancel\n".into(),
            PickReply::UseWindows => "windows\n".into(),
            PickReply::Check { id, on } => format!("check\t{id}\t{}\n", *on as u8),
            PickReply::Item { control, item } => format!("selected\t{control}\t{item}\n"),
            PickReply::Button { id } => format!("button\t{id}\n"),
            PickReply::Activating { id } => format!("activating\t{id}\n"),
            PickReply::Text { id, text } => format!("text\t{id}\t{}\n", esc_text(text)),
            PickReply::State { filter_index, folder, name, selection } => format!("state\t{filter_index}\t{}\t{}\t{}\n", esc_os(folder), esc_text(name), esc_os(selection)),
            PickReply::Picked { filter_index, paths } => {
                let list: Vec<String> = paths.iter().map(|p| esc_os(p)).collect();
                format!("picked\t{filter_index}\t{}\n", list.join("\t"))
            }
        }
    }

    pub fn decode(line: &str) -> Option<PickReply> {
        let mut parts = line.trim_end_matches(['\r', '\n']).split('\t');
        match parts.next()? {
            "ready" => Some(PickReply::Ready),
            "cancel" => Some(PickReply::Cancelled),
            "windows" => Some(PickReply::UseWindows),
            "check" => Some(PickReply::Check { id: parts.next()?.parse().ok()?, on: parts.next()? == "1" }),
            "selected" => Some(PickReply::Item { control: parts.next()?.parse().ok()?, item: parts.next()?.parse().ok()? }),
            "button" => Some(PickReply::Button { id: parts.next()?.parse().ok()? }),
            "activating" => Some(PickReply::Activating { id: parts.next()?.parse().ok()? }),
            "text" => Some(PickReply::Text { id: parts.next()?.parse().ok()?, text: unesc_text(parts.next().unwrap_or("")) }),
            "state" => Some(PickReply::State { filter_index: parts.next()?.parse().ok()?, folder: unesc_os(parts.next()?), name: unesc_text(parts.next()?), selection: unesc_os(parts.next().unwrap_or("")) }),
            "picked" => {
                let filter_index = parts.next()?.parse().ok()?;
                let paths: Vec<OsString> = parts.map(unesc_os).collect();
                (!paths.is_empty()).then_some(PickReply::Picked { filter_index, paths })
            }
            _ => None,
        }
    }
}

/// Does a file name match patterns such as `*.txt;*.md`? No patterns, or `*.*`, is everything.
pub fn matches_filter(name: &str, spec: &str) -> bool {
    let lower = name.to_lowercase();
    let mut any = false;
    for pattern in spec.split(';').map(|p| p.trim().to_lowercase()).filter(|p| !p.is_empty()) {
        any = true;
        if pattern == "*" || pattern == "*.*" || wildcard(pattern.as_bytes(), lower.as_bytes()) {
            return true;
        }
    }
    !any
}

fn wildcard(pattern: &[u8], text: &[u8]) -> bool {
    match pattern.split_first() {
        None => text.is_empty(),
        Some((b'*', rest)) => (0..=text.len()).any(|i| wildcard(rest, &text[i..])),
        Some((b'?', rest)) => !text.is_empty() && wildcard(rest, &text[1..]),
        Some((c, rest)) => text.first() == Some(c) && wildcard(rest, &text[1..]),
    }
}

/// The first extension a filter names, without its dot, for adding to a name typed without one.
pub fn first_extension(spec: &str) -> Option<&str> {
    spec.split(';').map(str::trim).find_map(|p| p.strip_prefix("*.")).filter(|e| !e.is_empty() && !e.contains(['*', '?']))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_survives_the_trip() {
        let r = PickRequest {
            kind: Kind::Save,
            multi: true,
            must_exist: true,
            overwrite_prompt: true,
            title: "Save\tas\nnew \\ thing".into(),
            ok_label: "Keep".into(),
            name_label: "Name:".into(),
            file_name: "a b.txt".into(),
            folder: OsString::from(r"C:\Temp\ä"),
            default_ext: "txt".into(),
            filters: vec![("Text".into(), "*.txt;*.md".into()), ("All".into(), "*.*".into())],
            filter_index: 1,
            owner: -5,
            host: "app.exe".into(),
            controls: sample_controls(),
        };
        assert_eq!(PickRequest::decode(&r.encode()), Some(r));
    }

    fn sample_controls() -> Vec<Control> {
        vec![
            Control { id: 1, kind: ControlKind::Group, label: "Group\ta".into(), state: STATE_NORMAL, ..Default::default() },
            Control { id: 2, kind: ControlKind::Check, label: "Thumb".into(), checked: true, state: STATE_ENABLED, group: Some(1), prominent: true, ..Default::default() },
            Control {
                id: 3,
                kind: ControlKind::Combo,
                selected: Some(11),
                state: STATE_NORMAL,
                items: vec![ControlItem { id: 10, label: "one".into(), state: STATE_NORMAL }, ControlItem { id: 11, label: "two".into(), state: 0 }],
                ..Default::default()
            },
            Control { id: 4, kind: ControlKind::Edit, text: "a\nb".into(), state: STATE_NORMAL, ..Default::default() },
        ]
    }

    #[test]
    fn what_the_app_says_while_the_picker_is_up_survives_the_trip() {
        let messages = [HostMessage::Controls(sample_controls()), HostMessage::Busy(true), HostMessage::SetName("x y.txt".into()), HostMessage::SetFilter(2), HostMessage::SetFolder(OsString::from(r"C:\a")), HostMessage::Controls(Vec::new())];
        let mut decoder = HostDecoder::default();
        let mut got = Vec::new();
        for m in &messages {
            for line in m.encode().lines() {
                got.extend(decoder.feed(line));
            }
        }
        assert_eq!(got, messages);
    }

    #[test]
    fn what_the_picker_says_while_its_up_survives_the_trip() {
        for r in [
            PickReply::Check { id: 5, on: true },
            PickReply::Item { control: 5, item: 6 },
            PickReply::Button { id: 7 },
            PickReply::Activating { id: 8 },
            PickReply::Text { id: 9, text: "tab\there".into() },
            PickReply::State { filter_index: 1, folder: OsString::from(r"C:\a b"), name: "n.txt".into(), selection: OsString::new() },
        ] {
            assert_eq!(PickReply::decode(&r.encode()), Some(r));
        }
    }

    #[test]
    fn half_a_request_is_no_request() {

        let r = PickRequest::default().encode();
        assert!(PickRequest::decode(r.trim_end_matches("end\n")).is_none());
        assert!(PickRequest::decode("kind\tnonsense\nend\n").is_none());
        assert!(PickRequest::decode("from the future\tx\nend\n").is_some());
    }

    #[cfg(windows)]
    #[test]
    fn names_that_arent_unicode_survive() {
        use std::os::windows::ffi::OsStringExt;
        let odd = OsString::from_wide(&[0x43, 0x3a, 0x5c, 0xd800, 0x41]);
        let r = PickReply::Picked { filter_index: 2, paths: vec![odd, OsString::from(r"C:\a\b c.txt")] };
        assert_eq!(PickReply::decode(&r.encode()), Some(r));
    }

    #[test]
    fn replies_are_understood_and_nonsense_isnt() {
        for r in [PickReply::Ready, PickReply::Cancelled, PickReply::UseWindows] {
            assert_eq!(PickReply::decode(&r.encode()), Some(r));
        }
        assert_eq!(PickReply::decode("picked\tx\ty"), None);
        assert_eq!(PickReply::decode("picked\t0"), None);
        assert_eq!(PickReply::decode("whatever"), None);
    }

    #[test]
    fn filters_match_like_windows() {
        assert!(matches_filter("A.TXT", "*.txt;*.md"));
        assert!(matches_filter("readme.md", "*.txt;*.md"));
        assert!(!matches_filter("a.png", "*.txt;*.md"));
        assert!(matches_filter("a.png", "*.*"));
        assert!(matches_filter("a", "*.*"));
        assert!(matches_filter("a.png", ""));
        assert!(matches_filter("photo1.jpg", "photo?.jpg"));
        assert_eq!(first_extension("*.txt;*.md"), Some("txt"));
        assert_eq!(first_extension("*.*"), None);
    }
}
