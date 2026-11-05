//! Everything the UI asks for is queued as an `Act` and carried out here,
//! after the frame is drawn (so no UI borrow is held while dialogs run).

use super::*;

impl App {
    pub(super) fn apply(&mut self, a: Act) {
        let ai = self.active;
        let mut t = std::mem::replace(&mut self.tabs[ai], Tab::new(0, Loc::ThisPc));
        match a {
            Act::Navigate(loc) => self.navigate(&mut t, loc),
            Act::NavigateNewTab(loc) => {
                self.tabs[ai] = t;
                self.open_tab(loc, true);
                return;
            }
            Act::Open(p) => {
                if !self.picker_takes_open(std::slice::from_ref(&p), &mut t) {
                    shell::open(&p);
                }
            }
            Act::OpenWith(p) => shell::open_with(&p),
            Act::RunAdmin(p) => shell::run_as_admin(&p),
            Act::OpenLocation(p) => {
                if let Some(parent) = p.parent() {
                    let name = p.file_name().map(|n| n.to_string_lossy().to_string());
                    self.navigate(&mut t, Loc::Dir(parent.to_path_buf()));
                    t.select_after_load = name;
                }
            }
            Act::Back => {
                if let Some(l) = t.back.pop() {
                    let cur = std::mem::replace(&mut t.loc, l);
                    t.fwd.push(cur);
                    self.start_load(&mut t, false);
                }
            }
            Act::Forward => {
                if let Some(l) = t.fwd.pop() {
                    let cur = std::mem::replace(&mut t.loc, l);
                    t.back.push(cur);
                    self.start_load(&mut t, false);
                }
            }
            Act::Up => {
                if let Loc::Dir(p) = &t.loc {
                    let up = match p.parent() {
                        Some(par) => Loc::Dir(par.to_path_buf()),
                        None => Loc::ThisPc,
                    };
                    self.navigate(&mut t, up);
                }
            }
            Act::Refresh => {
                if let Some(s) = &t.search {
                    let roots = s.index.roots.clone();
                    self.indexes.retain(|i| !search::roots_equal(&i.roots, &roots));
                    self.query_changed(&mut t);
                } else {
                    self.refresh_clouds();
                    if let Some(d) = t.dir() {
                        self.sizer.invalidate_under(d); // recount folder sizes too
                    }
                    self.start_load(&mut t, true);
                }
            }
            Act::NewTab => {
                let loc = t.loc.clone();
                self.tabs[ai] = t;
                self.open_tab(loc, true);
                return;
            }
            Act::CloseTab(i) => {
                self.tabs[ai] = t;
                if i >= self.tabs.len() {
                    // a second close in the same frame, from when the list was longer
                } else if self.tabs.len() > 1 {
                    self.tabs.remove(i);
                    if self.active > i || self.active >= self.tabs.len() {
                        self.active = self.active.saturating_sub(1);
                    }
                } else {
                    self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                return;
            }
            Act::SwitchTab(i) => {
                self.tabs[ai] = t;
                if i < self.tabs.len() {
                    self.active = i;
                    self.tabs[i].dirty = true;
                }
                return;
            }
            Act::Copy | Act::Cut => {
                let paths = self.selected_paths(&t);
                if !paths.is_empty() {
                    let cut = matches!(a, Act::Cut);
                    if shell::clipboard_set_files(&paths, cut) {
                        let n = paths.len();
                        self.toast(format!("{} {} item{}. Paste here or in any app", if cut { "Cut" } else { "Copied" }, n, if n == 1 { "" } else { "s" }), false);
                    }
                }
            }
            Act::Paste => {
                if let (Some(dir), false) = (t.dir().map(|d| d.to_path_buf()), t.searching()) {
                    if let Some((files, cut)) = shell::clipboard_get_files().filter(|(f, _)| !f.is_empty()) {
                        self.paste_files(files, cut, dir);
                    }
                }
            }
            Act::CopyPath => {
                let paths = self.selected_paths(&t);
                let text = paths.iter().map(|p| format!("\"{}\"", p.display())).collect::<Vec<_>>().join("\r\n");
                shell::clipboard_set_text(&text);
                self.toast("Path copied", false);
            }
            Act::StartRename => {
                let paths = self.selected_paths(&t);
                if let [p] = paths.as_slice() {
                    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    let key = if t.searching() { p.to_string_lossy().to_string() } else { name.clone() };
                    t.rename = Some(Rename { key, path: p.clone(), buf: name, focus: true });
                }
            }
            Act::CommitRename => {
                if let Some(r) = t.rename.take() {
                    self.commit_rename(&mut t, r);
                }
            }
            Act::Delete(permanent) => {
                let paths = self.selected_paths(&t);
                if !paths.is_empty() {
                    self.run_op(if permanent { Op::Delete } else { Op::Recycle }, paths, None);
                }
            }
            Act::NewFolder | Act::NewTextFile => {
                if let Some(dir) = t.dir().map(|d| d.to_path_buf()) {
                    self.create_new(&mut t, &dir, matches!(a, Act::NewFolder));
                }
            }
            Act::NativeProperties(p) => shell::properties(&p),
            Act::CustomizeMenu => {
                // edit exactly the menu that was just open
                if self.customizer.target.is_some() {
                    self.customizer.sample = 5;
                }
                self.customizer.built_for = None;
                self.tabs[ai] = t;
                self.apply(Act::OpenSettings(Some(settings_page::Page::ContextMenu)));
                return;
            }
            Act::Properties(p) => {
                if self.settings.native_props {
                    shell::properties(&p);
                } else {
                    // the whole selection when the clicked item is part of it
                    let sel = self.selected_paths(&t);
                    let paths = if sel.iter().any(|s| fsx::same_path(s, &p)) { sel } else { vec![p] };
                    self.open_props(paths);
                }
            }
            Act::NativeMenu(items) => {
                if let Some(dir) = t.dir().map(|d| d.to_path_buf()) {
                    let paths = if items { self.selected_paths(&t) } else { Vec::new() };
                    let base = if items { paths.first().and_then(|p| p.parent()).map(|p| p.to_path_buf()).unwrap_or(dir) } else { dir };
                    if let shell::MenuOutcome::Rename = shell::native_menu(&paths, &base) {
                        self.tabs[ai] = t;
                        self.apply(Act::StartRename);
                        return;
                    }
                }
            }
            Act::PinTo(..) | Act::NewSectionWith(_) | Act::Unpin(_) if policy::current().sidebar_locked() => {}
            Act::PinTo(sec, p) => {
                let title = self.settings.sections.iter().find(|s| s.id == sec).map(|s| s.title.clone()).unwrap_or_default();
                self.pin_to(&sec, p);
                self.toast(format!("Pinned to {title}"), false);
            }
            Act::NewSectionWith(p) => {
                self.new_section(Some(p));
            }
            Act::Unpin(p) => {
                for s in &mut self.settings.sections {
                    s.pins.retain(|x| !fsx::same_path(x, &p));
                }
            }
            Act::OpenMany(paths) => {
                if !self.picker_takes_open(&paths, &mut t) {
                    paths.iter().for_each(|p| shell::open(p));
                }
            }
            Act::TerminalAt(d) => shell::open_terminal(&d),
            Act::PasteInto(dest) => {
                if let Some((files, cut)) = shell::clipboard_get_files().filter(|(f, _)| !f.is_empty()) {
                    self.paste_files(files, cut, dest);
                }
            }
            Act::SetView(_) if policy::is_fixed("view") => {}
            Act::TogglePreview if policy::is_fixed("preview") => {}
            Act::ToggleHidden if policy::is_fixed("hidden") => {}
            Act::SetView(v) => self.settings.view = v,
            Act::TogglePreview => self.settings.show_preview = !self.settings.show_preview,
            Act::ToggleHidden => {
                self.settings.show_hidden = !self.settings.show_hidden;
                t.dirty = true;
                self.tabs.iter_mut().for_each(|o| o.dirty = true);
            }
            Act::OpenSettings(page) => {
                if let Some(p) = page {
                    self.settings_page = p;
                }
                if t.loc != Loc::Settings {
                    self.tabs[ai] = t;
                    match self.tabs.iter().position(|x| x.loc == Loc::Settings) {
                        Some(i) => self.active = i,
                        None => self.open_tab(Loc::Settings, true),
                    }
                    return;
                }
            }
            Act::KeepLocal(keep) => {
                let fails = self.selected_paths(&t).iter().filter(|p| fsx::set_pin(p, keep).is_err()).count();
                if fails > 0 {
                    self.toast(format!("{fails} item(s) couldn't be changed"), true);
                } else if keep {
                    self.toast("Downloading. These will always be kept on this device", false);
                } else {
                    self.toast("Freeing up space. The files stay available online", false);
                }
                self.start_load(&mut t, true);
            }
            Act::Terminal => {
                if let Some(d) = t.dir() {
                    shell::open_terminal(d);
                }
            }
            Act::MapDrive => {
                #[cfg(windows)]
                {
                    shell::map_network_drive();
                    self.refresh_drives();
                }
                #[cfg(not(windows))]
                {
                    self.connect = Some(("smb://".into(), String::new(), String::new()));
                }
            }
            Act::ConnectServer(url, user, pass) => {
                #[cfg(not(windows))]
                {
                    let tx = self.tx.clone();
                    let ctx = self.ctx.clone();
                    self.toast(format!("Connecting to {url}…"), false);
                    shell::connect_server(url, user, pass, move |r| {
                        let _ = tx.send(Msg::Connected(r));
                        ctx.request_repaint();
                    });
                }
                #[cfg(windows)]
                let _ = (url, user, pass);
            }
            Act::Disconnect(l) => {
                #[cfg(windows)]
                let r = shell::disconnect_drive(l);
                #[cfg(not(windows))]
                let r = match self.drives.iter().find(|d| d.letter == l) {
                    Some(d) => shell::unmount(&d.root),
                    None => Ok(()),
                };
                if let Err(e) = r {
                    self.toast(e, true);
                }
                self.refresh_drives();
            }
            Act::SelectAll => {
                let n = self.row_count(&t);
                t.sel = self.rows(&t, 0..n).into_iter().map(|r| r.key).collect();
                self.update_sel_info(&mut t);
            }
            Act::Drop { srcs, dest, copy } => {
                let srcs: Vec<PathBuf> = srcs.into_iter().filter(|s| !fsx::same_path(s, &dest) && !fsx::is_under(&dest, s)).collect();
                let pointless = !copy && srcs.iter().all(|s| s.parent().is_some_and(|p| fsx::same_path(p, &dest)));
                if !srcs.is_empty() && !pointless {
                    // dragging across drives copies, like Explorer
                    let same_drive = srcs.iter().all(|s| s.components().next() == dest.components().next());
                    self.run_op(if copy || !same_drive { Op::Copy } else { Op::Move }, srcs, Some(dest));
                }
            }
            Act::Import(files) => {
                if let Some(dir) = t.dir().map(|d| d.to_path_buf()) {
                    self.run_op(Op::Copy, files, Some(dir));
                }
            }
            Act::ShellInvoke(id) => {
                let outcome = self.shell_menu.as_ref().map(|sm| sm.invoke(id));
                if let Some(shell::MenuOutcome::Rename) = outcome {
                    self.tabs[ai] = t;
                    self.apply(Act::StartRename);
                    return;
                }
            }
            Act::CopyContents(p) => {
                let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
                bg::submit(&p.clone(), "read", move || {
                    let text = std::fs::read(&p).map(|b| String::from_utf8_lossy(&b).into_owned()).map_err(|e| e.to_string());
                    let _ = tx.send(Msg::FileText(text));
                    ctx.request_repaint();
                });
            }
            Act::OpenTarget(lnk) => match shell::shortcut_target(&lnk) {
                Some(target) => {
                    let parent = target.parent().map(|p| p.to_path_buf());
                    if let Some(parent) = parent {
                        let name = target.file_name().map(|n| n.to_string_lossy().to_string());
                        self.navigate(&mut t, Loc::Dir(parent));
                        t.select_after_load = name;
                    }
                }
                None => self.toast("This shortcut doesn't point to a file or folder", true),
            },
            Act::NewWindow(p) => {
                if let Ok(exe) = std::env::current_exe() {
                    let _ = std::process::Command::new(exe).arg("--path").arg(p).spawn();
                }
            }
            Act::RunCustom(i) => {
                if let (Some(c), Some(dir)) = (self.settings.custom.get(i).cloned(), t.dir().map(|d| d.to_path_buf())) {
                    let paths = self.selected_paths(&t);
                    let dir = paths.first().and_then(|p| p.parent()).map(|p| p.to_path_buf()).unwrap_or(dir);
                    if let Err(e) = menu::run_custom(&c, &paths, &dir) {
                        self.toast(e, true);
                    }
                }
            }
            Act::Sort(k, asc) => {
                let mut s = t.spec();
                if let Some(a) = asc {
                    s.asc = a;
                } else if s.key != k {
                    s.key = k;
                    s.asc = k.starts_ascending();
                }
                self.set_sort(&mut t, s);
            }
            Act::FoldersFirst(on) => {
                let mut s = t.spec();
                s.dirs_first = on;
                self.set_sort(&mut t, s);
            }
            Act::ResetSort => self.reset_sort(&mut t),
            Act::ToggleDotfiles if policy::is_fixed("hide_dots") => {}
            Act::ToggleDotfiles => {
                self.settings.hide_dotfiles = !self.settings.hide_dotfiles;
                t.dirty = true;
                self.tabs.iter_mut().for_each(|o| o.dirty = true);
                if t.searching() {
                    self.query_changed(&mut t);
                }
                self.save_settings();
            }
            Act::CancelLoad => {
                t.cancel.store(true, Ordering::Relaxed);
                t.gen += 1;
                t.loading = false;
                t.refreshing = false;
                t.retry_at = None;
                // what's already there can stay, if this was only a refresh
                if t.entries.is_empty() {
                    t.error = Some(FolderError::new(FailKind::Cancelled, ""));
                }
            }
            Act::Retry => {
                t.retry_wait = FIRST_RETRY;
                self.start_load(&mut t, false);
            }
            Act::Reconnect => {
                if let Some(p) = t.dir().map(Path::to_path_buf) {
                    t.reconnecting = true;
                    t.retry_at = None;
                    let (tx, ctx, tab) = (self.tx.clone(), self.ctx.clone(), t.id);
                    let owner = self.owner_window();
                    bg::spawn("reconnect", move || {
                        let result = fsx::reconnect_for(&p, owner, true);
                        let _ = tx.send(Msg::Reconnected { tab, result });
                        ctx.request_repaint();
                    });
                }
            }
            Act::ClearFilter => {
                t.filter = ListFilter::default();
                t.filter_ext.clear();
                t.dirty = true;
                if t.searching() {
                    self.query_changed(&mut t);
                }
            }
        }
        self.tabs[ai] = t;
        if self.started.elapsed() > Duration::from_secs(1) {
            self.save_settings();
        }
    }

    /// A folder can't go into itself: copying it there would keep copying the copy until paths get too long.
    fn paste_files(&mut self, files: Vec<PathBuf>, cut: bool, dest: PathBuf) {
        if files.iter().any(|f| fsx::same_path(f, &dest) || fsx::is_under(&dest, f)) {
            self.toast("A folder can't be pasted into itself", true);
            return;
        }
        self.run_op(if cut { Op::Move } else { Op::Copy }, files, Some(dest));
    }

    fn run_op(&self, op: Op, files: Vec<PathBuf>, dest: Option<PathBuf>) {
        let tx = self.tx.clone();
        shell::file_op(op, files, dest, move |r| {
            let _ = tx.send(Msg::OpDone(r));
        });
    }

    fn commit_rename(&mut self, t: &mut Tab, r: Rename) {
        let new = r.buf.trim().to_string();
        let old = r.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if new.is_empty() || new == old {
            return;
        }
        if new.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|']) {
            self.toast("A file name can't contain any of these characters: \\ / : * ? \" < > |", true);
            return;
        }
        let dest = r.path.with_file_name(&new);
        let (from, to) = (r.path.clone(), dest);
        match bg::within(&from.clone(), DISK_WAIT, move || std::fs::rename(&from, &to)) {
            Some(Ok(())) if !t.searching() => {
                t.sel.clear();
                t.sel.insert(new.clone());
                t.select_after_load = Some(new);
                self.start_load(t, true);
            }
            Some(Ok(())) => {}
            Some(Err(e)) => self.toast(format!("Rename failed: {e}"), true),
            None => self.toast("The server isn't answering yet. The new name will show if it gets there.", true),
        }
    }

