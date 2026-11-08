//! Updates from GitHub releases. Every release carries `latest.json`, listing its version and the SHA-256 of each file,
//! and `latest.json.sig`, an Ed25519 signature of that list. Nothing is installed unless the signature matches the key
//! built in here and the file matches its hash. How an update lands depends on how the program was installed:
//!
//! - Installed for one person: installed silently when the program closes.
//! - Installed for the whole machine: a scheduled task runs `--update-service` as SYSTEM and installs it once no copy is
//!   running, so nobody sees an admin prompt.
//! - Portable, AppImage or macOS app: the program replaces itself, which takes effect on the next start.
//! - Linux packages: the person is told, and asked for their password when they choose to install.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};

pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
pub const PROJECT: &str = "https://github.com/MyDrift-user/daleris";
pub const RELEASES: &str = "https://github.com/MyDrift-user/daleris/releases";
const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);
/// Long enough that a check doesn't compete with starting up.
const FIRST_CHECK_AFTER: Duration = Duration::from_secs(20);
/// The public half of the key that signs releases. The private half is a GitHub secret.
#[cfg_attr(feature = "update-test", allow(dead_code))]
const PUBLIC_KEY: [u8; 32] = [0x9c, 0x15, 0x85, 0x04, 0x5a, 0x56, 0x53, 0xf1, 0x50, 0x60, 0xd8, 0x08, 0x7b, 0xdf, 0xa5, 0x9b, 0xc6, 0x92, 0x77, 0x13, 0x17, 0x62, 0x04, 0x65, 0x5c, 0x95, 0x08, 0xd9, 0x18, 0x3a, 0xc8, 0x2b];
/// A second key, kept offline and never used, so a leaked main key can be replaced: a release signed with this one
/// can still reach every copy, and leave the leaked key out from then on.
#[cfg_attr(feature = "update-test", allow(dead_code))]
const BACKUP_PUBLIC_KEY: [u8; 32] = [0xfe, 0x59, 0xb6, 0xe3, 0x3e, 0x36, 0xa4, 0x88, 0xc6, 0x46, 0xd6, 0x66, 0xfa, 0xd4, 0xf7, 0xc7, 0x60, 0x75, 0x84, 0x1c, 0xcc, 0x9b, 0xd3, 0x8d, 0x0e, 0x44, 0x29, 0x0f, 0xb7, 0xe4, 0x0e, 0x04];

/// The `update-test` feature is only for the release workflow's update test, which must not depend on the real key.
/// A program built with it trusts this published key, and is never released: the release job checks that the program
/// it publishes accepts the real signature.
#[cfg(feature = "update-test")]
const TEST_SEED: [u8; 32] = [7; 32];

#[cfg(feature = "update-test")]
fn trusted_keys() -> Vec<[u8; 32]> {
    vec![ed25519_dalek::SigningKey::from_bytes(&TEST_SEED).verifying_key().to_bytes()]
}

#[cfg(not(feature = "update-test"))]
fn trusted_keys() -> Vec<[u8; 32]> {
    vec![PUBLIC_KEY, BACKUP_PUBLIC_KEY]
}

// ---------------------------------------------------------------- versions and the signed list

/// (major, minor, patch), so they compare as numbers. None for anything else.
fn parse_version(v: &str) -> Option<(u32, u32, u32)> {
    let mut parts = v.trim().trim_start_matches('v').split('.');
    let t = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
    parts.next().is_none().then_some(t)
}

fn is_newer(candidate: &str, than: &str) -> bool {
    matches!((parse_version(candidate), parse_version(than)), (Some(a), Some(b)) if a > b)
}

#[derive(Debug, PartialEq)]
struct Manifest {
    version: String,
    /// (file name, SHA-256 in lowercase hex)
    files: Vec<(String, String)>,
}

/// Lines of `version 1.2.3` and `file <sha256> <name>`. Only called on a list whose signature checked out.
fn parse_manifest(text: &str) -> Option<Manifest> {
    let mut version = None;
    let mut files = Vec::new();
    for line in text.lines() {
        match line.split_once(' ') {
            Some(("version", v)) => version = Some(v.trim().to_string()),
            Some(("file", rest)) => {
                let (hash, name) = rest.split_once(' ')?;
                files.push((name.trim().to_string(), hash.trim().to_ascii_lowercase()));
            }
            _ => {}
        }
    }
    let version = version.filter(|v| parse_version(v).is_some())?;
    Some(Manifest { version, files })
}

