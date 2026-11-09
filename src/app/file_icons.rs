//! The Daleris icon pack, all drawn here. Files look like what they are, so their outlines differ at a glance: a box,
//! a crate, a film strip, a bare symbol or a tile. Folders are folders with a picture on them. To add a file type,
//! add its extension to a list in `by_extension`. The Phosphor pack is separate, in `phosphor_icons.rs`.

use eframe::egui::{self, vec2, Align2, Color32, FontId, Rect};

use crate::fsx::DriveKind;
use crate::theme::file_icon_font;
use crate::util::Kind;

/// What's being drawn. `name` is the lowercase file name.
pub(super) struct IconFor<'a> {
    pub kind: Kind,
    pub ext: &'a str,
    pub name: &'a str,
    pub place: Option<&'a str>,
    pub drive: Option<DriveKind>,
    pub is_root: bool,
    pub is_system: bool,
    pub cloud: Option<&'a str>,
    pub dim: bool,
}

/// How to draw an icon.
pub(super) enum Art {
    /// A picture, made only when it isn't in the texture cache yet, with text on it when there's room.
    Svg { id: String, svg: SvgSource, label: Option<String> },
    /// A glyph from the Phosphor font.
    Glyph(&'static str, Color32),
    /// Phosphor's page in a colour, with the extension on it when there's room.
    Page { color: Color32, label: Option<String> },
}

pub(super) enum SvgSource {
    Page(Family),
    Folder(Family),
    Fixed(String),
}

pub(super) fn svg_bytes(source: &SvgSource) -> Vec<u8> {
    match source {
        SvgSource::Page(family) => family.file_svg().into_bytes(),
        SvgSource::Folder(family) => family.folder_svg().into_bytes(),
        SvgSource::Fixed(svg) => svg.clone().into_bytes(),
    }
}

// ---------------------------------------------------------------- pictures

/// What's on a page or folder, in the area from (9,12) to (23,26) of a 32 unit square.
#[derive(Clone, Copy, Debug)]
pub(super) enum Picture {
    /// Nothing, for a page that only carries text.
    Blank,
    /// White lines.
    Line(&'static str),
    /// Any shapes. `{C}` becomes the page's colour and `{S}` its darker shade.
    Art(&'static str),
}

const ZIPPER: Picture = Picture::Art("<g fill='#fff'><rect x='14' y='10' width='3' height='2'/><rect x='17' y='12' width='3' height='2'/><rect x='14' y='14' width='3' height='2'/><rect x='17' y='16' width='3' height='2'/><rect x='14' y='18' width='3' height='2'/><rect x='14.500' y='20.500' width='5' height='6' rx='1.200'/></g><rect x='16' y='23' width='2' height='2' fill='{S}'/>"); // i18n-ignore: SVG drawing
const NOTE: Picture = Picture::Art("<path fill='#fff' d='M21 11.500V22a2.800 2.800 0 1 1-1.800-2.600v-5l-6 1.500v8a2.800 2.800 0 1 1-1.800-2.600v-8z'/>"); // i18n-ignore: SVG drawing
const PLAY: Picture = Picture::Art("<circle cx='16' cy='19' r='7' fill='#fff'/><path fill='{C}' d='M14 15.500l5.500 3.500-5.500 3.500z'/>"); // i18n-ignore: SVG drawing
const BOX: Picture = Picture::Art("<path fill='#fff' d='M16 12l7 3.500V23l-7 3.500L9 23v-7.500z'/><path fill='none' stroke='{S}' stroke-width='1.300' stroke-linejoin='round' d='M9 15.500l7 3.500 7-3.500M16 19v7.500'/>"); // i18n-ignore: SVG drawing
const CUBE: Picture = Picture::Line("M16 12l7 3.500V23l-7 3.500L9 23v-7.500zM9 15.500l7 3.500 7-3.500M16 19v7.500");
const DISC: Picture = Picture::Art("<circle cx='16' cy='19' r='7.200' fill='#fff'/><circle cx='16' cy='19' r='2' fill='{C}'/>"); // i18n-ignore: SVG drawing
const LINES: Picture = Picture::Line("M10.500 14.500h11M10.500 19h11M10.500 23.500h7");
const CODE: Picture = Picture::Line("M13 13.500L9.500 17l3.500 3.500M19 13.500l3.500 3.500-3.500 3.500");
const BRACES: Picture = Picture::Line("M14 12.500c-2 0-2.500 1-2.500 2.500v1.500c0 1.200-1 2.500-2 2.500 1 0 2 1.300 2 2.500V23c0 1.500.5 2.500 2.500 2.500M18 12.500c2 0 2.500 1 2.500 2.500v1.500c0 1.200 1 2.500 2 2.500-1 0-2 1.300-2 2.500V23c0 1.500-.5 2.500-2.500 2.500");
const ANGLES: Picture = Picture::Line("M13 14.500L9 19l4 4.500M19 14.500l4 4.500-4 4.500M17.500 13l-3 12");
const GEAR: Picture = Picture::Art("<circle cx='16' cy='19' r='5.800' fill='none' stroke='#fff' stroke-width='2.800' stroke-dasharray='2.300 2.250'/><circle cx='16' cy='19' r='4.300' fill='#fff'/><circle cx='16' cy='19' r='1.700' fill='{C}'/>"); // i18n-ignore: SVG drawing
const KEY: Picture = Picture::Line("M15 17.500a3.400 3.400 0 1 1-4.800-4.800 3.400 3.400 0 0 1 4.800 4.800zM15 17.500l7.500 7.500M19.500 22l2.500-2.500");
const LOCK: Picture = Picture::Art("<rect x='10' y='18' width='12' height='8.500' rx='1.800' fill='#fff'/><path fill='none' stroke='#fff' stroke-width='2.200' d='M12.500 18v-2.500a3.500 3.500 0 0 1 7 0V18'/><circle cx='16' cy='22' r='1.500' fill='{C}'/>"); // i18n-ignore: SVG drawing
const SEAL: Picture = Picture::Art("<circle cx='16' cy='16.500' r='4.700' fill='#fff'/><circle cx='16' cy='16.500' r='1.800' fill='{C}'/><path fill='#fff' d='M12.800 20l-1.500 6.500 4.700-2.500 4.700 2.500-1.500-6.500z'/>"); // i18n-ignore: SVG drawing
const LETTERS: Picture = Picture::Line("M9 25.500l4-11.500 4 11.500M10.500 21.500h5M23 19.500v6M23 22.500a2.700 2.700 0 1 1-5.400 0 2.700 2.700 0 0 1 5.400 0z");
const PHOTO: Picture = Picture::Art("<rect x='9' y='12.500' width='14' height='13' rx='1.500' fill='#fff'/><circle cx='13' cy='16.500' r='1.600' fill='{C}'/><path fill='{C}' d='M10.500 24l4-5 2.500 3 2-2 2.500 4z'/>"); // i18n-ignore: SVG drawing
const VECTOR: Picture = Picture::Line("M10 24c1-7 5-11 12-12M9 22.500h3v3H9zM20.500 10.500h3v3h-3z");
const DATABASE: Picture = Picture::Art("<ellipse cx='16' cy='14.500' rx='6.500' ry='2.500' fill='#fff'/><path fill='#fff' d='M9.500 16.200c1 1.300 3.500 2.100 6.500 2.100s5.500-.8 6.500-2.100V19c0 1.400-2.900 2.500-6.500 2.500S9.500 20.400 9.500 19zM9.500 21c1 1.300 3.500 2.100 6.500 2.100s5.500-.8 6.500-2.100v2.500c0 1.400-2.900 2.500-6.500 2.500s-6.500-1.100-6.500-2.500z'/>"); // i18n-ignore: SVG drawing
const TERMINAL: Picture = Picture::Line("M10 14.500l4.500 4.500-4.500 4.500M16.500 24.500H22");
const LINK: Picture = Picture::Line("M13.800 21.200l4.400-4.400M15 15l1.700-1.700a3.400 3.400 0 0 1 4.800 4.800L19.800 19.800M17 23l-1.700 1.700a3.400 3.400 0 0 1-4.800-4.800L12.200 18.200");
const BOOK: Picture = Picture::Line("M16 15c-1.800-1.500-4-2-7-2v11c3 0 5.200.5 7 2 1.800-1.500 4-2 7-2V13c-3 0-5.200.5-7 2zM16 15v11");
const MARKDOWN: Picture = Picture::Line("M9 24.500v-11l3.500 5 3.500-5v11M21 13.500v10M18.500 20.500l2.500 3.500 2.500-3.500");
const PDF: Picture = Picture::Line("M10 25c2-1 5-7 5.500-10.500.2-1.700-1.500-2-1.700 0-.3 3.500 4 8 8.700 7.500 1.500-.2 1.200-1.800-.5-1.500-4 .5-8.500 2-12 4.500z");
const BRANCH: Picture = Picture::Art("<g fill='none' stroke='#fff' stroke-width='2.200'><path d='M12 15.500v7M21 15.500c0 3.500-3 4-9 4'/></g><circle cx='12' cy='13.500' r='2.300' fill='#fff'/><circle cx='12' cy='24.500' r='2.300' fill='#fff'/><circle cx='21' cy='13.500' r='2.300' fill='#fff'/>"); // i18n-ignore: SVG drawing
const HAMMER: Picture = Picture::Art("<path fill='#fff' d='M14.500 12.500h5l3.500 3.500-2 2-2-1.500-1.500 1.500-3-3 .5-.5z'/><path fill='none' stroke='#fff' stroke-width='2.600' stroke-linecap='round' d='M15.500 17.500l-5.500 7'/>"); // i18n-ignore: SVG drawing
const BINARY: Picture = Picture::Line("M12.500 14c2 0 3 2.200 3 5s-1 5-3 5-3-2.200-3-5 1-5 3-5zM19 16l2.500-2v10");
const CHIP: Picture = Picture::Art("<rect x='11' y='14' width='10' height='10' rx='1.500' fill='#fff'/><rect x='14' y='17' width='4' height='4' fill='{C}'/><path fill='none' stroke='#fff' stroke-width='1.500' d='M13.500 11.500V14M18.500 11.500V14M13.500 24v2.500M18.500 24v2.500M8.500 16.500H11M8.500 21.500H11M21 16.500h2.500M21 21.500h2.500'/>"); // i18n-ignore: SVG drawing
const BUG: Picture = Picture::Art("<ellipse cx='16' cy='20' rx='4.500' ry='5.500' fill='#fff'/><path fill='none' stroke='#fff' stroke-width='1.800' stroke-linecap='round' d='M13.500 13.500l-1.500-1.500M18.500 13.500l1.500-1.500M11.500 18H9M20.500 18H23M11.500 22H9.500M20.500 22h2'/><path fill='none' stroke='{C}' stroke-width='1.200' d='M16 15.500v10'/>"); // i18n-ignore: SVG drawing
const TEMP: Picture = Picture::Art("<rect x='10' y='13.500' width='12' height='12' rx='2' fill='none' stroke='#fff' stroke-width='2' stroke-dasharray='2.500 2'/>"); // i18n-ignore: SVG drawing
const DOWNLOAD: Picture = Picture::Line("M16 12.500V22M11.500 18l4.500 4.500 4.500-4.500M10 26h12");
const PIN: Picture = Picture::Art("<path fill='#fff' d='M16 26.500c-4-5-6.200-7.700-6.200-10.200a6.200 6.200 0 1 1 12.400 0c0 2.500-2.200 5.200-6.200 10.200z'/><circle cx='16' cy='16.300' r='2.200' fill='{C}'/>"); // i18n-ignore: SVG drawing
const PAD: Picture = Picture::Art("<rect x='8.500' y='15' width='15' height='9.500' rx='4.500' fill='#fff'/><path fill='none' stroke='{C}' stroke-width='1.400' stroke-linecap='round' d='M11 19.700h3.500M12.700 18v3.500'/><circle cx='19' cy='18.500' r='1' fill='{C}'/><circle cx='21' cy='21' r='1' fill='{C}'/>"); // i18n-ignore: SVG drawing
const ROBOT: Picture = Picture::Art("<rect x='10' y='15.500' width='12' height='10.500' rx='2.500' fill='#fff'/><circle cx='13.500' cy='20' r='1.500' fill='{C}'/><circle cx='18.500' cy='20' r='1.500' fill='{C}'/><path fill='none' stroke='#fff' stroke-width='1.800' stroke-linecap='round' d='M16 15.500V12.500'/>"); // i18n-ignore: SVG drawing
const MAIL: Picture = Picture::Line("M9.500 15h13v9h-13zM9.500 15.500L16 20l6.500-4.500");
const CALENDAR: Picture = Picture::Art("<rect x='10' y='13.500' width='12' height='12.500' rx='1.800' fill='#fff'/><rect x='10' y='13.500' width='12' height='3.500' fill='{S}'/><g fill='{C}'><rect x='12' y='19' width='2.200' height='2.200'/><rect x='15' y='19' width='2.200' height='2.200'/><rect x='18' y='19' width='2.200' height='2.200'/><rect x='12' y='22.300' width='2.200' height='2.200'/></g>"); // i18n-ignore: SVG drawing
const PERSON: Picture = Picture::Art("<circle cx='16' cy='15.500' r='3.300' fill='#fff'/><path fill='#fff' d='M9.500 26c.5-4 3-6 6.500-6s6 2 6.500 6z'/>"); // i18n-ignore: SVG drawing
const CAPTIONS: Picture = Picture::Art("<rect x='9' y='14' width='14' height='11' rx='2' fill='#fff'/><path fill='none' stroke='{C}' stroke-width='1.600' stroke-linecap='round' d='M11.500 18h4M17.500 18h3M11.500 21.500h2M15.500 21.500h5'/>"); // i18n-ignore: SVG drawing
const PALETTE: Picture = Picture::Art("<path fill='#fff' d='M16 12.500c-4.500 0-7.500 2.900-7.500 6.500s3 6.500 7.500 6.500c1.700 0 2-1.200 1.200-2.200-.8-1 0-2.300 1.300-2.300h2.300c1.700 0 2.700-1 2.700-2.500 0-3.200-3.300-6-7.500-6z'/><circle cx='12' cy='18' r='1.300' fill='{C}'/><circle cx='15' cy='15.300' r='1.300' fill='{C}'/><circle cx='19' cy='16' r='1.300' fill='{C}'/>"); // i18n-ignore: SVG drawing
const GRID: Picture = Picture::Line("M9.500 14h13v11h-13zM9.500 19.500h13M16 14v11");
const RULER: Picture = Picture::Line("M10 25.500V13l12.500 12.500zM13 22.500h2.500V20");
const DOCKER: Picture = Picture::Art("<g fill='#fff'><rect x='9.500' y='18' width='3' height='3'/><rect x='13' y='18' width='3' height='3'/><rect x='16.500' y='18' width='3' height='3'/><rect x='13' y='14.500' width='3' height='3'/><rect x='16.500' y='14.500' width='3' height='3'/><path d='M8.500 21.500h15c-.7 2.800-3 4.500-7 4.500-5 0-7.500-1.800-8-4.500z'/></g>"); // i18n-ignore: SVG drawing
const FLASK: Picture = Picture::Art("<path fill='#fff' d='M14 12.500h4v5.500l4.500 7c.5.800 0 1.500-.8 1.500H10.300c-.8 0-1.300-.7-.8-1.500l4.500-7z'/><path fill='{C}' d='M12.800 22.500h6.400l1 1.700h-8.400z'/>"); // i18n-ignore: SVG drawing

// Places on folders.
const HOUSE: Picture = Picture::Art("<path fill='#fff' d='M16 12.500l7.500 6.500h-2v6.500h-3.800v-4.500h-3.400v4.500h-3.800V19h-2z'/>"); // i18n-ignore: SVG drawing
const SCREEN: Picture = Picture::Art("<rect x='9' y='13' width='14' height='9.500' rx='1.500' fill='#fff'/><path fill='none' stroke='#fff' stroke-width='2' stroke-linecap='round' d='M13 25.500h6M16 22.500v3'/>"); // i18n-ignore: SVG drawing
const STAR: Picture = Picture::Art("<path fill='#fff' d='M16 12.500l2.100 4.400 4.900.6-3.600 3.400.9 4.800-4.300-2.400-4.300 2.400.9-4.800L9 17.500l4.900-.6z'/>"); // i18n-ignore: SVG drawing

/// The outline of a file's icon.
#[derive(Clone, Copy, Debug)]
enum Frame {
    /// A page, only for documents of plain text and for unknown types.
    Page,
    /// The picture alone, large and in the colour.
    Bare,
    /// A rounded square with the picture on it in white.
    Tile,
    /// A crate with a lid.
    Crate,
    /// A film strip.
    Film,
    /// A picture frame.
    Photo,
    /// A program window.
    App,
}

/// A kind of file or folder with its own colours, as SVG colours.
#[derive(Clone, Copy, Debug)]
pub(super) struct Family {
    frame: Frame,
    picture: Picture,
    color: &'static str,
    shade: &'static str,
    labelled: bool,
}

impl Family {
    fn picture_svg(&self) -> String {
        match self.picture {
            Picture::Blank => String::new(),
            Picture::Line(path) => format!("<path fill='none' stroke='#fff' stroke-width='2.200' stroke-linecap='round' stroke-linejoin='round' d='{path}'/>"),
            Picture::Art(shapes) => shapes.replace("{C}", self.color).replace("{S}", self.shade),
        }
    }

    /// The picture in white, moved to the middle and scaled up from its usual place.
    fn scaled(&self, scale: f32) -> String {
        format!("<g transform='translate(16 16) scale({scale}) translate(-16 -19)'>{}</g>", self.picture_svg()) // i18n-ignore: SVG drawing
    }

    /// The picture alone in the colour: what's white becomes the colour, and its details become white.
    fn bare(&self) -> String {
        let shapes = match self.picture {
            Picture::Blank => String::new(),
            Picture::Line(path) => format!("<path fill='none' stroke='{}' stroke-width='2.400' stroke-linecap='round' stroke-linejoin='round' d='{path}'/>", self.color),
            Picture::Art(shapes) => shapes.replace("#fff", self.color).replace("{C}", "#fff").replace("{S}", "#fff"),
        };
        format!("<g transform='translate(16 16) scale(1.95) translate(-16 -19)'>{shapes}</g>") // i18n-ignore: SVG drawing
    }

    fn file_svg(&self) -> String {
        let (c, s) = (self.color, self.shade);
        let inner = match self.frame {
            Frame::Page => format!("<path fill='{c}' d='M8 1.500h11.500L27 9v19.500a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2v-25a2 2 0 0 1 2-2z'/><path fill='{s}' d='M19.500 1.500L27 9h-5.500a2 2 0 0 1-2-2z'/>{}", self.picture_svg()), // i18n-ignore: SVG drawing
            Frame::Bare => self.bare(),
            Frame::Tile => format!("<rect x='2.500' y='2.500' width='27' height='27' rx='6' fill='{c}'/>{}", self.scaled(1.5)), // i18n-ignore: SVG drawing
            Frame::Crate => format!("<rect x='4.500' y='10' width='23' height='19' rx='2.500' fill='{c}'/><rect x='2.500' y='4' width='27' height='8' rx='2' fill='{s}'/><g fill='#fff'><rect x='14' y='4' width='4' height='2'/><rect x='14' y='8' width='4' height='2'/><rect x='14' y='12' width='4' height='2.500'/><rect x='14' y='17' width='4' height='2.500'/><rect x='12.500' y='21' width='7' height='6' rx='1.500'/></g><rect x='14.700' y='23' width='2.600' height='2.200' fill='{s}'/>"), // i18n-ignore: SVG drawing
            Frame::Film => format!("<rect x='2' y='5' width='28' height='22' rx='3' fill='{c}'/><g fill='{s}'><rect x='4' y='7' width='3' height='3' rx='.7'/><rect x='4' y='14.500' width='3' height='3' rx='.7'/><rect x='4' y='22' width='3' height='3' rx='.7'/><rect x='25' y='7' width='3' height='3' rx='.7'/><rect x='25' y='14.500' width='3' height='3' rx='.7'/><rect x='25' y='22' width='3' height='3' rx='.7'/></g><path fill='#fff' d='M12.500 10.500L22 16l-9.500 5.500z'/>"), // i18n-ignore: SVG drawing
            Frame::Photo => format!("<rect x='2' y='5' width='28' height='22' rx='3' fill='{c}'/><circle cx='10' cy='12' r='2.700' fill='#fff'/><path fill='#fff' d='M4.500 24.500l7-8 5 5.500 3.500-3.500 7.500 6z'/>"), // i18n-ignore: SVG drawing
            Frame::App => format!("<rect x='2' y='5' width='28' height='22' rx='3.500' fill='{c}'/><path fill='{s}' d='M2 8.500A3.500 3.500 0 0 1 5.500 5h21A3.500 3.500 0 0 1 30 8.500V11H2z'/><g fill='#fff'><circle cx='6' cy='8' r='1.200'/><circle cx='10' cy='8' r='1.200'/><rect x='6' y='15' width='14' height='2.200' rx='1'/><rect x='6' y='20' width='9' height='2.200' rx='1'/></g>"), // i18n-ignore: SVG drawing
        };
        format!("<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'>{inner}</svg>")
    }

    fn folder_svg(&self) -> String {
        format!(
            "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'><path fill='{}' d='M3 7a2 2 0 0 1 2-2h7.500l3 3H27a2 2 0 0 1 2 2v4H3z'/><rect x='3' y='10' width='26' height='18' rx='2.500' fill='{}'/>{}</svg>",
            self.shade,
            self.color,
            self.picture_svg()
        )
    }
}

// The colours, each with its darker shade.
const BLUE: (&str, &str) = ("#2f86d6", "#1c62a6");
const NAVY: (&str, &str) = ("#3c6fd8", "#2652a8");
const SKY: (&str, &str) = ("#3aa0e0", "#2279b3");
const INDIGO: (&str, &str) = ("#5a62d6", "#3f46ad");
const PURPLE: (&str, &str) = ("#8e5bd0", "#6a3fa6");
const VIOLET: (&str, &str) = ("#7a5be0", "#5a3eb5");
const PINK: (&str, &str) = ("#e0569a", "#b13a76");
const RED: (&str, &str) = ("#e0483f", "#b0302a");
const CRIMSON: (&str, &str) = ("#b53a4d", "#8c2636");
const ORANGE: (&str, &str) = ("#e0762f", "#b4551a");
const AMBER: (&str, &str) = ("#d9a520", "#a87d10");
const BROWN: (&str, &str) = ("#a9744a", "#7f5332");
const GREEN: (&str, &str) = ("#2e9d58", "#1f7640");
const LIME: (&str, &str) = ("#6fae3a", "#527f2a");
const TEAL: (&str, &str) = ("#26a69a", "#1a7d74");
const SLATE: (&str, &str) = ("#5f7a96", "#46596e");
const GRAY: (&str, &str) = ("#8a8f9c", "#646975");

const fn make(frame: Frame, picture: Picture, c: (&'static str, &'static str), labelled: bool) -> Family {
    Family { frame, picture, color: c.0, shade: c.1, labelled }
}

const fn with(picture: Picture, c: (&'static str, &'static str)) -> Family {
    make(Frame::Page, picture, c, false)
}

const fn bare(picture: Picture, c: (&'static str, &'static str)) -> Family {
    make(Frame::Bare, picture, c, false)
}

const fn tile(picture: Picture, c: (&'static str, &'static str)) -> Family {
    make(Frame::Tile, picture, c, false)
}

const fn crate_of(c: (&'static str, &'static str)) -> Family {
    make(Frame::Crate, Picture::Blank, c, false)
}

const fn film(c: (&'static str, &'static str)) -> Family {
    make(Frame::Film, Picture::Blank, c, false)
}

/// A tile of code that also shows its extension, as languages can share a colour.
const fn lang(c: (&'static str, &'static str)) -> Family {
    make(Frame::Tile, CODE, c, true)
}

// ---------------------------------------------------------------- what gets what

/// Files recognised by their whole name, before their extension counts.
fn by_file_name(name: &str) -> Option<Family> {
    let stem = name.split('.').next().unwrap_or(name);
    Some(match name {
        "dockerfile" | "containerfile" | ".dockerignore" | "docker-compose.yml" | "docker-compose.yaml" | "compose.yml" | "compose.yaml" => tile(DOCKER, SKY),
        "makefile" | "gnumakefile" | "justfile" | "cmakelists.txt" | "meson.build" | "rakefile" | "build.rs" => bare(HAMMER, ORANGE),
        ".gitignore" | ".gitattributes" | ".gitmodules" | ".gitconfig" | ".mailmap" => bare(BRANCH, ORANGE),
        "package.json" | "cargo.toml" | "pyproject.toml" | "requirements.txt" | "go.mod" | "gemfile" | "pom.xml" | "build.gradle" | "build.gradle.kts" | "composer.json" => bare(BOX, GREEN),
        "cargo.lock" | "package-lock.json" | "yarn.lock" | "pnpm-lock.yaml" | "gemfile.lock" | "poetry.lock" | "go.sum" | "composer.lock" | "flake.lock" => bare(LOCK, GRAY),
        ".env" | ".envrc" | ".npmrc" | ".netrc" | "id_rsa" | "id_ed25519" | "authorized_keys" | "known_hosts" => bare(KEY, AMBER),
        ".editorconfig" | ".prettierrc" | ".eslintrc" | ".clang-format" | ".rustfmt.toml" | "rustfmt.toml" | "tsconfig.json" => bare(GEAR, GRAY),
        ".bashrc" | ".bash_profile" | ".zshrc" | ".profile" | ".bash_history" | ".zsh_history" => tile(TERMINAL, GREEN),
        _ if name.starts_with(".env.") => bare(KEY, AMBER),
        _ if name.ends_with(".pkg.tar.zst") || name.ends_with(".pkg.tar.xz") => bare(BOX, ORANGE),
        _ => match stem {
            "readme" => bare(BOOK, BLUE),
            "license" | "licence" | "copying" | "notice" => bare(SEAL, AMBER),
            "changelog" | "changes" | "history" => with(LINES, BLUE),
            _ => return None,
        },
    })
}

fn by_extension(ext: &str) -> Option<Family> {
    Some(match ext {
        // programs and installers, by system
        "exe" | "com" | "scr" | "app" => make(Frame::App, Picture::Blank, NAVY, false),
        "msi" | "msix" | "msixbundle" | "appx" | "appxbundle" | "appinstaller" | "msp" | "msu" | "mst" | "application" => bare(BOX, BLUE),
        "deb" | "rpm" | "snap" | "flatpak" | "flatpakref" | "appimage" | "pacman" | "apkg" => bare(BOX, ORANGE),
        "dmg" | "pkg" | "mpkg" | "ipa" => bare(BOX, GRAY),
        "apk" | "aab" | "xapk" | "apks" | "apkm" => bare(BOX, GREEN),
        "nupkg" | "snupkg" | "vsix" | "vsixpackage" | "crx" | "xpi" | "whl" | "gem" | "crate" | "jar" | "war" | "ear" | "unitypackage" => bare(BOX, TEAL),
        // archives
        "zip" | "zipx" => crate_of(AMBER),
        "7z" => crate_of(TEAL),
        "rar" | "r00" | "r01" => crate_of(PURPLE),
        "tar" | "cpio" | "shar" => crate_of(BROWN),
        "gz" | "gzip" | "tgz" | "z" => crate_of(ORANGE),
        "bz" | "bz2" | "tbz" | "tbz2" | "xz" | "txz" | "lzma" | "lz" | "lz4" | "lzo" | "zst" | "zstd" | "br" => crate_of(RED),
        "cab" | "arc" | "ace" | "arj" | "sit" | "sitx" | "wim" | "swm" | "esd" | "lha" | "lzh" | "ar" => crate_of(SLATE),
        // disk images
        "iso" | "img" | "nrg" | "mdf" | "mds" | "toast" | "ccd" => bare(DISC, INDIGO),
        "vhd" | "vhdx" | "vmdk" | "vdi" | "qcow" | "qcow2" | "ova" | "ovf" => bare(DISC, SLATE),
        // audio
        "mp3" | "aac" | "m4a" | "m4b" | "ogg" | "oga" | "opus" | "wma" | "amr" | "ra" | "mka" | "3ga" => bare(NOTE, PINK),
        "flac" | "wav" | "wave" | "aiff" | "aif" | "alac" | "ape" | "wv" | "dsf" | "dff" | "pcm" | "au" => bare(NOTE, PURPLE),
        "mid" | "midi" | "kar" | "mscz" | "musicxml" => bare(NOTE, ORANGE),
        "m3u" | "m3u8" | "pls" | "cue" | "wpl" | "xspf" | "asx" => with(LINES, PINK),
        // video and subtitles
        "mp4" | "m4v" | "mov" | "qt" => film(RED),
        "mkv" | "webm" | "ogv" => film(VIOLET),
        "avi" | "wmv" | "asf" | "divx" | "xvid" | "flv" | "f4v" | "rm" | "rmvb" => film(ORANGE),
        "mpg" | "mpeg" | "mpe" | "m2v" | "mts" | "m2ts" | "vob" | "3gp" | "3g2" => film(TEAL),
        "srt" | "vtt" | "ass" | "ssa" | "sub" => tile(CAPTIONS, VIOLET),
        // office, each app by its first letter
        "doc" | "docx" | "docm" | "dot" | "dotx" | "dotm" | "rtf" | "odt" | "ott" | "wps" | "pages" | "wpd" | "hwp" => tile(Picture::Line("M10 13.500l2.500 11L16 16l3.500 8.500L22 13.500"), NAVY),
        "xls" | "xlsx" | "xlsm" | "xlsb" | "xlt" | "xltx" | "xltm" | "ods" | "ots" | "numbers" | "et" => tile(Picture::Line("M11 13.500l10 11M21 13.500l-10 11"), GREEN),
        "csv" | "tsv" => tile(GRID, GREEN),
        "ppt" | "pptx" | "pptm" | "pps" | "ppsx" | "pot" | "potx" | "odp" | "otp" | "dps" => tile(Picture::Line("M12 25V13.500h5a3.800 3.800 0 0 1 0 7.600h-5"), ORANGE),
        "mdb" | "accdb" | "accde" | "accdr" | "odb" => tile(Picture::Line("M10.500 25L16 13.500 21.500 25M12.500 21h7"), CRIMSON),
        "one" | "onetoc2" | "onepkg" => tile(Picture::Line("M11 25V13.500l10 11.500V13.500"), PURPLE),
        "vsd" | "vsdx" | "vsdm" | "vssx" | "vstx" => tile(Picture::Line("M10.500 13.500L16 25l5.500-11.500"), INDIGO),
        "pst" | "ost" | "oft" | "msg" | "eml" | "emlx" | "mbox" => bare(MAIL, BLUE),
        "pdf" => with(PDF, RED),
        // text and books
        "txt" | "text" => with(LINES, GRAY),
        "log" | "out" => with(LINES, SLATE),
        "md" | "markdown" | "mdx" | "rst" | "adoc" | "org" => tile(MARKDOWN, BLUE),
        "tex" | "latex" | "bib" => with(LINES, TEAL),
        "epub" | "mobi" | "azw" | "azw3" | "fb2" | "djvu" | "cbz" | "cbr" => bare(BOOK, TEAL),
        "ipynb" => bare(BOOK, ORANGE),
        "ics" => tile(CALENDAR, RED),
        "vcf" => bare(PERSON, BLUE),
        // pictures and design
        "png" | "jpg" | "jpeg" | "jfif" | "jpe" | "gif" | "webp" | "bmp" | "ico" | "icns" | "cur" | "tif" | "tiff" | "heic" | "heif" | "avif" | "jxl" | "raw" | "cr2" | "cr3" | "nef" | "arw" | "dng" | "orf" | "rw2" | "tga" | "dds" | "exr" | "hdr" | "qoi" | "pbm" | "pgm" | "ppm" => make(Frame::Photo, Picture::Blank, GREEN, false),
        "svg" | "svgz" => tile(VECTOR, ORANGE),
        "psd" | "psb" | "ai" | "xd" | "fig" | "sketch" | "indd" | "afdesign" | "afphoto" | "afpub" | "kra" | "xcf" | "clip" | "cdr" | "procreate" | "ase" | "aco" | "gpl" => bare(PALETTE, VIOLET),
        // code, with its extension on it
        "rs" => lang(ORANGE),
        "py" | "pyw" | "pyi" => lang(SKY),
        "js" | "mjs" | "cjs" | "jsx" => lang(AMBER),
        "ts" | "cts" | "tsx" => lang(NAVY),
        "html" | "htm" | "xhtml" => lang(RED),
        "css" | "scss" | "sass" | "less" | "styl" => lang(PINK),
        "vue" | "svelte" | "astro" => lang(GREEN),
        "c" | "cpp" | "cc" | "cxx" | "c++" | "h" | "hh" | "hpp" | "hxx" | "m" | "mm" => lang(BLUE),
        "cs" | "vb" | "fs" => lang(PURPLE),
        "go" | "dart" => lang(TEAL),
        "java" | "kt" | "kts" | "scala" | "groovy" | "gradle" | "clj" => lang(BROWN),
        "swift" | "rb" | "ex" | "exs" | "erl" | "elm" | "cr" => lang(CRIMSON),
        "php" | "pl" | "pm" | "hs" | "ml" => lang(INDIGO),
        "lua" | "luau" | "r" | "jl" | "f90" | "nim" | "zig" | "asm" | "sol" => lang(SLATE),
        "glsl" | "hlsl" | "wgsl" | "shader" => lang(VIOLET),
        "sql" => bare(DATABASE, AMBER),
        "sh" | "bash" | "zsh" | "fish" | "ps1" | "psm1" | "psd1" | "bat" | "cmd" | "nu" => tile(TERMINAL, GREEN),
        "patch" | "diff" => bare(BRANCH, ORANGE),
        // data and settings
        "json" | "jsonc" | "json5" | "jsonl" | "ndjson" | "geojson" => bare(BRACES, AMBER),
        "xml" | "xsd" | "xsl" | "xslt" | "plist" | "wxs" | "resx" | "xaml" => bare(ANGLES, ORANGE),
        "yaml" | "yml" | "toml" | "ini" | "cfg" | "conf" | "config" | "env" | "properties" | "editorconfig" | "reg" | "inf" | "policy" | "admx" | "adml" => bare(GEAR, GRAY),
        "lock" => bare(LOCK, GRAY),
        "db" | "sqlite" | "sqlite3" | "db3" | "dbf" | "parquet" | "avro" | "orc" | "feather" => bare(DATABASE, AMBER),
        // libraries and binaries
        "dll" | "so" | "dylib" | "sys" | "lib" | "a" | "o" | "ocx" | "drv" | "ko" | "ino" | "kicad_pcb" | "gbr" => bare(CHIP, SLATE),
        "wasm" | "bin" | "dat" | "hex" | "elf" | "pyc" | "class" => tile(BINARY, GRAY),
        "torrent" | "crdownload" | "part" | "partial" | "download" => bare(DOWNLOAD, TEAL),
        // keys and certificates
        "key" | "pem" | "pub" | "ppk" | "kdbx" => bare(KEY, AMBER),
        "crt" | "cer" | "der" | "p12" | "pfx" | "csr" => bare(SEAL, AMBER),
        "gpg" | "pgp" | "asc" | "sig" | "age" | "enc" => bare(LOCK, AMBER),
        // the rest
        "ttf" | "otf" | "ttc" | "woff" | "woff2" | "eot" | "fnt" => tile(LETTERS, PINK),
        "obj" | "stl" | "fbx" | "blend" | "gltf" | "glb" | "dae" | "3ds" | "ply" | "usd" | "usdz" | "3mf" => bare(CUBE, PURPLE),
        "dwg" | "dxf" | "step" | "stp" | "iges" | "skp" => bare(RULER, TEAL),
        "url" | "webloc" | "desktop" | "lnk" | "website" => bare(LINK, BLUE),
        "gpx" | "kml" | "kmz" | "shp" => bare(PIN, GREEN),
        "nes" | "sfc" | "gba" | "gb" | "nds" | "sav" | "rom" | "pak" => bare(PAD, VIOLET),
        "gguf" | "safetensors" | "onnx" | "pth" | "ckpt" | "tflite" => bare(ROBOT, TEAL),
        "bak" | "old" | "orig" | "tmp" | "temp" | "swp" | "swo" | "cache" => with(TEMP, GRAY),
        "dmp" | "mdmp" | "core" | "crash" => bare(BUG, RED),
        _ => return None,
    })
}

const YELLOW: (&str, &str) = ("#f2c24e", "#d39f2c");

fn by_place(place: &str) -> Option<Family> {
    Some(match place {
        "Home" => with(HOUSE, BLUE), // i18n-ignore: place name compared as a key
        "Desktop" => with(SCREEN, SKY), // i18n-ignore: place name compared as a key
        "Downloads" => with(DOWNLOAD, TEAL), // i18n-ignore: place name compared as a key
        "Documents" => with(LINES, NAVY), // i18n-ignore: place name compared as a key
        "Pictures" => with(PHOTO, GREEN), // i18n-ignore: place name compared as a key
        "Music" => with(NOTE, PINK), // i18n-ignore: place name compared as a key
        "Videos" => with(PLAY, VIOLET), // i18n-ignore: place name compared as a key
        _ => return None,
    })
}

fn by_folder_name(name: &str) -> Family {
    match name {
        ".git" => with(BRANCH, ORANGE),
        ".github" | ".gitlab" => with(BRANCH, GRAY),
        "node_modules" | "vendor" | "packages" => with(BOX, GREEN),
        ".trash" | "trash" | "$recycle.bin" => with(TEMP, GRAY),
        "src" | "source" | "sources" | "lib" | "code" => with(CODE, TEAL),
        "doc" | "docs" | "documentation" | "wiki" => with(LINES, BLUE),
        "test" | "tests" | "spec" | "specs" | "__tests__" => with(FLASK, LIME),
        "build" | "dist" | "target" | "out" | "bin" | "obj" | "release" | "debug" => with(HAMMER, GRAY),
        "assets" | "images" | "img" | "icons" | "media" | "public" | "static" => with(PHOTO, PURPLE),
        "config" | "configs" | "settings" | ".config" | ".vscode" | ".idea" => with(GEAR, SLATE),
        "backup" | "backups" | "archive" | "archives" => with(ZIPPER, BROWN),
        "favorites" | "favourites" | "starred" => with(STAR, AMBER),
        _ => with(Picture::Blank, YELLOW),
    }
}

/// A colour of its own for an unknown type, the same every time.
fn colour_of(ext: &str) -> (&'static str, &'static str) {
    const COLOURS: [(&str, &str); 10] = [BLUE, INDIGO, PURPLE, PINK, RED, ORANGE, AMBER, GREEN, TEAL, SLATE];
    let hash = ext.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    COLOURS[hash as usize % COLOURS.len()]
}

// ---------------------------------------------------------------- drives and clouds

fn drive_body() -> &'static str {
    concat!(
        "<defs><linearGradient id='b' x1='0' y1='0' x2='0' y2='1'><stop offset='0' stop-color='#e8ebf0'/><stop offset='1' stop-color='#aab1bd'/></linearGradient></defs>", // i18n-ignore: SVG drawing
        "<path fill='url(#b)' d='M6.500 8h19a2.500 2.500 0 0 1 2.400 1.800L30 17v6a2.500 2.500 0 0 1-2.500 2.500h-23A2.500 2.500 0 0 1 2 23v-6l2.100-7.200A2.500 2.500 0 0 1 6.500 8z'/>", // i18n-ignore: SVG drawing
        "<rect x='2' y='17' width='28' height='8.500' rx='2.500' fill='#59606e'/><rect x='5' y='20' width='13' height='2.400' rx='1.200' fill='#7f8795'/><circle cx='26' cy='21.200' r='1.700' fill='#4fe07f'/>", // i18n-ignore: SVG drawing
    )
}

fn square_svg(inner: &str) -> String {
    format!("<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'>{inner}</svg>")
}

fn fixed(id: &str, inner: String) -> Art {
    Art::Svg { id: format!("own:{id}"), svg: SvgSource::Fixed(square_svg(&inner)), label: None }
}

fn by_drive(kind: DriveKind, is_system: bool) -> Art {
    match kind {
        DriveKind::Removable => fixed("usb", "<rect x='11' y='2' width='10' height='8' rx='1' fill='#c3c9d3'/><rect x='13' y='4' width='2' height='2' fill='#59606e'/><rect x='17' y='4' width='2' height='2' fill='#59606e'/><rect x='8' y='9' width='16' height='21' rx='3' fill='#e0762f'/><rect x='11' y='14' width='10' height='9' rx='1.500' fill='#fff' opacity='.9'/><circle cx='16' cy='26.500' r='1.300' fill='#ffe2c9'/>".into()), // i18n-ignore: SVG drawing
        DriveKind::Network => fixed("network", "<rect x='5' y='3' width='22' height='7' rx='2' fill='#59606e'/><rect x='5' y='12' width='22' height='7' rx='2' fill='#6e7584'/><circle cx='23' cy='6.500' r='1.300' fill='#4fe07f'/><circle cx='23' cy='15.500' r='1.300' fill='#4fe07f'/><path fill='none' stroke='#26a69a' stroke-width='2.200' stroke-linecap='round' d='M16 19v5M6 27h20M16 24v3'/><circle cx='16' cy='27' r='2.500' fill='#26a69a'/>".into()), // i18n-ignore: SVG drawing
        DriveKind::Optical => fixed("optical", "<defs><linearGradient id='c' x1='0' y1='0' x2='1' y2='1'><stop offset='0' stop-color='#8fd3ff'/><stop offset='.35' stop-color='#d7b7ff'/><stop offset='.65' stop-color='#ffd0a1'/><stop offset='1' stop-color='#9be8c5'/></linearGradient></defs><circle cx='16' cy='16' r='14' fill='url(#c)'/><circle cx='16' cy='16' r='5' fill='#fff'/><circle cx='16' cy='16' r='2' fill='#59606e'/>".into()), // i18n-ignore: SVG drawing
        DriveKind::Ram => fixed("ram", "<rect x='3' y='9' width='26' height='14' rx='2' fill='#2e9d58'/><g fill='#1f7640'><rect x='6' y='12' width='5' height='7'/><rect x='13.500' y='12' width='5' height='7'/><rect x='21' y='12' width='5' height='7'/></g><g fill='#d9a520'><rect x='5' y='23' width='2' height='3'/><rect x='9' y='23' width='2' height='3'/><rect x='13' y='23' width='2' height='3'/><rect x='17' y='23' width='2' height='3'/><rect x='21' y='23' width='2' height='3'/><rect x='25' y='23' width='2' height='3'/></g>".into()), // i18n-ignore: SVG drawing
        // The drive the system runs from gets a blue badge with panes, so it stands out from the others.
        DriveKind::Fixed | DriveKind::Unknown if is_system => fixed("system", format!("{}<rect x='11' y='2' width='11' height='11' rx='2.200' fill='#2f86d6' stroke='#fff' stroke-width='1'/><g fill='#fff'><rect x='13' y='4' width='3.200' height='3.200'/><rect x='16.900' y='4' width='3.200' height='3.200'/><rect x='13' y='7.900' width='3.200' height='3.200'/><rect x='16.900' y='7.900' width='3.200' height='3.200'/></g>", drive_body())), // i18n-ignore: SVG drawing
        DriveKind::Fixed | DriveKind::Unknown => fixed("drive", drive_body().to_string()),
    }
}

fn by_cloud(provider: &str) -> Art {
    let p = provider.to_lowercase();
    let (id, top, bottom) = if p.contains("dropbox") {
        ("dropbox", "#4f9dff", "#0061ff")
    } else if p.contains("google") {
        ("google", "#8fd68a", "#34a853")
    } else if p.contains("icloud") {
        ("icloud", "#eef1f6", "#b4bcc9")
    } else {
        ("cloud", "#6cc0ff", "#2f7fd6")
    };
    fixed(id, format!("<defs><linearGradient id='c' x1='0' y1='0' x2='0' y2='1'><stop offset='0' stop-color='{top}'/><stop offset='1' stop-color='{bottom}'/></linearGradient></defs><path fill='url(#c)' d='M24.500 26H9a7 7 0 0 1-1.200-13.900A9 9 0 0 1 25 13.100 6.500 6.500 0 0 1 24.500 26z'/>")) // i18n-ignore: SVG drawing
}

// ---------------------------------------------------------------- choosing and drawing

fn page_art(family: Family, label: Option<String>) -> Art {
    Art::Svg { id: format!("page:{:?}:{}", family.picture, family.color), svg: SvgSource::Page(family), label }
}

fn folder_art(family: Family) -> Art {
    Art::Svg { id: format!("folder:{:?}:{}", family.picture, family.color), svg: SvgSource::Folder(family), label: None }
}

pub(super) fn resolve(t: &IconFor) -> Art {
    if let Some(kind) = t.drive {
        return by_drive(kind, t.is_system);
    }
    if t.is_root {
        return by_drive(DriveKind::Fixed, t.is_system);
    }
    if let Some(family) = t.place.and_then(by_place) {
        return folder_art(family);
    }
    if let Some(provider) = t.cloud {
        return by_cloud(provider);
    }
    if t.kind == Kind::Folder {
        return folder_art(by_folder_name(t.name));
    }
    if let Some(family) = by_file_name(t.name).or_else(|| by_extension(t.ext)) {
        let label = family.labelled.then(|| t.ext.chars().take(4).collect::<String>().to_uppercase());
        return page_art(family, label);
    }
    if t.ext.is_empty() {
        return page_art(with(LINES, GRAY), None);
    }
    page_art(with(Picture::Blank, colour_of(t.ext)), Some(t.ext.chars().take(4).collect::<String>().to_uppercase()))
}

/// Text on an icon, once it's big enough to read.
pub(super) fn paint_label(p: &egui::Painter, r: Rect, label: &str, dim: bool) {
    if r.height() >= 28.0 {
        let size = r.height() * if label.chars().count() > 3 { 0.15 } else { 0.18 };
        let color = if dim { Color32::WHITE.gamma_multiply(0.5) } else { Color32::WHITE };
        p.text(r.center() + vec2(0.0, r.height() * 0.24), Align2::CENTER_CENTER, label, FontId::proportional(size), color);
    }
}

/// Glyph and page icons of the Phosphor pack need no picture.
pub(super) fn paint_font_art(p: &egui::Painter, r: Rect, art: &Art, dim: bool) {
    let shade = |c: Color32| if dim { c.gamma_multiply(0.5) } else { c };
    match art {
        Art::Glyph(glyph, color) => {
            p.text(r.center(), Align2::CENTER_CENTER, *glyph, file_icon_font(r.height()), shade(*color));
        }
        Art::Page { color, label } => {
            p.text(r.center(), Align2::CENTER_CENTER, egui_phosphor::fill::FILE, file_icon_font(r.height()), shade(*color));
            if let Some(label) = label {
                paint_label(p, r, label, dim);
            }
        }
        Art::Svg { .. } => {}
    }
}

/// An SVG drawn at `side` pixels square, as premultiplied RGBA.
pub(super) fn render_svg(svg: &[u8], side: u32) -> Option<egui::ColorImage> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_data(svg, &usvg::Options::default()).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(side, side)?;
    let size = tree.size();
    let scale = side as f32 / size.width().max(size.height());
    let offset = |length: f32| (side as f32 - length * scale) / 2.0;
    let transform = tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, offset(size.width()), offset(size.height()));
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Some(egui::ColorImage::from_rgba_premultiplied([side as usize, side as usize], pixmap.data()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> IconFor<'_> {
        let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
        IconFor { kind: crate::util::kind_of(ext, false), ext, name, place: None, drive: None, is_root: false, is_system: false, cloud: None, dim: false }
    }

    fn folder(name: &str) -> IconFor<'_> {
        IconFor { kind: Kind::Folder, ext: "", name, place: None, drive: None, is_root: false, is_system: false, cloud: None, dim: false }
    }

