use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, TextStyle};

/// The colours of the current theme. Read through the `BG()`, `TEXT()`… functions.
#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub bg: Color32,
    pub panel: Color32,
    pub chrome: Color32,
    pub raised: Color32,
    pub hover: Color32,
    pub sel: Color32,
    pub sel_strong: Color32,
    pub accent: Color32,
    pub text: Color32,
    pub dim: Color32,
    pub faint: Color32,
    pub line: Color32,
    pub green: Color32,
    pub cloud: Color32,
    pub warn: Color32,
    /// something that works, but asks first, such as for administrator rights
    pub caution: Color32,
    pub code_bg: Color32,
    /// the ribbon bar under the address bar
    pub ribbon: Color32,
    /// the sidebar and the preview pane
    pub side: Color32,
    pub status: Color32,
    /// the places on This PC
    pub tile: Color32,
    /// rows and places in lists, which Explorer colours differently from its buttons
    pub row_hover: Color32,
    pub row_sel: Color32,
    pub row_cursor: Color32,
    /// the address bar and search box, and their outline
    pub field: Color32,
    pub field_line: Color32,
    /// the tab in front and the address bar row under it
    pub top: Color32,
    /// Explorer's look, where it differs in shape as well as colour
    pub explorer: bool,
}

/// Accent colour choices (name, colour on dark, colour on light).
pub const ACCENTS: [(&str, Color32, Color32); 7] = [
    ("Blue", Color32::from_rgb(0x4c, 0xa3, 0xff), Color32::from_rgb(0x00, 0x5f, 0xb8)),
    ("Teal", Color32::from_rgb(0x2e, 0xc4, 0xb6), Color32::from_rgb(0x00, 0x7a, 0x70)),
    ("Purple", Color32::from_rgb(0xa8, 0x8b, 0xff), Color32::from_rgb(0x6b, 0x3f, 0xd4)),
    ("Pink", Color32::from_rgb(0xff, 0x6f, 0xb1), Color32::from_rgb(0xc2, 0x18, 0x6b)),
    ("Orange", Color32::from_rgb(0xff, 0xa2, 0x4c), Color32::from_rgb(0xc2, 0x5a, 0x00)),
    ("Green", Color32::from_rgb(0x5f, 0xd0, 0x7a), Color32::from_rgb(0x1e, 0x7e, 0x34)),
    ("Red", Color32::from_rgb(0xff, 0x6b, 0x6b), Color32::from_rgb(0xc4, 0x2b, 0x1c)),
];

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

impl Palette {
    pub fn new(dark: bool, accent: Color32) -> Palette {
        let p = Palette::own(dark, accent);
        Palette { top: p.panel, ribbon: p.panel, side: p.panel, status: p.chrome, tile: p.panel, row_hover: p.hover, row_sel: p.sel, row_cursor: p.sel_strong, field: p.raised, ..p }
    }