fn signature_ok(key: &[u8; 32], text: &[u8], sig: &[u8]) -> bool {
    let (Ok(key), Ok(sig)) = (VerifyingKey::from_bytes(key), <[u8; 64]>::try_from(sig)) else { return false };
    key.verify_strict(text, &Signature::from_bytes(&sig)).is_ok()
}

fn verified_manifest(text: &[u8], sig: &[u8]) -> Result<Manifest, String> {
    if !trusted_keys().iter().any(|key| signature_ok(key, text, sig)) {
        return Err("The update's signature doesn't match, so it was ignored".into());
    }
    std::str::from_utf8(text).ok().and_then(parse_manifest).ok_or_else(|| "The update's file list couldn't be read".to_string())
}

fn sha256_of(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = std::io::Read::read(&mut file, &mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Some(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

// ---------------------------------------------------------------- how this copy was installed

#[derive(Clone, Debug, PartialEq)]
pub enum Install {
    /// A setup.exe or MSI install in a folder this person can write to.
    PerUser { msi: bool },
    /// A setup.exe or MSI install for everyone, which the scheduled task keeps up to date.
    Machine { msi: bool },
    /// One file that can be replaced.
    Portable(PathBuf),
    AppImage(PathBuf),
    MacApp(PathBuf),
    /// Installed by a Linux package manager.
    LinuxPackage(LinuxPackager),
    /// Can't be changed from here, such as a plain program in a read-only folder.
    Unmanaged,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LinuxPackager {
    Apt,
    Dnf,
    Zypper,
    Pacman,
}

fn folder_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".daleris-write-test-{}", std::process::id()));
    let ok = std::fs::File::create(&probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

pub fn detect() -> Install {
    let Ok(exe) = std::env::current_exe() else { return Install::Unmanaged };
    let Some(dir) = exe.parent().map(Path::to_path_buf) else { return Install::Unmanaged };
    if cfg!(windows) {
        // the installers add the licence as LICENSE.txt, which the portable program doesn't have next to it
        if dir.join("LICENSE.txt").exists() {
            let msi = !dir.join("unins000.exe").exists();
            // PC-wide installs are marked: being able to write to the folder says nothing, as admins and SYSTEM can
            let machine = dir.join("machine-install.txt").exists() || !folder_writable(&dir);
            return if machine { Install::Machine { msi } } else { Install::PerUser { msi } };
        }
        return if folder_writable(&dir) { Install::Portable(exe) } else { Install::Unmanaged };
    }
    if let Some(image) = std::env::var_os("APPIMAGE").map(PathBuf::from) {
        return Install::AppImage(image);
    }
    if cfg!(target_os = "macos") {
        if let Some(app) = exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")) {
            return Install::MacApp(app.to_path_buf());
        }
    }
    if exe.starts_with("/usr") {
        let has = |tool: &str| ["/usr/bin", "/bin"].iter().any(|d| Path::new(d).join(tool).exists());
        return match () {
            _ if has("apt-get") => Install::LinuxPackage(LinuxPackager::Apt),
            _ if has("zypper") => Install::LinuxPackage(LinuxPackager::Zypper),
            _ if has("dnf") => Install::LinuxPackage(LinuxPackager::Dnf),
            _ if has("pacman") => Install::LinuxPackage(LinuxPackager::Pacman),
            _ => Install::Unmanaged,
        };
    }
    if folder_writable(&dir) {
        Install::Portable(exe)
    } else {
        Install::Unmanaged
    }
}

/// The ending of the release file this install needs, after `daleris-v1.2.3-`.
fn asset_suffix(install: &Install) -> Option<String> {
    let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" };
    Some(match install {
        Install::PerUser { msi: true } | Install::Machine { msi: true } => "windows-x64.msi".into(),
        Install::PerUser { msi: false } | Install::Machine { msi: false } => "windows-x64-setup.exe".into(),
        Install::Portable(_) if cfg!(windows) => "windows-x64-portable.exe".into(),
        Install::Portable(_) if cfg!(target_os = "macos") => format!("macos-{arch}"),
        Install::Portable(_) => format!("linux-{arch}"),
        Install::AppImage(_) => format!("linux-{arch}.AppImage"),
        Install::MacApp(_) => format!("macos-{arch}.dmg"),
        Install::LinuxPackage(LinuxPackager::Apt) => format!("linux-{arch}.deb"),
        Install::LinuxPackage(LinuxPackager::Dnf | LinuxPackager::Zypper) => format!("linux-{arch}.rpm"),
        Install::LinuxPackage(LinuxPackager::Pacman) => format!("linux-{arch}.pkg.tar.zst"),
        Install::Unmanaged => return None,
    })
}

// ---------------------------------------------------------------- downloading

fn hidden(tool: &str) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(tool);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // no console window
    }
    c
}

