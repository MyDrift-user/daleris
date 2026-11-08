//! Linux & macOS system integration: opening files, the trash, copy/move, the
//! clipboard, picture thumbnails, terminals, folder pickers, and connecting to
//! network servers (SMB, SFTP, FTP, WebDAV, NFS): through GVFS (`gio`) on Linux
//! and the built-in network file systems on macOS.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use crate::fscommon::KfId;

/// No COM on Linux/macOS.
pub fn init_com() {}

/// Linux/macOS have no shared "Properties" dialog to hand over to.
pub const HAS_NATIVE_PROPERTIES: bool = false;

static CTX: OnceLock<eframe::egui::Context> = OnceLock::new();

fn spawn_detached(cmd: &mut Command) -> bool {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok()
}

fn has(tool: &str) -> bool {
    Command::new("sh").arg("-c").arg(format!("command -v {tool}")).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

// ---------------------------------------------------------------- images

pub struct RawImg {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
    pub premultiplied: bool,
}

#[derive(Clone)]
pub enum ImgReq {
    #[allow(dead_code)]
    Icon { key: String, path: String, by_attrs: bool, is_dir: bool, px: u32 },
    Thumb { key: String, path: PathBuf, size: i32, icon_only: bool, #[allow(dead_code)] preview_only: bool },
    #[allow(dead_code)]
    KnownFolder { key: String, id: KfId, px: u32 },
}

impl ImgReq {
    fn key(&self) -> &str {
        match self {
            ImgReq::Icon { key, .. } | ImgReq::Thumb { key, .. } | ImgReq::KnownFolder { key, .. } => key,
        }
    }
}

pub struct ImgResult {
    pub key: String,
    pub img: Option<RawImg>,
    pub type_name: Option<String>,
}

type Queue = Arc<(Mutex<Vec<ImgReq>>, Condvar)>;

/// Background image loader. File icons are drawn by the app itself on Linux/macOS
/// (the built-in flat set); pictures get real thumbnails decoded here.
pub struct Loader {
    queue: Queue,
    pub rx: Receiver<ImgResult>,
}

impl Loader {
    pub fn new(ctx: eframe::egui::Context) -> Self {
        let _ = CTX.set(ctx.clone());
        let (tx, rx) = channel();
        let queue: Queue = Arc::new((Mutex::new(Vec::new()), Condvar::new()));
        for i in 0..3 {
            spawn_worker(i, queue.clone(), tx.clone(), ctx.clone());
        }
        Loader { queue, rx }
    }

    pub fn request(&self, r: ImgReq) {
        let mut v = self.queue.0.lock().unwrap();
        v.push(r);
        self.queue.1.notify_one();
    }

    pub fn clear_thumbs(&self) -> Vec<String> {
        let mut v = self.queue.0.lock().unwrap();
        let (thumbs, rest): (Vec<ImgReq>, Vec<ImgReq>) = v.drain(..).partition(|r| matches!(r, ImgReq::Thumb { .. }));
        *v = rest;
        thumbs.iter().map(|r| r.key().to_string()).collect()
    }
}

fn spawn_worker(i: usize, q: Queue, tx: Sender<ImgResult>, ctx: eframe::egui::Context) {
    std::thread::Builder::new()
        .name(format!("img{i}"))
        .spawn(move || loop {
            let req = {
                let mut v = q.0.lock().unwrap();
                while v.is_empty() {
                    v = q.1.wait(v).unwrap();
                }
                v.pop().unwrap()
            };
            let res = match &req {
                ImgReq::Thumb { key, path, size, icon_only, .. } => ImgResult { key: key.clone(), img: if *icon_only { None } else { thumbnail(path, *size as u32) }, type_name: None },
                other => ImgResult { key: other.key().to_string(), img: None, type_name: None },
            };
            if tx.send(res).is_err() {
                return;
            }
            ctx.request_repaint();
        })
        .ok();
}

/// A picture file at about `size` pixels, such as a person's account picture.
pub fn picture(path: &Path, size: u32) -> Option<RawImg> {
    thumbnail(path, size)
}

fn thumbnail(path: &Path, size: u32) -> Option<RawImg> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    if !matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp") {
        return None;
    }
    if std::fs::metadata(path).ok()?.len() > 80 * 1024 * 1024 {
        return None;
    }
    let img = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok()?;
    let t = img.thumbnail(size.max(16), size.max(16)).to_rgba8();
    let (w, h) = (t.width() as usize, t.height() as usize);
    Some(RawImg { w, h, rgba: t.into_raw(), premultiplied: false })
}

