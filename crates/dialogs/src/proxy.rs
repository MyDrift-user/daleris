//! The dialog objects apps get: every call also goes to a real dialog, kept ready to take over and holding what the app
//! reads back, and `Show` runs the picker.
//!
//! Nothing here returns `E_NOTIMPL`. Controls an app adds through `IFileDialogCustomize` are mirrored here and shown by
//! the picker, and their events reach the app as it picks. What can't be honoured at all (an item filter, a navigation
//! root, non-file items) sends that dialog to Windows' own `Show`, and so does anything going wrong in the picker.

use std::ffi::{c_void, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use daleris_integration::pick::{first_extension, Control, ControlItem, ControlKind, HostMessage, Kind, PickReply, PickRequest, STATE_NORMAL};
use daleris_integration::reg::Hive;
use windows::core::{implement, IUnknown, IUnknownImpl, Interface, GUID, HRESULT, PCWSTR, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Ole::*;
use windows::Win32::UI::Shell::Common::{COMDLG_FILTERSPEC, ITEMIDLIST};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyDescriptionList, IPropertyStore, PSCreateMemoryPropertyStore, GPS_READWRITE};
use windows::Win32::UI::Shell::*;

use crate::log::note;
use crate::modal::{self, Link, Outcome};
use crate::{guard, Alive};

const CANCELLED: HRESULT = HRESULT(0x800704C7u32 as i32);

/// Where the person is in the picker now, for what the app reads or sets while it's up.
#[derive(Clone, Default, PartialEq)]
struct Live {
    filter_index: usize,
    folder: PathBuf,
    name: String,
    selection: PathBuf,
}

#[derive(Default)]
struct State {
    title: String,
    ok_label: String,
    name_label: String,
    default_ext: String,
    filters: Vec<(String, String)>,
    /// counted from 1, as the interface does. 0 until the app sets one.
    filter_index: u32,
    events: Vec<(u32, IFileDialogEvents)>,
    /// something the picker can't honour was asked for, and what
    needs_windows: Option<&'static str>,
    /// the real dialog was shown, so results come from it
    used_windows: bool,
    picked: Option<Vec<PathBuf>>,
    /// the app called `Close` while the picker was up
    closing: Option<HRESULT>,
    owner: HWND,
    /// what the app added through `IFileDialogCustomize`, as the picker needs to draw it
    controls: Vec<Control>,
    group: Option<u32>,
    /// the controls changed since the picker was last told
    dirty: bool,
    live: Option<Live>,
    /// other things to tell the picker
    outgoing: Vec<HostMessage>,
    properties: Option<IPropertyStore>,
}

impl State {
    fn control(&mut self, id: u32) -> Option<&mut Control> {
        self.controls.iter_mut().find(|c| c.id == id)
    }

    fn add(&mut self, kind: ControlKind, id: u32, label: String) {
        let group = self.group;
        self.controls.push(Control { id, kind, label, state: STATE_NORMAL, group, ..Default::default() });
    }
}

struct Inner {
    real: IFileDialog,
    custom: Option<IFileDialogCustomize>,
    second: Option<IFileDialog2>,
    state: Mutex<State>,
    kind_is_save: bool,
    _alive: Alive,
}

fn text(p: &PCWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: COM's rules: a string the caller passed, valid for this call.
    unsafe { p.to_string() }.unwrap_or_default()
}

fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

fn path_of(item: &IShellItem) -> Option<PathBuf> {
    let name = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }.ok()?;
    // SAFETY: a null-terminated string from the shell, freed right after copying.
    let path = unsafe { OsString::from_wide(name.as_wide()) };
    unsafe { CoTaskMemFree(Some(name.0 as *const c_void)) };
    Some(PathBuf::from(path))
}

/// A shell item for a path, which for a Save need not exist yet.
fn item_of(path: &Path) -> windows::core::Result<IShellItem> {
    let w = wide(path.as_os_str());
    if let Ok(item) = unsafe { SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(w.as_ptr()), None) } {
        return Ok(item);
    }
    // A name that's not on disk: an ID list that doesn't check, same as the real dialog gives for a new file.
    let pidl = unsafe { SHSimpleIDListFromPath(PCWSTR(w.as_ptr())) };
    if pidl.is_null() {
        return Err(E_FAIL.into());
    }
    let item = unsafe { SHCreateItemFromIDList::<IShellItem>(pidl) };
    unsafe { CoTaskMemFree(Some(pidl as *const c_void)) };
    item
}

fn dup(s: &[u16]) -> windows::core::Result<PWSTR> {
    let bytes = (s.len() + 1) * 2;
    let p = unsafe { CoTaskMemAlloc(bytes) } as *mut u16;
    if p.is_null() {
        return Err(E_OUTOFMEMORY.into());
    }
    // SAFETY: just allocated for this many characters and the terminator.
    unsafe {
        std::ptr::copy_nonoverlapping(s.as_ptr(), p, s.len());
        *p.add(s.len()) = 0;
    }
    Ok(PWSTR(p))
}

impl Inner {
    fn new(real: IFileDialog, kind_is_save: bool) -> Inner {
        Inner { custom: real.cast().ok(), second: real.cast().ok(), real, state: Mutex::new(State::default()), kind_is_save, _alive: Alive::new() }
    }