/// The tools come from the system folder by full path, as this also runs as SYSTEM.
#[cfg(windows)]
fn system_tool(name: &str) -> String {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    format!(r"{root}\System32\{name}.exe")
}

#[cfg(not(windows))]
fn system_tool(name: &str) -> String {
    name.to_string()
}

/// Test hook: DALERIS_TEST_UPDATE_URL points at another server. Everything must still be signed by the real key.
fn base_url(service: bool) -> Option<String> {
    // the update task has no environment of its own to set, so a test build reads the address from a file
    #[cfg(feature = "update-test")]
    if let Ok(url) = std::fs::read_to_string(if cfg!(windows) { r"C:\daleris-update-test.txt" } else { "/tmp/daleris-update-test.txt" }) {
        return Some(url.trim().to_string());
    }
    if service {
        return None;
    }
    std::env::var("DALERIS_TEST_UPDATE_URL").ok()
}

fn download(url: &str, to: &Path, tested: bool) -> Result<(), String> {
    let mut c = hidden(&system_tool("curl"));
    c.args(["-fsSL", "--retry", "2", "--connect-timeout", "20", "-o"]).arg(to).arg(url);
    if !tested {
        c.args(["--proto", "=https", "--proto-redir", "=https"]);
    }
    match c.status() {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("The download failed ({s})")),
        Err(e) => Err(format!("Couldn't run curl: {e}")),
    }
}

fn updates_dir(service: bool) -> Option<PathBuf> {
    // The update task keeps its download in the install folder: only admins can swap a file there before it's installed.
    let dir = if service {
        std::env::current_exe().ok()?.parent()?.join("updates")
    } else {
        crate::util::log_dir()?.join("updates")
    };
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn fetch_manifest(dir: &Path, service: bool) -> Result<Manifest, String> {
    let test = base_url(service);
    let base = test.clone().map(|u| u.trim_end_matches('/').to_string()).unwrap_or_else(|| format!("{RELEASES}/latest/download"));
    let (list, sig) = (dir.join("latest.json"), dir.join("latest.json.sig"));
    download(&format!("{base}/latest.json"), &list, test.is_some())?;
    download(&format!("{base}/latest.json.sig"), &sig, test.is_some())?;
    let read = |p: &Path| std::fs::read(p).map_err(|e| e.to_string());
    verified_manifest(&read(&list)?, &read(&sig)?)
}

/// Downloads the file this install needs and checks it against the signed list.
fn fetch_asset(manifest: &Manifest, install: &Install, dir: &Path, service: bool) -> Result<PathBuf, String> {
    let suffix = asset_suffix(install).ok_or("This copy can't update itself")?;
    let name = format!("daleris-v{}-{suffix}", manifest.version);
    let hash = manifest.files.iter().find(|(n, _)| *n == name).map(|(_, h)| h.as_str()).ok_or_else(|| format!("The release has no {name}"))?;
    let path = dir.join(&name);
    if sha256_of(&path).as_deref() != Some(hash) {
        let test = base_url(service);
        let url = match &test {
            Some(base) => format!("{}/{name}", base.trim_end_matches('/')),
            None => format!("{RELEASES}/download/v{}/{name}", manifest.version),
        };
        download(&url, &path, test.is_some())?;
        if sha256_of(&path).as_deref() != Some(hash) {
            let _ = std::fs::remove_file(&path);
            return Err("The downloaded update didn't match its signed checksum, so it was deleted".into());
        }
    }
    Ok(path)
}

/// Removes what earlier updates left behind.
fn clean_dir(dir: &Path, keep: Option<&Path>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if keep != Some(path.as_path()) && path.is_file() {
            let _ = std::fs::remove_file(path);
        }
    }
}

