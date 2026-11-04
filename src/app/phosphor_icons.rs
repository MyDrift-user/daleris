//! The Phosphor icon pack: every drive, folder and file drawn with a solid glyph from the Phosphor set.

use eframe::egui::Color32;
use egui_phosphor::fill as g;

use crate::fsx::DriveKind;
use crate::util::Kind;

use super::file_icons::{Art, IconFor};

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

type Icon = (&'static str, Color32);

fn by_drive(kind: DriveKind) -> Icon {
    match kind {
        DriveKind::Removable => (g::USB, ORANGE),
        DriveKind::Network => (g::HARD_DRIVES, TEAL),
        DriveKind::Optical => (g::DISC, PURPLE),
        DriveKind::Ram => (g::CPU, LIME),
        DriveKind::Fixed | DriveKind::Unknown => (g::HARD_DRIVE, GRAY),
    }
}

fn by_place(place: &str) -> Option<Icon> {
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

fn by_cloud(provider: &str) -> Icon {
    let p = provider.to_lowercase();
    if p.contains("dropbox") {
        (g::DROPBOX_LOGO, BLUE)
    } else if p.contains("google") {
        (g::GOOGLE_DRIVE_LOGO, LIME)
    } else if p.contains("icloud") {
        (g::CLOUD, GRAY)
    } else {
        (g::CLOUD, BLUE)
    }
}

fn by_folder_name(name: &str) -> Icon {
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

/// Files recognised by their whole name, before their extension counts.
fn by_file_name(name: &str) -> Option<Icon> {
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

fn by_extension(ext: &str) -> Option<Icon> {
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

pub(super) fn resolve(t: &IconFor) -> Art {
    let known = if let Some(kind) = t.drive {
        Some(by_drive(kind))
    } else if t.is_root {
        Some(by_drive(DriveKind::Fixed))
    } else if let Some(icon) = t.place.and_then(by_place) {
        Some(icon)
    } else if let Some(provider) = t.cloud {
        Some(by_cloud(provider))
    } else if t.kind == Kind::Folder {
        Some(by_folder_name(t.name))
    } else {
        by_file_name(t.name).or_else(|| by_extension(t.ext))
    };
    if let Some((glyph, color)) = known {
        return Art::Glyph(glyph, color);
    }
    if t.ext.is_empty() {
        return Art::Page { color: GRAY, label: None };
    }
    Art::Page { color: colour_of(t.ext), label: Some(t.ext.chars().take(4).collect::<String>().to_uppercase()) }
}
