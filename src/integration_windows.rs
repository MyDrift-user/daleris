use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

use daleris_integration::pipe::{self, Request, Server};
use daleris_integration::reg::{self, policy_fixed, Hive, Support};
use daleris_integration::hookctl;
use windows::Win32::Foundation::*;
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Threading::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use super::{Feature, Live};

/// Copies the launcher and the DLLs from next to this program to the person's own folder. Windows then loads them from
/// there, so that a PC-wide install's folder can't block an upgrade by having a DLL loaded in someone's app, and an
/// uninstall can't leave any account pointing at a DLL that's gone. The version and contents in the name let a new copy go
/// next to an old one still loaded, even a rebuild of the same version.
pub fn install_support() -> Result<Support, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let from = exe.parent().unwrap_or(Path::new("."));
    let dir = reg::support_dir().ok_or("There's no folder for this person's programs")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    let version = crate::update::CURRENT;
    let first = |names: &[&str]| names.iter().map(|n| from.join(n)).find(|p| p.is_file());
    let missing = "This copy of Daleris doesn't have the files for this next to it. Install it with the MSI or setup.exe.";
    let open_from = first(&["daleris-open.exe"]).ok_or(missing)?;
    let dll64_from = first(&["daleris-dialogs-x64.dll", "daleris_dialogs.dll"]).ok_or(missing)?;
    let dll32_from = first(&["daleris-dialogs-x86.dll"]);
    let copy = |src: &Path, stem: &str, ext: &str| -> Result<PathBuf, String> {
        use sha2::{Digest, Sha256};
        let bytes = std::fs::read(src).map_err(|e| format!("Couldn't read {}: {e}", src.display()))?;
        let hash: String = Sha256::digest(&bytes).iter().take(4).map(|b| format!("{b:02x}")).collect();
        let to = dir.join(format!("{stem}-{version}-{hash}.{ext}"));
        if !to.is_file() {
            std::fs::write(&to, &bytes).map_err(|e| format!("Couldn't copy to {}: {e}", to.display()))?;
        }
        Ok(to)
    };
    let launcher = copy(&open_from, "daleris-open", "exe")?;
    let dll64 = copy(&dll64_from, "daleris-dialogs-x64", "dll")?;
    let dll32 = match dll32_from {
        Some(src) => Some(copy(&src, "daleris-dialogs-x86", "dll")?),
        None => None,
    };
    let support = Support { launcher, dll64, dll32 };
    remove_old_copies(&dir, &support);
    Ok(support)
}