    fn create_new(&mut self, t: &mut Tab, dir: &Path, folder: bool) {
        let (base, ext) = if folder { ("New folder", "") } else { ("New Text Document", ".txt") };
        let mut name = format!("{base}{ext}");
        let mut k = 2;
        // what's in the list is what's there, and asking the disk could take as long as a server takes to not answer
        let taken: HashSet<&str> = t.entries.iter().map(|e| e.lower.as_str()).collect();
        while taken.contains(name.to_lowercase().as_str()) {
            name = format!("{base} ({k}){ext}");
            k += 1;
        }
        drop(taken);
        let p = dir.join(&name);
        let target = p.clone();
        let done = bg::within(dir, DISK_WAIT, move || {
            let made = if folder { std::fs::create_dir(&target) } else { std::fs::write(&target, b"") };
            made.map(|()| fsx::stat(&target))
        });
        match done {
            Some(Ok(entry)) => {
                if let Some(e) = entry {
                    t.entries.push(e);
                    t.order_for = None;
                }
            }
            Some(Err(e)) => {
                self.toast(format!("Couldn't create it: {e}"), true);
                return;
            }
            None => {
                self.toast("The server isn't answering yet. It will show up here if it gets there.", true);
                return;
            }
        }
        t.select_after_load = Some(name.clone());
        self.rebuild_view(t);
        t.rename = Some(Rename { key: name.clone(), path: p, buf: name, focus: true });
    }
}