    /// Fails with the name of any icon whose SVG doesn't draw, or that draws nothing.
    fn assert_draws(art: Art, what: &str) {
        let Art::Svg { svg, .. } = art else { panic!("{what} isn't drawn by the Daleris pack") };
        let img = render_svg(&svg_bytes(&svg), 48).unwrap_or_else(|| panic!("{what}: the SVG doesn't parse"));
        assert!(img.pixels.iter().any(|p| p.a() > 0), "{what}: draws nothing");
    }

    const KNOWN: &[&str] = &[
        "exe", "com", "app", "msi", "msix", "msixbundle", "appx", "appxbundle", "appinstaller", "msu", "deb", "rpm", "snap", "flatpak", "appimage", "dmg", "pkg", "ipa", "apk", "aab", "xapk", "nupkg", "vsix", "crx", "xpi", "whl", "gem", "crate", "jar",
        "zip", "7z", "rar", "tar", "gz", "tgz", "bz2", "xz", "zst", "lz4", "cab", "wim", "iso", "img", "vhdx", "vmdk", "qcow2",
        "mp3", "aac", "m4a", "ogg", "opus", "wma", "flac", "wav", "aiff", "midi", "m3u", "mp4", "mov", "mkv", "webm", "avi", "wmv", "flv", "mpg", "3gp", "srt", "vtt",
        "doc", "docx", "odt", "rtf", "xls", "xlsx", "ods", "csv", "ppt", "pptx", "odp", "accdb", "one", "vsdx", "msg", "pst", "eml", "pdf",
        "txt", "log", "md", "tex", "epub", "ipynb", "ics", "vcf", "png", "jpg", "gif", "webp", "heic", "raw", "svg", "psd", "ai", "fig", "sketch",
        "rs", "py", "js", "ts", "tsx", "jsx", "html", "css", "scss", "vue", "c", "cpp", "h", "cs", "go", "java", "kt", "swift", "rb", "php", "lua", "r", "zig", "sql", "sh", "ps1", "bat", "patch",
        "json", "xml", "yaml", "toml", "ini", "env", "reg", "lock", "db", "sqlite", "parquet", "dll", "so", "wasm", "bin", "torrent", "part", "pem", "key", "crt", "gpg",
        "ttf", "woff2", "obj", "stl", "blend", "gltf", "dwg", "step", "url", "lnk", "gpx", "rom", "gguf", "onnx", "bak", "tmp", "dmp",
    ];

