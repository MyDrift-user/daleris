#![windows_subsystem = "windows"]

mod app;
mod dirsize;
mod fscommon;
#[cfg(windows)]
mod fsx;
#[cfg(not(windows))]
#[path = "fsx_unix.rs"]
mod fsx;
mod logging;
mod policy;
mod search;
#[cfg(windows)]
mod shell;
#[cfg(not(windows))]
#[path = "shell_unix.rs"]
mod shell;
mod theme;
mod util;

use std::path::PathBuf;

fn main() -> eframe::Result<()> {
    logging::init();
    util::adopt_old_settings();
    // The same ID as the installers' shortcuts, so the taskbar treats the window and shortcut as one app.
    #[cfg(windows)]
    unsafe {
        let _ = windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(windows::core::w!("MyDrift.Daleris"));
    }
    let mut screenshot: Option<PathBuf> = None;
    let mut start: Option<PathBuf> = None;
    let mut query: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--screenshot" => screenshot = args.next().map(PathBuf::from),
            "--path" => start = args.next().map(PathBuf::from),
            "--query" => query = args.next(),
            "--icons" => std::env::set_var("DALERIS_VIEW", "icons"),
            "--props" => std::env::set_var("DALERIS_TEST_PROPS", args.next().unwrap_or_default()),
            "--native-title" =>std::env::set_var("DALERIS_TEST_NATIVE_TITLE", "1"),
            "--settings" => std::env::set_var("DALERIS_PAGE", args.next().unwrap_or_default()),
            other if !other.starts_with("--") && start.is_none() => {
                let p = PathBuf::from(other);
                if p.is_dir() {
                    start = Some(p);
                }
            }
            _ => {}
        }
    }
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Daleris")
            // matches the .desktop file, which is how Wayland desktops find the icon
            .with_app_id("daleris")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([720.0, 420.0])
            .with_icon(std::sync::Arc::new(eframe::egui::IconData { rgba: theme::logo_rgba(64, theme::ACCENTS[0].1), width: 64, height: 64 }))
            .with_decorations(!app::starts_with_custom_titlebar(screenshot.is_some()))
            .with_drag_and_drop(true),
        vsync: true,
        ..Default::default()
    };
    let result = eframe::run_native("Daleris", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, start, query, screenshot)))));
    match &result {
        Ok(()) => log::info!("exited"),
        Err(e) => log::error!("exited with an error: {e}"),
    }
    result
}