    /// Windows Explorer's colours, from the visual style it draws its lists with and the WinUI colours of its bars.
    /// Explorer's sidebar, preview pane and status bar are the same colour as its list, and its places have no boxes.
    pub fn explorer(dark: bool, accent: Color32) -> Palette {
        let own = Palette::own(dark, accent);
        let rgb = |v: u32| Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8);
        let (bg, bar, mica, hover, sel, idle, text, dim, faint, line) = if dark {
            (0x191919, 0x2c2c2c, 0x202020, 0x383838, 0x626262, 0x4d4d4d, 0xffffff, 0xcdcdcd, 0x969696, 0x333333)
        } else {
            (0xffffff, 0xf9f9f9, 0xf3f3f3, 0xf0f0f0, 0xcce8ff, 0xe5f3ff, 0x1a1a1a, 0x5d5d5d, 0x8d8d8d, 0xe5e5e5)
        };
        // Explorer's address and search boxes stand out by their fill alone, with no outline
        let (field, field_line) = if dark { (rgb(0x373737), Color32::TRANSPARENT) } else { (rgb(0xffffff), Color32::TRANSPARENT) };
        Palette {
            bg: rgb(bg),
            panel: rgb(bar),
            chrome: rgb(mica),
            raised: rgb(bar),
            hover: rgb(hover),
            text: rgb(text),
            dim: rgb(dim),
            faint: rgb(faint),
            line: rgb(line),
            ribbon: rgb(bg),
            side: rgb(bg),
            status: rgb(bg),
            tile: Color32::TRANSPARENT,
            row_hover: rgb(idle),
            row_sel: rgb(sel),
            row_cursor: rgb(sel),
            field,
            field_line,
            top: rgb(bar),
            explorer: true,
            ..own
        }
    }

    fn own(dark: bool, accent: Color32) -> Palette {
        let none = Color32::TRANSPARENT;
        if dark {
            let bg = Color32::from_rgb(0x1b, 0x1b, 0x1d);
            Palette {
                dark,
                bg,
                panel: Color32::from_rgb(0x22, 0x22, 0x25),
                chrome: Color32::from_rgb(0x16, 0x16, 0x18),
                raised: Color32::from_rgb(0x2c, 0x2c, 0x30),
                hover: Color32::from_rgb(0x2e, 0x2f, 0x34),
                sel: mix(bg, accent, 0.28),
                sel_strong: mix(bg, accent, 0.40),
                accent,
                text: Color32::from_rgb(0xe8, 0xe8, 0xea),
                dim: Color32::from_rgb(0x9c, 0x9e, 0xa6),
                faint: Color32::from_rgb(0x5d, 0x5f, 0x66),
                line: Color32::from_rgb(0x33, 0x34, 0x39),
                green: Color32::from_rgb(0x3f, 0xc1, 0x6b),
                cloud: Color32::from_rgb(0x6c, 0xb4, 0xff),
                warn: Color32::from_rgb(0xe8, 0x6a, 0x5a),
                caution: Color32::from_rgb(0xe8, 0xa2, 0x3d),
                code_bg: Color32::from_rgb(0x16, 0x16, 0x18),
                ribbon: none,
                side: none,
                status: none,
                tile: none,
                row_hover: none,
                row_sel: none,
                row_cursor: none,
                field: none,
                field_line: none,
                top: none,
                explorer: false,
            }
        } else {
            let bg = Color32::from_rgb(0xfb, 0xfb, 0xfc);
            Palette {
                dark,
                bg,
                panel: Color32::from_rgb(0xf3, 0xf3, 0xf5),
                chrome: Color32::from_rgb(0xe6, 0xe7, 0xea),
                raised: Color32::from_rgb(0xff, 0xff, 0xff),
                hover: Color32::from_rgb(0xe4, 0xe5, 0xe9),
                sel: mix(bg, accent, 0.16),
                sel_strong: mix(bg, accent, 0.26),
                accent,
                text: Color32::from_rgb(0x1b, 0x1b, 0x1f),
                dim: Color32::from_rgb(0x5c, 0x5e, 0x66),
                faint: Color32::from_rgb(0x9a, 0x9c, 0xa4),
                line: Color32::from_rgb(0xd9, 0xda, 0xde),
                green: Color32::from_rgb(0x1e, 0x8e, 0x45),
                cloud: Color32::from_rgb(0x1f, 0x74, 0xd1),
                warn: Color32::from_rgb(0xc4, 0x41, 0x3a),
                caution: Color32::from_rgb(0xb4, 0x62, 0x0c),
                code_bg: Color32::from_rgb(0xee, 0xef, 0xf2),
                ribbon: none,
                side: none,
                status: none,
                tile: none,
                row_hover: none,
                row_sel: none,
                row_cursor: none,
                field: none,
                field_line: none,
                top: none,
                explorer: false,
            }
        }
    }
}

/// The window was made see-through at start, so Mica can show behind it. Only then can the palette leave the tab
/// strip empty.
static MICA_WINDOW: AtomicBool = AtomicBool::new(false);

#[cfg_attr(not(windows), allow(dead_code))]
pub fn set_mica_window(on: bool) {
    MICA_WINDOW.store(on, Ordering::Relaxed);
}

pub fn mica_window() -> bool {
    MICA_WINDOW.load(Ordering::Relaxed)
}

thread_local! {
    static PAL: std::cell::Cell<Palette> = std::cell::Cell::new(Palette::new(true, ACCENTS[0].1));
}

pub fn pal() -> Palette {
    PAL.with(|p| p.get())
}