    #[test]
    fn every_known_type_has_its_own_icon() {
        for ext in KNOWN {
            assert!(by_extension(ext).is_some(), "the Daleris pack has no icon for .{ext}");
            assert_draws(resolve(&file(&format!("a.{ext}"))), ext);
        }
    }

    #[test]
    fn special_names_and_folders_draw() {
        for name in ["dockerfile", "makefile", ".gitignore", "cargo.toml", "cargo.lock", ".env", ".env.local", "tsconfig.json", ".bashrc", "readme.md", "license", "changelog.md", "arch.pkg.tar.zst"] {
            assert!(by_file_name(name).is_some(), "{name}");
            assert_draws(resolve(&file(name)), name);
        }
        for name in [".git", ".github", "node_modules", "src", "docs", "tests", "build", "assets", "config", "backup", "favorites", "anything"] {
            assert_draws(resolve(&folder(name)), name);
        }
        for place in ["Home", "Desktop", "Downloads", "Documents", "Pictures", "Music", "Videos"] {
            assert_draws(folder_art(by_place(place).expect(place)), place);
        }
    }

    #[test]
    fn drives_clouds_and_unknown_types_draw() {
        for kind in [DriveKind::Fixed, DriveKind::Removable, DriveKind::Network, DriveKind::Optical, DriveKind::Ram, DriveKind::Unknown] {
            assert_draws(by_drive(kind, false), "a drive");
            assert_draws(by_drive(kind, true), "the system drive");
        }
        for provider in ["OneDrive", "Dropbox", "GoogleDrive", "iCloud", "Nextcloud"] {
            assert_draws(by_cloud(provider), provider);
        }
        assert_draws(resolve(&file("a.qqqq")), "an unknown type");
        assert_draws(resolve(&file("noextension")), "a file with no extension");
    }
}