// ---------------------------------------------------------------- simple verbs

/// Open with the default app (xdg-open / open).
/// Opens a web page in the default browser.
pub fn open_url(url: &str) {
    open(Path::new(url));
}

/// Windows only: there's no such place on Linux or macOS.
pub fn credential_manager() {}

/// Starts a program with these arguments in `dir`. The New menu only offers templates here, so this is rarely used.
pub fn run_program(program: &str, args: &str, dir: &Path) {
    let _ = std::process::Command::new(program).args(args.split_whitespace()).current_dir(dir).spawn();
}

pub fn open(path: &Path) {
    let tool = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    spawn_detached(Command::new(tool).arg(path));
}

/// macOS lets you pick the app; on Linux this opens with the default app.
pub fn open_with(path: &Path) {
    if cfg!(target_os = "macos") {
        let script = "POSIX path of (choose application as alias)";
        if let Ok(out) = Command::new("osascript").arg("-e").arg(script).output() {
            let app = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !app.is_empty() {
                spawn_detached(Command::new("open").arg("-a").arg(app).arg(path));
            }
        }
    } else if !(has("gio") && spawn_detached(Command::new("gio").arg("open").arg(path))) {
        open(path);
    }
}

pub fn run_as_admin(path: &Path) {
    if cfg!(target_os = "macos") {
        let p = path.display().to_string().replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!("do shell script quoted form of \"{p}\" with administrator privileges");
        spawn_detached(Command::new("osascript").arg("-e").arg(script));
    } else {
        spawn_detached(Command::new("pkexec").arg(path));
    }
}

/// No system Properties dialog on Linux/macOS, so the app's own window is used.
pub fn properties(_path: &Path) {}

pub fn default_app(_ext: &str) -> Option<String> {
    None
}

pub fn open_terminal(dir: &Path) {
    if cfg!(target_os = "macos") {
        spawn_detached(Command::new("open").arg("-a").arg("Terminal").arg(dir));
        return;
    }
    if let Ok(t) = std::env::var("TERMINAL") {
        if spawn_detached(Command::new(t).current_dir(dir)) {
            return;
        }
    }
    let d = dir.display().to_string();
    let tries: [(&str, Vec<String>); 10] = [
        ("x-terminal-emulator", vec![]),
        ("gnome-terminal", vec![format!("--working-directory={d}")]),
        ("ptyxis", vec!["--new-window".into(), "--working-directory".into(), d.clone()]),
        ("konsole", vec!["--workdir".into(), d.clone()]),
        ("xfce4-terminal", vec![format!("--working-directory={d}")]),
        ("kitty", vec!["--directory".into(), d.clone()]),
        ("alacritty", vec!["--working-directory".into(), d.clone()]),
        ("wezterm", vec!["start".into(), "--cwd".into(), d.clone()]),
        ("foot", vec!["-D".into(), d.clone()]),
        ("xterm", vec![]),
    ];
    for (tool, args) in tries {
        if has(tool) && spawn_detached(Command::new(tool).args(&args).current_dir(dir)) {
            return;
        }
    }
}

/// System folder picker: zenity / kdialog on Linux, the Finder chooser on macOS.
pub fn pick_folder() -> Option<PathBuf> {
    let out = if cfg!(target_os = "macos") {
        Command::new("osascript").arg("-e").arg("POSIX path of (choose folder)").output().ok()?
    } else if has("zenity") {
        Command::new("zenity").args(["--file-selection", "--directory"]).output().ok()?
    } else if has("kdialog") {
        Command::new("kdialog").arg("--getexistingdirectory").arg(std::env::var("HOME").unwrap_or_default()).output().ok()?
    } else {
        return None;
    };
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !s.is_empty()).then(|| PathBuf::from(s))
}

