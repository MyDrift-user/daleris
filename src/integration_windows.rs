use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

use daleris_integration::pipe::{self, Request, Server};
use daleris_integration::reg::{self, policy_fixed, Hive, Support};
use daleris_integration::hookctl;
use windows::Win32::Foundation::*;
use windows::core::BOOL;
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Threading::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use super::{Feature, Live};
use crate::i18n::tr;

/// Copies the launcher and the DLLs from next to this program to the person's own folder. Windows then loads them from
/// there, so that a PC-wide install's folder can't block an upgrade by having a DLL loaded in someone's app, and an
/// uninstall can't leave any account pointing at a DLL that's gone. The version and contents in the name let a new copy go
/// next to an old one still loaded, even a rebuild of the same version.
pub fn install_support() -> Result<Support, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let from = exe.parent().unwrap_or(Path::new("."));
    let dir = reg::support_dir().ok_or(tr("There's no folder for this person's programs"))?;
    std::fs::create_dir_all(&dir).map_err(|e| trf!("Couldn't create {folder}: {error}", folder = dir.display(), error = e))?;
    let version = crate::update::CURRENT;
    let first = |names: &[&str]| names.iter().map(|n| from.join(n)).find(|p| p.is_file());
    let missing = tr("This copy of Daleris doesn't have the files for this next to it. Install it with the MSI or setup.exe.");
    let open_from = first(&["daleris-open.exe"]).ok_or(missing)?;
    let dll64_from = first(&["daleris-dialogs-x64.dll", "daleris_dialogs.dll"]).ok_or(missing)?;
    let dll32_from = first(&["daleris-dialogs-x86.dll"]);
    let copy = |src: &Path, stem: &str, ext: &str| -> Result<PathBuf, String> {
        use sha2::{Digest, Sha256};
        let bytes = std::fs::read(src).map_err(|e| trf!("Couldn't read {file}: {error}", file = src.display(), error = e))?;
        let hash: String = Sha256::digest(&bytes).iter().take(4).map(|b| format!("{b:02x}")).collect();
        let to = dir.join(format!("{stem}-{version}-{hash}.{ext}"));
        if !to.is_file() {
            std::fs::write(&to, &bytes).map_err(|e| trf!("Couldn't copy to {file}: {error}", file = to.display(), error = e))?;
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
        return Err(tr("Daleris is running as administrator, so it won't change what the signed-in person's Explorer does. Run it normally.").into());
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
                return Err(tr("Open and Save dialogs aren't available on ARM PCs yet: ARM apps couldn't load this DLL, so they'd lose their dialogs.").into());
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
    let value = format!("Started{}", agent.arg); // i18n-ignore: registry value name
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
        return if in_place { Live::Problem(tr("Your organization turned this off, and it's being removed.").into()) } else { Live::Off };
    }
    if !in_place {
        return Live::Off;
    }
    match feature {
        Feature::Folders => Live::Active(tr("Active. Folders from the desktop, Start and most apps open here. Apps that start explorer.exe themselves still open File Explorer.").into()),
        Feature::WinE => Live::Active(tr("Registered. Press Win + E to see whether your Windows build goes by it.").into()),
        Feature::WinEHook => {
            if hookctl::HOOK.is_running() {
                Live::Active(tr("Active. The hook is running, in a small process of its own.").into())
            } else {
                Live::Problem(tr("Set to start at sign-in, but it isn't running now.").into())
            }
        }
        Feature::Select => {
            if hive.is_in_place(Feature::Folders) {
                Live::Active(tr("Active. \"Show in folder\" selects the file when the app uses the Windows call for it.").into())
            } else {
                Live::Problem(tr("Needs \"Open folders in Daleris\" too, or Windows doesn't come to Daleris for this at all.").into())
            }
        }
        Feature::SingleWindow => {
            if pipe::is_up() {
                Live::Active(tr("Active. This window takes new folders as tabs.").into())
            } else {
                Live::Problem(tr("On, but no window is taking requests right now. The next Daleris to start will.").into())
            }
        }
        Feature::Dialogs => {
            let key = format!(r"Software\Classes\CLSID\{}\InprocServer32", reg::DIALOG_CLSIDS[0]);
            let key32 = format!(r"Software\Classes\WOW6432Node\CLSID\{}\InprocServer32", reg::DIALOG_CLSIDS[0]);
            let exists = |k: &str| hive.read_text(k, "").is_some_and(|p| Path::new(&p).is_file());
            match (exists(&key), exists(&key32)) {
                (true, true) => Live::Active(tr("Active for 64-bit and 32-bit apps that use the modern Windows dialogs. Browsers and other sandboxed apps keep Windows's own.").into()),
                (true, false) => Live::Active(tr("Active for 64-bit apps. 32-bit apps, browsers and other sandboxed apps keep Windows's own.").into()),
                _ => Live::Problem(tr("The DLL is missing, so apps keep the Windows dialog.").into()),
            }
        }
        Feature::DialogJump => {
            if !hookctl::JUMP.is_running() {
                Live::Problem(tr("Set to start at sign-in, but it isn't running now.").into())
            } else if hive.read_text(reg::OWN_KEY, reg::JUMP_PROBLEM).is_some_and(|p| !p.is_empty()) {
                // the watcher writes a code, "hotkey_taken", the only problem it has; an older one wrote the
                // sentence itself, which is about the same thing
                Live::Problem(tr("Another program has taken Ctrl + G, so it can't send file dialogs to the Daleris folder right now.").into())
            } else {
                Live::Active(tr("Active. Press Ctrl + G in a Windows file dialog while Daleris shows the folder you want.").into())
            }
        }
        Feature::DialogLog => match reg::logs_dir() {
            Some(dir) => Live::Active(trf!("Writing to {file}", file = dir.join(reg::DIALOG_LOG).display())),
            None => Live::Problem(tr("There's no folder to write it to.").into()),
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
    let _ = std::thread::Builder::new().name("single window".into()).spawn(move || { // i18n-ignore: thread name
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
    unsafe extern "system" fn each(hwnd: HWND, _: LPARAM) -> BOOL { unsafe {
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
    }}
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

/// Tells the Ctrl + G watcher which language to say things in: "en", "de" or "it". Written only when it changes.
pub fn set_language(code: &str) {
    let hive = Hive::real();
    if hive.read_text(reg::OWN_KEY, reg::LANGUAGE).as_deref() != Some(code) {
        hive.write_text(reg::OWN_KEY, reg::LANGUAGE, code);
    }
}

pub fn clear_jump_folder() {
    let hive = Hive::real();
    if hive.read_text(reg::OWN_KEY, reg::JUMP_PID).as_deref() == Some(std::process::id().to_string().as_str()) {
        hive.delete_value(reg::OWN_KEY, reg::JUMP_FOLDER);
        hive.delete_value(reg::OWN_KEY, reg::JUMP_PID);
    }
}

// ---------------------------------------------------------------- tabs between windows

/// This window's own pipe, where another Daleris window hands over a tab dragged onto this one. Runs for the life of
/// the process. None if it couldn't start, such as when running as administrator, as the shared pipe doesn't either.
pub fn serve_window_requests(ctx: eframe::egui::Context) -> Option<Receiver<Request>> {
    if is_elevated() {
        return None;
    }
    let server = Server::start_for_window()?;
    let (tx, rx) = channel();
    let _ = std::thread::Builder::new().name("tab handover".into()).spawn(move || { // i18n-ignore: thread name
        server.run(
            || false,
            |request| {
                let _ = tx.send(request);
                focus_this_window();
                ctx.request_repaint();
            },
        );
    });
    Some(rx)
}

/// Where a tab let go of outside the tab strip lands.
pub enum TabLanding {
    /// still over this window
    Here,
    /// over another Daleris window, of this process id
    Window(u32),
    /// anywhere else, at this point on the screen in pixels
    Desktop(i32, i32),
}

/// What's under the pointer right now, asked of Windows: the window's own coordinates don't reach past its edges.
pub fn tab_landing() -> TabLanding {
    let mut at = POINT::default();
    if unsafe { GetCursorPos(&mut at) }.is_err() {
        return TabLanding::Here;
    }
    let hwnd = unsafe { WindowFromPoint(at) };
    let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(root, Some(&mut pid)) };
    if pid == std::process::id() {
        return TabLanding::Here;
    }
    if pid != 0 && is_daleris(pid) {
        return TabLanding::Window(pid);
    }
    TabLanding::Desktop(at.x, at.y)
}

/// Whether process `pid` is a Daleris window, by its program's name.
fn is_daleris(pid: u32) -> bool {
    let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else { return false };
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len) }.is_ok();
    let _ = unsafe { CloseHandle(process) };
    ok && Path::new(&String::from_utf16_lossy(&buf[..len as usize])).file_name().is_some_and(|n| n.eq_ignore_ascii_case("daleris.exe"))
}

/// Hands a tab to the window of process `pid`, letting it come to the front. True if it took it.
pub fn hand_tab_to(pid: u32, request: &Request) -> bool {
    let _ = unsafe { AllowSetForegroundWindow(pid) };
    pipe::send_to_window(pid, request)
}
