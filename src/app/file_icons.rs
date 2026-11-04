//! The built-in icon pack: which picture each drive, folder and file gets. Daleris' own art for drives, installers,
//! archives, audio, video and office files; font glyphs for the rest; and for an unknown type, a page in its own
//! colour with the extension on it. To add a file type, add it to a list in `by_family` or `by_extension`.

use std::collections::HashMap;
use std::sync::OnceLock;

use eframe::egui::{self, vec2, Align2, Color32, FontId, Rect};
use egui_phosphor::fill as g;

use crate::fsx::DriveKind;
use crate::theme::file_icon_font;
use crate::util::Kind;

const BLUE: Color32 = Color32::from_rgb(0x4a, 0x90, 0xe2);
const RED: Color32 = Color32::from_rgb(0xe0, 0x56, 0x4f);
const LIME: Color32 = Color32::from_rgb(0x3f, 0xb3, 0x76);
const ORANGE: Color32 = Color32::from_rgb(0xf0, 0x8c, 0x3a);
const PURPLE: Color32 = Color32::from_rgb(0x9b, 0x72, 0xe8);
const TEAL: Color32 = Color32::from_rgb(0x2f, 0xb5, 0xb0);
const GOLD: Color32 = Color32::from_rgb(0xc9, 0xa2, 0x27);
const PINK: Color32 = Color32::from_rgb(0xe0, 0x6c, 0xa8);
const GRAY: Color32 = Color32::from_rgb(0x8d, 0x93, 0xa1);
const AMBER: Color32 = Color32::from_rgb(0xf2, 0xc2, 0x4e);

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
    /// A picture: an id for the texture cache, and its SVG, made only when it isn't cached.
    Svg { id: String, svg: SvgSource },
    /// A glyph from the font.
    Glyph(&'static str, Color32),
    /// A page in a colour, with the extension on it when there's room.
    Page { color: Color32, label: Option<String> },
}