/// Older versions' files, unless an app still has one loaded.
fn remove_old_copies(dir: &Path, keep: &Support) {
    let Ok(list) = std::fs::read_dir(dir) else { return };
    for entry in list.flatten() {
        let p = entry.path();
        if p != keep.launcher && p != keep.dll64 && Some(&p) != keep.dll32.as_ref() {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// Makes the system match the switches: `wanted` is every switch as Settings holds it, after policy.
pub fn sync(wanted: &[(Feature, bool)], exclusions: &str) -> Result<(), String> {
    let hive = Hive::real();
    if wanted.iter().all(|(_, on)| !on) {
        if hive.key_exists(reg::OWN_KEY) || reg::support_dir().is_some_and(|d| d.exists()) {
            restore();
        }
        return Ok(());
    }
    if is_elevated() {
        return Err("Daleris is running as administrator, so it won't change what the signed-in person's Explorer does. Run it normally.".into());
    }
    // a removal still waiting for the next sign-in would take these files away again
    reg::clear_cleanup_note(&hive);
    let support = install_support()?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    hive.set_program_path(&exe);
    hive.set_exclusions(exclusions);
    for &(feature, on) in wanted {
        if on {
            if feature == Feature::Dialogs && is_arm64() {
                hive.remove(feature);
                return Err("Open and Save dialogs aren't available on ARM PCs yet: ARM apps couldn't load this DLL, so they'd lose their dialogs.".into());
            }
            hive.apply(feature, &support)?;
        } else {
            hive.remove(feature);
        }
    }
    let hook_wanted = wanted.iter().any(|(f, on)| *f == Feature::WinEHook && *on);
    keep_agent(&hive, hookctl::HOOK, hook_wanted, &support.launcher);
    let jump_wanted = wanted.iter().any(|(f, on)| *f == Feature::DialogJump && *on);
    keep_agent(&hive, hookctl::JUMP, jump_wanted, &support.launcher);
    Ok(())
}

/// Stops or starts something that stays running. One from an older launcher is replaced, so that an update takes effect.
fn keep_agent(hive: &Hive, agent: hookctl::Agent, wanted: bool, launcher: &Path) {
    let value = format!("Started{}", agent.arg);
    let path = launcher.display().to_string();
    if !wanted || hive.read_text(reg::OWN_KEY, &value).as_deref() != Some(path.as_str()) {
        agent.stop();
    }
    if !wanted {
        hive.delete_value(reg::OWN_KEY, &value);
    } else if !agent.is_running() && std::process::Command::new(launcher).arg(agent.arg).spawn().is_ok() {
        hive.write_text(reg::OWN_KEY, &value, &path);
    }
}

/// `--restore-explorer`, the button in Settings, and uninstalling: everything Daleris wrote goes, and only that.
pub fn restore() {
    let hive = Hive::real();
    hookctl::stop_all();
    hive.restore_all();
    reg::clear_cleanup_note(&hive);
    reg::remove_support_files(&hive, false);
}

/// Asked every frame, and it can't change while the program runs.
pub fn is_elevated() -> bool {
    static ELEVATED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ELEVATED.get_or_init(token_is_elevated)
}

fn token_is_elevated() -> bool {
    let mut token = HANDLE::default();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.is_err() {
        return false;
    }
    let mut value = TOKEN_ELEVATION::default();
    let mut len = 0;
    let got = unsafe { GetTokenInformation(token, TokenElevation, Some(&mut value as *mut _ as *mut _), std::mem::size_of::<TOKEN_ELEVATION>() as u32, &mut len) };
    let _ = unsafe { CloseHandle(token) };
    got.is_ok() && value.TokenIsElevated != 0
}

fn is_arm64() -> bool {
    let (mut process, mut native) = (windows::Win32::System::SystemInformation::IMAGE_FILE_MACHINE::default(), windows::Win32::System::SystemInformation::IMAGE_FILE_MACHINE::default());
    unsafe { IsWow64Process2(GetCurrentProcess(), &mut process, Some(&mut native)) }.is_ok() && native == windows::Win32::System::SystemInformation::IMAGE_FILE_MACHINE_ARM64
}

/// What a switch is really doing right now, read back from the system rather than from Settings.
pub fn status(feature: Feature) -> Live {
    let hive = Hive::real();
    let in_place = hive.is_in_place(feature);
    let flag_off_by_policy = policy_fixed(feature) == Some(false);
    if flag_off_by_policy {
        return if in_place { Live::Problem("Your organization turned this off, and it's being removed.".into()) } else { Live::Off };
    }
    if !in_place {
        return Live::Off;
    }
    match feature {
        Feature::Folders => Live::Active("Active. Folders from the desktop, Start and most apps open here. Apps that start explorer.exe themselves still open File Explorer.".into()),
        Feature::WinE => Live::Active("Registered. Press Win + E to see whether your Windows build goes by it.".into()),
        Feature::WinEHook => {
            if hookctl::HOOK.is_running() {
                Live::Active("Active. The hook is running, in a small process of its own.".into())
            } else {
                Live::Problem("Set to start at sign-in, but it isn't running now.".into())
            }
        }
        Feature::Select => {
            if hive.is_in_place(Feature::Folders) {
                Live::Active("Active. \"Show in folder\" selects the file when the app uses the Windows call for it.".into())
            } else {
                Live::Problem("Needs \"Open folders in Daleris\" too, or Windows doesn't come to Daleris for this at all.".into())
            }
        }
        Feature::SingleWindow => {
            if pipe::is_up() {
                Live::Active("Active. This window takes new folders as tabs.".into())
            } else {
                Live::Problem("On, but no window is taking requests right now. The next Daleris to start will.".into())
            }
        }
        Feature::Dialogs => {
            let key = format!(r"Software\Classes\CLSID\{}\InprocServer32", reg::DIALOG_CLSIDS[0]);
            let key32 = format!(r"Software\Classes\WOW6432Node\CLSID\{}\InprocServer32", reg::DIALOG_CLSIDS[0]);
            let exists = |k: &str| hive.read_text(k, "").is_some_and(|p| Path::new(&p).is_file());
            match (exists(&key), exists(&key32)) {
                (true, true) => Live::Active("Active for 64-bit and 32-bit apps that use the modern Windows dialogs.".into()),
                (true, false) => Live::Active("Active for 64-bit apps. 32-bit apps keep the Windows dialog.".into()),
                _ => Live::Problem("The DLL is missing, so apps keep the Windows dialog.".into()),
            }
        }
        Feature::DialogJump => {
            if !hookctl::JUMP.is_running() {
                Live::Problem("Set to start at sign-in, but it isn't running now.".into())
            } else if let Some(problem) = hive.read_text(reg::OWN_KEY, reg::JUMP_PROBLEM).filter(|p| !p.is_empty()) {
                Live::Problem(problem)
            } else {
                Live::Active("Active. Press Ctrl + G in a Windows file dialog while Daleris shows the folder you want.".into())
            }
        }
        Feature::DialogLog => match reg::logs_dir() {
            Some(dir) => Live::Active(format!("Writing to {}", dir.join(reg::DIALOG_LOG).display())),
            None => Live::Problem("There's no folder to write it to.".into()),
        },
    }
}

// ---------------------------------------------------------------- new folders as tabs in the running window

/// Takes requests from new launches, if no other window does already. Stops when `running` goes false.
pub fn serve_requests(ctx: eframe::egui::Context, running: Arc<AtomicBool>) -> Option<Receiver<Request>> {
    if is_elevated() {
        return None;
    }
    let server = Server::start()?;
    let (tx, rx) = channel();
    let _ = std::thread::Builder::new().name("single window".into()).spawn(move || {
        let stop = running.clone();
        server.run(
            move || !stop.load(Ordering::SeqCst),
            |request| {
                let _ = tx.send(request);
                focus_this_window();
                ctx.request_repaint();
            },
        );
    });
    Some(rx)
}

/// Stops `serve_requests`, once `running` has gone false.
pub fn stop_serving() {
    pipe::wake();
}

/// Comes to the front on its own thread, as the window's own thread may be waiting for a frame.
fn focus_this_window() {
    unsafe extern "system" fn each(hwnd: HWND, _: LPARAM) -> BOOL {
        let mut owner = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut owner));
        if owner == GetCurrentProcessId() && IsWindowVisible(hwnd).as_bool() {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(hwnd);
            return false.into();
        }
        true.into()
    }
    let _ = unsafe { EnumWindows(Some(each), LPARAM(0)) };
}

// ---------------------------------------------------------------- Ctrl + G in a Windows file dialog

/// Tells the Ctrl + G watcher, a process of its own, which folder this window shows. See `jump.rs` in the launcher.
/// None when the tab in front isn't a folder, and `clear_jump_folder` when the window goes.
pub fn set_jump_folder(folder: Option<&Path>) {
    let hive = Hive::real();
    hive.write_text(reg::OWN_KEY, reg::JUMP_FOLDER, &folder.map_or(String::new(), |p| p.display().to_string()));
    hive.write_text(reg::OWN_KEY, reg::JUMP_PID, &std::process::id().to_string());
}

/// Where the file dialog DLL writes what it decided.
pub fn dialog_log_file() -> Option<PathBuf> {
    reg::logs_dir().map(|d| d.join(reg::DIALOG_LOG))
}

pub fn clear_jump_folder() {
    let hive = Hive::real();
    if hive.read_text(reg::OWN_KEY, reg::JUMP_PID).as_deref() == Some(std::process::id().to_string().as_str()) {
        hive.delete_value(reg::OWN_KEY, reg::JUMP_FOLDER);
        hive.delete_value(reg::OWN_KEY, reg::JUMP_PID);
    }
}