    fn st(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn picked_first(&self) -> Option<PathBuf> {
        self.st().picked.as_ref().and_then(|p| p.first().cloned())
    }

    fn live(&self) -> Option<Live> {
        self.st().live.clone()
    }

    fn request(&self, owner: HWND) -> windows::core::Result<PickRequest> {
        let options = unsafe { self.real.GetOptions() }?;
        let has = |f: FILEOPENDIALOGOPTIONS| options.0 & f.0 != 0;
        let folder = unsafe { self.real.GetFolder() }.ok().and_then(|i| path_of(&i)).map(|p| p.into_os_string()).unwrap_or_default();
        let file_name = unsafe { self.real.GetFileName() }
            .ok()
            .map(|p| {
                // SAFETY: a string the dialog allocated for this call, freed after copying.
                let s = unsafe { p.to_string() }.unwrap_or_default();
                unsafe { CoTaskMemFree(Some(p.0 as *const c_void)) };
                s
            })
            .unwrap_or_default();
        let st = self.st();
        let kind = if has(FOS_PICKFOLDERS) {
            Kind::Folder
        } else if self.kind_is_save {
            Kind::Save
        } else {
            Kind::Open
        };
        let host = crate::host_path();
        Ok(PickRequest {
            kind,
            multi: has(FOS_ALLOWMULTISELECT),
            must_exist: has(FOS_FILEMUSTEXIST) || has(FOS_PATHMUSTEXIST),
            overwrite_prompt: has(FOS_OVERWRITEPROMPT),
            title: st.title.clone(),
            ok_label: st.ok_label.clone(),
            name_label: st.name_label.clone(),
            file_name,
            folder,
            default_ext: st.default_ext.clone(),
            filters: st.filters.clone(),
            filter_index: st.filter_index.saturating_sub(1) as usize,
            owner: owner.0 as isize,
            host: host.rsplit('\\').next().unwrap_or(&host).to_string(),
            controls: st.controls.clone(),
        })
    }

    /// Something the picker can't do, for reasons the app set before showing.
    fn why_windows(&self) -> Option<&'static str> {
        if let Some(why) = self.st().needs_windows {
            return Some(why);
        }
        // items that aren't files: only Windows' dialog can offer those
        unsafe { self.real.GetOptions() }.is_ok_and(|o| o.0 & FOS_ALLNONSTORAGEITEMS.0 != 0).then_some("the app wants items that aren't files (FOS_ALLNONSTORAGEITEMS)")
    }

    fn show_windows(&self, owner: HWND) -> windows::core::Result<()> {
        {
            let mut st = self.st();
            st.used_windows = true;
            st.live = None;
        }
        unsafe { self.real.Show(owner) }
    }

    fn show(&self, owner: HWND, me: &IFileDialog, me_custom: &IFileDialogCustomize) -> windows::core::Result<()> {
        {
            let mut st = self.st();
            st.picked = None;
            st.used_windows = false;
            st.closing = None;
            st.owner = owner;
            st.live = None;
            st.dirty = false;
            st.outgoing.clear();
        }
        let options = unsafe { self.real.GetOptions() }.map_or(0, |o| o.0);
        let (controls, events) = {
            let st = self.st();
            (st.controls.len(), st.events.len())
        };
        note(&format!("Show: save={} options=0x{options:08X} controls={controls} events={events}", self.kind_is_save));
        let exe = Hive::real().program_path().filter(|p| p.is_file());
        let Some(exe) = exe else {
            note("Windows' own dialog: Daleris isn't there");
            return self.show_windows(owner);
        };
        if let Some(why) = self.why_windows() {
            note(&format!("Windows' own dialog: {why}"));
            return self.show_windows(owner);
        }
        let mut request = self.request(owner)?;
        note(&format!("picker for {:?} in {:?}, name {:?}", request.kind, request.folder, request.file_name));
        loop {
            request.controls = self.st().controls.clone();
            let outcome = modal::run(&exe, &request, owner, || self.st().closing, |link| self.tell_picker(link), |reply, link| self.handle_picker(reply, link, me, me_custom));
            self.st().live = None;
            match outcome {
                Outcome::Cancelled => {
                    note("cancelled");
                    return Err(CANCELLED.into());
                }
                Outcome::Closed(hr) => return if hr.is_ok() { Ok(()) } else { Err(hr.into()) },
                Outcome::UseWindows(why) => {
                    note(&format!("Windows' own dialog: {why}"));
                    return self.show_windows(owner);
                }
                Outcome::Picked { filter_index, paths } => {
                    note(&format!("picked {paths:?}"));
                    let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
                    {
                        let mut st = self.st();
                        st.filter_index = filter_index as u32 + 1;
                        st.picked = Some(paths.clone());
                    }
                    let _ = unsafe { self.real.SetFileTypeIndex(filter_index as u32 + 1) };
                    if self.events_accept(me) {
                        return Ok(());
                    }
                    // the app said no in OnFileOk: back to the picker where the person was
                    note("the app refused it in OnFileOk");
                    self.st().picked = None;
                    if let Some(folder) = paths.first().and_then(|p| p.parent()) {
                        request.folder = folder.as_os_str().to_os_string();
                    }
                    request.filter_index = filter_index;
                }
            }
        }
    }

    /// What's changed on the app's side since the picker last heard, each round of the wait.
    fn tell_picker(&self, link: &Link) {
        let (controls, others) = {
            let mut st = self.st();
            let controls = std::mem::take(&mut st.dirty).then(|| st.controls.clone());
            (controls, std::mem::take(&mut st.outgoing))
        };
        if let Some(list) = controls {
            link.send(&HostMessage::Controls(list));
        }
        for m in others {
            link.send(&m);
        }
    }