pub(super) enum SvgSource {
    Page(Family),
    Fixed(&'static str),
}

pub(super) fn svg_bytes(source: &SvgSource) -> Vec<u8> {
    match source {
        SvgSource::Page(family) => family.svg().into_bytes(),
        SvgSource::Fixed(svg) => svg.as_bytes().to_vec(),
    }
}

// ---------------------------------------------------------------- own art: pages

#[derive(Clone, Copy)]
enum Picture {
    Zipper,
    Note,
    Play,
    Box,
    Disc,
    List,
    Window,
    Letter(&'static str),
}

/// A kind of file with its own colours. `(colour, darker shade)` as SVG colours.
#[derive(Clone, Copy)]
pub(super) struct Family {
    picture: Picture,
    color: &'static str,
    shade: &'static str,
}

impl Family {
    fn svg(&self) -> String {
        let white = "fill=\"#fff\"";
        let picture = match self.picture {
            Picture::Zipper => format!("<g {white}><rect x='14' y='9' width='3' height='2'/><rect x='17' y='11' width='3' height='2'/><rect x='14' y='13' width='3' height='2'/><rect x='17' y='15' width='3' height='2'/><rect x='14' y='17' width='3' height='2'/><rect x='14.500' y='20' width='5' height='6' rx='1.200'/></g><rect x='16' y='22.500' width='2' height='2' fill='{}'/>", self.shade),
            Picture::Note => format!("<path {white} d='M21 11v10.500a2.800 2.800 0 1 1-1.800-2.600V14l-6 1.500v8a2.800 2.800 0 1 1-1.800-2.600V12.800z'/>"),
            Picture::Play => format!("<circle cx='16' cy='18.500' r='7' {white}/><path fill='{}' d='M14 15l5.500 3.500L14 22z'/>", self.color),
            Picture::Box => format!("<path {white} d='M16 12l7 3.500v7.500L16 26.500 9 23v-7.500z'/><path fill='none' stroke='{}' stroke-width='1.300' stroke-linejoin='round' d='M9 15.500l7 3.500 7-3.500M16 19v7.500'/>", self.shade),
            Picture::Disc => format!("<circle cx='16' cy='18.500' r='7.200' {white}/><circle cx='16' cy='18.500' r='2' fill='{}'/>", self.color),
            Picture::List => format!("<g {white}><rect x='10' y='13' width='12' height='2' rx='1'/><rect x='10' y='17.500' width='12' height='2' rx='1'/><rect x='10' y='22' width='8' height='2' rx='1'/></g>"),
            Picture::Window => format!("<rect x='9' y='12' width='14' height='13' rx='2' {white}/><rect x='9' y='12' width='14' height='3.500' rx='1.500' fill='{}'/><circle cx='11.500' cy='13.800' r='.800' {white}/><circle cx='14' cy='13.800' r='.800' {white}/>", self.shade),
            Picture::Letter(path) => format!("<path fill='none' stroke='#fff' stroke-width='2.400' stroke-linecap='round' stroke-linejoin='round' d='{path}'/>"),
        };
        format!(
            "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'><path fill='{}' d='M8 1.500h11.500L27 9v19.500a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2v-25a2 2 0 0 1 2-2z'/><path fill='{}' d='M19.500 1.500L27 9h-5.500a2 2 0 0 1-2-2z'/>{picture}</svg>",
            self.color, self.shade
        )
    }
}

const fn fam(picture: Picture, color: &'static str, shade: &'static str) -> Family {
    Family { picture, color, shade }
}

/// Installers, archives, disk images, audio, video and office files, each family looking different.
fn by_family(name: &str, ext: &str) -> Option<(&'static str, Family)> {
    use Picture::*;
    if name.ends_with(".pkg.tar.zst") || name.ends_with(".pkg.tar.xz") {
        return Some(("linux", fam(Box, "#e0762f", "#b4551a")));
    }
    Some(match ext {
        // programs
        "exe" | "com" | "scr" | "app" => ("program", fam(Window, "#3c78d8", "#2556a8")),
        // installers, by system
        "msi" | "msix" | "msixbundle" | "appx" | "appxbundle" | "appinstaller" | "msp" | "msu" | "mst" | "application" => ("windows", fam(Box, "#2f86d6", "#1c62a6")),
        "deb" | "rpm" | "snap" | "flatpak" | "flatpakref" | "appimage" | "pacman" | "apkg" => ("linux", fam(Box, "#e0762f", "#b4551a")),
        "dmg" | "pkg" | "mpkg" | "ipa" => ("apple", fam(Box, "#8a8f9c", "#646975")),
        "apk" | "aab" | "xapk" | "apks" | "apkm" => ("android", fam(Box, "#3fae6a", "#2a8549")),
        "nupkg" | "snupkg" | "vsix" | "vsixpackage" | "crx" | "xpi" | "whl" | "gem" | "crate" | "jar" | "war" | "ear" | "unitypackage" => ("package", fam(Box, "#26a69a", "#1a7d74")),
        // archives
        "zip" | "zipx" => ("zip", fam(Zipper, "#d9a520", "#a87d10")),
        "7z" => ("7z", fam(Zipper, "#2e9d8f", "#1f7469")),
        "rar" | "r00" | "r01" => ("rar", fam(Zipper, "#8e5bd0", "#6a3fa6")),
        "tar" | "cpio" | "shar" => ("tar", fam(Zipper, "#a9744a", "#7f5332")),
        "gz" | "gzip" | "tgz" | "z" => ("gz", fam(Zipper, "#d9722c", "#a9531a")),
        "bz" | "bz2" | "tbz" | "tbz2" | "xz" | "txz" | "lzma" | "lz" | "lz4" | "lzo" | "zst" | "zstd" | "br" => ("xz", fam(Zipper, "#c25a4a", "#963f31")),
        "cab" | "arc" | "ace" | "arj" | "sit" | "sitx" | "wim" | "swm" | "esd" | "lha" | "lzh" | "ar" => ("arc", fam(Zipper, "#5b84c4", "#3f6299")),
        // disk images
        "iso" | "img" | "nrg" | "mdf" | "mds" | "toast" | "ccd" => ("disc", fam(Disc, "#6d7dd6", "#4c5ab0")),
        "vhd" | "vhdx" | "vmdk" | "vdi" | "qcow" | "qcow2" | "ova" | "ovf" => ("vm", fam(Disc, "#4f8f9e", "#366c79")),
        // audio
        "mp3" | "aac" | "m4a" | "m4b" | "ogg" | "oga" | "opus" | "wma" | "amr" | "ra" | "mka" | "3ga" => ("audio", fam(Note, "#e0569a", "#b13a76")),
        "flac" | "wav" | "wave" | "aiff" | "aif" | "alac" | "ape" | "wv" | "dsf" | "dff" | "pcm" | "au" => ("lossless", fam(Note, "#9b4fd8", "#7433ad")),
        "mid" | "midi" | "kar" | "mscz" | "musicxml" => ("midi", fam(Note, "#f0883a", "#c26623")),
        "m3u" | "m3u8" | "pls" | "cue" | "wpl" | "xspf" | "asx" => ("playlist", fam(List, "#e0569a", "#b13a76")),
        // video
        "mp4" | "m4v" | "mov" | "qt" => ("video", fam(Play, "#e0483f", "#b0302a")),
        "mkv" | "webm" | "ogv" => ("webvideo", fam(Play, "#7a5be0", "#5a3eb5")),
        "avi" | "wmv" | "asf" | "divx" | "xvid" | "flv" | "f4v" | "rm" | "rmvb" => ("oldvideo", fam(Play, "#d9722c", "#a9531a")),
        "mpg" | "mpeg" | "mpe" | "m2v" | "mts" | "m2ts" | "vob" | "3gp" | "3g2" => ("mpeg", fam(Play, "#2e9d8f", "#1f7469")),
        // office, each app by its first letter
        "doc" | "docx" | "docm" | "dot" | "dotx" | "dotm" | "rtf" | "odt" | "ott" | "wps" | "pages" | "wpd" | "hwp" => ("word", fam(Letter("M10 13l2.500 11L16 15.500 19.500 24 22 13"), "#2f6fd0", "#1f50a0")),
        "xls" | "xlsx" | "xlsm" | "xlsb" | "xlt" | "xltx" | "xltm" | "ods" | "ots" | "numbers" | "et" => ("excel", fam(Letter("M11 13l10 11M21 13L11 24"), "#2e9d58", "#1f7640")),
        "ppt" | "pptx" | "pptm" | "pps" | "ppsx" | "pot" | "potx" | "odp" | "otp" | "dps" => ("powerpoint", fam(Letter("M12 25V13h5a3.800 3.800 0 0 1 0 7.600h-5"), "#de6a33", "#b04b1d")),
        "mdb" | "accdb" | "accde" | "accdr" | "odb" => ("access", fam(Letter("M10.500 25L16 13l5.500 12M12.500 21h7"), "#b53a4d", "#8c2636")),
        "one" | "onetoc2" | "onepkg" => ("onenote", fam(Letter("M11 25V13l10 12V13"), "#8144c4", "#5f2d99")),
        "pst" | "ost" | "oft" | "msg" | "eml" | "emlx" | "mbox" => ("mail", fam(Letter("M9.500 15h13v9h-13zM9.500 15.500L16 20l6.500-4.500"), "#2f86d6", "#1c62a6")),
        "vsd" | "vsdx" | "vsdm" | "vssx" | "vstx" => ("visio", fam(Letter("M10.500 13L16 25l5.500-12"), "#4a59c9", "#33409c")),
        _ => return None,
    })
}

// ---------------------------------------------------------------- own art: drives

const DRIVE_BODY: &str = "<defs><linearGradient id='b' x1='0' y1='0' x2='0' y2='1'><stop offset='0' stop-color='#e8ebf0'/><stop offset='1' stop-color='#aab1bd'/></linearGradient></defs>\
<path fill='url(#b)' d='M6.500 8h19a2.500 2.500 0 0 1 2.400 1.800L30 17v6a2.500 2.500 0 0 1-2.500 2.500h-23A2.500 2.500 0 0 1 2 23v-6l2.100-7.200A2.500 2.500 0 0 1 6.500 8z'/>\
<rect x='2' y='17' width='28' height='8.500' rx='2.500' fill='#59606e'/><rect x='5' y='20' width='13' height='2.400' rx='1.200' fill='#7f8795'/>";

const DRIVE: &str = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'>__BODY__<circle cx='26' cy='21.200' r='1.700' fill='#4fe07f'/></svg>";
/// The drive the system runs from gets a blue badge with panes, so it stands out from the others.
const DRIVE_SYSTEM: &str = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'>__BODY__<circle cx='26' cy='21.200' r='1.700' fill='#4fe07f'/><rect x='11' y='2' width='11' height='11' rx='2.200' fill='#2f86d6' stroke='#fff' stroke-width='1'/><g fill='#fff'><rect x='13' y='4' width='3.200' height='3.200'/><rect x='16.900' y='4' width='3.200' height='3.200'/><rect x='13' y='7.900' width='3.200' height='3.200'/><rect x='16.900' y='7.900' width='3.200' height='3.200'/></g></svg>";
const DRIVE_USB: &str = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'><rect x='11' y='2' width='10' height='8' rx='1' fill='#c3c9d3'/><rect x='13' y='4' width='2' height='2' fill='#59606e'/><rect x='17' y='4' width='2' height='2' fill='#59606e'/><rect x='8' y='9' width='16' height='21' rx='3' fill='#e0762f'/><rect x='11' y='14' width='10' height='9' rx='1.500' fill='#fff' opacity='.9'/><circle cx='16' cy='26.500' r='1.300' fill='#ffe2c9'/></svg>";
const DRIVE_OPTICAL: &str = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'><defs><linearGradient id='c' x1='0' y1='0' x2='1' y2='1'><stop offset='0' stop-color='#8fd3ff'/><stop offset='.35' stop-color='#d7b7ff'/><stop offset='.65' stop-color='#ffd0a1'/><stop offset='1' stop-color='#9be8c5'/></linearGradient></defs><circle cx='16' cy='16' r='14' fill='url(#c)'/><circle cx='16' cy='16' r='5' fill='#fff'/><circle cx='16' cy='16' r='2' fill='#59606e'/></svg>";
const DRIVE_NETWORK: &str = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'><rect x='5' y='3' width='22' height='7' rx='2' fill='#59606e'/><rect x='5' y='12' width='22' height='7' rx='2' fill='#6e7584'/><circle cx='23' cy='6.500' r='1.300' fill='#4fe07f'/><circle cx='23' cy='15.500' r='1.300' fill='#4fe07f'/><path fill='none' stroke='#2fb5b0' stroke-width='2.200' stroke-linecap='round' d='M16 19v5M6 27h20M16 24v3'/><circle cx='16' cy='27' r='2.500' fill='#2fb5b0'/></svg>";
const DRIVE_RAM: &str = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'><rect x='3' y='9' width='26' height='14' rx='2' fill='#3fae6a'/><g fill='#1f7640'><rect x='6' y='12' width='5' height='7'/><rect x='13.500' y='12' width='5' height='7'/><rect x='21' y='12' width='5' height='7'/></g><g fill='#d9a520'><rect x='5' y='23' width='2' height='3'/><rect x='9' y='23' width='2' height='3'/><rect x='13' y='23' width='2' height='3'/><rect x='17' y='23' width='2' height='3'/><rect x='21' y='23' width='2' height='3'/><rect x='25' y='23' width='2' height='3'/></g></svg>";
const CLOUD: &str = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'><defs><linearGradient id='c' x1='0' y1='0' x2='0' y2='1'><stop offset='0' stop-color='__TOP__'/><stop offset='1' stop-color='__BOTTOM__'/></linearGradient></defs><path fill='url(#c)' d='M24.500 26H9a7 7 0 0 1-1.200-13.900A9 9 0 0 1 25 13.100 6.500 6.500 0 0 1 24.500 26z'/></svg>";

/// The drive pictures, built once with the shared body filled in.
fn fixed_svg(key: &str, template: &'static str, fill: &[(&str, &str)]) -> &'static str {
    static BUILT: OnceLock<std::sync::Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let mut built = BUILT.get_or_init(Default::default).lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    built.entry(key.to_string()).or_insert_with(|| {
        let mut svg = template.to_string();
        for (from, to) in fill {
            svg = svg.replace(from, to);
        }
        Box::leak(svg.into_boxed_str())
    })
}

fn fixed(id: &str, template: &'static str, fill: &[(&str, &str)]) -> Art {
    Art::Svg { id: format!("own:{id}"), svg: SvgSource::Fixed(fixed_svg(id, template, fill)) }
}

fn by_drive(kind: DriveKind, is_system: bool) -> Art {
    let body = [("__BODY__", DRIVE_BODY)];
    match kind {
        DriveKind::Removable => fixed("usb", DRIVE_USB, &[]),
        DriveKind::Network => fixed("network", DRIVE_NETWORK, &[]),
        DriveKind::Optical => fixed("optical", DRIVE_OPTICAL, &[]),
        DriveKind::Ram => fixed("ram", DRIVE_RAM, &[]),
        DriveKind::Fixed | DriveKind::Unknown if is_system => fixed("system", DRIVE_SYSTEM, &body),
        DriveKind::Fixed | DriveKind::Unknown => fixed("drive", DRIVE, &body),
    }
}

fn by_cloud(provider: &str) -> Art {
    let p = provider.to_lowercase();
    if p.contains("dropbox") {
        Art::Glyph(g::DROPBOX_LOGO, BLUE)
    } else if p.contains("google") {
        Art::Glyph(g::GOOGLE_DRIVE_LOGO, LIME)
    } else if p.contains("icloud") {
        fixed("icloud", CLOUD, &[("__TOP__", "#eef1f6"), ("__BOTTOM__", "#b4bcc9")])
    } else {
        fixed("cloud", CLOUD, &[("__TOP__", "#6cc0ff"), ("__BOTTOM__", "#2f7fd6")])
    }
}

// ---------------------------------------------------------------- glyphs

fn by_place(place: &str) -> Option<(&'static str, Color32)> {
    Some(match place {
        "Home" => (g::HOUSE, BLUE),
        "Desktop" => (g::DESKTOP, BLUE),
        "Downloads" => (g::DOWNLOAD_SIMPLE, TEAL),
        "Documents" => (g::FILE_TEXT, BLUE),
        "Pictures" => (g::IMAGE, LIME),
        "Music" => (g::MUSIC_NOTES, ORANGE),
        "Videos" => (g::FILM_STRIP, PURPLE),
        _ => return None,
    })
}

fn by_folder_name(name: &str) -> (&'static str, Color32) {
    match name {
        ".git" => (g::GIT_BRANCH, ORANGE),
        ".github" => (g::GITHUB_LOGO, GRAY),
        ".gitlab" => (g::GITLAB_LOGO, ORANGE),
        "node_modules" | "vendor" | "packages" => (g::PACKAGE, LIME),
        ".trash" | "trash" | "$recycle.bin" => (g::TRASH, GRAY),
        "src" | "source" | "sources" | "lib" | "code" => (g::FOLDER, TEAL),
        "doc" | "docs" | "documentation" | "wiki" => (g::FOLDER, BLUE),
        "test" | "tests" | "spec" | "specs" | "__tests__" => (g::FOLDER, LIME),
        "build" | "dist" | "target" | "out" | "bin" | "obj" | "release" | "debug" => (g::FOLDER, GRAY),
        "assets" | "images" | "img" | "icons" | "media" | "public" | "static" => (g::FOLDER, PURPLE),
        "config" | "configs" | "settings" | ".config" | ".vscode" | ".idea" => (g::FOLDER, GRAY),
        "backup" | "backups" | "archive" | "archives" => (g::FOLDER, GOLD),
        _ => (g::FOLDER, AMBER),
    }
}

fn by_file_name(name: &str) -> Option<(&'static str, Color32)> {
    let stem = name.split('.').next().unwrap_or(name);
    Some(match name {
        "dockerfile" | "containerfile" | ".dockerignore" | "docker-compose.yml" | "docker-compose.yaml" | "compose.yml" | "compose.yaml" => (g::PACKAGE, BLUE),
        "makefile" | "gnumakefile" | "justfile" | "cmakelists.txt" | "meson.build" | "rakefile" | "build.rs" => (g::HAMMER, ORANGE),
        ".gitignore" | ".gitattributes" | ".gitmodules" | ".gitconfig" | ".mailmap" => (g::GIT_BRANCH, ORANGE),
        "package.json" | "cargo.toml" | "pyproject.toml" | "requirements.txt" | "go.mod" | "gemfile" | "pom.xml" | "build.gradle" | "build.gradle.kts" | "composer.json" => (g::PACKAGE, LIME),
        "cargo.lock" | "package-lock.json" | "yarn.lock" | "pnpm-lock.yaml" | "gemfile.lock" | "poetry.lock" | "go.sum" | "composer.lock" | "flake.lock" => (g::FILE_LOCK, GRAY),
        ".env" | ".envrc" | ".npmrc" | ".netrc" | "id_rsa" | "id_ed25519" | "authorized_keys" | "known_hosts" => (g::KEY, GOLD),
        ".editorconfig" | ".prettierrc" | ".eslintrc" | ".clang-format" | ".rustfmt.toml" | "rustfmt.toml" | "tsconfig.json" => (g::GEAR, GRAY),
        ".bashrc" | ".bash_profile" | ".zshrc" | ".profile" | ".bash_history" | ".zsh_history" => (g::TERMINAL_WINDOW, LIME),
        _ if name.starts_with(".env.") => (g::KEY, GOLD),
        _ => match stem {
            "readme" => (g::BOOK_OPEN, BLUE),
            "license" | "licence" | "copying" | "notice" => (g::CERTIFICATE, GOLD),
            "changelog" | "changes" | "history" => (g::LIST_BULLETS, BLUE),
            _ => return None,
        },
    })
}

fn by_extension(ext: &str) -> Option<(&'static str, Color32)> {
    Some(match ext {
        // pictures
        "png" => (g::FILE_PNG, LIME),
        "jpg" | "jpeg" | "jfif" | "jpe" => (g::FILE_JPG, LIME),
        "svg" | "svgz" => (g::FILE_SVG, ORANGE),
        "gif" | "webp" | "bmp" | "ico" | "icns" | "cur" | "tif" | "tiff" | "heic" | "heif" | "avif" | "jxl" | "raw" | "cr2" | "cr3" | "nef" | "arw" | "dng" | "orf" | "rw2" | "tga" | "dds" | "exr" | "hdr" | "qoi" | "pbm" | "pgm" | "ppm" => (g::FILE_IMAGE, LIME),
        // design
        "fig" => (g::FIGMA_LOGO, PURPLE),
        "sketch" => (g::SKETCH_LOGO, GOLD),
        "psd" | "psb" | "ai" | "xd" | "indd" | "afdesign" | "afphoto" | "afpub" | "kra" | "xcf" | "clip" | "cdr" | "procreate" => (g::PALETTE, PURPLE),
        "ase" | "aco" | "gpl" => (g::SWATCHES, PINK),
        // video and subtitles
        "mp4" | "mkv" | "mov" | "avi" | "wmv" | "webm" | "m4v" | "flv" | "mpg" | "mpeg" | "mpe" | "3gp" | "ogv" | "vob" | "mts" | "m2ts" | "asf" | "rm" | "rmvb" | "f4v" | "divx" => (g::FILE_VIDEO, PURPLE),
        "srt" | "vtt" | "ass" | "ssa" | "sub" => (g::SUBTITLES, PURPLE),
        // sound
        "mp3" | "wav" | "flac" | "ogg" | "oga" | "m4a" | "aac" | "wma" | "opus" | "aiff" | "aif" | "alac" | "ape" | "mka" | "amr" | "wv" | "au" => (g::FILE_AUDIO, ORANGE),
        "mid" | "midi" | "mscz" | "musicxml" => (g::MUSIC_NOTES, ORANGE),
        "m3u" | "m3u8" | "pls" | "cue" => (g::LIST_BULLETS, ORANGE),
        // documents
        "pdf" => (g::FILE_PDF, RED),
        "doc" | "docx" | "docm" | "dot" | "dotx" | "odt" | "rtf" | "pages" | "wpd" => (g::FILE_DOC, BLUE),
        "xls" | "xlsx" | "xlsm" | "xlsb" | "ods" | "numbers" => (g::FILE_XLS, LIME),
        "csv" | "tsv" => (g::FILE_CSV, LIME),
        "ppt" | "pptx" | "pptm" | "odp" => (g::FILE_PPT, ORANGE),
        "txt" | "text" => (g::FILE_TXT, GRAY),
        "log" | "out" => (g::SCROLL, GRAY),
        "md" | "markdown" | "mdx" | "rst" | "adoc" | "org" => (g::FILE_MD, BLUE),
        "tex" | "latex" | "bib" => (g::NOTE, TEAL),
        "epub" | "mobi" | "azw" | "azw3" | "fb2" | "djvu" | "cbz" | "cbr" => (g::BOOK_OPEN, TEAL),
        "one" | "ipynb" => (g::NOTEBOOK, ORANGE),
        "eml" | "msg" | "mbox" => (g::ENVELOPE, BLUE),
        "ics" => (g::CALENDAR, RED),
        "vcf" => (g::ADDRESS_BOOK, BLUE),
        // code with its own icon
        "rs" => (g::FILE_RS, ORANGE),
        "py" | "pyw" | "pyi" => (g::FILE_PY, BLUE),
        "js" | "mjs" | "cjs" => (g::FILE_JS, GOLD),
        "jsx" => (g::FILE_JSX, TEAL),
        "ts" | "cts" => (g::FILE_TS, BLUE),
        "tsx" => (g::FILE_TSX, BLUE),
        "html" | "htm" | "xhtml" => (g::FILE_HTML, ORANGE),
        "css" => (g::FILE_CSS, BLUE),
        "scss" | "sass" | "less" | "styl" => (g::FILE_CSS, PINK),
        "sql" => (g::FILE_SQL, GOLD),
        "vue" => (g::FILE_VUE, LIME),
        "c" => (g::FILE_C, BLUE),
        "cpp" | "cc" | "cxx" | "c++" => (g::FILE_CPP, BLUE),
        "cs" => (g::FILE_C_SHARP, PURPLE),
        // other code
        "h" | "hh" | "hpp" | "hxx" | "go" | "java" | "kt" | "kts" | "swift" | "rb" | "php" | "lua" | "luau" | "dart" | "scala" | "r" | "pl" | "pm" | "ex" | "exs" | "erl" | "hs" | "clj" | "zig" | "nim" | "cr" | "m" | "mm" | "jl" | "f90" | "asm" | "vb" | "fs" | "svelte" | "astro" | "groovy" | "gradle" | "elm" | "ml" | "sol" | "glsl" | "hlsl" | "wgsl" | "shader" => (g::FILE_CODE, TEAL),
        "sh" | "bash" | "zsh" | "fish" | "ps1" | "psm1" | "psd1" | "bat" | "cmd" | "nu" => (g::TERMINAL_WINDOW, LIME),
        "patch" | "diff" => (g::GIT_DIFF, ORANGE),
        // data and settings
        "json" | "jsonc" | "json5" | "jsonl" | "ndjson" | "geojson" => (g::BRACKETS_CURLY, GOLD),
        "xml" | "xsd" | "xsl" | "xslt" | "plist" | "wxs" | "resx" | "xaml" => (g::BRACKETS_ANGLE, ORANGE),
        "yaml" | "yml" | "toml" | "ini" | "cfg" | "conf" | "config" | "env" | "properties" | "editorconfig" => (g::FILE_INI, GRAY),
        "reg" | "inf" | "policy" | "admx" | "adml" => (g::GEAR, GRAY),
        "lock" => (g::FILE_LOCK, GRAY),
        "db" | "sqlite" | "sqlite3" | "db3" | "mdb" | "accdb" | "dbf" => (g::DATABASE, GOLD),
        "parquet" | "avro" | "orc" | "feather" => (g::TABLE, GOLD),
        // archives and disk images
        "zip" => (g::FILE_ZIP, GOLD),
        "7z" | "rar" | "tar" | "gz" | "tgz" | "bz2" | "tbz2" | "xz" | "txz" | "zst" | "lz" | "lz4" | "cab" | "sit" | "ar" => (g::FILE_ARCHIVE, GOLD),
        "iso" | "img" | "vhd" | "vhdx" | "vmdk" | "vdi" | "qcow2" | "nrg" | "toast" => (g::DISC, GRAY),
        // programs and packages
        "exe" | "com" | "scr" | "appx" | "appxbundle" => (g::APP_WINDOW, BLUE),
        "msi" | "msix" | "msixbundle" | "msp" => (g::WINDOWS_LOGO, BLUE),
        "apk" | "aab" | "xapk" => (g::ANDROID_LOGO, LIME),
        "dmg" | "pkg" | "app" | "ipa" => (g::APPLE_LOGO, GRAY),
        "deb" | "rpm" | "appimage" | "snap" | "flatpak" | "flatpakref" => (g::LINUX_LOGO, GOLD),
        "jar" | "war" | "ear" | "crate" | "whl" | "gem" | "nupkg" | "vsix" | "xpi" | "crx" => (g::PACKAGE, GOLD),
        "dll" | "so" | "dylib" | "sys" | "lib" | "a" | "o" | "ocx" | "drv" | "ko" => (g::CIRCUITRY, GRAY),
        "wasm" | "bin" | "dat" | "hex" | "elf" | "pyc" | "class" => (g::BINARY, GRAY),
        "torrent" => (g::FILE_ARROW_DOWN, TEAL),
        "crdownload" | "part" | "partial" | "download" => (g::FILE_ARROW_DOWN, GRAY),
        // keys and certificates
        "key" | "pem" | "pub" | "ppk" | "kdbx" => (g::KEY, GOLD),
        "crt" | "cer" | "der" | "p12" | "pfx" | "csr" => (g::CERTIFICATE, GOLD),
        "gpg" | "pgp" | "asc" | "sig" | "age" | "enc" => (g::LOCK, GOLD),
        // the rest
        "ttf" | "otf" | "ttc" | "woff" | "woff2" | "eot" | "fnt" => (g::TEXT_AA, PINK),
        "obj" | "stl" | "fbx" | "blend" | "gltf" | "glb" | "dae" | "3ds" | "ply" | "usd" | "usdz" | "3mf" => (g::CUBE, PURPLE),
        "dwg" | "dxf" | "step" | "stp" | "iges" | "skp" => (g::PENCIL_RULER, TEAL),
        "url" | "webloc" | "desktop" | "lnk" | "website" => (g::LINK, BLUE),
        "gpx" | "kml" | "kmz" | "shp" => (g::MAP_TRIFOLD, LIME),
        "nes" | "sfc" | "gba" | "gb" | "nds" | "sav" | "rom" | "pak" | "unitypackage" => (g::GAME_CONTROLLER, PURPLE),
        "gguf" | "safetensors" | "onnx" | "pth" | "ckpt" | "tflite" => (g::ROBOT, TEAL),
        "ino" | "kicad_pcb" | "gbr" => (g::CPU, TEAL),
        "bak" | "old" | "orig" | "tmp" | "temp" | "swp" | "swo" | "cache" => (g::FILE_DASHED, GRAY),
        "dmp" | "mdmp" | "core" | "crash" => (g::BUG, RED),
        _ => return None,
    })
}

/// A colour of its own for an unknown type, the same every time.
fn colour_of(ext: &str) -> Color32 {
    const PALETTE: [Color32; 8] = [BLUE, RED, LIME, ORANGE, PURPLE, TEAL, GOLD, PINK];
    let hash = ext.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    PALETTE[hash as usize % PALETTE.len()]
}

// ---------------------------------------------------------------- choosing and drawing

pub(super) fn resolve(t: &IconFor) -> Art {
    let glyph = |(glyph, color): (&'static str, Color32)| Art::Glyph(glyph, color);
    if let Some(kind) = t.drive {
        return by_drive(kind, t.is_system);
    }
    if t.is_root {
        return by_drive(DriveKind::Fixed, t.is_system);
    }
    if let Some(icon) = t.place.and_then(by_place) {
        return glyph(icon);
    }
    if let Some(provider) = t.cloud {
        return by_cloud(provider);
    }
    if t.kind == Kind::Folder {
        return glyph(by_folder_name(t.name));
    }
    if let Some(icon) = by_file_name(t.name) {
        return glyph(icon);
    }
    if let Some((id, family)) = by_family(t.name, t.ext) {
        return Art::Svg { id: format!("fam:{id}"), svg: SvgSource::Page(family) };
    }
    if let Some(icon) = by_extension(t.ext) {
        return glyph(icon);
    }
    if t.ext.is_empty() {
        return Art::Page { color: GRAY, label: None };
    }
    Art::Page { color: colour_of(t.ext), label: Some(t.ext.chars().take(4).collect::<String>().to_uppercase()) }
}

/// Glyph and page icons need no picture.
pub(super) fn paint_font_art(p: &egui::Painter, r: Rect, art: &Art, dim: bool) {
    let shade = |c: Color32| if dim { c.gamma_multiply(0.5) } else { c };
    match art {
        Art::Glyph(glyph, color) => {
            p.text(r.center(), Align2::CENTER_CENTER, *glyph, file_icon_font(r.height()), shade(*color));
        }
        Art::Page { color, label } => {
            p.text(r.center(), Align2::CENTER_CENTER, g::FILE, file_icon_font(r.height()), shade(*color));
            if let Some(label) = label.as_ref().filter(|_| r.height() >= 28.0) {
                let size = r.height() * if label.chars().count() > 3 { 0.15 } else { 0.18 };
                p.text(r.center() + vec2(0.0, r.height() * 0.16), Align2::CENTER_CENTER, label, FontId::proportional(size), shade(Color32::WHITE));
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
