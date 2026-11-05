//! Showing the picker: a separate Daleris process, while this thread keeps the app's messages moving, as a real modal
//! dialog does. Every way this can go wrong ends in `UseWindows`, which shows Windows' own dialog instead.

use std::ffi::{c_void, OsStr};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use daleris_integration::pick::{HostMessage, PickReply, PickRequest};
use windows::core::{HRESULT, PCWSTR, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Security::SECURITY_ATTRIBUTES;
use windows::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows::Win32::System::LibraryLoader::{GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN};
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::*;

/// How long Daleris gets to put its window up before Windows' own dialog takes over.
const READY_WAIT: Duration = Duration::from_secs(8);
/// How long the picker may stop responding to the system before it's given up on.
const HUNG_WAIT: Duration = Duration::from_secs(10);

pub enum Outcome {
    Picked { filter_index: usize, paths: Vec<std::ffi::OsString> },
    Cancelled,
    /// The app called `Close` with this.
    Closed(HRESULT),
    /// Show Windows' own dialog, because Daleris couldn't or the person asked. The reason is for the log.
    UseWindows(&'static str),
}

/// A handle closed when it goes out of scope.
struct Owned(HANDLE);

impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_invalid() && !self.0 .0.is_null() {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}

// SAFETY: handles can be used from any thread, and the threads here never use the same one at the same time
// except to cancel, which is what that call is for.
unsafe impl Send for Owned {}
unsafe impl Sync for Owned {}

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// The way to the picker while it's up. A thread does the writing, as the picker may not be reading.
pub struct Link {
    tx: Sender<String>,
}

impl Link {
    pub fn send(&self, message: &HostMessage) {
        let _ = self.tx.send(message.encode());
    }
}

struct Child {
    process: Owned,
    pid: u32,
    /// where the reply lines arrive
    replies: Receiver<PickReply>,
    reader: std::thread::JoinHandle<()>,
    writer: std::thread::JoinHandle<()>,
    link: Link,
    /// kept so that this end can be cancelled if the reader must be stopped
    read_end: Arc<Owned>,
}

/// Starts `daleris --pick`, whose only inherited handles are its ends of two new pipes.
fn start(exe: &Path, request: &PickRequest) -> Option<Child> {
    let inheritable = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: std::ptr::null_mut(), bInheritHandle: true.into() };
    let (mut in_read, mut in_write, mut out_read, mut out_write) = (HANDLE::default(), HANDLE::default(), HANDLE::default(), HANDLE::default());
    unsafe { CreatePipe(&mut in_read, &mut in_write, Some(&inheritable), 65536) }.ok()?;
    let (in_read, in_write) = (Owned(in_read), Owned(in_write));
    unsafe { CreatePipe(&mut out_read, &mut out_write, Some(&inheritable), 65536) }.ok()?;
    let (out_read, out_write) = (Arc::new(Owned(out_read)), Owned(out_write));
    unsafe {
        SetHandleInformation(in_write.0, HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0)).ok()?;
        SetHandleInformation(out_read.0, HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0)).ok()?;
    }

    // exactly these two handles, never whatever else this app left inheritable
    let handles = [in_read.0, out_write.0];
    let mut size = 0;
    let _ = unsafe { InitializeProcThreadAttributeList(LPPROC_THREAD_ATTRIBUTE_LIST(std::ptr::null_mut()), 1, 0, &mut size) };
    let mut list_buf = vec![0u8; size];
    let list = LPPROC_THREAD_ATTRIBUTE_LIST(list_buf.as_mut_ptr() as *mut c_void);
    unsafe { InitializeProcThreadAttributeList(list, 1, 0, &mut size) }.ok()?;
    let updated = unsafe { UpdateProcThreadAttribute(list, 0, PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize, Some(handles.as_ptr() as *const c_void), std::mem::size_of_val(&handles), None, None) };
    let started = updated.ok().and_then(|()| {
        let mut info = STARTUPINFOEXW::default();
        info.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        info.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        info.StartupInfo.hStdInput = in_read.0;
        info.StartupInfo.hStdOutput = out_write.0;
        info.lpAttributeList = list;
        let mut command = wide(OsStr::new(&format!("\"{}\" --pick", exe.display())));
        let app = wide(exe.as_os_str());
        let dir = exe.parent().map(|d| wide(d.as_os_str()));
        let mut pi = PROCESS_INFORMATION::default();
        unsafe {
            CreateProcessW(
                PCWSTR(app.as_ptr()),
                PWSTR(command.as_mut_ptr()),
                None,
                None,
                true,
                EXTENDED_STARTUPINFO_PRESENT,
                None,
                dir.as_ref().map_or(PCWSTR::null(), |d| PCWSTR(d.as_ptr())),
                &info.StartupInfo,
                &mut pi,
            )
        }
        .ok()?;
        let _ = unsafe { CloseHandle(pi.hThread) };
        Some((Owned(pi.hProcess), pi.dwProcessId))
    });
    unsafe { DeleteProcThreadAttributeList(list) };
    let (process, pid) = started?;
    // these ends are the child's now; holding them would keep the pipes open after it dies
    drop(in_read);
    drop(out_write);

    let (to_picker, outgoing) = channel::<String>();
    to_picker.send(request.encode()).ok()?;
    let writer = std::thread::Builder::new()
        .name("daleris picker writer".into())
        .spawn(move || {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                while let Ok(text) = outgoing.recv() {
                    let mut written = 0;
                    if unsafe { WriteFile(in_write.0, Some(text.as_bytes()), Some(&mut written), None) }.is_err() {
                        break;
                    }
                }
                // this end closes here, which tells a picker whose app has stopped caring to end
                drop(in_write);
            }));
        })
        .ok()?;

    let (tx, replies) = channel();
    let reader_end = out_read.clone();
    let reader = std::thread::Builder::new()
        .name("daleris picker".into())
        .spawn(move || {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut pending = Vec::new();
                let mut buf = [0u8; 8192];
                loop {
                    let mut read = 0;
                    if unsafe { ReadFile(reader_end.0, Some(&mut buf), Some(&mut read), None) }.is_err() || read == 0 {
                        break;
                    }
                    pending.extend_from_slice(&buf[..read as usize]);
                    while let Some(end) = pending.iter().position(|&b| b == b'\n') {
                        let line: Vec<u8> = pending.drain(..=end).collect();
                        if let Some(reply) = std::str::from_utf8(&line).ok().and_then(PickReply::decode) {
                            let _ = tx.send(reply);
                        }
                    }
                }
            }));
        })
        .ok()?;
    Some(Child { process, pid, replies, reader, writer, link: Link { tx: to_picker }, read_end: out_read })
}

