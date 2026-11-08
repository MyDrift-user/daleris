//! The channel from a new launch to the running window: a named pipe per user and session, only open to that user,
//! with short timeouts. Whoever can't reach a window just opens a new one.

use std::ffi::{c_void, OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::time::{Duration, Instant};

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Security::Authorization::{ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows::Win32::Security::{EqualSid, GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER};
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Pipes::*;
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::Threading::{CreateEventW, GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow;

const REPLY_WAIT: Duration = Duration::from_millis(1500);
const MAX_MESSAGE: usize = 65_536;

/// What a new launch asks the running window to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// Just come to the front.
    Show,
    ThisPc,
    Open(OsString),
    Select(OsString),
}

impl Request {
    pub fn encode(&self) -> Vec<u16> {
        let (tag, path) = match self {
            Request::Show => ('N', None),
            Request::ThisPc => ('P', None),
            Request::Open(p) => ('O', Some(p)),
            Request::Select(p) => ('S', Some(p)),
        };
        std::iter::once(tag as u16).chain(path.map(|p| p.encode_wide().collect::<Vec<_>>()).unwrap_or_default()).collect()
    }

    pub fn decode(words: &[u16]) -> Option<Request> {
        let (tag, rest) = words.split_first()?;
        let path = || OsString::from_wide(rest);
        match char::from_u32(*tag as u32)? {
            'N' => Some(Request::Show),
            'P' => Some(Request::ThisPc),
            'O' if !rest.is_empty() => Some(Request::Open(path())),
            'S' if !rest.is_empty() => Some(Request::Select(path())),
            _ => None,
        }
    }
}

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// The user of a process's token, as the buffer `sid_of` reads.
fn user_of_process(process: HANDLE) -> Option<Vec<u8>> {
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }.ok()?;
    let mut needed = 0;
    let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut needed) };
    let mut buf = vec![0u8; needed as usize];
    let got = unsafe { GetTokenInformation(token, TokenUser, Some(buf.as_mut_ptr() as *mut c_void), needed, &mut needed) };
    let _ = unsafe { CloseHandle(token) };
    got.ok()?;
    Some(buf)
}

fn sid_of(buf: &[u8]) -> PSID {
    // SAFETY: the buffer is what GetTokenInformation filled in for TokenUser, and it outlives the use of the pointer.
    unsafe { (*(buf.as_ptr() as *const TOKEN_USER)).User.Sid }
}

fn sid_text() -> Option<String> {
    let me = user_of_process(unsafe { GetCurrentProcess() })?;
    let mut text = PWSTR::null();
    unsafe { ConvertSidToStringSidW(sid_of(&me), &mut text) }.ok()?;
    let s = unsafe { text.to_string() }.ok();
    let _ = unsafe { LocalFree(Some(HLOCAL(text.0 as *mut c_void))) };
    s
}

fn session() -> u32 {
    let mut id = 0;
    let _ = unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut id) };
    id
}

/// `\\.\pipe\daleris-<user>-<session>`.
pub fn pipe_name() -> Option<String> {
    Some(format!(r"\\.\pipe\daleris-{}-{}", sid_text()?, session()))
}

/// Same user and session as this process?
fn same_user_and_session(pid: u32) -> bool {
    let mut other_session = 0;
    if unsafe { ProcessIdToSessionId(pid, &mut other_session) }.is_err() || other_session != session() {
        return false;
    }
    let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else { return false };
    let theirs = user_of_process(process);
    let _ = unsafe { CloseHandle(process) };
    let (Some(theirs), Some(mine)) = (theirs, user_of_process(unsafe { GetCurrentProcess() })) else { return false };
    unsafe { EqualSid(sid_of(&theirs), sid_of(&mine)) }.is_ok()
}

/// Sends a request to the running window. True only if a window of this user and session took it.
/// Returns within about two seconds, whatever the other side does.
pub fn send(request: &Request) -> bool {
    pipe_name().is_some_and(|name| send_to(&name, request).is_some())
}