    /// What the person does in the picker, as it happens. The app's handlers run from here, on the thread it called
    /// `Show` on. A handler may show a window of its own, such as Office's Tools menu does, and the picker stays on top
    /// of everything else, so it's told to step back for as long as a handler takes.
    fn handle_picker(&self, reply: PickReply, link: &Link, me: &IFileDialog, me_custom: &IFileDialogCustomize) {
        let control_event = |run: &dyn Fn(&IFileDialogControlEvents)| {
            let events: Vec<IFileDialogEvents> = self.st().events.iter().map(|(_, e)| e.clone()).collect();
            link.send(&HostMessage::Busy(true));
            for e in &events {
                if let Ok(handler) = e.cast::<IFileDialogControlEvents>() {
                    run(&handler);
                }
            }
            link.send(&HostMessage::Busy(false));
        };
        match reply {
            PickReply::Check { id, on } => {
                note(&format!("picker: check {id} = {on}"));
                if let Some(c) = &self.custom {
                    let _ = unsafe { c.SetCheckButtonState(id, on) };
                }
                if let Some(c) = self.st().control(id) {
                    c.checked = on;
                }
                control_event(&|h| {
                    let _ = unsafe { h.OnCheckButtonToggled(me_custom, id, on) };
                });
            }
            PickReply::Item { control, item } => {
                note(&format!("picker: item {item} of {control}"));
                let is_menu = {
                    let mut st = self.st();
                    match st.control(control) {
                        Some(c) if c.kind == ControlKind::Menu => true,
                        Some(c) => {
                            c.selected = Some(item);
                            false
                        }
                        None => false,
                    }
                };
                if !is_menu {
                    if let Some(c) = &self.custom {
                        let _ = unsafe { c.SetSelectedControlItem(control, item) };
                    }
                }
                control_event(&|h| {
                    let _ = unsafe { h.OnItemSelected(me_custom, control, item) };
                });
            }
            PickReply::Button { id } => {
                note(&format!("picker: button {id}"));
                control_event(&|h| {
                    let _ = unsafe { h.OnButtonClicked(me_custom, id) };
                });
            }
            PickReply::Activating { id } => {
                note(&format!("picker: activating {id}"));
                control_event(&|h| {
                    let _ = unsafe { h.OnControlActivating(me_custom, id) };
                });
            }
            PickReply::Text { id, text } => {
                let w: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
                if let Some(c) = &self.custom {
                    let _ = unsafe { c.SetEditBoxText(id, PCWSTR(w.as_ptr())) };
                }
                if let Some(c) = self.st().control(id) {
                    c.text = text;
                }
            }
            PickReply::State { filter_index, folder, name, selection } => {
                let new = Live { filter_index, folder: folder.into(), name, selection: selection.into() };
                let old = self.st().live.replace(new.clone());
                self.st().filter_index = filter_index as u32 + 1;
                let events: Vec<IFileDialogEvents> = self.st().events.iter().map(|(_, e)| e.clone()).collect();
                if old.as_ref().is_none_or(|o| o.folder != new.folder) {
                    for e in &events {
                        let _ = unsafe { e.OnFolderChange(me) };
                    }
                }
                if old.as_ref().is_some_and(|o| o.filter_index != new.filter_index) {
                    for e in &events {
                        let _ = unsafe { e.OnTypeChange(me) };
                    }
                }
                if old.as_ref().is_some_and(|o| o.selection != new.selection) {
                    for e in &events {
                        let _ = unsafe { e.OnSelectionChange(me) };
                    }
                }
            }
            // these end the wait, in modal.rs
            PickReply::Ready | PickReply::Picked { .. } | PickReply::Cancelled | PickReply::UseWindows => {}
        }
    }

    /// What a real dialog tells its events on OK. Each can still refuse: anything but S_OK or "not implemented" does.
    fn events_accept(&self, me: &IFileDialog) -> bool {
        let events: Vec<IFileDialogEvents> = self.st().events.iter().map(|(_, e)| e.clone()).collect();
        for e in &events {
            let _ = unsafe { e.OnSelectionChange(me) };
        }
        for e in &events {
            // raw, as the wrapper counts S_FALSE as fine, which here means "stay open"
            let hr = unsafe { (Interface::vtable(e).OnFileOk)(Interface::as_raw(e), Interface::as_raw(me)) };
            if hr != S_OK && hr != E_NOTIMPL {
                return false;
            }
        }
        true
    }

    fn set_file_types(&self, count: u32, specs: *const COMDLG_FILTERSPEC) -> windows::core::Result<()> {
        let list = if specs.is_null() {
            Vec::new()
        } else {
            // SAFETY: COM's rules: `count` items, valid for this call.
            unsafe { std::slice::from_raw_parts(specs, count as usize) }.iter().map(|s| (text(&s.pszName), text(&s.pszSpec))).collect()
        };
        // SAFETY: passed on unchanged.
        unsafe { self.real.SetFileTypes(std::slice::from_raw_parts(specs, if specs.is_null() { 0 } else { count as usize })) }?;
        self.st().filters = list;
        Ok(())
    }

    fn file_name(&self) -> windows::core::Result<PWSTR> {
        if let Some(name) = self.picked_first().as_deref().and_then(Path::file_name) {
            return dup(&name.encode_wide().collect::<Vec<u16>>());
        }
        if let Some(live) = self.live() {
            return dup(&live.name.encode_utf16().collect::<Vec<u16>>());
        }
        unsafe { self.real.GetFileName() }
    }

    fn folder(&self) -> windows::core::Result<IShellItem> {
        if let Some(dir) = self.picked_first().as_deref().and_then(Path::parent) {
            return item_of(dir);
        }
        if let Some(live) = self.live().filter(|l| !l.folder.as_os_str().is_empty()) {
            return item_of(&live.folder);
        }
        unsafe { self.real.GetFolder() }
    }

    fn selection(&self) -> windows::core::Result<IShellItem> {
        if let Some(p) = self.picked_first() {
            return item_of(&p);
        }
        if let Some(live) = self.live().filter(|l| !l.selection.as_os_str().is_empty()) {
            return item_of(&live.selection);
        }
        unsafe { self.real.GetCurrentSelection() }
    }

    fn result(&self) -> windows::core::Result<IShellItem> {
        match self.picked_first() {
            Some(p) => item_of(&with_extension(&p, self)),
            None => unsafe { self.real.GetResult() },
        }
    }

    /// The results as the array the interface calls for.
    fn results(&self) -> windows::core::Result<IShellItemArray> {
        let paths = self.st().picked.clone().ok_or(windows::core::Error::from(E_UNEXPECTED))?;
        let mut ids: Vec<*const ITEMIDLIST> = Vec::with_capacity(paths.len());
        for p in &paths {
            let item = item_of(p)?;
            let pidl = unsafe { SHGetIDListFromObject(&item) }?;
            ids.push(pidl as *const ITEMIDLIST);
        }
        let array = unsafe { SHCreateShellItemArrayFromIDLists(&ids) };
        for id in ids {
            unsafe { CoTaskMemFree(Some(id as *const c_void)) };
        }
        array
    }

    fn picked_any(&self) -> bool {
        self.st().picked.is_some()
    }

    fn file_type_index(&self) -> windows::core::Result<u32> {
        let st = self.st();
        if st.picked.is_some() || st.live.is_some() {
            return Ok(st.filter_index);
        }
        drop(st);
        unsafe { self.real.GetFileTypeIndex() }
    }

    fn advise(&self, events: Option<&IFileDialogEvents>) -> windows::core::Result<u32> {
        let Some(e) = events else { return Err(E_INVALIDARG.into()) };
        let cookie = unsafe { self.real.Advise(e) }?;
        self.st().events.push((cookie, e.clone()));
        note(&format!("Advise: control events {}", if e.cast::<IFileDialogControlEvents>().is_ok() { "yes" } else { "no" }));
        Ok(cookie)
    }

    fn unadvise(&self, cookie: u32) -> windows::core::Result<()> {
        unsafe { self.real.Unadvise(cookie) }?;
        self.st().events.retain(|(c, _)| *c != cookie);
        Ok(())
    }