/// Leaves the owner usable again however this ends.
struct OwnerDisabled(HWND);

impl OwnerDisabled {
    fn new(owner: HWND) -> OwnerDisabled {
        if !owner.0.is_null() {
            let _ = unsafe { EnableWindow(owner, false) };
        }
        OwnerDisabled(owner)
    }
}

impl Drop for OwnerDisabled {
    fn drop(&mut self) {
        if !self.0 .0.is_null() {
            let _ = unsafe { EnableWindow(self.0, true) };
            let _ = unsafe { SetForegroundWindow(self.0) };
        }
    }
}

fn window_of(pid: u32) -> Option<HWND> {
    struct Find {
        pid: u32,
        found: Option<HWND>,
    }
    unsafe extern "system" fn each(hwnd: HWND, data: LPARAM) -> BOOL {
        let find = &mut *(data.0 as *mut Find);
        let mut owner = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut owner));
        if owner == find.pid && IsWindowVisible(hwnd).as_bool() {
            find.found = Some(hwnd);
            return false.into();
        }
        true.into()
    }
    let mut find = Find { pid, found: None };
    let _ = unsafe { EnumWindows(Some(each), LPARAM(&mut find as *mut Find as isize)) };
    find.found
}

/// Runs the picker until there's an outcome, with this thread's messages moving all the while.
/// `closing` is checked each round, for an app that calls `Close` from a timer while this runs. `tell` gets to send what
/// changed on the app's side each round, and `event` gets what the person does as they do it.
pub fn run(exe: &Path, request: &PickRequest, owner: HWND, closing: impl Fn() -> Option<HRESULT>, tell: impl FnMut(&Link), event: impl FnMut(PickReply, &Link)) -> Outcome {
    let Some(child) = start(exe, request) else { return Outcome::UseWindows("couldn't start Daleris") };
    let _ = unsafe { AllowSetForegroundWindow(child.pid) };
    let disabled = OwnerDisabled::new(owner);
    let outcome = wait(&child, closing, tell, event);
    drop(disabled);
    let _ = unsafe { TerminateProcess(child.process.0, 1) };
    finish_threads(child);
    outcome
}