macro_rules! colour_fns {
    ($($name:ident => $field:ident),* $(,)?) => {
        $( #[allow(non_snake_case)] #[inline] pub fn $name() -> Color32 { PAL.with(|p| p.get().$field) } )*
    };
}
colour_fns!(BG => bg, PANEL => panel, TOP => top, CHROME => chrome, RAISED => raised, HOVER => hover, SEL => sel, SEL_STRONG => sel_strong,
    ACCENT => accent, TEXT => text, DIM => dim, FAINT => faint, LINE => line, GREEN => green, CLOUD_BLUE => cloud, WARN => warn, CAUTION => caution,
    RIBBON => ribbon,     SIDE => side, STATUS => status, TILE => tile, ROW_HOVER => row_hover, ROW_SEL => row_sel, ROW_CURSOR => row_cursor, FIELD => field, FIELD_LINE => field_line);

/// Is Windows itself in dark mode (apps setting)?
#[cfg(windows)]
pub fn system_dark() -> bool {
    // tests look at both themes without changing the PC's
    if let Ok(t) = std::env::var("DALERIS_TEST_THEME") {
        return t != "light";
    }
    use windows::core::w;
    use windows::Win32::System::Registry::*;
    let mut v: u32 = 1;
    let mut len = 4u32;
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut v as *mut u32 as *mut _),
            Some(&mut len),
        )
    };
    rc.is_err() || v == 0
}

/// Windows' own accent colour, if set.
#[cfg(windows)]
pub fn system_accent() -> Option<Color32> {
    use windows::core::w;
    use windows::Win32::System::Registry::*;
    let mut v: u32 = 0;
    let mut len = 4u32;
    let rc = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, w!(r"Software\Microsoft\Windows\DWM"), w!("AccentColor"), RRF_RT_REG_DWORD, None, Some(&mut v as *mut u32 as *mut _), Some(&mut len))
    };
    // stored as 0xAABBGGRR
    rc.is_ok().then(|| Color32::from_rgb(v as u8, (v >> 8) as u8, (v >> 16) as u8))
}

/// Run a small system query and keep its answer for a while (it is polled every couple of seconds).
#[cfg(not(windows))]
fn cached_query(slot: &'static std::sync::Mutex<Option<(std::time::Instant, Option<String>)>>, cmd: &str, args: &[&str]) -> Option<String> {
    let mut g = slot.lock().unwrap();
    if let Some((at, v)) = g.as_ref() {
        if at.elapsed() < std::time::Duration::from_secs(10) {
            return v.clone();
        }
    }
    let v = std::process::Command::new(cmd)
        .args(args)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    *g = Some((std::time::Instant::now(), v.clone()));
    v
}

/// Is the system in dark mode? macOS: AppleInterfaceStyle; Linux: GNOME/GTK colour scheme.
#[cfg(not(windows))]
pub fn system_dark() -> bool {
    static SLOT: std::sync::Mutex<Option<(std::time::Instant, Option<String>)>> = std::sync::Mutex::new(None);
    if cfg!(target_os = "macos") {
        return cached_query(&SLOT, "defaults", &["read", "-g", "AppleInterfaceStyle"]).is_some_and(|v| v.contains("Dark"));
    }
    match cached_query(&SLOT, "gsettings", &["get", "org.gnome.desktop.interface", "color-scheme"]) {
        Some(v) if v.contains("dark") => true,
        Some(v) if v.contains("light") => false,
        _ => std::env::var("GTK_THEME").map(|t| t.to_lowercase().contains("dark")).unwrap_or(true),
    }
}

