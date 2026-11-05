//! Telling whether the small programs that stay running do so, and stopping them: the Win + E hook and the Ctrl + G
//! watcher, both in the launcher.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Threading::{OpenEventW, OpenMutexW, SetEvent, EVENT_MODIFY_STATE, SYNCHRONIZATION_SYNCHRONIZE};

#[derive(Clone, Copy)]
pub struct Agent {
    /// held while it runs
    pub running: PCWSTR,
    /// set to make it stop
    pub stop: PCWSTR,
    /// the launcher's argument for it
    pub arg: &'static str,
}

// SAFETY: these only ever point at string constants.
unsafe impl Sync for Agent {}

pub const HOOK: Agent = Agent { running: w!(r"Local\DalerisWinEHook"), stop: w!(r"Local\DalerisWinEHookStop"), arg: "--win-e-hook" };
pub const JUMP: Agent = Agent { running: w!(r"Local\DalerisDialogJump"), stop: w!(r"Local\DalerisDialogJumpStop"), arg: "--dialog-jump" };

impl Agent {
    pub fn is_running(&self) -> bool {
        match unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, self.running) } {
            Ok(h) => {
                let _ = unsafe { CloseHandle(h) };
                true
            }
            Err(_) => false,
        }
    }

    /// Tells it to stop, and returns once it has or after two seconds.
    pub fn stop(&self) {
        if let Ok(event) = unsafe { OpenEventW(EVENT_MODIFY_STATE, false, self.stop) } {
            let _ = unsafe { SetEvent(event) };
            let _ = unsafe { CloseHandle(event) };
            for _ in 0..40 {
                if !self.is_running() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
    }
}

/// Stops everything that stays running.
pub fn stop_all() {
    HOOK.stop();
    JUMP.stop();
}