/// Symbolic links point somewhere else; resolve them.
pub fn shortcut_target(lnk: &Path) -> Option<PathBuf> {
    std::fs::read_link(lnk).ok().map(|t| if t.is_relative() { lnk.parent().unwrap_or(Path::new("/")).join(t) } else { t })
}

// ---------------------------------------------------------------- network servers

/// Mount a server address (smb://server/share, sftp://host/path, ftp://, dav(s)://, nfs://)
/// and report where it appeared on disk. Runs on a worker thread.
pub fn connect_server(url: String, user: String, pass: String, done: impl FnOnce(Result<PathBuf, String>) + Send + 'static) {
    crate::bg::spawn("mount", move || done(mount_url(&url, &user, &pass)));
}

#[cfg(target_os = "linux")]
fn mount_url(url: &str, user: &str, pass: &str) -> Result<PathBuf, String> {
    use std::io::Write;
    if !has("gio") {
        return Err("Connecting to servers needs GVFS (the `gio` tool), which GNOME, Cinnamon, MATE and Xfce include".into());
    }
    let uid = unsafe { libc::getuid() };
    let gvfs = PathBuf::from(format!("/run/user/{uid}/gvfs"));
    let before: Vec<PathBuf> = std::fs::read_dir(&gvfs).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    let mut child = Command::new("gio").arg("mount").arg(url).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        // answers to gio's prompts: user, domain (default), password
        let _ = writeln!(stdin, "{user}\n\n{pass}");
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let err = String::from_utf8_lossy(&out.stderr).to_string();
    if !out.status.success() && !err.contains("already mounted") {
        return Err(err.lines().last().unwrap_or("Couldn't connect").trim().to_string());
    }
    let host = url.split("://").nth(1).unwrap_or("").split(['/', ':']).next().unwrap_or("").rsplit('@').next().unwrap_or("").to_lowercase();
    let after: Vec<PathBuf> = std::fs::read_dir(&gvfs).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    after
        .iter()
        .find(|p| !before.contains(p))
        .or_else(|| after.iter().find(|p| p.to_string_lossy().to_lowercase().contains(&host)))
        .cloned()
        .ok_or_else(|| "Connected, but the share's folder wasn't found".into())
}

#[cfg(target_os = "macos")]
fn mount_url(url: &str, user: &str, pass: &str) -> Result<PathBuf, String> {
    let lower = url.to_lowercase();
    if lower.starts_with("sftp://") || lower.starts_with("ssh://") {
        return Err("macOS can't mount SFTP by itself. Install macFUSE and sshfs, then mount it and it will show up here".into());
    }
    let before: Vec<PathBuf> = std::fs::read_dir("/Volumes").map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    let q = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let mut script = format!("mount volume \"{}\"", q(url));
    if !user.is_empty() {
        script += &format!(" as user name \"{}\" with password \"{}\"", q(user), q(pass));
    }
    let out = Command::new("osascript").arg("-e").arg(script).output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let after: Vec<PathBuf> = std::fs::read_dir("/Volumes").map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    let share = url.trim_end_matches('/').rsplit('/').next().unwrap_or("").to_lowercase();
    after
        .iter()
        .find(|p| !before.contains(p))
        .or_else(|| after.iter().find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase() == share)))
        .cloned()
        .ok_or_else(|| "Connected, but the share's folder wasn't found".into())
}

/// Disconnect / unmount (network shares, FUSE mounts, removable media).
pub fn unmount(root: &Path) -> Result<(), String> {
    let tries: Vec<(&str, Vec<&std::ffi::OsStr>)> = if cfg!(target_os = "macos") {
        vec![("diskutil", vec!["unmount".as_ref(), root.as_os_str()]), ("umount", vec![root.as_os_str()])]
    } else {
        vec![
            ("gio", vec!["mount".as_ref(), "-u".as_ref(), root.as_os_str()]),
            ("fusermount3", vec!["-u".as_ref(), root.as_os_str()]),
            ("fusermount", vec!["-u".as_ref(), root.as_os_str()]),
            ("umount", vec![root.as_os_str()]),
        ]
    };
    for (tool, args) in tries {
        if Command::new(tool).args(&args).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success()) {
            return Ok(());
        }
    }
    Err(format!("Couldn't disconnect {}", root.display()))
}

