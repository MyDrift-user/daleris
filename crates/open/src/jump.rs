//! Ctrl + G: sends a Windows file dialog to the folder Daleris shows.
//!
//! A process of its own that stays running from sign-in, so that the key does something visible even with no Daleris
//! window open. It only looks at which window is in front, from outside: nothing loads into any other app, and it holds
//! Ctrl + G only while a file dialog is in front, so that the key means what it always did everywhere else.
//!
//! What's wrong goes in `%LOCALAPPDATA%\Daleris\logs\dialog-jump.log`, and to the person if they pressed the key.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use daleris_integration::hookctl::JUMP;
use daleris_integration::reg::{self, policy_fixed, Hive};
use daleris_integration::{deny, Feature};
use crate::three;
use windows::core::{BOOL, w, PCWSTR, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const HOTKEY_ID: i32 = 0x4441;
/// How long a dialog gets to go to the folder before this says it didn't.
const NAVIGATE_WAIT: Duration = Duration::from_millis(2000);
const LOG_LIMIT: u64 = 512 * 1024;

/// Control IDs of the box a file name goes in: the Open dialog's combo box, the Save dialog's edit box (deep inside its
/// view, so not a direct child), and the edit box of folder pickers and older dialogs.
const OPEN_NAME_BOX: i32 = 1148;
const SAVE_NAME_BOX: i32 = 1001;
const FOLDER_NAME_BOX: i32 = 1152;

fn log(what: &str) {
    let Some(dir) = reg::logs_dir() else { return };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("dialog-jump.log"); // i18n-ignore: file name
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > LOG_LIMIT) {
        let _ = std::fs::rename(&path, dir.join("dialog-jump.old.log")); // i18n-ignore: file name
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = std::io::Write::write_all(&mut f, format!("{now} {what}\n").as_bytes());
    }
}

/// Tells the person, who pressed a key and would otherwise see nothing happen.
fn say(text: &str) {
    log(text);
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe { MessageBoxW(None, PCWSTR(wide.as_ptr()), w!("Daleris"), MB_OK | MB_ICONINFORMATION | MB_TOPMOST | MB_SETFOREGROUND) }; // i18n-ignore: the app's name
}

pub fn run() {
    let hive = Hive::real();
    if policy_fixed(Feature::DialogJump) == Some(false) || !hive.is_in_place(Feature::DialogJump) {
        return;
    }
    let Ok(mutex) = (unsafe { CreateMutexW(None, true, JUMP.running) }) else { return };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        return;
    }
    let Ok(stop) = (unsafe { CreateEventW(None, true, false, JUMP.stop) }) else { return };
    log("started");
    hive.delete_value(reg::OWN_KEY, reg::JUMP_PROBLEM);
    let mut watcher = Watcher::default();
    let mut msg = MSG::default();
    loop {
        // a dialog's controls can appear a moment after the dialog does, so this looks again rather than wait for an event
        if unsafe { MsgWaitForMultipleObjects(Some(&[stop]), false, 250, QS_ALLINPUT) } == WAIT_OBJECT_0 {
            break;
        }
        let mut quit = false;
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            match msg.message {
                WM_QUIT => quit = true,
                WM_HOTKEY if msg.wParam.0 as i32 == HOTKEY_ID => {
                    let finished = std::panic::catch_unwind(jump);
                    if finished.is_err() {
                        log("the jump panicked"); // i18n-ignore: log only
                    }
                }
                _ => {}
            }
            unsafe { DispatchMessageW(&msg) };
        }
        if quit {
            break;
        }
        watcher.look();
        if policy_fixed(Feature::DialogJump) == Some(false) || !hive.is_in_place(Feature::DialogJump) {
            log("switched off"); // i18n-ignore: log only
            break;
        }
    }
    let _ = unsafe { UnregisterHotKey(None, HOTKEY_ID) };
    log("stopped");
    let _ = unsafe { ReleaseMutex(mutex) };
    let _ = unsafe { CloseHandle(mutex) };
}

/// What's written to `reg::JUMP_PROBLEM` when another program holds Ctrl + G; Daleris shows its own sentence for it.
const HOTKEY_TAKEN: &str = "hotkey_taken";

/// Holds the hotkey while a file dialog is in front.
#[derive(Default)]
struct Watcher {
    in_front: isize,
    registered: bool,
    told_taken: bool,
    told_no_box: bool,
}