// ---------------------------------------------------------------- installing

/// What the installers need beyond their switches: an MSI goes to the default folder unless told where this copy is,
/// and neither can show what went wrong except in a log. Quoted by hand, as msiexec doesn't read a quoted `NAME=value`.
/// The log goes next to the installer: setup.exe is 32-bit, so it can't reach SYSTEM's own folder under System32 and
/// gives up when it can't write its log.
#[cfg(windows)]
fn add_installer_args(c: &mut Command, installer: &Path, msi: bool) {
    use std::os::windows::process::CommandExt;
    let log = installer.parent().map(|d| d.join("installer.log"));
    if msi {
        if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
            c.raw_arg(format!("APPLICATIONFOLDER=\"{}\"", dir.display()));
        }
        if let Some(log) = log {
            c.arg("/l*v").arg(log);
        }
    } else if let Some(log) = log {
        c.raw_arg(format!("/LOG=\"{}\"", log.display()));
    }
}

#[cfg(not(windows))]
fn add_installer_args(_: &mut Command, _: &Path, _: bool) {}

fn run_installer(installer: &Path, install: &Install) -> Result<Command, String> {
    let mut c;
    let msi = match install {
        Install::PerUser { msi: true } => {
            c = hidden(&system_tool("msiexec"));
            c.arg("/i").arg(installer).args(["/qn", "/norestart", "ALLUSERS=2", "MSIINSTALLPERUSER=1"]);
            true
        }
        Install::Machine { msi: true } => {
            c = hidden(&system_tool("msiexec"));
            c.arg("/i").arg(installer).args(["/qn", "/norestart", "ALLUSERS=1"]);
            true
        }
        Install::PerUser { msi: false } => {
            c = hidden(&installer.to_string_lossy());
            c.args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/CURRENTUSER"]);
            false
        }
        Install::Machine { msi: false } => {
            c = hidden(&installer.to_string_lossy());
            c.args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/ALLUSERS"]);
            false
        }
        _ => return Err("This copy isn't installed by an installer".into()),
    };
    add_installer_args(&mut c, installer, msi);
    Ok(c)
}

/// Puts `new` where `target` is. A running program's file can be moved aside, but not overwritten.
fn replace_file(target: &Path, new: &Path) -> Result<(), String> {
    let old = target.with_extension("old");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(target, &old).map_err(|e| e.to_string())?;
    if let Err(e) = std::fs::copy(new, target) {
        let _ = std::fs::rename(&old, target);
        return Err(e.to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(target, std::fs::Permissions::from_mode(0o755));
    }
    #[cfg(not(windows))]
    let _ = std::fs::remove_file(&old);
    Ok(())
}

/// Swaps the app in place from a disk image.
fn replace_mac_app(app: &Path, dmg: &Path) -> Result<(), String> {
    let output = hidden("hdiutil").args(["attach", "-nobrowse", "-readonly", "-plist"]).arg(dmg).output().map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    let volume = text.split("<string>").filter_map(|s| s.split("</string>").next()).find(|s| s.starts_with("/Volumes/")).ok_or("Couldn't open the update's disk image")?.to_string();
    let source = Path::new(&volume).join("Daleris.app");
    let fresh = app.with_extension("new");
    let _ = std::fs::remove_dir_all(&fresh);
    let copied = hidden("ditto").arg(&source).arg(&fresh).status().is_ok_and(|s| s.success());
    let _ = hidden("hdiutil").args(["detach", "-quiet"]).arg(&volume).status();
    if !copied {
        return Err("Couldn't copy the update into place".into());
    }
    let old = app.with_extension("old");
    let _ = std::fs::remove_dir_all(&old);
    std::fs::rename(app, &old).map_err(|e| e.to_string())?;
    std::fs::rename(&fresh, app).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}

/// The file moved aside by a self-replacing update can only go once the old program has ended.
pub fn clean_after_update() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("old"));
    }
}

// ---------------------------------------------------------------- the status the settings page shows

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Idle,
    Checking,
    UpToDate,
    /// Found, and waiting for the person to choose to install.
    Available(String),
    Downloading(String),
    /// Installs by itself when the program closes, or from the scheduled task.
    InstallsOnClose(String),
    /// Already in place, for the next start.
    RestartToFinish(String),
    Failed(String),
}