// ---------------------------------------------------------------- file operations

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    Copy,
    Move,
    Recycle,
}

/// "name (2).ext" when the name is taken.
fn free_name(dest: &Path, name: &std::ffi::OsStr) -> PathBuf {
    let first = dest.join(name);
    if std::fs::symlink_metadata(&first).is_err() {
        return first;
    }
    let p = Path::new(name);
    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (2..).map(|k| dest.join(format!("{stem} ({k}){ext}"))).find(|c| std::fs::symlink_metadata(c).is_err()).unwrap()
}

fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    let md = std::fs::symlink_metadata(src)?;
    if md.file_type().is_symlink() {
        std::os::unix::fs::symlink(std::fs::read_link(src)?, dst)
    } else if md.is_dir() {
        std::fs::create_dir(dst)?;
        for e in std::fs::read_dir(src)? {
            let e = e?;
            copy_tree(&e.path(), &dst.join(e.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(src, dst).map(|_| ())
    }
}

fn remove_tree(p: &Path) -> std::io::Result<()> {
    let md = std::fs::symlink_metadata(p)?;
    if md.is_dir() {
        std::fs::remove_dir_all(p)
    } else {
        std::fs::remove_file(p)
    }
}

fn trash(paths: &[PathBuf]) -> Result<(), String> {
    if cfg!(target_os = "macos") {
        // ~/.Trash, like Finder's "Move to Bin"
        let bin = PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".Trash");
        for p in paths {
            let name = p.file_name().ok_or("bad path")?;
            let to = free_name(&bin, name);
            std::fs::rename(p, &to).or_else(|_| copy_tree(p, &to).and_then(|_| remove_tree(p))).map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    // Linux: GVFS knows every trash location (also for other drives); else the freedesktop.org layout
    if has("gio") && Command::new("gio").arg("trash").args(paths).status().is_ok_and(|s| s.success()) {
        return Ok(());
    }
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share"));
    let (files, info) = (data.join("Trash/files"), data.join("Trash/info"));
    std::fs::create_dir_all(&files).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&info).map_err(|e| e.to_string())?;
    let st = crate::util::filetime_local(crate::util::now_filetime());
    for p in paths {
        let name = p.file_name().ok_or("bad path")?;
        let to = free_name(&files, name);
        let base = to.file_name().unwrap().to_string_lossy().to_string();
        let stamp = format!("{:04}-{:02}-{:02}T{:02}:{:02}:00", st.year, st.month, st.day, st.hour, st.minute);
        let abs = std::fs::canonicalize(p).unwrap_or(p.clone());
        std::fs::write(info.join(format!("{base}.trashinfo")), format!("[Trash Info]\nPath={}\nDeletionDate={stamp}\n", abs.display())).map_err(|e| e.to_string())?;
        std::fs::rename(p, &to).or_else(|_| copy_tree(p, &to).and_then(|_| remove_tree(p))).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Copy / move / trash / delete on a worker thread. Names that are taken get " (2)".
pub fn file_op(op: Op, src: Vec<PathBuf>, dest: Option<PathBuf>, done: impl FnOnce(Result<(), String>) + Send + 'static) {
    crate::bg::spawn("shell-op", move || {
        let r = (|| -> Result<(), String> {
            match op {
                Op::Recycle => trash(&src),
                Op::Copy | Op::Move => {
                    let dest = dest.ok_or("no destination")?;
                    for p in &src {
                        let name = p.file_name().ok_or("bad path")?;
                        let to = free_name(&dest, name);
                        let res = if op == Op::Copy {
                            copy_tree(p, &to)
                        } else {
                            // a rename across drives fails: copy, then remove
                            std::fs::rename(p, &to).or_else(|_| copy_tree(p, &to).and_then(|_| remove_tree(p)))
                        };
                        res.map_err(|e| format!("{}: {e}", p.display()))?;
                    }
                    Ok(())
                }
            }
        })();
        done(r);
    });
}

// ---------------------------------------------------------------- clipboard

/// Files copied/cut in this app, and the text we last put on the system clipboard.
static CLIP: Mutex<(Vec<PathBuf>, bool, String)> = Mutex::new((Vec::new(), false, String::new()));
/// Text that arrived with the last Ctrl/Cmd+V (may hold paths from another file manager).
static OFFERED: Mutex<Option<String>> = Mutex::new(None);

pub fn clipboard_set_files(paths: &[PathBuf], cut: bool) -> bool {
    // also as plain text, so pasting into a terminal or editor gives the paths
    let text = paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n");
    *CLIP.lock().unwrap() = (paths.to_vec(), cut, text.clone());
    clipboard_set_text(&text);
    true
}

pub fn clipboard_set_text(text: &str) {
    if let Some(ctx) = CTX.get() {
        ctx.copy_text(text.to_string());
    }
}

/// Remember the text of a paste event (see `clipboard_get_files`).
pub fn clipboard_offer_text(text: &str) {
    *OFFERED.lock().unwrap() = Some(text.to_string());
}

/// Paths in clipboard text: GNOME's "copy\nfile:///…" lists, file:// URIs, or plain paths.
fn paths_from_text(text: &str) -> Option<(Vec<PathBuf>, bool)> {
    let mut cut = false;
    let mut out = Vec::new();
    for (i, line) in text.lines().map(str::trim).filter(|l| !l.is_empty()).enumerate() {
        if i == 0 && (line == "copy" || line == "cut") {
            cut = line == "cut";
            continue;
        }
        let p = match line.strip_prefix("file://") {
            Some(rest) => PathBuf::from(percent_decode(rest)),
            None => PathBuf::from(line),
        };
        if !p.is_absolute() || std::fs::symlink_metadata(&p).is_err() {
            return None;
        }
        out.push(p);
    }
    (!out.is_empty()).then_some((out, cut))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            // by bytes: text from the clipboard can have a multi-byte character right after the %
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

pub fn clipboard_get_files() -> Option<(Vec<PathBuf>, bool)> {
    let offered = OFFERED.lock().unwrap().take();
    let mut clip = CLIP.lock().unwrap();
    if let Some(text) = offered {
        if text != clip.2 {
            if let Some(found) = paths_from_text(&text) {
                return Some(found);
            }
        }
    }
    if clip.0.is_empty() {
        return None;
    }
    let r = (clip.0.clone(), clip.1);
    if clip.1 {
        *clip = (Vec::new(), false, String::new()); // a cut is pasted once
    }
    Some(r)
}

pub fn clipboard_has_files() -> bool {
    !CLIP.lock().unwrap().0.is_empty()
}

// ---------------------------------------------------------------- context menu

/// There is no shared, extensible right-click menu on Linux/macOS: the app's own items are used.
pub enum MenuOutcome {
    Nothing,
    #[allow(dead_code)]
    Invoked,
    #[allow(dead_code)]
    Rename,
}

pub fn native_menu(_items: &[PathBuf], _dir: &Path) -> MenuOutcome {
    MenuOutcome::Nothing
}

pub struct ShellEntry {
    pub id: u32,
    pub label: String,
    pub key: String,
    pub icon: Option<RawImg>,
    pub children: Vec<ShellEntry>,
    pub separator: bool,
    pub disabled: bool,
    pub default: bool,
}

pub struct ShellMenu {
    pub entries: Vec<ShellEntry>,
    pub built: std::time::Instant,
}

impl ShellMenu {
    pub fn build(_items: &[PathBuf], _dir: &Path, _extended: bool) -> Option<ShellMenu> {
        None
    }
    pub fn late_icons(&self) -> Vec<(u32, RawImg)> {
        Vec::new()
    }
    pub fn invoke(&self, _id: u32) -> MenuOutcome {
        MenuOutcome::Nothing
    }
}

pub fn menu_key(verb: &str, label: &str) -> String {
    if !verb.is_empty() {
        return format!("v:{}", verb.to_lowercase());
    }
    format!("l:{}", clean_label(label).to_lowercase().trim_end_matches("...").trim_end_matches('…').trim())
}

pub fn clean_label(s: &str) -> String {
    let s = s.split('\t').next().unwrap_or("").replace("&&", "\u{1}").replace('&', "").replace('\u{1}', "&").trim().to_string();
    crate::fscommon::undash_text(&s)
}

/// Mica is a Windows 11 material.
pub fn has_mica() -> bool {
    false
}