impl Watcher {
    fn look(&mut self) {
        let hwnd = unsafe { GetForegroundWindow() };
        // a message of ours is in front while the key's being handled, and that isn't a change
        let same = hwnd.0 as isize == self.in_front;
        if same && (self.registered || !looks_like_a_dialog(hwnd)) {
            return;
        }
        if !same {
            self.in_front = hwnd.0 as isize;
            self.told_no_box = false;
        }
        let wanted = is_file_dialog(hwnd) && file_name_box(hwnd).is_some() && app_may_be_touched(hwnd);
        if wanted && !self.registered {
            match unsafe { RegisterHotKey(None, HOTKEY_ID, MOD_CONTROL | MOD_NOREPEAT, VK_G.0 as u32) } {
                Ok(()) => {
                    self.registered = true;
                    Hive::real().delete_value(reg::OWN_KEY, reg::JUMP_PROBLEM);
                }
                Err(e) => {
                    log(&format!("RegisterHotKey failed: {e}")); // i18n-ignore: log only
                    Hive::real().write_text(reg::OWN_KEY, reg::JUMP_PROBLEM, HOTKEY_TAKEN);
                    if !self.told_taken {
                        self.told_taken = true;
                        say(three(
                            "Another program has taken Ctrl + G, so it can't send file dialogs to the Daleris folder right now.",
                            "Ein anderes Programm verwendet bereits Strg + G, daher können Dateidialoge gerade nicht zum Daleris-Ordner geschickt werden.",
                            "Un altro programma usa già Ctrl + G, quindi al momento non è possibile portare le finestre di dialogo dei file alla cartella di Daleris.",
                        ));
                    }
                }
            }
        } else if !wanted && self.registered {
            let _ = unsafe { UnregisterHotKey(None, HOTKEY_ID) };
            self.registered = false;
        } else if !wanted && is_file_dialog(hwnd) && file_name_box(hwnd).is_none() && !self.told_no_box {
            self.told_no_box = true;
            log(&format!("a file dialog with no name box found: {}", tree_of(hwnd))); // i18n-ignore: log only
        }
    }
}

fn class_of(hwnd: HWND) -> String {
    let mut class = [0u16; 64];
    let n = unsafe { GetClassNameW(hwnd, &mut class) } as usize;
    String::from_utf16_lossy(&class[..n])
}

fn looks_like_a_dialog(hwnd: HWND) -> bool {
    class_of(hwnd) == "#32770"
}

fn descendants(parent: HWND) -> Vec<HWND> {
    unsafe extern "system" fn each(hwnd: HWND, data: LPARAM) -> BOOL { unsafe {
        (*(data.0 as *mut Vec<HWND>)).push(hwnd);
        true.into()
    }}
    let mut list: Vec<HWND> = Vec::new();
    let _ = unsafe { EnumChildWindows(Some(parent), Some(each), LPARAM(&mut list as *mut Vec<HWND> as isize)) };
    list
}

/// A dialog with a view of files in it, and not some other dialog that has an edit box with the same number.
fn is_file_dialog(hwnd: HWND) -> bool {
    looks_like_a_dialog(hwnd) && descendants(hwnd).into_iter().any(|h| matches!(class_of(h).as_str(), "DUIViewWndClassName" | "SHELLDLL_DefView"))
}

/// The address bar and the search box have edit boxes of their own, which are never the one.
fn inside_a_bar(mut hwnd: HWND, dialog: HWND) -> bool {
    while hwnd != dialog {
        if matches!(class_of(hwnd).as_str(), "Address Band Root" | "UniversalSearchBand" | "Search Box" | "SearchEditBoxWrapperClass" | "ReBarWindow32") { // i18n-ignore: window class names
            return true;
        }
        match unsafe { GetParent(hwnd) } {
            Ok(p) if !p.0.is_null() => hwnd = p,
            _ => return true,
        }
    }
    false
}

/// The box a file name goes in, wherever this kind of dialog keeps it.
fn file_name_box(dialog: HWND) -> Option<HWND> {
    let edits: Vec<HWND> = descendants(dialog).into_iter().filter(|&h| class_of(h) == "Edit" && !inside_a_bar(h, dialog)).collect(); // i18n-ignore: window class name
    let id = |h: HWND| unsafe { GetDlgCtrlID(h) };
    let parent_class = |h: HWND| unsafe { GetParent(h) }.map(class_of).unwrap_or_default();
    // Each is its own kind of dialog, and any of them may have an edit box the app added with a number of its own.
    let open = edits.iter().find(|&&h| id(h) == OPEN_NAME_BOX);
    let save = edits.iter().find(|&&h| id(h) == SAVE_NAME_BOX && parent_class(h) == "ComboBox");
    let folder = edits.iter().find(|&&h| id(h) == FOLDER_NAME_BOX);
    open.or(save).or(folder).copied()
}