fn send_to(name: &str, request: &Request) -> Option<()> {
    let wname = wide(OsStr::new(name));
    let started = Instant::now();
    let pipe = loop {
        let h = unsafe { CreateFileW(PCWSTR(wname.as_ptr()), (GENERIC_READ | GENERIC_WRITE).0, FILE_SHARE_NONE, None, OPEN_EXISTING, FILE_FLAG_OVERLAPPED | SECURITY_IDENTIFICATION, None) };
        match h {
            Ok(h) => break h,
            // busy with another launch: try again briefly
            Err(e) if e.code() == ERROR_PIPE_BUSY.to_hresult() && started.elapsed() < Duration::from_millis(500) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => return None,
        }
    };
    let result = talk(pipe, request);
    let _ = unsafe { CloseHandle(pipe) };
    result
}

/// Overlapped, so that no read or write can wait longer than `REPLY_WAIT`.
fn talk(pipe: HANDLE, request: &Request) -> Option<()> {
    let mut server_pid = 0;
    unsafe { GetNamedPipeServerProcessId(pipe, &mut server_pid) }.ok()?;
    if !same_user_and_session(server_pid) {
        return None;
    }
    // lets the window that takes this come to the front
    let _ = unsafe { AllowSetForegroundWindow(server_pid) };
    let mode = PIPE_READMODE_MESSAGE;
    unsafe { SetNamedPipeHandleState(pipe, Some(&mode), None, None) }.ok()?;
    let event = unsafe { CreateEventW(None, true, false, None) }.ok()?;
    let result = exchange(pipe, event, request);
    let _ = unsafe { CloseHandle(event) };
    result
}

fn exchange(pipe: HANDLE, event: HANDLE, request: &Request) -> Option<()> {
    let bytes: Vec<u8> = request.encode().iter().flat_map(|w| w.to_le_bytes()).collect();
    let mut ov = OVERLAPPED { hEvent: event, ..Default::default() };
    let started = unsafe { WriteFile(pipe, Some(&bytes), None, Some(&mut ov)) };
    finish(pipe, event, &ov, started)?;
    let mut reply = [0u8; 2];
    let mut ov = OVERLAPPED { hEvent: event, ..Default::default() };
    let started = unsafe { ReadFile(pipe, Some(&mut reply), None, Some(&mut ov)) };
    finish(pipe, event, &ov, started)?;
    (reply == *b"ok").then_some(())
}

fn finish(pipe: HANDLE, event: HANDLE, ov: &OVERLAPPED, started: windows::core::Result<()>) -> Option<()> {
    match started {
        Ok(()) => Some(()),
        Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => {
            let mut n = 0;
            if unsafe { WaitForSingleObject(event, REPLY_WAIT.as_millis() as u32) } == WAIT_OBJECT_0 {
                return unsafe { GetOverlappedResult(pipe, ov, &mut n, false) }.ok();
            }
            let _ = unsafe { CancelIoEx(pipe, Some(ov)) };
            let _ = unsafe { GetOverlappedResult(pipe, ov, &mut n, true) };
            None
        }
        Err(_) => None,
    }
}

/// The window's side. Makes the pipe only if no other window has one, so the first window is the one that stays.
pub struct Server {
    pipe: HANDLE,
}

// SAFETY: a pipe handle can be used from any thread, and only the thread that owns the Server uses this one.
unsafe impl Send for Server {}

impl Server {
    pub fn start() -> Option<Server> {
        Self::create(FILE_FLAG_FIRST_PIPE_INSTANCE)
    }