enum Order {
    Check { auto: bool },
    Install,
}

#[derive(Default)]
struct Shared {
    status: Option<Status>,
    /// Stays set, for `run_pending_on_exit`.
    pending_installer: Option<PathBuf>,
}

pub struct Updater {
    shared: Arc<Mutex<Shared>>,
    orders: Sender<Order>,
    pub install: Install,
}

impl Updater {
    /// `auto` is read before each timed check, so turning it off stops the next one.
    pub fn start(ctx: eframe::egui::Context, auto: Arc<std::sync::atomic::AtomicBool>) -> Updater {
        let shared: Arc<Mutex<Shared>> = Arc::default();
        let install = detect();
        log::info!("update: running {CURRENT}, install type {install:?}");
        let (orders, inbox) = channel();
        let worker = Worker { shared: shared.clone(), install: install.clone(), ctx, manifest: None };
        let _ = std::thread::Builder::new().name("updater".into()).spawn(move || {
            let mut worker = worker;
            let mut wait = FIRST_CHECK_AFTER;
            loop {
                match inbox.recv_timeout(wait) {
                    Ok(Order::Check { auto }) => worker.check(auto),
                    Ok(Order::Install) => worker.install_now(),
                    Err(RecvTimeoutError::Timeout) => {
                        wait = CHECK_EVERY;
                        if auto.load(std::sync::atomic::Ordering::Relaxed) {
                            worker.check(true);
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
        });
        Updater { shared, orders, install }
    }

    pub fn status(&self) -> Status {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner).status.clone().unwrap_or(Status::Idle)
    }

    pub fn check_now(&self, auto: bool) {
        let _ = self.orders.send(Order::Check { auto });
    }

    pub fn install_now(&self) {
        let _ = self.orders.send(Order::Install);
    }

    /// Starts a per-user installer once the window is gone, as it can't replace a running program.
    pub fn run_pending_on_exit(&self) {
        install_pending(&self.shared, &self.install);
    }

    pub fn fail(&self, message: String) {
        log::warn!("update: {message}");
        self.shared.lock().unwrap_or_else(PoisonError::into_inner).status = Some(Status::Failed(message));
    }
}

fn install_pending(shared: &Mutex<Shared>, install: &Install) {
    let pending = shared.lock().unwrap_or_else(PoisonError::into_inner).pending_installer.take();
    let Some(installer) = pending else { return };
    if crate::policy::current().no_updates {
        return log::info!("update: updates are managed by the administrator, so nothing is installed");
    }
    // the installer can't replace a program that's running: the last window to close installs it
    if another_copy_running() {
        return log::info!("update: another window is still open, so the update waits");
    }
    match run_installer(&installer, install).and_then(|mut c| c.spawn().map_err(|e| e.to_string())) {
        Ok(_) => log::info!("update: installing {}", installer.display()),
        Err(e) => log::error!("update: couldn't start the installer: {e}"),
    }
}

struct Worker {
    shared: Arc<Mutex<Shared>>,
    install: Install,
    ctx: eframe::egui::Context,
    manifest: Option<Manifest>,
}

impl Worker {
    fn set(&self, status: Status) {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner).status = Some(status);
        self.ctx.request_repaint();
    }

    fn fail(&self, message: String) {
        log::warn!("update: {message}");
        self.set(Status::Failed(message));
    }

    fn check(&mut self, auto: bool) {
        if crate::policy::current().no_updates {
            return self.set(Status::Idle);
        }
        self.set(Status::Checking);
        let Some(dir) = updates_dir(false) else { return self.fail("There's no folder to download updates to".into()) };
        match fetch_manifest(&dir, false) {
            Err(e) => self.fail(e),
            Ok(manifest) if !is_newer(&manifest.version, CURRENT) => {
                log::info!("update: {CURRENT} is current, the newest release is {}", manifest.version);
                self.set(Status::UpToDate);
                clean_dir(&dir, None);
            }
            Ok(manifest) => {
                log::info!("update: {} is available", manifest.version);
                let version = manifest.version.clone();
                self.manifest = Some(manifest);
                match &self.install {
                    // the scheduled task does this without anyone here being asked
                    Install::Machine { .. } if machine_may_update(&crate::policy::current(), machine_switch()) => self.set(Status::InstallsOnClose(version)),
                    Install::Machine { .. } | Install::LinuxPackage(_) | Install::Unmanaged => self.set(Status::Available(version)),
                    _ if auto => self.install_now(),
                    _ => self.set(Status::Available(version)),
                }
            }
        }
    }

    fn install_now(&mut self) {
        if crate::policy::current().no_updates {
            return self.set(Status::Idle);
        }
        let Some(manifest) = self.manifest.take() else { return };
        let version = manifest.version.clone();
        let Some(dir) = updates_dir(false) else { return self.fail("There's no folder to download updates to".into()) };
        self.set(Status::Downloading(version.clone()));
        let path = match fetch_asset(&manifest, &self.install, &dir, false) {
            Ok(path) => path,
            Err(e) => return self.fail(e),
        };
        clean_dir(&dir, Some(&path));
        let result = match &self.install {
            Install::PerUser { .. } => {
                self.shared.lock().unwrap_or_else(PoisonError::into_inner).pending_installer = Some(path.clone());
                return self.set(Status::InstallsOnClose(version));
            }
            Install::Portable(target) | Install::AppImage(target) => replace_file(target, &path),
            Install::MacApp(app) => replace_mac_app(app, &path),
            Install::LinuxPackage(packager) => install_linux_package(*packager, &path),
            Install::Machine { .. } | Install::Unmanaged => Err("This copy can't update itself".to_string()),
        };
        match result {
            Ok(()) => {
                log::info!("update: {version} is in place");
                self.set(Status::RestartToFinish(version));
            }
            Err(e) => self.fail(format!("Couldn't install the update: {e}")),
        }
    }
}

/// Asks for the password through the desktop's own dialog.
fn install_linux_package(packager: LinuxPackager, package: &Path) -> Result<(), String> {
    let mut c = Command::new("pkexec");
    match packager {
        LinuxPackager::Apt => c.args(["apt-get", "install", "-y"]),
        LinuxPackager::Dnf => c.args(["dnf", "install", "-y"]),
        LinuxPackager::Zypper => c.args(["zypper", "-n", "--no-gpg-checks", "in", "--allow-unsigned-rpm"]),
        LinuxPackager::Pacman => c.args(["pacman", "-U", "--noconfirm"]),
    };
    let status = c.arg(package).status().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("the package manager reported {status}"))
    }
}