    fn close(&self, hr: HRESULT) -> windows::core::Result<()> {
        let mut st = self.st();
        if !st.used_windows {
            st.closing = Some(hr);
        }
        drop(st);
        let _ = unsafe { self.real.Close(hr) };
        Ok(())
    }

    fn query_real_for<T: Interface>(&self) -> Option<T> {
        self.real.cast().ok()
    }

    /// One call on the real dialog's `IFileDialogCustomize`. What it holds is what the app reads back, so only what
    /// the picker needs to draw is kept here, and only if the real one took it.
    fn customize<T>(&self, call: &str, detail: String, run: impl FnOnce(&IFileDialogCustomize) -> windows::core::Result<T>, mirror: impl FnOnce(&mut State)) -> windows::core::Result<T> {
        guard(|| {
            note(&format!("Customize.{call} {detail}"));
            let Some(c) = &self.custom else { return Err(E_NOINTERFACE.into()) };
            let result = run(c);
            match &result {
                Ok(_) => {
                    let mut st = self.st();
                    mirror(&mut st);
                    st.dirty = true;
                }
                Err(e) => note(&format!("Customize.{call} failed: {:?}", e.code())),
            }
            result
        })
    }

    /// Reads go straight to the real dialog, which the picker's changes are written to as they happen.
    fn customize_read<T>(&self, run: impl FnOnce(&IFileDialogCustomize) -> windows::core::Result<T>) -> windows::core::Result<T> {
        guard(|| match &self.custom {
            Some(c) => run(c),
            None => Err(E_NOINTERFACE.into()),
        })
    }

    fn set_windows_only(&self, why: &'static str) {
        note(&format!("needs Windows' own dialog: {why}"));
        self.st().needs_windows = Some(why);
    }
}

/// A name typed without an extension gets the one the app asks for, as the real dialog does. The picker already did for
/// the names it typed itself, so this is for apps that change the extension in their events.
fn with_extension(path: &Path, inner: &Inner) -> PathBuf {
    if !inner.kind_is_save || path.extension().is_some() || daleris_integration::target::kind_within(path, std::time::Duration::from_millis(500)) == daleris_integration::target::PathKind::Dir {
        return path.to_path_buf();
    }
    let st = inner.st();
    let index = st.filter_index.saturating_sub(1) as usize;
    let ext = if st.default_ext.is_empty() { st.filters.get(index).and_then(|(_, s)| first_extension(s)).unwrap_or("") } else { st.default_ext.as_str() };
    if ext.is_empty() {
        return path.to_path_buf();
    }
    path.with_extension(ext)
}