/// For the log, when one can't be found: this is what would say why on another Windows version.
fn tree_of(dialog: HWND) -> String {
    descendants(dialog).into_iter().filter(|&h| matches!(class_of(h).as_str(), "Edit" | "ComboBox" | "ComboBoxEx32")).map(|h| format!("{}#{}", class_of(h), unsafe { GetDlgCtrlID(h) })).collect::<Vec<_>>().join(" ") // i18n-ignore: window class names
}

/// Only messages go to the dialog, but games and security software aren't sent even those.
fn app_may_be_touched(hwnd: HWND) -> bool {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else { return false };
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    let got = unsafe { QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len) };
    let _ = unsafe { CloseHandle(process) };
    got.is_ok() && !matches!(deny::reason_to_leave_alone(&String::from_utf16_lossy(&buf[..len as usize]), ""), Some("built-in list" | "a game's or store's folder")) // i18n-ignore: reasons the deny list gives, compared
}

/// Every message has a time limit, as the app may not be responding.
fn send(window: HWND, msg: u32, wparam: usize, lparam: isize) -> Option<usize> {
    let mut result = 0;
    let rc = unsafe { SendMessageTimeoutW(window, msg, WPARAM(wparam), LPARAM(lparam), SMTO_ABORTIFHUNG | SMTO_NORMAL, 500, Some(&mut result)) };
    (rc.0 != 0).then_some(result)
}

fn text_of(window: HWND) -> Option<Vec<u16>> {
    let mut buf = vec![0u16; 2048];
    let n = send(window, WM_GETTEXT, buf.len(), buf.as_mut_ptr() as isize)?.min(buf.len());
    buf.truncate(n);
    Some(buf)
}

fn set_text(window: HWND, text: &[u16]) -> bool {
    let mut z = text.to_vec();
    z.push(0);
    send(window, WM_SETTEXT, 0, z.as_ptr() as isize).is_some()
}

/// What the address bar reads, such as `Address: C:\Windows`, which says where the dialog is now.
fn address_of(dialog: HWND) -> Option<String> {
    let bar = descendants(dialog).into_iter().find(|&h| class_of(h) == "ToolbarWindow32" && unsafe { GetParent(h) }.is_ok_and(|p| class_of(p) == "Breadcrumb Parent"))?; // i18n-ignore: window class names
    Some(String::from_utf16_lossy(&text_of(bar)?))
}

fn is_at(dialog: HWND, folder: &Path) -> bool {
    let want = folder.display().to_string().trim_end_matches('\\').to_lowercase();
    address_of(dialog).is_some_and(|a| a.trim_end_matches('\\').to_lowercase().ends_with(&want))
}

/// The folder the Daleris window in front shows, or what to tell the person instead.
fn target() -> Result<PathBuf, &'static str> {
    let hive = Hive::real();
    let pid = hive.read_text(reg::OWN_KEY, reg::JUMP_PID).and_then(|p| p.parse::<u32>().ok());
    let open = pid.is_some_and(|pid| {
        let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else { return false };
        let mut code = 0;
        let alive = unsafe { GetExitCodeProcess(process, &mut code) }.is_ok() && code == STILL_ACTIVE.0 as u32;
        let _ = unsafe { CloseHandle(process) };
        alive
    });
    if !open {
        return Err(three(
            "Ctrl + G sends this dialog to the folder Daleris is showing, but Daleris isn't open. Open it at the folder you want, then press Ctrl + G here again.",
            "Strg + G wechselt in diesem Dialog zu dem Ordner, den Daleris anzeigt, aber Daleris ist nicht geöffnet. Öffnen Sie Daleris im gewünschten Ordner, und drücken Sie dann hier erneut Strg + G.",
            "Ctrl + G porta questa finestra di dialogo alla cartella mostrata da Daleris, ma Daleris non è aperto. Aprilo nella cartella che vuoi, poi premi di nuovo Ctrl + G qui.",
        ));
    }
    let folder = hive.read_text(reg::OWN_KEY, reg::JUMP_FOLDER).unwrap_or_default();
    if folder.is_empty() {
        return Err(three(
            "Ctrl + G sends this dialog to the folder Daleris is showing, but the tab in front in Daleris isn't a folder. Go to a folder there, then press Ctrl + G here again.",
            "Strg + G wechselt in diesem Dialog zu dem Ordner, den Daleris anzeigt, aber die vordere Registerkarte in Daleris ist kein Ordner. Wechseln Sie dort zu einem Ordner, und drücken Sie dann hier erneut Strg + G.",
            "Ctrl + G porta questa finestra di dialogo alla cartella mostrata da Daleris, ma la scheda in primo piano in Daleris non è una cartella. Apri lì una cartella, poi premi di nuovo Ctrl + G qui.",
        ));
    }
    let folder = PathBuf::from(folder);
    if !folder.is_dir() {
        return Err(three(
            "The folder Daleris is showing can't be reached any more.",
            "Der Ordner, den Daleris anzeigt, ist nicht mehr erreichbar.",
            "La cartella mostrata da Daleris non è più raggiungibile.",
        ));
    }
    Ok(folder)
}

