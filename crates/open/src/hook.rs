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
/// Was E kept back during this press of Win? Then its release is too, and so is the Start menu.
static KEPT_BACK: AtomicBool = AtomicBool::new(false);
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
    let Ok(hook) = (unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(on_key), module, 0) }) else { return };
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

fn pressed() {
    let Ok(me) = std::env::current_exe() else { return };
    let _ = std::process::Command::new(me).arg("--win-e-press").spawn();
}

unsafe extern "system" fn on_key(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
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
    if vk == VK_E && WIN_DOWN.load(Ordering::SeqCst) {
        if down {
            // Win + Ctrl + E and the like belong to someone else
            let held = |k: VIRTUAL_KEY| GetAsyncKeyState(k.0 as i32) < 0;
            if held(VK_CONTROL) || held(VK_MENU) || held(VK_SHIFT) {
                return next();
            }
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
            return LRESULT(1);
        }
        if KEPT_BACK.load(Ordering::SeqCst) {
            return LRESULT(1);
        }
    }
    next()
}

/// Windows opens the Start menu when Win goes up with nothing pressed in between. Something now was.
unsafe fn mask_start_menu() {
    let press = |flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(MASK_KEY), wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    };
    let inputs = [press(KEYBD_EVENT_FLAGS(0)), press(KEYEVENTF_KEYUP)];
    SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
}