/// The same methods on both dialog types, which each need their own COM object type.
macro_rules! dialog_impls {
    ($impl_type:ident) => {
        impl IModalWindow_Impl for $impl_type {
            fn Show(&self, owner: HWND) -> windows::core::Result<()> {
                // A panic here would take the app's dialog with it: fall back on the real one.
                let me: IFileDialog = self.to_interface::<IFileDialog2>().cast()?;
                let me_custom: IFileDialogCustomize = self.to_interface();
                match guard(|| self.0.show(owner, &me, &me_custom)) {
                    Err(e) if e.code() == E_FAIL => {
                        note("Windows' own dialog: Show failed");
                        unsafe { self.0.real.Show(owner) }
                    }
                    other => other,
                }
            }
        }

        impl IFileDialog_Impl for $impl_type {
            fn SetFileTypes(&self, count: u32, specs: *const COMDLG_FILTERSPEC) -> windows::core::Result<()> {
                guard(|| self.0.set_file_types(count, specs))
            }
            fn SetFileTypeIndex(&self, index: u32) -> windows::core::Result<()> {
                guard(|| {
                    unsafe { self.0.real.SetFileTypeIndex(index) }?;
                    let mut st = self.0.st();
                    st.filter_index = index;
                    if st.live.is_some() {
                        st.outgoing.push(HostMessage::SetFilter(index.saturating_sub(1) as usize));
                    }
                    Ok(())
                })
            }
            fn GetFileTypeIndex(&self) -> windows::core::Result<u32> {
                guard(|| self.0.file_type_index())
            }
            fn Advise(&self, events: Option<&IFileDialogEvents>) -> windows::core::Result<u32> {
                guard(|| self.0.advise(events))
            }
            fn Unadvise(&self, cookie: u32) -> windows::core::Result<()> {
                guard(|| self.0.unadvise(cookie))
            }
            fn SetOptions(&self, options: FILEOPENDIALOGOPTIONS) -> windows::core::Result<()> {
                guard(|| {
                    note(&format!("SetOptions 0x{:08X}", options.0 as u32));
                    unsafe { self.0.real.SetOptions(options) }
                })
            }
            fn GetOptions(&self) -> windows::core::Result<FILEOPENDIALOGOPTIONS> {
                guard(|| unsafe { self.0.real.GetOptions() })
            }
            fn SetDefaultFolder(&self, item: Option<&IShellItem>) -> windows::core::Result<()> {
                guard(|| unsafe { self.0.real.SetDefaultFolder(item) })
            }
            fn SetFolder(&self, item: Option<&IShellItem>) -> windows::core::Result<()> {
                guard(|| {
                    unsafe { self.0.real.SetFolder(item) }?;
                    let mut st = self.0.st();
                    if st.live.is_some() {
                        if let Some(p) = item.and_then(path_of) {
                            st.outgoing.push(HostMessage::SetFolder(p.into_os_string()));
                        }
                    }
                    Ok(())
                })
            }
            fn GetFolder(&self) -> windows::core::Result<IShellItem> {
                guard(|| self.0.folder())
            }
            fn GetCurrentSelection(&self) -> windows::core::Result<IShellItem> {
                guard(|| self.0.selection())
            }
            fn SetFileName(&self, name: &PCWSTR) -> windows::core::Result<()> {
                guard(|| {
                    unsafe { self.0.real.SetFileName(*name) }?;
                    let mut st = self.0.st();
                    if let Some(live) = &mut st.live {
                        live.name = text(name);
                        st.outgoing.push(HostMessage::SetName(text(name)));
                    }
                    Ok(())
                })
            }
            fn GetFileName(&self) -> windows::core::Result<PWSTR> {
                guard(|| self.0.file_name())
            }
            fn SetTitle(&self, title: &PCWSTR) -> windows::core::Result<()> {
                guard(|| {
                    unsafe { self.0.real.SetTitle(*title) }?;
                    self.0.st().title = text(title);
                    Ok(())
                })
            }
            fn SetOkButtonLabel(&self, label: &PCWSTR) -> windows::core::Result<()> {
                guard(|| {
                    unsafe { self.0.real.SetOkButtonLabel(*label) }?;
                    self.0.st().ok_label = text(label);
                    Ok(())
                })
            }
            fn SetFileNameLabel(&self, label: &PCWSTR) -> windows::core::Result<()> {
                guard(|| {
                    unsafe { self.0.real.SetFileNameLabel(*label) }?;
                    self.0.st().name_label = text(label);
                    Ok(())
                })
            }
            fn GetResult(&self) -> windows::core::Result<IShellItem> {
                guard(|| self.0.result())
            }
            fn AddPlace(&self, item: Option<&IShellItem>, where_: FDAP) -> windows::core::Result<()> {
                guard(|| unsafe { self.0.real.AddPlace(item, where_) })
            }
            fn SetDefaultExtension(&self, ext: &PCWSTR) -> windows::core::Result<()> {
                guard(|| {
                    unsafe { self.0.real.SetDefaultExtension(*ext) }?;
                    self.0.st().default_ext = text(ext);
                    Ok(())
                })
            }
            fn Close(&self, hr: HRESULT) -> windows::core::Result<()> {
                guard(|| self.0.close(hr))
            }
            fn SetClientGuid(&self, guid: *const GUID) -> windows::core::Result<()> {
                // SAFETY: COM's rules: a valid GUID for this call.
                guard(|| unsafe { self.0.real.SetClientGuid(guid) })
            }
            fn ClearClientData(&self) -> windows::core::Result<()> {
                guard(|| unsafe { self.0.real.ClearClientData() })
            }
            fn SetFilter(&self, filter: Option<&IShellItemFilter>) -> windows::core::Result<()> {
                guard(|| {
                    unsafe { self.0.real.SetFilter(filter) }?;
                    if filter.is_some() {
                        self.0.set_windows_only("the app filters items itself (SetFilter)");
                    }
                    Ok(())
                })
            }
        }

        impl IFileDialog2_Impl for $impl_type {
            fn SetCancelButtonLabel(&self, label: &PCWSTR) -> windows::core::Result<()> {
                guard(|| match &self.0.second {
                    Some(d) => unsafe { d.SetCancelButtonLabel(*label) },
                    None => Ok(()),
                })
            }
            fn SetNavigationRoot(&self, item: Option<&IShellItem>) -> windows::core::Result<()> {
                guard(|| {
                    // a dialog that may only show part of the disk is one the picker can't honour
                    self.0.set_windows_only("the app limits where the dialog can go (SetNavigationRoot)");
                    match (&self.0.second, item) {
                        (Some(d), Some(item)) => unsafe { d.SetNavigationRoot(item) },
                        _ => Ok(()),
                    }
                })
            }
        }

        impl IOleWindow_Impl for $impl_type {
            // WPF asks this for the owner
            fn GetWindow(&self) -> windows::core::Result<HWND> {
                guard(|| {
                    if let Ok(w) = self.0.real.cast::<IOleWindow>().and_then(|w| unsafe { w.GetWindow() }) {
                        return Ok(w);
                    }
                    let owner = self.0.st().owner;
                    if owner.0.is_null() {
                        Err(E_FAIL.into())
                    } else {
                        Ok(owner)
                    }
                })
            }
            fn ContextSensitiveHelp(&self, enter: BOOL) -> windows::core::Result<()> {
                guard(|| match self.0.query_real_for::<IOleWindow>() {
                    Some(w) => unsafe { w.ContextSensitiveHelp(enter) },
                    None => Ok(()),
                })
            }
        }

        impl IObjectWithSite_Impl for $impl_type {
            fn SetSite(&self, site: Option<&IUnknown>) -> windows::core::Result<()> {
                guard(|| match self.0.query_real_for::<IObjectWithSite>() {
                    Some(w) => unsafe { w.SetSite(site) },
                    None => Ok(()),
                })
            }
            fn GetSite(&self, iid: *const GUID, out: *mut *mut c_void) -> windows::core::Result<()> {
                guard(|| match self.0.query_real_for::<IObjectWithSite>() {
                    // SAFETY: COM's rules: the pointers came from the caller.
                    Some(w) => unsafe { (Interface::vtable(&w).GetSite)(Interface::as_raw(&w), iid, out) }.ok(),
                    None => Err(E_FAIL.into()),
                })
            }
        }

        impl IFileDialogCustomize_Impl for $impl_type {
            fn EnableOpenDropDown(&self, id: u32) -> windows::core::Result<()> {
                self.0.customize("EnableOpenDropDown", format!("{id}"), |c| unsafe { c.EnableOpenDropDown(id) }, |st| st.add(ControlKind::OpenDropDown, id, String::new()))
            }
            fn AddMenu(&self, id: u32, label: &PCWSTR) -> windows::core::Result<()> {
                let t = text(label);
                self.0.customize("AddMenu", format!("{id} {t:?}"), |c| unsafe { c.AddMenu(id, *label) }, |st| st.add(ControlKind::Menu, id, t.clone()))
            }
            fn AddPushButton(&self, id: u32, label: &PCWSTR) -> windows::core::Result<()> {
                let t = text(label);
                self.0.customize("AddPushButton", format!("{id} {t:?}"), |c| unsafe { c.AddPushButton(id, *label) }, |st| st.add(ControlKind::Button, id, t.clone()))
            }
            fn AddComboBox(&self, id: u32) -> windows::core::Result<()> {
                self.0.customize("AddComboBox", format!("{id}"), |c| unsafe { c.AddComboBox(id) }, |st| st.add(ControlKind::Combo, id, String::new()))
            }
            fn AddRadioButtonList(&self, id: u32) -> windows::core::Result<()> {
                self.0.customize("AddRadioButtonList", format!("{id}"), |c| unsafe { c.AddRadioButtonList(id) }, |st| st.add(ControlKind::Radio, id, String::new()))
            }
            fn AddCheckButton(&self, id: u32, label: &PCWSTR, checked: BOOL) -> windows::core::Result<()> {
                let t = text(label);
                self.0.customize("AddCheckButton", format!("{id} {t:?} {}", checked.as_bool()), |c| unsafe { c.AddCheckButton(id, *label, checked) }, |st| {
                    st.add(ControlKind::Check, id, t.clone());
                    if let Some(c) = st.control(id) {
                        c.checked = checked.as_bool();
                    }
                })
            }
            fn AddEditBox(&self, id: u32, value: &PCWSTR) -> windows::core::Result<()> {
                let t = text(value);
                self.0.customize("AddEditBox", format!("{id} {t:?}"), |c| unsafe { c.AddEditBox(id, *value) }, |st| {
                    st.add(ControlKind::Edit, id, String::new());
                    if let Some(c) = st.control(id) {
                        c.text = t.clone();
                    }
                })
            }
            fn AddSeparator(&self, id: u32) -> windows::core::Result<()> {
                self.0.customize("AddSeparator", format!("{id}"), |c| unsafe { c.AddSeparator(id) }, |st| st.add(ControlKind::Separator, id, String::new()))
            }
            fn AddText(&self, id: u32, value: &PCWSTR) -> windows::core::Result<()> {
                let t = text(value);
                self.0.customize("AddText", format!("{id} {t:?}"), |c| unsafe { c.AddText(id, *value) }, |st| st.add(ControlKind::Text, id, t.clone()))
            }
            fn SetControlLabel(&self, id: u32, label: &PCWSTR) -> windows::core::Result<()> {
                let t = text(label);
                self.0.customize("SetControlLabel", format!("{id} {t:?}"), |c| unsafe { c.SetControlLabel(id, *label) }, |st| {
                    if let Some(c) = st.control(id) {
                        c.label = t.clone();
                    }
                })
            }
            fn GetControlState(&self, id: u32) -> windows::core::Result<CDCONTROLSTATEF> {
                self.0.customize_read(|c| unsafe { c.GetControlState(id) })
            }
            fn SetControlState(&self, id: u32, state: CDCONTROLSTATEF) -> windows::core::Result<()> {
                self.0.customize("SetControlState", format!("{id} {}", state.0), |c| unsafe { c.SetControlState(id, state) }, |st| {
                    if let Some(c) = st.control(id) {
                        c.state = state.0 as u32;
                    }
                })
            }
            fn GetEditBoxText(&self, id: u32) -> windows::core::Result<*mut u16> {
                self.0.customize_read(|c| unsafe { c.GetEditBoxText(id) })
            }
            fn SetEditBoxText(&self, id: u32, value: &PCWSTR) -> windows::core::Result<()> {
                let t = text(value);
                self.0.customize("SetEditBoxText", format!("{id} {t:?}"), |c| unsafe { c.SetEditBoxText(id, *value) }, |st| {
                    if let Some(c) = st.control(id) {
                        c.text = t.clone();
                    }
                })
            }
            fn GetCheckButtonState(&self, id: u32) -> windows::core::Result<BOOL> {
                self.0.customize_read(|c| unsafe { c.GetCheckButtonState(id) })
            }
            fn SetCheckButtonState(&self, id: u32, checked: BOOL) -> windows::core::Result<()> {
                self.0.customize("SetCheckButtonState", format!("{id} {}", checked.as_bool()), |c| unsafe { c.SetCheckButtonState(id, checked) }, |st| {
                    if let Some(c) = st.control(id) {
                        c.checked = checked.as_bool();
                    }
                })
            }
            fn AddControlItem(&self, id: u32, item: u32, label: &PCWSTR) -> windows::core::Result<()> {
                let t = text(label);
                self.0.customize("AddControlItem", format!("{id}/{item} {t:?}"), |c| unsafe { c.AddControlItem(id, item, *label) }, |st| {
                    if let Some(c) = st.control(id) {
                        c.items.push(ControlItem { id: item, label: t.clone(), state: STATE_NORMAL });
                    }
                })
            }
            fn RemoveControlItem(&self, id: u32, item: u32) -> windows::core::Result<()> {
                self.0.customize("RemoveControlItem", format!("{id}/{item}"), |c| unsafe { c.RemoveControlItem(id, item) }, |st| {
                    if let Some(c) = st.control(id) {
                        c.items.retain(|i| i.id != item);
                    }
                })
            }
            fn RemoveAllControlItems(&self, id: u32) -> windows::core::Result<()> {
                self.0.customize("RemoveAllControlItems", format!("{id}"), |c| unsafe { c.RemoveAllControlItems(id) }, |st| {
                    if let Some(c) = st.control(id) {
                        c.items.clear();
                        c.selected = None;
                    }
                })
            }
            fn GetControlItemState(&self, id: u32, item: u32) -> windows::core::Result<CDCONTROLSTATEF> {
                self.0.customize_read(|c| unsafe { c.GetControlItemState(id, item) })
            }
            fn SetControlItemState(&self, id: u32, item: u32, state: CDCONTROLSTATEF) -> windows::core::Result<()> {
                self.0.customize("SetControlItemState", format!("{id}/{item} {}", state.0), |c| unsafe { c.SetControlItemState(id, item, state) }, |st| {
                    if let Some(i) = st.control(id).and_then(|c| c.items.iter_mut().find(|i| i.id == item)) {
                        i.state = state.0 as u32;
                    }
                })
            }
            fn GetSelectedControlItem(&self, id: u32) -> windows::core::Result<u32> {
                self.0.customize_read(|c| unsafe { c.GetSelectedControlItem(id) })
            }
            fn SetSelectedControlItem(&self, id: u32, item: u32) -> windows::core::Result<()> {
                self.0.customize("SetSelectedControlItem", format!("{id}/{item}"), |c| unsafe { c.SetSelectedControlItem(id, item) }, |st| {
                    if let Some(c) = st.control(id) {
                        c.selected = Some(item);
                    }
                })
            }
            fn StartVisualGroup(&self, id: u32, label: &PCWSTR) -> windows::core::Result<()> {
                let t = text(label);
                self.0.customize("StartVisualGroup", format!("{id} {t:?}"), |c| unsafe { c.StartVisualGroup(id, *label) }, |st| {
                    st.group = None;
                    st.add(ControlKind::Group, id, t.clone());
                    st.group = Some(id);
                })
            }
            fn EndVisualGroup(&self) -> windows::core::Result<()> {
                self.0.customize("EndVisualGroup", String::new(), |c| unsafe { c.EndVisualGroup() }, |st| st.group = None)
            }
            fn MakeProminent(&self, id: u32) -> windows::core::Result<()> {
                self.0.customize("MakeProminent", format!("{id}"), |c| unsafe { c.MakeProminent(id) }, |st| {
                    if let Some(c) = st.control(id) {
                        c.prominent = true;
                    }
                })
            }
            fn SetControlItemText(&self, id: u32, item: u32, label: &PCWSTR) -> windows::core::Result<()> {
                let t = text(label);
                self.0.customize("SetControlItemText", format!("{id}/{item} {t:?}"), |c| unsafe { c.SetControlItemText(id, item, *label) }, |st| {
                    if let Some(i) = st.control(id).and_then(|c| c.items.iter_mut().find(|i| i.id == item)) {
                        i.label = t.clone();
                    }
                })
            }
        }
    };
}