// ---------------------------------------------------------------- the scheduled task, and other command line runs

/// The settings page's switch for a machine install, as the task can't read anyone's settings: 0 or 1 in this file.
#[cfg(windows)]
fn machine_switch_file() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("ProgramData")?).join("Daleris").join("auto-update.txt"))
}

#[cfg(not(windows))]
fn machine_switch_file() -> Option<PathBuf> {
    None
}

/// What the switch says, if anyone has used it.
pub fn machine_switch() -> Option<bool> {
    let text = std::fs::read_to_string(machine_switch_file()?).ok()?;
    match text.trim() {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    }
}

/// Writing it needs no admin rights, but only the account that first wrote it, or an admin, can change it.
pub fn set_machine_switch(on: bool) -> Result<(), String> {
    let Some(file) = machine_switch_file() else { return Ok(()) };
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(&file, if on { "1" } else { "0" }).map_err(|_| "Another account set this for the whole PC. That account or an administrator can change it".to_string())
}

/// Whether the task installs updates: an administrator's policy, then the switch, then an administrator's default.
fn machine_may_update(policy: &crate::policy::Policy, switch: Option<bool>) -> bool {
    if policy.no_updates {
        return false;
    }
    if let Some(v) = policy.fixed_value("auto_update") {
        return v == "1";
    }
    switch.unwrap_or_else(|| policy.recommended_value("auto_update") != Some("0"))
}

#[cfg(windows)]
fn another_copy_running() -> bool {
    let output = hidden(&system_tool("tasklist")).args(["/FI", "IMAGENAME eq daleris.exe", "/NH", "/FO", "CSV"]).output();
    // this process is one of them
    output.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).matches("daleris.exe").count() > 1)
}

#[cfg(not(windows))]
fn another_copy_running() -> bool {
    false
}