/// How long the window waits for a file operation someone just asked for. It goes on by itself after that.
const DISK_WAIT: Duration = Duration::from_millis(300);

pub(super) fn display_name(row: &Row, show_ext: bool) -> String {
    if show_ext || row.is_dir || row.ext.is_empty() || row.name.len() <= row.ext.len() + 1 {
        row.name.clone()
    } else {
        row.name[..row.name.len() - row.ext.len() - 1].to_string()
    }
}

/// `%APPDATA%\x` and `~\x` in the address bar.
pub(super) fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(a) = rest.find('%') {
        out.push_str(&rest[..a]);
        let after = &rest[a + 1..];
        match after.find('%') {
            Some(b) => {
                let var = &after[..b];
                match std::env::var(var) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => out.push_str(&format!("%{var}%")),
                }
                rest = &after[b + 1..];
            }
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    if let Some(r) = out.strip_prefix('~') {
        if let Some(h) = std::env::var_os("USERPROFILE") {
            return format!("{}{}", h.to_string_lossy(), r);
        }
    }
    out
}

pub(super) fn load_preview(p: &Path, e: &Entry) -> PvContent {
    if e.is_dir {
        let (mut files, mut dirs, mut size) = (0usize, 0usize, 0u64);
        let cancel = AtomicBool::new(false);
        let _ = fsx::list_dir(p, &cancel, |b| {
            for x in b {
                if x.is_dir {
                    dirs += 1;
                } else {
                    files += 1;
                    size += x.size;
                }
            }
        });
        return PvContent::Folder { files, dirs, size };
    }
    // never read an online-only file: that would download it
    if e.attrs & (fsx::A_RECALL_ON_DATA | fsx::A_OFFLINE) != 0 {
        return PvContent::CloudOnly;
    }
    if !is_text_ext(&e.ext) || e.size > 64 * 1024 * 1024 {
        return PvContent::Binary;
    }
    use std::io::Read;
    let Ok(f) = std::fs::File::open(p) else { return PvContent::Binary };
    let limit = 96 * 1024;
    let mut buf = Vec::with_capacity(limit);
    let _ = f.take(limit as u64 + 1).read_to_end(&mut buf);
    if buf.iter().take(8192).any(|&b| b == 0) {
        return PvContent::Binary;
    }
    let mut truncated = buf.len() > limit;
    buf.truncate(limit);
    let mut text = String::from_utf8_lossy(&buf).replace("\r\n", "\n").replace('\t', "    ");
    let lines = text.lines().count();
    if lines > 1500 {
        let cut: usize = text.lines().take(1500).map(|l| l.len() + 1).sum();
        text.truncate(cut.min(text.len()));
        truncated = true;
    }
    PvContent::Text { text, truncated, lines }
}

pub(super) fn save_png(path: &Path, img: &egui::ColorImage) -> std::io::Result<()> {
    let f = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), img.size[0] as u32, img.size[1] as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(std::io::Error::other)?;
    let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
    w.write_image_data(&bytes).map_err(std::io::Error::other)?;
    Ok(())
}
