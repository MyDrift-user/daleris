//! Win + E through a low-level keyboard hook, for Windows builds where the shortcut no longer asks the registry.
//!
//! This is a process of its own that stays running, started at sign-in. A low-level hook is called in this process, so
//! nothing loads into any other app, and only Win + E is ever kept back; every other key passes straight through
//! without being stored or looked at further.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use daleris_integration::hookctl::HOOK;
use daleris_integration::reg::{policy_fixed, Hive};
use daleris_integration::Feature;

use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Asks the thread with the message loop to act on a press, as the hook itself must return quickly.
const WM_PRESSED: u32 = WM_APP + 1;
/// A key with no meaning, pressed so that releasing Win doesn't open the Start menu.
const MASK_KEY: u16 = 0xE8;

static WIN_DOWN: AtomicBool = AtomicBool::new(false);
/// Was E kept back during this press of Win? Then so is the Start menu when Win goes up.
static KEPT_BACK: AtomicBool = AtomicBool::new(false);
/// Is the E key that was kept back still down? Its release is kept back too, and only that one.
static E_KEPT: AtomicBool = AtomicBool::new(false);
static THREAD: AtomicU32 = AtomicU32::new(0);

pub fn run() {
    if policy_fixed(Feature::WinEHook) == Some(false) || !Hive::real().is_in_place(Feature::WinEHook) {
        return;
    }
    let Ok(mutex) = (unsafe { CreateMutexW(None, true, HOOK.running) }) else { return };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        return;
    }
    let Ok(stop) = (unsafe { CreateEventW(None, true, false, HOOK.stop) }) else { return };
    THREAD.store(unsafe { GetCurrentThreadId() }, Ordering::SeqCst);
    let Ok(module) = (unsafe { GetModuleHandleW(None) }) else { return };
    let Ok(hook) = (unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(on_key), Some(module.into()), 0) }) else { return };
    let mut msg = MSG::default();
    loop {
        let rc = unsafe { MsgWaitForMultipleObjects(Some(&[stop]), false, INFINITE, QS_ALLINPUT) };
        if rc == WAIT_OBJECT_0 {
            break;
        }
        let mut quit = false;
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            if msg.message == WM_QUIT {
                quit = true;
            } else if msg.message == WM_PRESSED {
                pressed();
            }
            unsafe { DispatchMessageW(&msg) };
        }
        if quit {
            break;
        }
    }
    let _ = unsafe { UnhookWindowsHookEx(hook) };
    let _ = unsafe { ReleaseMutex(mutex) };
    let _ = unsafe { CloseHandle(mutex) };
}

/// Opens Daleris from this process, on a thread of its own so the hook answers at once. Starting another copy of this
/// program first would add a whole process start, which antivirus software makes slow, to every Win + E.
fn pressed() {
    let _ = std::thread::Builder::new().name("win-e".into()).spawn(|| crate::open(&[], daleris_integration::Feature::WinEHook));
}

unsafe extern "system" fn on_key(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT { unsafe {
    let next = || CallNextHookEx(None, code, wparam, lparam);
    if code != HC_ACTION as i32 {
        return next();
    }
    let info = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
    let vk = VIRTUAL_KEY(info.vkCode as u16);
    let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
    if vk == VK_LWIN || vk == VK_RWIN {
        WIN_DOWN.store(down, Ordering::SeqCst);
        if !down && KEPT_BACK.swap(false, Ordering::SeqCst) {
            mask_start_menu();
        }
        return next();
    }
    if vk == VK_E && !down && E_KEPT.swap(false, Ordering::SeqCst) {
        return LRESULT(1);
    }
    if vk == VK_E && down {
        let held = |k: VIRTUAL_KEY| GetAsyncKeyState(k.0 as i32) < 0;
        let keys = Keys {
            remembered_win: WIN_DOWN.load(Ordering::SeqCst),
            win_held: held(VK_LWIN) || held(VK_RWIN),
            other_modifier: held(VK_CONTROL) || held(VK_MENU) || held(VK_SHIFT),
        };
        match on_e(keys) {
            ForE::Pass => {}
            ForE::Forget => {
                WIN_DOWN.store(false, Ordering::SeqCst);
                KEPT_BACK.store(false, Ordering::SeqCst);
            }
            ForE::Take => {
                // Daleris is gone or an administrator turned this off: Windows keeps the shortcut. The launcher cleans up.
                let usable = Hive::real().program_path().is_some_and(|p| p.is_file()) && policy_fixed(Feature::WinEHook) != Some(false);
                if !usable {
                    let _ = PostThreadMessageW(THREAD.load(Ordering::SeqCst), WM_PRESSED, WPARAM(0), LPARAM(0));
                    return next();
                }
                // a key held down repeats, which should only open one window
                if !KEPT_BACK.swap(true, Ordering::SeqCst) {
                    let _ = PostThreadMessageW(THREAD.load(Ordering::SeqCst), WM_PRESSED, WPARAM(0), LPARAM(0));
                }
                E_KEPT.store(true, Ordering::SeqCst);
                return LRESULT(1);
            }
        }
    }
    next()
}}

/// What the hook knows when E goes down.
#[derive(Clone, Copy)]
struct Keys {
    /// Win went down, and this hook hasn't heard it go up since
    remembered_win: bool,
    /// Windows says a Win key is down right now
    win_held: bool,
    /// Ctrl, Alt or Shift is down too
    other_modifier: bool,
}

#[derive(Debug, PartialEq)]
enum ForE {
    /// an E like any other, for whatever has the focus
    Pass,
    /// Win's release happened where this hook never hears it: Win + L, the UAC prompt, Ctrl + Alt + Del or switching
    /// users. Remembering Win as held would keep back every E typed from then on, so it's forgotten and E passes.
    Forget,
    /// Win + E: kept back, and Daleris opens
    Take,
}

fn on_e(k: Keys) -> ForE {
    match k {
        Keys { remembered_win: false, .. } => ForE::Pass,
        Keys { win_held: false, .. } => ForE::Forget,
        // Win + Ctrl + E and the like belong to someone else
        Keys { other_modifier: true, .. } => ForE::Pass,
        _ => ForE::Take,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_win_and_e_together_are_kept_back() {
        let k = |remembered_win, win_held, other_modifier| on_e(Keys { remembered_win, win_held, other_modifier });
        assert_eq!(k(true, true, false), ForE::Take, "Win + E");
        assert_eq!(k(false, false, false), ForE::Pass, "a plain E");
        assert_eq!(k(false, true, false), ForE::Pass, "Win pressed before the hook started: Windows keeps it");
        assert_eq!(k(true, true, true), ForE::Pass, "Win + Shift + E belongs to someone else");
        // the bug: Win went up on the lock screen, and every E typed afterwards vanished
        assert_eq!(k(true, false, false), ForE::Forget, "a Win release the hook never heard");
        assert_eq!(k(true, false, true), ForE::Forget);
    }
}

/// Windows opens the Start menu when Win goes up with nothing pressed in between. Something now was.
unsafe fn mask_start_menu() { unsafe {
    let press = |flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(MASK_KEY), wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    };
    let inputs = [press(KEYBD_EVENT_FLAGS(0)), press(KEYEVENTF_KEYUP)];
    SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
}}