/// The system accent colour (macOS accent setting, GNOME 47+ accent-color).
#[cfg(not(windows))]
pub fn system_accent() -> Option<Color32> {
    static SLOT: std::sync::Mutex<Option<(std::time::Instant, Option<String>)>> = std::sync::Mutex::new(None);
    if cfg!(target_os = "macos") {
        // -1 graphite, 0 red, 1 orange, 2 yellow, 3 green, 4 blue (default, key absent), 5 purple, 6 pink
        let v = cached_query(&SLOT, "defaults", &["read", "-g", "AppleAccentColor"]);
        let n: i32 = v.as_deref().and_then(|s| s.parse().ok()).unwrap_or(4);
        return Some(match n {
            -1 => Color32::from_rgb(0x8c, 0x8c, 0x8c),
            0 => Color32::from_rgb(0xff, 0x52, 0x57),
            1 => Color32::from_rgb(0xf7, 0x82, 0x1b),
            2 => Color32::from_rgb(0xff, 0xc6, 0x00),
            3 => Color32::from_rgb(0x62, 0xba, 0x46),
            5 => Color32::from_rgb(0xa5, 0x50, 0xa7),
            6 => Color32::from_rgb(0xf7, 0x4f, 0x9e),
            _ => Color32::from_rgb(0x00, 0x7a, 0xff),
        });
    }
    let v = cached_query(&SLOT, "gsettings", &["get", "org.gnome.desktop.interface", "accent-color"])?;
    Some(match v.trim_matches('\'') {
        "teal" => Color32::from_rgb(0x21, 0x90, 0xa4),
        "green" => Color32::from_rgb(0x3a, 0x94, 0x4a),
        "yellow" => Color32::from_rgb(0xc8, 0x88, 0x00),
        "orange" => Color32::from_rgb(0xed, 0x5b, 0x00),
        "red" => Color32::from_rgb(0xe6, 0x2d, 0x42),
        "pink" => Color32::from_rgb(0xd5, 0x61, 0x99),
        "purple" => Color32::from_rgb(0x91, 0x41, 0xac),
        "slate" => Color32::from_rgb(0x6f, 0x83, 0x96),
        "blue" => Color32::from_rgb(0x35, 0x84, 0xe4),
        _ => return None,
    })
}

/// Apply a palette to the app and egui's widgets.
pub fn apply(ctx: &egui::Context, p: Palette) {
    PAL.with(|c| c.set(p));
    // Left to follow the system, egui would swap in its own stock style whenever the system's mode differs from `p`.
    ctx.set_theme(if p.dark { egui::Theme::Dark } else { egui::Theme::Light });
    let mut style = (*ctx.global_style()).clone();
    let v = &mut style.visuals;
    *v = if p.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = p.panel;
    v.window_fill = p.raised;
    v.extreme_bg_color = if p.dark { Color32::from_rgb(0x14, 0x14, 0x16) } else { Color32::WHITE };
    v.faint_bg_color = if p.dark { Color32::from_rgb(0x20, 0x20, 0x23) } else { Color32::from_rgb(0xf0, 0xf0, 0xf3) };
    v.override_text_color = Some(p.text);
    v.selection.bg_fill = p.sel_strong;
    v.selection.stroke = egui::Stroke::new(1.0_f32, p.accent);
    v.hyperlink_color = p.accent;
    v.window_stroke = egui::Stroke::new(1.0_f32, p.line);
    v.window_corner_radius = egui::CornerRadius::same(8);
    v.menu_corner_radius = egui::CornerRadius::same(8);
    v.window_shadow = egui::epaint::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(if p.dark { 110 } else { 40 }) };
    v.popup_shadow = v.window_shadow;
    for (w, fill) in [
        (&mut v.widgets.noninteractive, p.panel),
        (&mut v.widgets.inactive, p.raised),
        (&mut v.widgets.hovered, p.hover),
        (&mut v.widgets.active, p.sel),
        (&mut v.widgets.open, p.hover),
    ] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.corner_radius = egui::CornerRadius::same(5);
        w.bg_stroke = egui::Stroke::NONE;
        w.fg_stroke.color = p.text;
    }
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, p.line);
    v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    v.widgets.inactive.bg_stroke = egui::Stroke::NONE;
    ctx.set_global_style(style);
}

static BUNDLED_ICONS: AtomicBool = AtomicBool::new(false);

/// Everywhere but Windows, and on a Windows without its icon font, the app draws with the icons it
/// brings along. Test hook: DALERIS_TEST_BUNDLED_ICONS=1 shows them on Windows.
pub fn prefers_bundled_icons() -> bool {
    !cfg!(windows) || std::env::var_os("DALERIS_TEST_BUNDLED_ICONS").is_some()
}

pub fn icon_family() -> FontFamily {
    FontFamily::Name("icons".into())
}

pub fn icon_font(size: f32) -> FontId {
    FontId::new(size, icon_family())
}

/// The solid icons that files and folders are drawn with.
pub fn file_icon_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("file-icons".into()))
}

