//! What was decided and why, for finding out why an app kept the Windows dialog on the PC where that happens.
//!
//! Only if the "Log file dialog decisions" switch is on, which goes in `%LOCALAPPDATA%\Daleris\logs\file-dialogs.log`,
//! or DALERIS_DIALOG_LOG names a file, for tests. Otherwise this writes nothing from inside other people's apps.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use daleris_integration::reg::{self, Hive};
use daleris_integration::Feature;
use windows::core::PCWSTR;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Threading::GetCurrentProcessId;

static ON: AtomicBool = AtomicBool::new(false);
const LIMIT: u64 = 2 * 1024 * 1024;

/// Looks at the switch again, as each dialog starts.
pub fn refresh() {
    ON.store(Hive::real().flag(Feature::DialogLog), Ordering::Relaxed);
}

fn append(path: PathBuf, line: &str) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > LIMIT) {
        let _ = std::fs::rename(&path, path.with_extension("old.log"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = std::io::Write::write_all(&mut f, line.as_bytes());
    }
}

/// This DLL's file, whose name says which build it is.
pub fn this_dll() -> String {
    let mut me = HMODULE::default();
    let here = this_dll as *const () as *const u16;
    if unsafe { GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, PCWSTR(here), &mut me) }.is_err() {
        return "?".into();
    }
    let mut buf = [0u16; 1024];
    let n = unsafe { GetModuleFileNameW(me, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n]).rsplit('\\').next().unwrap_or("?").to_string()
}

pub fn note(what: &str) {
    let env = std::env::var_os("DALERIS_DIALOG_LOG");
    let on = ON.load(Ordering::Relaxed);
    if env.is_none() && !on {
        return;
    }
    let t = unsafe { GetLocalTime() };
    let line = format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02} [{} {}] {what}\n", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, crate::host_path(), unsafe { GetCurrentProcessId() });
    if let Some(path) = env {
        append(PathBuf::from(path), &line);
    }
    if on {
        if let Some(dir) = reg::logs_dir() {
            append(dir.join(reg::DIALOG_LOG), &line);
        }
    }
}