#[implement(IFileOpenDialog, IFileDialog2, IFileDialogCustomize, IOleWindow, IObjectWithSite)]
struct OpenProxy(Inner);

#[implement(IFileSaveDialog, IFileDialog2, IFileDialogCustomize, IOleWindow, IObjectWithSite)]
struct SaveProxy(Inner);

dialog_impls!(OpenProxy_Impl);
dialog_impls!(SaveProxy_Impl);

impl IFileOpenDialog_Impl for OpenProxy_Impl {
    fn GetResults(&self) -> windows::core::Result<IShellItemArray> {
        guard(|| {
            if self.0.picked_any() {
                return self.0.results();
            }
            match self.0.real.cast::<IFileOpenDialog>() {
                Ok(d) => unsafe { d.GetResults() },
                Err(e) => Err(e),
            }
        })
    }
    fn GetSelectedItems(&self) -> windows::core::Result<IShellItemArray> {
        guard(|| {
            if self.0.picked_any() {
                return self.0.results();
            }
            match self.0.real.cast::<IFileOpenDialog>() {
                Ok(d) => unsafe { d.GetSelectedItems() },
                Err(e) => Err(e),
            }
        })
    }
}

impl IFileSaveDialog_Impl for SaveProxy_Impl {
    fn SetSaveAsItem(&self, item: Option<&IShellItem>) -> windows::core::Result<()> {
        guard(|| {
            // a suggested existing item: the name and folder are what the picker needs of it
            self.real_save().and_then(|d| unsafe { d.SetSaveAsItem(item) })
        })
    }
    fn SetProperties(&self, store: Option<&IPropertyStore>) -> windows::core::Result<()> {
        guard(|| {
            self.0.st().properties = store.cloned();
            self.real_save().and_then(|d| unsafe { d.SetProperties(store) })
        })
    }
    /// The real dialog has a panel for editing these. The picker doesn't, so what the app set stays as it was.
    fn SetCollectedProperties(&self, list: Option<&IPropertyDescriptionList>, append: BOOL) -> windows::core::Result<()> {
        guard(|| {
            note("SetCollectedProperties: the picker has no panel for these, so they aren't editable there");
            self.real_save().and_then(|d| unsafe { d.SetCollectedProperties(list, append) })
        })
    }
    fn GetProperties(&self) -> windows::core::Result<IPropertyStore> {
        guard(|| {
            if let Ok(store) = self.real_save().and_then(|d| unsafe { d.GetProperties() }) {
                return Ok(store);
            }
            // The real one only has these once it's been shown.
            if let Some(store) = self.0.st().properties.clone() {
                return Ok(store);
            }
            let mut out = std::ptr::null_mut();
            unsafe { PSCreateMemoryPropertyStore(&IPropertyStore::IID, &mut out) }?;
            // SAFETY: on success it handed over an IPropertyStore reference.
            Ok(unsafe { IPropertyStore::from_raw(out) })
        })
    }
    fn ApplyProperties(&self, item: Option<&IShellItem>, store: Option<&IPropertyStore>, hwnd: HWND, sink: Option<&IFileOperationProgressSink>) -> windows::core::Result<()> {
        guard(|| {
            let forwarded = self.real_save().and_then(|d| unsafe { d.ApplyProperties(item, store, hwnd, sink) });
            if forwarded.is_ok() || self.0.st().used_windows {
                return forwarded;
            }
            note(&format!("ApplyProperties: the real dialog said {:?} as it wasn't shown, so writing them here", forwarded.err().map(|e| e.code())));
            let source = store.cloned().or_else(|| self.0.st().properties.clone());
            match (item, source) {
                (Some(item), Some(source)) => apply_properties(item, &source),
                _ => Ok(()),
            }
        })
    }
}