/// The glyph to draw: a Segoe Fluent / MDL2 one on Windows, its counterpart from the bundled set elsewhere.
pub fn ic(glyph: &'static str) -> &'static str {
    use egui_phosphor::regular as ph;
    if !BUNDLED_ICONS.load(Ordering::Relaxed) {
        return glyph;
    }
    match glyph {
        I_LOCK => ph::LOCK_SIMPLE,
        I_BACK => ph::ARROW_LEFT,
        I_FWD => ph::ARROW_RIGHT,
        I_UP => ph::ARROW_UP,
        I_REFRESH => ph::ARROW_CLOCKWISE,
        I_SEARCH => ph::MAGNIFYING_GLASS,
        I_ADD => ph::PLUS,
        I_CLOSE => ph::X,
        I_COPY => ph::COPY,
        I_CUT => ph::SCISSORS,
        I_PASTE => ph::CLIPBOARD,
        I_RENAME => ph::PENCIL_SIMPLE,
        I_EDIT => ph::PENCIL_SIMPLE_LINE,
        I_DELETE => ph::TRASH,
        I_NEWFOLDER => ph::FOLDER_PLUS,
        I_SORT => ph::ARROWS_DOWN_UP,
        I_LIST => ph::LIST_BULLETS,
        I_GRID => ph::SQUARES_FOUR,
        I_EYE => ph::EYE,
        I_PANE => ph::SIDEBAR_SIMPLE,
        I_CLOUD => ph::CLOUD,
        I_DRIVE => ph::HARD_DRIVE,
        I_NETDRIVE => ph::HARD_DRIVES,
        I_HOME => ph::HOUSE,
        I_PIN => ph::PUSH_PIN,
        I_MORE => ph::DOTS_THREE,
        I_CHEV_R => ph::CARET_RIGHT,
        I_CHEV_D => ph::CARET_DOWN,
        I_PC => ph::DESKTOP,
        I_TERMINAL => ph::TERMINAL_WINDOW,
        I_INFO => ph::INFO,
        I_FILTER => ph::FUNNEL,
        I_SETTINGS => ph::GEAR_SIX,
        I_CHECK => ph::CHECK,
        I_GRIP => ph::DOTS_SIX_VERTICAL,
        I_CHEV_U => ph::CARET_UP,
        I_HIDE => ph::EYE_SLASH,
        I_MIN => ph::MINUS,
        I_MAX => ph::SQUARE,
        I_RESTORE => ph::COPY_SIMPLE,
        I_WCLOSE => ph::X,
        I_PALETTE => ph::PALETTE,
        I_PICTURE => ph::IMAGE,
        I_NAVPANE => ph::SIDEBAR,
        I_FOLDER => ph::FOLDER,
        other => other,
    }
}

pub const I_LOCK: &str = "\u{E72E}";
pub const I_BACK: &str = "\u{E72B}";
pub const I_FWD: &str = "\u{E72A}";
pub const I_UP: &str = "\u{E74A}";
pub const I_REFRESH: &str = "\u{E72C}";
pub const I_SEARCH: &str = "\u{E721}";
pub const I_ADD: &str = "\u{E710}";
pub const I_CLOSE: &str = "\u{E711}";
pub const I_COPY: &str = "\u{E8C8}";
pub const I_CUT: &str = "\u{E8C6}";
pub const I_PASTE: &str = "\u{E77F}";
pub const I_RENAME: &str = "\u{E8AC}";
/// a pencil: something is being written
pub const I_EDIT: &str = "\u{E70F}";
pub const I_DELETE: &str = "\u{E74D}";
pub const I_NEWFOLDER: &str = "\u{E8F4}";
pub const I_SORT: &str = "\u{E8CB}";
pub const I_LIST: &str = "\u{E8FD}";
pub const I_GRID: &str = "\u{F0E2}";
pub const I_EYE: &str = "\u{E7B3}";
pub const I_PANE: &str = "\u{E8A0}";
pub const I_CLOUD: &str = "\u{E753}";
pub const I_DRIVE: &str = "\u{EDA2}";
pub const I_NETDRIVE: &str = "\u{E8CE}";
pub const I_HOME: &str = "\u{E80F}";
pub const I_PIN: &str = "\u{E718}";
pub const I_MORE: &str = "\u{E712}";
pub const I_CHEV_R: &str = "\u{E76C}";
pub const I_CHEV_D: &str = "\u{E70D}";
pub const I_PC: &str = "\u{E7F8}";
pub const I_TERMINAL: &str = "\u{E756}";
pub const I_INFO: &str = "\u{E946}";
pub const I_FILTER: &str = "\u{E71C}";
pub const I_SETTINGS: &str = "\u{E713}";
pub const I_CHECK: &str = "\u{E73E}";
pub const I_GRIP: &str = "\u{E76F}";
pub const I_CHEV_U: &str = "\u{E70E}";
pub const I_HIDE: &str = "\u{ED1A}";
pub const I_MIN: &str = "\u{E921}";
pub const I_MAX: &str = "\u{E922}";
pub const I_RESTORE: &str = "\u{E923}";
pub const I_WCLOSE: &str = "\u{E8BB}";
pub const I_PALETTE: &str = "\u{E790}";
pub const I_PICTURE: &str = "\u{E8B9}";
pub const I_NAVPANE: &str = "\u{E700}";
pub const I_FOLDER: &str = "\u{E8B7}";

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    // the system's own UI fonts: Segoe (Windows), SF/Helvetica (macOS), Noto/Cantarell/Ubuntu/DejaVu (Linux)
    let fdir = std::path::PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into())).join("Fonts");
    let load = |names: &[&str]| -> Option<Vec<u8>> {
        names.iter().find_map(|n| {
            let p = std::path::Path::new(n);
            std::fs::read(if p.is_absolute() { p.to_path_buf() } else { fdir.join(n) }).ok()
        })
    };
    let ui_fonts: &[&str] = if cfg!(windows) {
        &["segoeui.ttf"]
    } else if cfg!(target_os = "macos") {
        &["/System/Library/Fonts/SFNS.ttf", "/System/Library/Fonts/Helvetica.ttc", "/Library/Fonts/Arial Unicode.ttf"]
    } else {
        &[
            "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/google-noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/truetype/ubuntu/Ubuntu-R.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/usr/share/fonts/dejavu/DejaVuSans.ttf",
        ]
    };
    let mono_fonts: &[&str] = if cfg!(windows) {
        &["consola.ttf"]
    } else if cfg!(target_os = "macos") {
        &["/System/Library/Fonts/SFNSMono.ttf", "/System/Library/Fonts/Menlo.ttc"]
    } else {
        &["/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf", "/usr/share/fonts/TTF/DejaVuSansMono.ttf", "/usr/share/fonts/truetype/noto/NotoSansMono-Regular.ttf"]
    };
    let symbol_fonts: &[&str] = if cfg!(windows) {
        &["seguisym.ttf"]
    } else if cfg!(target_os = "macos") {
        &["/System/Library/Fonts/Apple Symbols.ttf"]
    } else {
        &["/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", "/usr/share/fonts/TTF/DejaVuSans.ttf", "/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf"]
    };

    if let Some(b) = load(ui_fonts) {
        fonts.font_data.insert("segoe".into(), Arc::new(FontData::from_owned(b)));
        fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "segoe".into());
    }
    if let Some(b) = load(mono_fonts) {
        fonts.font_data.insert("consolas".into(), Arc::new(FontData::from_owned(b)));
        fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "consolas".into());
    }
    if let Some(b) = load(symbol_fonts) {
        fonts.font_data.insert("segsym".into(), Arc::new(FontData::from_owned(b)));
        fonts.families.entry(FontFamily::Proportional).or_default().push("segsym".into());
    }
    let segoe = if prefers_bundled_icons() { None } else { load(&["SegoeIcons.ttf", "segmdl2.ttf"]) };
    let bundled = segoe.is_none();
    BUNDLED_ICONS.store(bundled, Ordering::Relaxed);
    let icons = segoe.map(FontData::from_owned).unwrap_or_else(|| FontData::from_static(egui_phosphor::Variant::Regular.font_bytes()));
    fonts.font_data.insert("icons".into(), Arc::new(icons));
    fonts.font_data.insert("file-icons".into(), Arc::new(FontData::from_static(egui_phosphor::Variant::Fill.font_bytes())));
    let mut fam = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    // The bundled fonts have blank lowercase letters that would hide the text next to an icon, so they
    // go after the text fonts, but before the emoji fonts that use the same private character codes.
    let at = if bundled { fam.iter().position(|f| f == "NotoEmoji-Regular").unwrap_or(fam.len()) } else { 0 };
    fam.insert(at, "icons".to_string());
    fonts.families.insert(icon_family(), fam.clone());
    fam[at] = "file-icons".to_string();
    fonts.families.insert(FontFamily::Name("file-icons".into()), fam);
    ctx.set_fonts(fonts);

    let mut style = (*ctx.global_style()).clone();
    style.text_styles = [
        (TextStyle::Small, FontId::proportional(11.5)),
        (TextStyle::Body, FontId::proportional(13.5)),
        (TextStyle::Button, FontId::proportional(13.5)),
        (TextStyle::Heading, FontId::proportional(18.0)),
        (TextStyle::Monospace, FontId::monospace(12.5)),
    ]
    .into();
    style.spacing.item_spacing = egui::vec2(6.0, 4.0);
    style.spacing.button_padding = egui::vec2(8.0, 4.0);
    style.spacing.menu_margin = egui::Margin::same(6);
    style.spacing.interact_size.y = 26.0;
    style.spacing.scroll.bar_width = 8.0;
    style.spacing.scroll.floating = true;
    ctx.set_global_style(style);
    apply(ctx, pal());
}