    fn create(extra: FILE_FLAGS_AND_ATTRIBUTES) -> Option<Server> {
        let name = wide(OsStr::new(&pipe_name()?));
        let sid = sid_text()?;
        // this user is the one entry, so nobody else gets in
        let sddl = wide(OsStr::new(&format!("D:P(A;;GA;;;{sid})")));
        let mut sd = PSECURITY_DESCRIPTOR::default();
        unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(PCWSTR(sddl.as_ptr()), SDDL_REVISION_1, &mut sd, None) }.ok()?;
        let attributes = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd.0, bInheritHandle: false.into() };
        let pipe = unsafe {
            CreateNamedPipeW(
                PCWSTR(name.as_ptr()),
                PIPE_ACCESS_DUPLEX | extra,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                4096,
                MAX_MESSAGE as u32,
                0,
                Some(&attributes),
            )
        };
        let _ = unsafe { LocalFree(Some(HLOCAL(sd.0))) };
        (!pipe.is_invalid()).then_some(Server { pipe })
    }

    /// Serves requests until `stop` says otherwise, which it's asked after each connection. The handler runs on this
    /// thread. `wake` below gets a server that waits for its next connection to look at `stop`.
    pub fn run(self, stop: impl Fn() -> bool, mut handle: impl FnMut(Request)) {
        let pipe = self.pipe;
        std::mem::forget(self);
        while !stop() {
            if unsafe { ConnectNamedPipe(pipe, None) }.is_err() && unsafe { GetLastError() } != ERROR_PIPE_CONNECTED {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            if !stop() {
                if let Some(request) = read_request(pipe) {
                    let mut written = 0;
                    let _ = unsafe { WriteFile(pipe, Some(b"ok"), Some(&mut written), None) };
                    handle(request);
                }
            }
            let _ = unsafe { FlushFileBuffers(pipe) };
            let _ = unsafe { DisconnectNamedPipe(pipe) };
        }
        let _ = unsafe { CloseHandle(pipe) };
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.pipe) };
    }
}

/// Is a window taking requests? Doesn't touch the connection a window is waiting on.
pub fn is_up() -> bool {
    let Some(name) = pipe_name() else { return false };
    let wname = wide(OsStr::new(&name));
    unsafe { WaitNamedPipeW(PCWSTR(wname.as_ptr()), 1) }.as_bool() || unsafe { GetLastError() } == ERROR_SEM_TIMEOUT
}

/// Makes a server that waits for a connection look at its `stop`.
pub fn wake() {
    let Some(name) = pipe_name() else { return };
    let wname = wide(OsStr::new(&name));
    if let Ok(h) = unsafe { CreateFileW(PCWSTR(wname.as_ptr()), GENERIC_READ.0, FILE_SHARE_NONE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None) } {
        let _ = unsafe { CloseHandle(h) };
    }
}

fn read_request(pipe: HANDLE) -> Option<Request> {
    let mut client = 0;
    unsafe { GetNamedPipeClientProcessId(pipe, &mut client) }.ok()?;
    if !same_user_and_session(client) {
        return None;
    }
    let mut buf = vec![0u8; MAX_MESSAGE];
    let mut read = 0;
    unsafe { ReadFile(pipe, Some(&mut buf), Some(&mut read), None) }.ok()?;
    let words: Vec<u16> = buf[..read as usize].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    Request::decode(&words)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_survive_the_trip_even_with_names_windows_allows_but_unicode_doesnt() {
        let odd = OsString::from_wide(&[0x43, 0x3a, 0x5c, 0xd800]);
        for r in [Request::Show, Request::ThisPc, Request::Open(odd.clone()), Request::Select(odd)] {
            assert_eq!(Request::decode(&r.encode()), Some(r));
        }
        assert_eq!(Request::decode(&[]), None);
        assert_eq!(Request::decode(&['O' as u16]), None);
        assert_eq!(Request::decode(&['X' as u16, 65]), None);
    }

    #[test]
    fn the_name_is_per_user_and_session() {
        let name = pipe_name().unwrap();
        assert!(name.starts_with(r"\\.\pipe\daleris-S-1-"), "{name}");
        assert!(name.ends_with(&format!("-{}", session())), "{name}");
    }

    #[test]
    fn nothing_listening_means_no_and_quickly() {
        let started = Instant::now();
        assert!(send_to(r"\\.\pipe\daleris-test-nobody-here", &Request::Show).is_none());
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
