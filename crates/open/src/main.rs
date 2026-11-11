//! `daleris-open`: what Windows runs for a folder or for Win + E once those switches are on.
//!
//! It stays small because it has one job that must never fail: get the person to a folder. If Daleris is gone, or a
//! policy turned the switch off, it takes our registry entries away and opens File Explorer, so an uninstall can't
//! leave folders that won't open, for this person or for anyone else on the PC.
//!
//! - `--folder "%1"`: the Folder class's verbs.
//! - `--win-e`: the registry route for Win + E.
//! - `--win-e-press`: the keyboard hook saw Win + E.
//! - `--win-e-hook`: stays running with that hook. See `hook.rs`.
//! - `--dialog-jump`: stays running for Ctrl + G in file dialogs. See `jump.rs`.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
use daleris_integration::target::{kind_within, PathKind};
#[cfg(windows)]
const LOOK_FOR: std::time::Duration = std::time::Duration::from_millis(1500);

#[cfg(windows)]
mod hook;
#[cfg(windows)]
mod jump;
#[cfg(windows)]
mod shellwin;

/// Text in the language Daleris shows, as it last wrote it to the registry ("en", "de" or "it"); before it has, the
/// language Windows is shown in. Read every time, as this can run for days while the setting changes. The launcher
/// can't use the app's catalogs, so each text carries its own translations.
#[cfg(windows)]
fn three(en: &'static str, de: &'static str, it: &'static str) -> &'static str {
    use daleris_integration::reg::{self, Hive};
    let code = Hive::real().read_text(reg::OWN_KEY, reg::LANGUAGE).unwrap_or_default();
    let code = match code.as_str() {
        "en" | "de" | "it" => code,
        // the primary language of Windows's display language
        _ => match unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() } & 0x3ff {
            0x07 => "de".to_string(),
            0x10 => "it".to_string(),
            _ => "en".to_string(),
        },
    };
    match code.as_str() {
        "de" => de,
        "it" => it,
        _ => en,
    }
}

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn main() {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    run(args);
}

#[cfg(windows)]
fn run(args: Vec<std::ffi::OsString>) {
    use daleris_integration::Feature;
    let first = args.first().and_then(|a| a.to_str()).unwrap_or("").to_string();
    match first.as_str() {
        "--win-e-hook" => hook::run(),
        "--dialog-jump" => jump::run(),
        "--folder" => open(&args[1..], Feature::Folders),
        "--win-e" => open(&[], Feature::WinE),
        "--win-e-press" => open(&[], Feature::WinEHook),
        _ => open(&args, Feature::Folders),
    }
}

#[cfg(windows)]
fn open(args: &[std::ffi::OsString], feature: daleris_integration::Feature) {
    use daleris_integration::explorer::{open_in_explorer, resolve, select_in_explorer};
    use daleris_integration::pipe::Request;
    use daleris_integration::reg::{self, policy_fixed, Hive};
    use daleris_integration::target::{parse, Target};
    use daleris_integration::Feature;
    use std::ffi::{OsStr, OsString};
    use std::path::Path;

    let hive = Hive::real();
    let original: Option<OsString> = args.first().cloned();
    // Never wait long for a path: this runs in front of whatever is opening it. No answer may well be a server that's down, which Daleris can say more about than Explorer can.
    let target = parse(args, |p: &Path| matches!(kind_within(p, LOOK_FOR), PathKind::Dir | PathKind::Unknown), |p: &Path| kind_within(p, LOOK_FOR) != PathKind::Missing);
    let to_explorer = |t: &Target| match t {
        Target::Select(p) => select_in_explorer(p.as_os_str()),
        Target::Dir(p) => open_in_explorer(Some(p.as_os_str())),
        Target::ThisPc | Target::Nothing => open_in_explorer(None),
        Target::Shell(s) => open_in_explorer(Some(OsStr::new(s))),
        Target::Missing(m) => open_in_explorer(Some(m)),
    };
    // Daleris was uninstalled, or moved: put everything back for this person, and carry on with Explorer.
    let Some(exe) = hive.program_path().filter(|p| p.is_file()) else {
        daleris_integration::hookctl::stop_all();
        hive.restore_all();
        reg::remove_support_files(&hive, true);
        to_explorer(&target);
        return;
    };
    // an administrator turned this off since Daleris last ran
    if policy_fixed(feature) == Some(false) {
        hive.remove(feature);
        to_explorer(&target);
        return;
    }
    let launch = |request: &Request| {
        // Daleris itself decides whether starting it again opens a window or a tab, by a setting of its own
        let show = matches!(request, Request::Show);
        if !show && hive.flag(Feature::SingleWindow) && daleris_integration::pipe::send(request) {
            return true;
        }
        let mut c = std::process::Command::new(&exe);
        if !show {
            c.arg("--new-window");
        }
        match request {
            Request::Show => {}
            Request::ThisPc => {
                c.arg("--this-pc");
            }
            Request::Open(p) => {
                c.arg("--path").arg(p);
            }
            Request::Select(p) => {
                c.arg("--select").arg(p);
            }
        }
        // the new window may take the front, as this process was allowed to
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(windows::Win32::UI::WindowsAndMessaging::ASFW_ANY);
        }
        c.spawn().is_ok()
    };
    let opened = match &target {
        Target::Nothing => launch(&Request::Show),
        Target::ThisPc => launch(&Request::ThisPc),
        Target::Select(p) => launch(&Request::Select(p.clone().into_os_string())),
        Target::Dir(p) => {
            if matches!(feature, Feature::Folders) {
                // Always there for "Show in folder", switch or no switch: with no shell window to find, that call makes
                // the app wait 20 seconds and fail. The switch decides whether its file gets selected.
                let wants_it = hive.flag(Feature::Select) && policy_fixed(Feature::Select) != Some(false);
                shellwin::open_watching_for_selection(p, |selected| match selected.filter(|_| wants_it) {
                    Some(item) => launch(&Request::Select(item.into_os_string())),
                    None => launch(&Request::Open(p.clone().into_os_string())),
                })
            } else {
                launch(&Request::Open(p.clone().into_os_string()))
            }
        }
        // a phone or camera: Daleris shows those itself
        Target::Shell(name) if daleris_integration::target::is_device(name) => launch(&Request::Open(name.strip_prefix("shell:").unwrap_or(name).into())),
        Target::Shell(name) => match resolve(name) {
            Some(dir) => launch(&Request::Open(dir.into_os_string())),
            // the Recycle Bin, Control Panel and so on: only Explorer can show those
            None => to_explorer(&target),
        },
        Target::Missing(_) => to_explorer(&target),
    };
    // Whatever went wrong, someone clicked a folder and should get one.
    if !opened {
        match original {
            Some(o) => {
                open_in_explorer(Some(&o));
            }
            None => {
                open_in_explorer(None);
            }
        }
    }
}
