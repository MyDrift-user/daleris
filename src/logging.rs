//! The log file and crash capture. Panics on any thread go in with a backtrace. On Linux and
//! macOS so do fatal signals, and, when not started from a terminal, whatever system libraries
//! print to stderr. A "started" line with no "exited" line after it means the process was killed.
//! DALERIS_LOG=debug or DALERIS_LOG=trace logs more.

use std::fs::File;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock, PoisonError};

const MAX_SIZE: u64 = 2 * 1024 * 1024;

static FILE: Mutex<Option<File>> = Mutex::new(None);
static PATH: OnceLock<PathBuf> = OnceLock::new();

/// The log file, if one could be opened.
pub fn path() -> Option<&'static PathBuf> {
    PATH.get()
}

fn stamp() -> String {
    let ft = crate::util::now_filetime();
    let t = crate::util::filetime_local(ft);
    let sec = ft / 10_000_000 % 60;
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{sec:02}", t.year, t.month, t.day, t.hour, t.minute)
}

fn write_line(line: &str) {
    if let Some(f) = FILE.lock().unwrap_or_else(PoisonError::into_inner).as_mut() {
        let _ = f.write_all(line.as_bytes());
    }
    let mut err = std::io::stderr();
    if err.is_terminal() {
        let _ = err.write_all(line.as_bytes());
    }
}

struct Logger;

impl log::Log for Logger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::max_level()
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            write_line(&format!("{} [{}] {:<5} {}: {}\n", stamp(), std::process::id(), r.level(), r.target(), r.args()));
        }
    }
    fn flush(&self) {}
}

/// The system, desktop and session type are the first things to know about a crash.
fn log_start() {
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| "-".into());
    log::info!("Daleris {} started on {} {}, arguments: {:?}", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH, std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>());
    if cfg!(target_os = "linux") {
        log::info!(
            "session: XDG_SESSION_TYPE={} XDG_CURRENT_DESKTOP={} WAYLAND_DISPLAY={} DISPLAY={} WINIT_UNIX_BACKEND={}",
            env("XDG_SESSION_TYPE"),
            env("XDG_CURRENT_DESKTOP"),
            env("WAYLAND_DISPLAY"),
            env("DISPLAY"),
            env("WINIT_UNIX_BACKEND")
        );
    }
}

pub fn init() {
    let level = match std::env::var("DALERIS_LOG").as_deref() {
        Ok("trace") => log::LevelFilter::Trace,
        Ok("debug") => log::LevelFilter::Debug,
        _ => log::LevelFilter::Info,
    };
    if let Some(dir) = crate::util::log_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("daleris.log");
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_SIZE) {
            let _ = std::fs::rename(&path, dir.join("daleris.old.log"));
        }
        if let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            #[cfg(unix)]
            crash::install(&f);
            *FILE.lock().unwrap_or_else(PoisonError::into_inner) = Some(f);
            let _ = PATH.set(path);
        }
    }
    let _ = log::set_logger(&Logger);
    log::set_max_level(level);
    std::panic::set_hook(Box::new(|info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("unnamed");
        write_line(&format!("{} [{}] CRASH panic on thread '{name}': {info}\n{}\n", stamp(), std::process::id(), std::backtrace::Backtrace::force_capture()));
    }));
    log_start();
    test_crash();
}

/// Test hook: DALERIS_TEST_CRASH=panic or =signal, to check that a crash really reaches the log.
fn test_crash() {
    match std::env::var("DALERIS_TEST_CRASH").as_deref() {
        Ok("panic") => panic!("test panic from DALERIS_TEST_CRASH"),
        #[cfg(unix)]
        // Handlers hand over to the next one and return, which ends things for a real fault as that
        // repeats by itself. A raised signal doesn't, so keep raising it.
        Ok("signal") => loop {
            unsafe {
                libc::raise(libc::SIGSEGV);
            }
        },
        _ => {}
    }
}

/// Fatal signals, such as a segmentation fault in a graphics driver. A signal handler may only do
/// very little, so the log file is reached through a kept file descriptor, with no locks or allocation.
#[cfg(unix)]
mod crash {
    use std::ffi::{c_int, c_void};
    use std::io::IsTerminal;
    use std::os::fd::AsRawFd;
    use std::sync::atomic::{AtomicI32, Ordering};
    use std::sync::OnceLock;

    unsafe extern "C" {
        fn backtrace(buffer: *mut *mut c_void, size: c_int) -> c_int;
        fn backtrace_symbols_fd(buffer: *const *mut c_void, size: c_int, fd: c_int);
    }

    const SIGNALS: [(c_int, &[u8]); 5] =
        [(libc::SIGSEGV, b"SIGSEGV"), (libc::SIGBUS, b"SIGBUS"), (libc::SIGABRT, b"SIGABRT"), (libc::SIGILL, b"SIGILL"), (libc::SIGFPE, b"SIGFPE")];

    struct Previous(Vec<(c_int, libc::sigaction)>);
    // Only written once, before any handler can read it.
    unsafe impl Send for Previous {}
    unsafe impl Sync for Previous {}

    static FD: AtomicI32 = AtomicI32::new(-1);
    static PREVIOUS: OnceLock<Previous> = OnceLock::new();

    fn put(fd: c_int, b: &[u8]) {
        unsafe {
            libc::write(fd, b.as_ptr() as *const c_void, b.len());
        }
    }

    extern "C" fn on_signal(sig: c_int) {
        let fd = FD.load(Ordering::Relaxed);
        put(fd, b"\nCRASH fatal signal ");
        put(fd, SIGNALS.iter().find(|(s, _)| *s == sig).map_or(b"unknown", |(_, n)| *n));
        put(fd, b", native backtrace:\n");
        let mut frames = [std::ptr::null_mut(); 64];
        unsafe {
            let n = backtrace(frames.as_mut_ptr(), frames.len() as c_int);
            backtrace_symbols_fd(frames.as_ptr(), n, fd);
            // Hand over to what was there before; returning repeats the fault, and that ends the process.
            if let Some(prev) = PREVIOUS.get().and_then(|p| p.0.iter().find(|(s, _)| *s == sig)) {
                libc::sigaction(sig, &prev.1, std::ptr::null_mut());
            }
        }
    }

    pub fn install(file: &std::fs::File) {
        let fd = file.as_raw_fd();
        FD.store(fd, Ordering::Relaxed);
        // Started from a desktop, stderr goes nowhere. X11, Wayland and Mesa report fatal errors there.
        if !std::io::stderr().is_terminal() {
            unsafe {
                libc::dup2(fd, libc::STDERR_FILENO);
            }
        }
        let mut frames = [std::ptr::null_mut(); 4];
        unsafe {
            // The first call loads a library, which isn't safe inside a signal handler.
            backtrace(frames.as_mut_ptr(), frames.len() as c_int);
            let mut prev = Vec::with_capacity(SIGNALS.len());
            for (sig, _) in SIGNALS {
                let mut new: libc::sigaction = std::mem::zeroed();
                new.sa_sigaction = on_signal as extern "C" fn(c_int) as usize;
                // Rust's own alternate stack lets this run after a stack overflow, and a crash inside
                // the handler itself falls through to the default action.
                new.sa_flags = libc::SA_ONSTACK | libc::SA_RESETHAND;
                libc::sigemptyset(&mut new.sa_mask);
                let mut old: libc::sigaction = std::mem::zeroed();
                if libc::sigaction(sig, &new, &mut old) == 0 {
                    prev.push((sig, old));
                }
            }
            let _ = PREVIOUS.set(Previous(prev));
        }
    }
}
