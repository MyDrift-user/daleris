//! Reaching the real File Explorer, and asking Windows what a shell name means.

use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use windows::core::PCWSTR;
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
use windows::Win32::UI::Shell::{SHGetPathFromIDListEx, SHParseDisplayName, GPFIDL_DEFAULT};

fn explorer_exe() -> PathBuf {
    std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows")).join("explorer.exe")
}

/// Starts Explorer itself, never through a verb, so that this can't come back to Daleris. (The one thing never to use
/// for this is an Image File Execution Options entry for explorer.exe, which risks the whole shell.)
/// With nothing to show it opens This PC, as a bare `explorer.exe` goes through what Win + E is set to.
pub fn open_in_explorer(target: Option<&OsStr>) -> bool {
    let this_pc = OsString::from("::{20D04FE0-3AEA-1069-A2D8-08002B30309D}");
    Command::new(explorer_exe()).arg(target.unwrap_or(&this_pc)).spawn().is_ok()
}

/// Opens Explorer with an item selected.
pub fn select_in_explorer(path: &OsStr) -> bool {
    let mut arg = OsString::from("/select,\"");
    arg.push(path);
    arg.push("\"");
    Command::new(explorer_exe()).raw_arg(arg).spawn().is_ok()
}

/// The folder on disk behind a `shell:` or `::{…}` name. None for what's only virtual, such as the Recycle Bin.
pub fn resolve(name: &str) -> Option<PathBuf> {
    // on the caller's thread, where COM may or may not be set up yet
    let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
    let wname: Vec<u16> = OsStr::new(name).encode_wide().chain(std::iter::once(0)).collect();
    let mut pidl = std::ptr::null_mut();
    let parsed = unsafe { SHParseDisplayName(PCWSTR(wname.as_ptr()), None, &mut pidl, 0, None) };
    let mut result = None;
    if parsed.is_ok() && !pidl.is_null() {
        let mut buf = [0u16; 1024];
        if unsafe { SHGetPathFromIDListEx(pidl, &mut buf, GPFIDL_DEFAULT) }.as_bool() {
            let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            if end > 0 {
                result = Some(PathBuf::from(OsString::from_wide(&buf[..end])));
            }
        }
        unsafe { CoTaskMemFree(Some(pidl as *const _)) };
    }
    if com.is_ok() {
        unsafe { CoUninitialize() };
    }
    result.filter(|p| p.is_dir())
}
