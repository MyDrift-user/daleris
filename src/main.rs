#![windows_subsystem = "windows"]

mod app;
mod bg;
mod dirsize;
mod fileops;
mod fscommon;
#[cfg(windows)]
mod fsx;
#[cfg(not(windows))]
#[path = "fsx_unix.rs"]
mod fsx;
mod integration;
mod listing;
mod logging;
mod newitems;
mod policy;
mod search;
#[cfg(windows)]
mod shell;
#[cfg(not(windows))]
#[path = "shell_unix.rs"]
mod shell;
mod theme;
mod update;
mod util;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use daleris_integration::target::{self, Target};

fn main() -> eframe::Result<()> {
    logging::init();
    // A picker is part of another app's dialog, and does nothing but that.
    let picker = if std::env::args_os().nth(1).is_some_and(|a| a == "--pick") {
        let Some(request) = app::read_pick_request() else { std::process::exit(2) };
        Some(app::Picker::new(request))
    } else {
        util::adopt_old_settings();
        update::clean_after_update();
        if let Some(code) = update::handle_command_line().or_else(app::handle_integration_command) {
            std::process::exit(code);
        }
        None
    };
    // The same ID as the installers' shortcuts, so the taskbar treats the window and shortcut as one app.
    #[cfg(windows)]
    unsafe {
        let _ = windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(windows::core::w!("MyDrift.Daleris"));
    }
    let mut screenshot: Option<PathBuf> = None;
    let mut wanted = app::Wanted::default();
    let mut query: Option<String> = None;
    let mut new_window = false;
    let mut at: Option<(f32, f32)> = None;
    let mut named: Vec<OsString> = Vec::new();
    // Not `args()`: that panics on a path that isn't valid Unicode, which Windows allows.
    let mut args = std::env::args_os().skip(1);
    while let Some(a) = args.next() {
        let lossy = |v: Option<OsString>| v.map(|s| s.to_string_lossy().into_owned());
        match a.to_str().unwrap_or("") {
            "--screenshot" => screenshot = args.next().map(PathBuf::from),
            "--path" => wanted.dir = args.next().map(PathBuf::from),
            "--select" => {
                let item = args.next().map(PathBuf::from);
                wanted.select = item.as_deref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
                wanted.dir = item.as_deref().and_then(Path::parent).map(Path::to_path_buf);
            }
            "--this-pc" => wanted.this_pc = true,
            "--new-window" => new_window = true,
            // a tab dragged out: the new window's top left, in points
            "--at" => at = lossy(args.next()).and_then(|v| v.split_once(',').and_then(|(x, y)| Some((x.trim().parse::<f32>().ok()?, y.trim().parse::<f32>().ok()?)))),
            "--pick" => {}
            "--query" => query = lossy(args.next()),
            // FIXME: Audit that the environment access only happens in single-threaded code.
            "--icons" => unsafe { std::env::set_var("DALERIS_VIEW", "icons") },
            // FIXME: Audit that the environment access only happens in single-threaded code.
            "--props" => unsafe { std::env::set_var("DALERIS_TEST_PROPS", args.next().unwrap_or_default()) },
            // FIXME: Audit that the environment access only happens in single-threaded code.
            "--native-title" => unsafe { std::env::set_var("DALERIS_TEST_NATIVE_TITLE", "1") },
            // FIXME: Audit that the environment access only happens in single-threaded code.
            "--settings" => unsafe { std::env::set_var("DALERIS_PAGE", args.next().unwrap_or_default()) },
            other if other.starts_with("--") => {}
            _ => named.push(a),
        }
    }
    if !named.is_empty() && wanted.dir.is_none() {
        if let Some(code) = read_named(&named, &mut wanted) {
            std::process::exit(code);
        }
    }
    // started again with nothing to open: a window of its own, unless the person prefers a tab in the open one
    #[cfg(windows)]
    let plain = wanted.dir.is_none() && !wanted.this_pc;
    #[cfg(windows)]
    if picker.is_none() && screenshot.is_none() && !new_window && !(plain && app::starts_in_new_window()) && single_window_took(&wanted) {
        std::process::exit(0);
    }
    // no window at all for a drive Windows opens because it was just connected, when that's turned off
    #[cfg(windows)]
    if picker.is_none() && screenshot.is_none() && wanted.dir.as_deref().is_some_and(app::unwanted_new_drive) {
        std::process::exit(0);
    }
    let _ = new_window;
    let is_picker = picker.is_some();
    let window_title = picker.as_ref().map_or("Daleris".to_string(), app::Picker::window_title);
    if is_picker {
        app::end_pick_with_parent();
        wanted.dir = picker.as_ref().and_then(app::Picker::start_folder);
    }
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title(window_title)
        // matches the .desktop file, which is how Wayland desktops find the icon
        .with_app_id("daleris")
        .with_inner_size(test_size().unwrap_or(if is_picker { [920.0, 600.0] } else { [1280.0, 800.0] }))
        .with_min_inner_size(if is_picker { [560.0, 400.0] } else { [720.0, 420.0] })
        .with_icon(std::sync::Arc::new(eframe::egui::IconData { rgba: theme::logo_rgba(64, theme::ACCENTS[0].1), width: 64, height: 64 }))
        // a dialog has the frame dialogs have
        .with_decorations(is_picker || !app::starts_with_custom_titlebar(screenshot.is_some()))
        .with_drag_and_drop(true);
    // under the pointer, a little left of it, as if the tab were picked up by its title
    if let Some((x, y)) = at {
        viewport = viewport.with_position([x - 120.0, y - 16.0]);
    }
    let background = std::env::var_os("DALERIS_TEST_BACKGROUND").is_some();
    if background {
        // test runs while someone is using this PC: out of sight, and it never takes focus
        // a sliver stays on screen, as a window that's wholly off it isn't asked to paint
        let width = test_size().map_or(if is_picker { 920.0 } else { 1280.0 }, |s| s[0]);
        viewport = viewport.with_active(false).with_position([2.0 - width, 40.0]);
    }
    if is_picker && !background {
        // It takes the place of a window that would be owned by the app, which another process's can't be.
        viewport = viewport.with_always_on_top().with_active(true);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    let result = eframe::run_native("Daleris", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, wanted, query, screenshot, picker)))));
    match &result {
        Ok(()) => log::info!("exited"),
        Err(e) => log::error!("exited with an error: {e}"),
    }
    result
}