/// Sends the dialog to the folder, the way it goes to a folder typed in its own name box: that navigates and never
/// saves or closes, in Open, Save and folder dialogs alike, as long as it isn't there already. A folder dialog would
/// take a path to the folder it's in as the answer, which is why that's checked first.
fn jump() {
    let dialog = unsafe { GetForegroundWindow() };
    let Some(edit) = file_name_box(dialog) else {
        log("the key was pressed, but the window in front isn't a file dialog any more"); // i18n-ignore: log only
        return;
    };
    let folder = match target() {
        Ok(f) => f,
        Err(message) => {
            say(message);
            return;
        }
    };
    if is_at(dialog, &folder) {
        log(&format!("already at {}", folder.display())); // i18n-ignore: log only
        return;
    }
    let Some(typed) = text_of(edit) else {
        say(three(
            "That dialog isn't answering, so it couldn't be sent to the Daleris folder.",
            "Dieser Dialog reagiert nicht und konnte deshalb nicht zum Daleris-Ordner wechseln.",
            "La finestra di dialogo non risponde, quindi non è stato possibile portarla alla cartella di Daleris.",
        ));
        return;
    };
    let path: Vec<u16> = folder.display().to_string().encode_utf16().collect();
    if !set_text(edit, &path) || text_of(edit).is_none_or(|t| t != path) {
        say(three(
            "Daleris can't reach that dialog. It may belong to an app running as administrator.",
            "Daleris kann diesen Dialog nicht erreichen. Er gehört möglicherweise zu einer App, die als Administrator ausgeführt wird.",
            "Daleris non riesce a raggiungere la finestra di dialogo. Potrebbe appartenere a un'app eseguita come amministratore.",
        ));
        return;
    }
    let ok = unsafe { GetDlgItem(Some(dialog), IDOK.0) }.unwrap_or_default();
    // Posted: this returns only after the dialog has finished navigating, or showing whatever it wants to say.
    if unsafe { PostMessageW(Some(dialog), WM_COMMAND, WPARAM(IDOK.0 as usize), LPARAM(ok.0 as isize)) }.is_err() {
        let _ = set_text(edit, &typed);
        say(three("Daleris can't reach that dialog.", "Daleris kann diesen Dialog nicht erreichen.", "Daleris non riesce a raggiungere la finestra di dialogo."));
        return;
    }
    let started = Instant::now();
    let mut arrived = false;
    while started.elapsed() < NAVIGATE_WAIT {
        std::thread::sleep(Duration::from_millis(50));
        if !unsafe { IsWindow(Some(dialog)) }.as_bool() {
            log(&format!("the dialog closed instead of going to {}", folder.display())); // i18n-ignore: log only
            return;
        }
        if is_at(dialog, &folder) {
            arrived = true;
            break;
        }
    }
    // its own name box, as the dialog may have cleared it or left the path there
    if !set_text(edit, &typed) {
        log("couldn't put the typed name back"); // i18n-ignore: log only
    }
    if arrived {
        log(&format!("went to {}", folder.display())); // i18n-ignore: log only
    } else {
        // Windows shows some folders by name rather than path, such as Desktop, so this isn't always a failure
        log(&format!("sent to {}, and the address bar reads {:?}", folder.display(), address_of(dialog))); // i18n-ignore: log only
    }
}