/// Writes properties to a file, for a real dialog that won't as it was never shown.
fn apply_properties(item: &IShellItem, source: &IPropertyStore) -> windows::core::Result<()> {
    let item: IShellItem2 = item.cast()?;
    let target: IPropertyStore = unsafe { item.GetPropertyStore(GPS_READWRITE) }?;
    unsafe {
        for i in 0..source.GetCount()? {
            let mut key = Default::default();
            source.GetAt(i, &mut key)?;
            let value = source.GetValue(&key)?;
            target.SetValue(&key, &value)?;
        }
        target.Commit()
    }
}

impl SaveProxy_Impl {
    fn real_save(&self) -> windows::core::Result<IFileSaveDialog> {
        self.0.real.cast()
    }
}

// ---------------------------------------------------------------- interfaces this doesn't implement

/// The `implement` macro's QueryInterface answers E_NOINTERFACE for anything it wasn't told about, with no way to step
/// in. An app may ask for more, such as a private interface of the real dialog. So each object's tables of functions
/// (its own memory, nobody else's) are switched for copies whose QueryInterface comes here first.
struct Tables {
    /// (the macro's table, the copy of it)
    list: Vec<(usize, usize)>,
}

static TABLES: Mutex<Tables> = Mutex::new(Tables { list: Vec::new() });
/// (the object's identity, its real dialog), for as long as it's alive
static OBJECTS: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

type QueryInterface = unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT;