/// Test hook: DALERIS_TEST_SIZE=w,h for a window of that size.
fn test_size() -> Option<[f32; 2]> {
    let v = std::env::var("DALERIS_TEST_SIZE").ok()?;
    let (w, h) = v.split_once(',')?;
    Some([w.parse().ok()?, h.parse().ok()?])
}

/// What a command line without switches names. Some(exit code) if it's for File Explorer rather than this program.
fn read_named(named: &[OsString], wanted: &mut app::Wanted) -> Option<i32> {
    // a server that's down doesn't get to hold up the window: the window can say what's wrong
    let wait = std::time::Duration::from_millis(600);
    let is_dir = |p: &Path| matches!(target::kind_within(p, wait), target::PathKind::Dir | target::PathKind::Unknown);
    let exists = |p: &Path| target::kind_within(p, wait) != target::PathKind::Missing;
    match target::parse(named, is_dir, exists) {
        Target::Nothing => {}
        Target::Dir(p) => wanted.dir = Some(p),
        Target::Select(p) => {
            wanted.select = p.file_name().map(|n| n.to_string_lossy().into_owned());
            wanted.dir = p.parent().map(Path::to_path_buf);
        }
        Target::ThisPc => wanted.this_pc = true,
        #[cfg(windows)]
        Target::Shell(name) => match daleris_integration::explorer::resolve(&name) {
            Some(dir) => wanted.dir = Some(dir),
            // the Recycle Bin, Control Panel and so on: only Explorer can show those
            None => {
                daleris_integration::explorer::open_in_explorer(Some(std::ffi::OsStr::new(&name)));
                return Some(0);
            }
        },
        #[cfg(windows)]
        Target::Missing(m) => {
            daleris_integration::explorer::open_in_explorer(Some(&m));
            return Some(0);
        }
        // nothing else can open these on Linux or macOS, so it's just the usual window
        #[cfg(not(windows))]
        _ => {}
    }
    None
}

/// With "Open tabs in the running window" on, a window that's already there takes this.
#[cfg(windows)]
fn single_window_took(wanted: &app::Wanted) -> bool {
    use daleris_integration::pipe::{self, Request};
    use daleris_integration::reg::{policy_fixed, Hive};
    use daleris_integration::Feature;
    if policy_fixed(Feature::SingleWindow) == Some(false) || !Hive::real().flag(Feature::SingleWindow) {
        return false;
    }
    let request = match (&wanted.dir, &wanted.select) {
        (Some(dir), Some(name)) => Request::Select(dir.join(name).into_os_string()),
        (Some(dir), None) => Request::Open(dir.clone().into_os_string()),
        _ if wanted.this_pc => Request::ThisPc,
        _ => Request::Show,
    };
    pipe::send(&request)
}