// ---------------------------------------------------------------- logo

/// The logo's white shape. `assets/logos/build.ps1` makes it from the SVG in use.
fn logo_mark() -> &'static (usize, Vec<u8>) {
    static MARK: std::sync::OnceLock<(usize, Vec<u8>)> = std::sync::OnceLock::new();
    MARK.get_or_init(|| {
        let decode = || -> Option<(usize, Vec<u8>)> {
            let mut dec = png::Decoder::new(std::io::Cursor::new(include_bytes!("../assets/logo-mark.png").as_slice()));
            dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
            let mut reader = dec.read_info().ok()?;
            let mut buf = vec![0; reader.output_buffer_size()?];
            let info = reader.next_frame(&mut buf).ok()?;
            let channels = info.color_type.samples();
            Some((info.width as usize, buf[..info.buffer_size()].chunks_exact(channels).map(|px| px[channels - 1]).collect()))
        };
        decode().unwrap_or((1, vec![0]))
    })
}

/// Paint the app logo (rounded accent square with the white mark) into `r`.
pub fn paint_logo(p: &egui::Painter, r: egui::Rect, accent: Color32) {
    p.rect_filled(r, egui::CornerRadius::same((r.width() * 0.23) as u8), accent);
    let id = egui::Id::unique("logo-mark");
    let ctx = p.ctx();
    let tex = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)).unwrap_or_else(|| {
        let (side, alpha) = logo_mark();
        let rgba: Vec<u8> = alpha.iter().flat_map(|&a| [255, 255, 255, a]).collect();
        let options = egui::TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..egui::TextureOptions::LINEAR };
        let tex = ctx.load_texture("logo-mark", egui::ColorImage::from_rgba_unmultiplied([*side, *side], &rgba), options);
        ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
        tex
    });
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    p.image(tex.id(), r, uv, Color32::WHITE);
}