/// The threads end when the child's handles do. Should anything hold those open, stop waiting rather than hang the app,
/// and keep this DLL in memory for a thread that's still in it.
fn finish_threads(child: Child) {
    let Child { reader, writer, link, read_end, .. } = child;
    drop(link);
    let started = Instant::now();
    while !(reader.is_finished() && writer.is_finished()) && started.elapsed() < Duration::from_millis(500) {
        std::thread::sleep(Duration::from_millis(5));
    }
    if reader.is_finished() && writer.is_finished() {
        let _ = reader.join();
        let _ = writer.join();
        return;
    }
    let _ = unsafe { windows::Win32::System::IO::CancelIoEx(read_end.0, None) };
    let mut me = HMODULE::default();
    let here = finish_threads as *const () as *const u16;
    let _ = unsafe { GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN, PCWSTR(here), &mut me) };
    std::mem::forget(reader);
    std::mem::forget(writer);
}

fn wait(child: &Child, closing: impl Fn() -> Option<HRESULT>, mut tell: impl FnMut(&Link), mut event: impl FnMut(PickReply, &Link)) -> Outcome {
    let started = Instant::now();
    let mut ready = false;
    let mut hung_since: Option<Instant> = None;
    let mut last_check = Instant::now();
    loop {
        if let Some(hr) = closing() {
            return Outcome::Closed(hr);
        }
        tell(&child.link);
        loop {
            match child.replies.try_recv() {
                Ok(PickReply::Ready) => ready = true,
                Ok(PickReply::Picked { filter_index, paths }) => return Outcome::Picked { filter_index, paths },
                Ok(PickReply::Cancelled) => return Outcome::Cancelled,
                Ok(PickReply::UseWindows) => return Outcome::UseWindows("the person asked for it in the picker"),
                Ok(other) => event(other, &child.link),
                Err(TryRecvError::Empty) => break,
                // the pipe is gone: the process ended or closed it, and it said nothing that counts
                Err(TryRecvError::Disconnected) => return Outcome::UseWindows("the picker went away"),
            }
        }
        if !ready && started.elapsed() > READY_WAIT {
            return Outcome::UseWindows("the picker didn't start in time");
        }
        if ready && last_check.elapsed() > Duration::from_secs(1) {
            last_check = Instant::now();
            let hung = window_of(child.pid).is_some_and(|w| unsafe { IsHungAppWindow(w) }.as_bool());
            hung_since = if hung { hung_since.or(Some(Instant::now())) } else { None };
            if hung_since.is_some_and(|t| t.elapsed() > HUNG_WAIT) {
                return Outcome::UseWindows("the picker stopped responding");
            }
        }
        let handles = [child.process.0];
        let rc = unsafe { MsgWaitForMultipleObjectsEx(Some(&handles), 30, QS_ALLINPUT, MWMO_INPUTAVAILABLE) };
        // the process ended: what it said is already on its way down the pipe, so one more pass finds it
        if rc == WAIT_OBJECT_0 {
            std::thread::sleep(Duration::from_millis(20));
            while let Ok(reply) = child.replies.try_recv() {
                match reply {
                    PickReply::Picked { filter_index, paths } => return Outcome::Picked { filter_index, paths },
                    PickReply::Cancelled => return Outcome::Cancelled,
                    PickReply::UseWindows => return Outcome::UseWindows("the person asked for it in the picker"),
                    PickReply::Ready => {}
                    other => event(other, &child.link),
                }
            }
            return Outcome::UseWindows("the picker went away");
        }
        let mut msg = MSG::default();
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            if msg.message == WM_QUIT {
                // the app is closing: leave, and let its own loop see the same message
                unsafe { PostQuitMessage(msg.wParam.0 as i32) };
                return Outcome::Cancelled;
            }
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}