/// What the scheduled task runs. Nobody is watching, so it only logs.
fn run_service() -> i32 {
    let install = detect();
    log::info!("update service: running {CURRENT}, install type {install:?}");
    if !matches!(install, Install::Machine { .. }) {
        log::warn!("update service: this isn't a machine install");
        return 1;
    }
    if !machine_may_update(&crate::policy::current(), machine_switch()) {
        log::info!("update service: automatic updates are off");
        return 0;
    }
    let Some(dir) = updates_dir(true) else { return 1 };
    let manifest = match fetch_manifest(&dir, true) {
        Ok(m) => m,
        Err(e) => {
            log::warn!("update service: {e}");
            return 1;
        }
    };
    if !is_newer(&manifest.version, CURRENT) {
        clean_dir(&dir, None);
        return 0;
    }
    if another_copy_running() {
        log::info!("update service: {} is waiting for Daleris to close", manifest.version);
        return 0;
    }
    let path = match fetch_asset(&manifest, &install, &dir, true) {
        Ok(p) => p,
        Err(e) => {
            log::warn!("update service: {e}");
            return 1;
        }
    };
    log::info!("update service: installing {}", manifest.version);
    // Not waiting for it, as this program is one of the files it replaces. It writes installer.log next to itself.
    match run_installer(&path, &install).and_then(|mut c| c.spawn().map_err(|e| e.to_string())) {
        Ok(_) => 0,
        Err(e) => {
            log::error!("update service: {e}");
            1
        }
    }
}

/// `--update-now`: everything the program does about an update, without a window, for scripts. A machine install
/// leaves the installing to the task, and asks for no rights it doesn't have.
fn run_now() -> i32 {
    if crate::policy::current().no_updates {
        println!("Updates are managed by the administrator");
        return 0;
    }
    let install = detect();
    let shared: Arc<Mutex<Shared>> = Arc::default();
    let mut worker = Worker { shared: shared.clone(), install: install.clone(), ctx: eframe::egui::Context::default(), manifest: None };
    worker.check(true);
    if matches!(install, Install::LinuxPackage(_)) && worker.manifest.is_some() {
        worker.install_now();
    }
    let status = shared.lock().unwrap_or_else(PoisonError::into_inner).status.clone().unwrap_or(Status::Idle);
    println!("{status:?}");
    install_pending(&shared, &install);
    i32::from(matches!(status, Status::Failed(_)))
}