/// The logo as RGBA pixels, for the window/taskbar icon.
pub fn logo_rgba(size: u32, accent: Color32) -> Vec<u8> {
    let s = size as f32;
    let rad = s * 0.23;
    let (side, alpha) = logo_mark();
    let n = size as usize;
    // how much of the mark covers one output pixel: the average of the source pixels under it
    let cover = |x: usize, y: usize| {
        let (x0, x1) = (x * side / n, ((x + 1) * side / n).max(x * side / n + 1).min(*side));
        let (y0, y1) = (y * side / n, ((y + 1) * side / n).max(y * side / n + 1).min(*side));
        let mut sum = 0u32;
        for sy in y0..y1 {
            sum += alpha[sy * side + x0..sy * side + x1].iter().map(|&a| a as u32).sum::<u32>();
        }
        sum as f32 / (((x1 - x0) * (y1 - y0)) as f32 * 255.0)
    };
    let mut out = Vec::with_capacity(n * n * 4);
    for y in 0..n {
        for x in 0..n {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let (cx, cy) = (fx.clamp(rad, s - rad), fy.clamp(rad, s - rad));
            let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
            let a = (rad - d + 0.5).clamp(0.0, 1.0);
            let c = mix(accent, Color32::WHITE, cover(x, y));
            out.extend_from_slice(&[c.r(), c.g(), c.b(), (a * 255.0) as u8]);
        }
    }
    out
}