/// What COM itself asks for to move a pointer between threads and apartments, or to see what it's dealing with. The
/// real dialog's answer would describe the real dialog, not this one, so these get the macro's own "no".
const NEVER_FORWARDED: [GUID; 7] = [
    GUID::from_u128(0x00000003_0000_0000_C000_000000000046), // IMarshal
    GUID::from_u128(0x000001CF_0000_0000_C000_000000000046), // IMarshal2
    GUID::from_u128(0x00000018_0000_0000_C000_000000000046), // IStdMarshalInfo
    GUID::from_u128(0x00000019_0000_0000_C000_000000000046), // IExternalConnection
    GUID::from_u128(0x0000013D_0000_0000_C000_000000000046), // IClientSecurity
    GUID::from_u128(0x0000013E_0000_0000_C000_000000000046), // IServerSecurity
    GUID::from_u128(0x1C733A30_2A1C_11CE_ADE5_00AA0044773D), // ICallFactory
];

unsafe extern "system" fn query_interface(this: *mut c_void, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let copy = *(this as *const usize);
        let table = {
            let tables = TABLES.lock().unwrap_or_else(PoisonError::into_inner);
            tables.list.iter().find(|(_, c)| *c == copy).map(|(m, _)| *m)
        };
        let Some(table) = table else { return E_NOINTERFACE };
        // SAFETY: the first entry of a COM table is its QueryInterface.
        let macro_qi: QueryInterface = std::mem::transmute(*(table as *const usize));
        let hr = macro_qi(this, iid, out);
        if hr != E_NOINTERFACE || iid.is_null() || out.is_null() {
            return hr;
        }
        let wanted = *iid;
        if NEVER_FORWARDED.contains(&wanted) {
            note(&format!("asked for COM's own interface {wanted:?}: not implemented, as for any plain object"));
            return hr;
        }
        let mut identity = std::ptr::null_mut();
        if macro_qi(this, &IUnknown::IID, &mut identity).is_err() {
            return hr;
        }
        let real = {
            let objects = OBJECTS.lock().unwrap_or_else(PoisonError::into_inner);
            objects.iter().find(|(o, _)| *o == identity as usize).map(|(_, r)| *r as *mut c_void).and_then(|r| IFileDialog::from_raw_borrowed(&r).cloned())
        };
        drop(IUnknown::from_raw(identity));
        let Some(real) = real else {
            note(&format!("asked for unknown interface {wanted:?}: no"));
            return hr;
        };
        // Its methods run on the real dialog: whatever the app sets there, it also has if Windows' dialog is shown, and
        // results it reads there are what the real one has. Nothing else about this dialog changes.
        let forwarded = real.query(&wanted, out);
        note(&format!("asked for unknown interface {wanted:?}: the real dialog says {forwarded:?}"));
        if forwarded.is_ok() {
            forwarded
        } else {
            hr
        }
    }))
    .unwrap_or(E_NOINTERFACE)
}

/// Switches one interface of an object to the copy of its table. `words` is how many entries that table has.
///
/// # Safety
/// `interface` is an interface pointer of one of this file's objects, and has at least that many entries.
unsafe fn watch_queries(interface: *mut c_void, words: usize) {
    let slot = interface as *mut usize;
    let current = *slot;
    let mut tables = TABLES.lock().unwrap_or_else(PoisonError::into_inner);
    if tables.list.iter().any(|(_, c)| *c == current) {
        return;
    }
    let copy = match tables.list.iter().find(|(m, _)| *m == current) {
        Some((_, c)) => *c,
        None => {
            let mut table: Vec<usize> = std::slice::from_raw_parts(current as *const usize, words).to_vec();
            table[0] = query_interface as QueryInterface as usize;
            // one per interface of each kind of dialog, kept for as long as the DLL is loaded
            let copy = Box::leak(table.into_boxed_slice()).as_ptr() as usize;
            tables.list.push((current, copy));
            copy
        }
    };
    *slot = copy;
}

fn words<T>() -> usize {
    std::mem::size_of::<T>() / std::mem::size_of::<usize>()
}

/// An object whose every interface comes through `query_interface`, and whose real dialog that can find.
fn watched(object: IUnknown, real: &IFileDialog, first: (*mut c_void, usize), others: &[(*mut c_void, usize)]) -> IUnknown {
    OBJECTS.lock().unwrap_or_else(PoisonError::into_inner).push((object.as_raw() as usize, real.as_raw() as usize));
    // SAFETY: all are this object's interfaces, with the sizes of their own tables. The identity's table is IInspectable's.
    unsafe {
        watch_queries(object.as_raw(), 6);
        watch_queries(first.0, first.1);
        for (p, n) in others {
            watch_queries(*p, *n);
        }
    }
    object
}

impl Drop for Inner {
    fn drop(&mut self) {
        let real = self.real.as_raw() as usize;
        OBJECTS.lock().unwrap_or_else(PoisonError::into_inner).retain(|(_, r)| *r != real);
    }
}

/// An app's Open dialog object, around the real one.
pub fn open(real: IFileOpenDialog) -> windows::core::Result<IUnknown> {
    let real: IFileDialog = real.cast()?;
    let d: IFileOpenDialog = OpenProxy(Inner::new(real.clone(), false)).into();
    let second: IFileDialog2 = d.cast()?;
    let custom: IFileDialogCustomize = d.cast()?;
    let window: IOleWindow = d.cast()?;
    let site: IObjectWithSite = d.cast()?;
    Ok(watched(
        d.cast()?,
        &real,
        (d.as_raw(), words::<IFileOpenDialog_Vtbl>()),
        &[(second.as_raw(), words::<IFileDialog2_Vtbl>()), (custom.as_raw(), words::<IFileDialogCustomize_Vtbl>()), (window.as_raw(), words::<IOleWindow_Vtbl>()), (site.as_raw(), words::<IObjectWithSite_Vtbl>())],
    ))
}

/// An app's Save dialog object, around the real one.
pub fn save(real: IFileSaveDialog) -> windows::core::Result<IUnknown> {
    let real: IFileDialog = real.cast()?;
    let d: IFileSaveDialog = SaveProxy(Inner::new(real.clone(), true)).into();
    let second: IFileDialog2 = d.cast()?;
    let custom: IFileDialogCustomize = d.cast()?;
    let window: IOleWindow = d.cast()?;
    let site: IObjectWithSite = d.cast()?;
    Ok(watched(
        d.cast()?,
        &real,
        (d.as_raw(), words::<IFileSaveDialog_Vtbl>()),
        &[(second.as_raw(), words::<IFileDialog2_Vtbl>()), (custom.as_raw(), words::<IFileDialogCustomize_Vtbl>()), (window.as_raw(), words::<IOleWindow_Vtbl>()), (site.as_raw(), words::<IObjectWithSite_Vtbl>())],
    ))
}