/// Runs that do their job without a window. Returns the exit code if this was one.
pub fn handle_command_line() -> Option<i32> {
    let args: Vec<String> = std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    match args.first().map(String::as_str) {
        Some("--version") => {
            println!("Daleris {CURRENT}");
            log::info!("--version: {CURRENT}");
            Some(0)
        }
        Some("--update-service") => Some(run_service()),
        Some("--update-now") => Some(run_now()),
        // for the update test: signs a list with the test build's published key
        #[cfg(feature = "update-test")]
        Some("--test-sign") => {
            use ed25519_dalek::Signer;
            let text = std::fs::read(args.get(1)?).ok()?;
            let signature = ed25519_dalek::SigningKey::from_bytes(&TEST_SEED).sign(&text);
            std::fs::write(args.get(2)?, signature.to_bytes()).ok()?;
            Some(0)
        }
        // for the release workflow: is this list signed by the key built into this program?
        Some("--verify-manifest") => {
            let read = |i: usize| args.get(i).and_then(|p| std::fs::read(p).ok()).unwrap_or_default();
            match verified_manifest(&read(1), &read(2)) {
                Ok(m) => {
                    println!("signature good: version {}, {} files", m.version, m.files.len());
                    Some(0)
                }
                Err(e) => {
                    eprintln!("{e}");
                    Some(1)
                }
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 8032, section 7.1, test 2.
    const KEY: [u8; 32] = [
        0x3d, 0x40, 0x17, 0xc3, 0xe8, 0x43, 0x89, 0x5a, 0x92, 0xb7, 0x0a, 0xa7, 0x4d, 0x1b, 0x7e, 0xbc, 0x9c, 0x98, 0x2c, 0xcf, 0x2e, 0xc4, 0x96, 0x8c, 0xc0, 0xcd, 0x55, 0xf1, 0x2a, 0xf4, 0x66, 0x0c,
    ];
    const SIG: [u8; 64] = [
        0x92, 0xa0, 0x09, 0xa9, 0xf0, 0xd4, 0xca, 0xb8, 0x72, 0x0e, 0x82, 0x0b, 0x5f, 0x64, 0x25, 0x40, 0xa2, 0xb2, 0x7b, 0x54, 0x16, 0x50, 0x3f, 0x8f, 0xb3, 0x76, 0x22, 0x23, 0xeb, 0xdb, 0x69, 0xda, 0x08, 0x5a, 0xc1, 0xe4, 0x3e, 0x15, 0x99, 0x6e, 0x45, 0x8f, 0x36, 0x13, 0xd0, 0xf1, 0x1d, 0x8c, 0x38, 0x7b, 0x2e, 0xae, 0xb4, 0x30, 0x2a, 0xee, 0xb0, 0x0d, 0x29, 0x16, 0x12, 0xbb, 0x0c, 0x00,
    ];

    #[test]
    fn a_good_signature_passes_and_anything_changed_fails() {
        assert!(signature_ok(&KEY, &[0x72], &SIG));
        assert!(!signature_ok(&KEY, &[0x73], &SIG), "a changed message");
        let mut bad = SIG;
        bad[0] ^= 1;
        assert!(!signature_ok(&KEY, &[0x72], &bad), "a changed signature");
        assert!(!signature_ok(&KEY, &[0x72], &SIG[..63]), "a short signature");
        assert!(!signature_ok(&PUBLIC_KEY, &[0x72], &SIG), "another key");
    }

    #[test]
    fn the_real_key_is_a_valid_key() {
        assert!(VerifyingKey::from_bytes(&PUBLIC_KEY).is_ok());
        assert!(VerifyingKey::from_bytes(&BACKUP_PUBLIC_KEY).is_ok());
        assert_ne!(PUBLIC_KEY, BACKUP_PUBLIC_KEY);
    }

    #[cfg(not(feature = "update-test"))]
    #[test]
    fn the_real_program_refuses_the_test_builds_key() {
        use ed25519_dalek::Signer;
        let text = b"version 99.0.0\n";
        let signature = ed25519_dalek::SigningKey::from_bytes(&[7; 32]).sign(text);
        assert!(verified_manifest(text, &signature.to_bytes()).is_err());
    }

    #[test]
    fn administrators_decide_before_the_switch_and_the_switch_before_their_default() {
        use crate::policy::Policy;
        let may = machine_may_update;
        let fixed = |v: &str| Policy { fixed: vec![("auto_update", v.to_string())], ..Default::default() };
        let default = |v: &str| Policy { recommended: vec![("auto_update", v.to_string())], ..Default::default() };
        assert!(may(&Policy::default(), None), "on unless someone says otherwise");
        assert!(!may(&Policy::default(), Some(false)));
        assert!(may(&fixed("1"), Some(false)), "a fixed on beats the switch");
        assert!(!may(&fixed("0"), Some(true)), "a fixed off beats the switch");
        assert!(!may(&default("0"), None));
        assert!(may(&default("0"), Some(true)), "someone's choice beats a default");
        assert!(!may(&Policy { no_updates: true, ..fixed("1") }, Some(true)), "turned off altogether");
    }

    #[test]
    fn unsigned_lists_are_refused() {
        assert!(verified_manifest(b"version 99.0.0\n", &[0; 64]).is_err());
    }

    #[test]
    fn versions_compare_as_numbers() {
        assert!(is_newer("0.0.10", "0.0.9"));
        assert!(is_newer("0.1.0", "0.0.99"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(is_newer("v0.0.2", "0.0.1"));
        assert!(!is_newer("0.0.1", "0.0.1"), "the same version");
        assert!(!is_newer("0.0.1", "0.0.2"), "older");
        assert!(!is_newer("nonsense", "0.0.1"));
        assert!(!is_newer("1.2", "0.0.1"));
    }

    #[test]
    fn lists_are_read_and_rubbish_is_refused() {
        let m = parse_manifest("version 1.2.3\nfile ABC123 daleris-v1.2.3-windows-x64.msi\nfile def456 name with spaces.zip\n").expect("reads");
        assert_eq!(m.version, "1.2.3");
        assert_eq!(m.files[0], ("daleris-v1.2.3-windows-x64.msi".to_string(), "abc123".to_string()));
        assert_eq!(m.files[1].0, "name with spaces.zip");
        assert!(parse_manifest("file abc name\n").is_none(), "no version");
        assert!(parse_manifest("version soon\n").is_none(), "a version that isn't one");
    }
}
