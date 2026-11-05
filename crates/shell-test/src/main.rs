//! A small program of our own that does what real apps do with the shell, for testing the Explorer integration.
//! It prints one line of `key=value` per run, for scripts.
//!
//!   dialog-test open [--multi] [--folder-pick] [--start DIR] [--filter-ext txt] [--customize] [--reject-first] [--close-windows-dialog-after MS]
//!   dialog-test save [--name x.txt] [--ext txt] [--start DIR]
//!   dialog-test show-in-folder C:\path\file.txt
//!   dialog-test shell-open C:\path
//!   dialog-test office-save [--name x.docx] [--start DIR] [--tool-ui MS]   an imitation of how Office's documentation says
//!       it uses this dialog: not Office, and it can't show what real Office does
//!   dialog-test press-win-e
//!
//! Copied to a file called daleris-probe.exe it stands in for daleris.exe instead: it writes down what it was started
//! with to the log named in daleris-probe.cfg next to it (log, real program, APPDATA, LOCALAPPDATA, one per line), and
//! runs the real program with those folders, so that a test can't reach anyone's real settings.

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn main() {
    if let Some(code) = win::probe() {
        std::process::exit(code);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
    }
    let code = match args.first().map(String::as_str) {
        Some("open") => win::dialog(false, &args[1..]),
        Some("save") => win::dialog(true, &args[1..]),
        Some("office-save") => win::office_save(&args[1..]),
        Some("show-in-folder") => win::show_in_folder(&args[1]),
        Some("shell-open") => win::shell_open(&args[1]),
        Some("press-win-e") => win::press_win_e(),
        Some("press-ctrl-g") => win::press_ctrl_g(),
        Some("foreground") => win::foreground(),
        Some("odd-arg") => win::odd_arg(&args[1], &args[2]),
        _ => {
            println!("result=usage");
            2
        }
    };
    std::process::exit(code);
}

#[cfg(windows)]
mod win {
    use std::cell::Cell;
    use std::ffi::{c_void, OsStr};
    use std::os::windows::ffi::OsStrExt;
    use std::time::Instant;

    use windows::core::{implement, w, Interface, HRESULT, PCWSTR};
    use windows::Win32::Foundation::*;
    use windows::Win32::System::Com::*;
    use windows::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT};
    use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
    use windows::Win32::UI::Shell::*;
    use windows::Win32::UI::WindowsAndMessaging::*;

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }

    /// The DLL a COM object's code lives in: what tells a replaced dialog from Windows' own.
    fn module_of(object: &impl Interface) -> String {
        // SAFETY: an interface pointer points at a pointer to its table of functions.
        let code = unsafe { **(object.as_raw() as *const *const *const c_void) };
        let mut module = HMODULE::default();
        let flags = GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
        if unsafe { GetModuleHandleExW(flags, PCWSTR(code as *const u16), &mut module) }.is_err() {
            return "?".into();
        }
        let mut buf = [0u16; 512];
        let len = unsafe { GetModuleFileNameW(module, &mut buf) } as usize;
        String::from_utf16_lossy(&buf[..len]).rsplit('\\').next().unwrap_or("?").to_string()
    }

    fn path_of(item: &IShellItem) -> String {
        match unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) } {
            Ok(p) => unsafe {
                let s = p.to_string().unwrap_or_default();
                CoTaskMemFree(Some(p.0 as *const c_void));
                s
            },
            Err(e) => format!("<{e}>"),
        }
    }

    #[implement(IFileDialogEvents, IFileDialogControlEvents)]
    struct Events {
        /// says no to this many OnFileOk calls
        reject: Cell<u32>,
        ok_calls: Cell<u32>,
        /// a window of its own this long when the Tools menu is used, as Office opens one
        tool_ui: Cell<u32>,
    }

    impl IFileDialogEvents_Impl for Events_Impl {
        fn OnFileOk(&self, dialog: Option<&IFileDialog>) -> windows::core::Result<()> {
            self.ok_calls.set(self.ok_calls.get() + 1);
            // what apps read here must already be the answer
            if let Some(d) = dialog {
                if let Ok(name) = unsafe { d.GetFileName() } {
                    println!("event=OnFileOk name={}", unsafe { name.to_string() }.unwrap_or_default());
                    unsafe { CoTaskMemFree(Some(name.0 as *const c_void)) };
                }
            }
            if self.reject.get() > 0 {
                self.reject.set(self.reject.get() - 1);
                return Err(S_FALSE.into());
            }
            Ok(())
        }
        fn OnFolderChanging(&self, _: Option<&IFileDialog>, _: Option<&IShellItem>) -> windows::core::Result<()> {
            Ok(())
        }
        fn OnFolderChange(&self, dialog: Option<&IFileDialog>) -> windows::core::Result<()> {
            if let Some(folder) = dialog.and_then(|d| unsafe { d.GetFolder() }.ok()) {
                println!("event=OnFolderChange folder={}", path_of(&folder));
            }
            Ok(())
        }
        fn OnSelectionChange(&self, _: Option<&IFileDialog>) -> windows::core::Result<()> {
            Ok(())
        }
        fn OnShareViolation(&self, _: Option<&IFileDialog>, _: Option<&IShellItem>) -> windows::core::Result<FDE_SHAREVIOLATION_RESPONSE> {
            Ok(FDESVR_DEFAULT)
        }
        fn OnTypeChange(&self, _: Option<&IFileDialog>) -> windows::core::Result<()> {
            Ok(())
        }
        fn OnOverwrite(&self, _: Option<&IFileDialog>, _: Option<&IShellItem>) -> windows::core::Result<FDE_OVERWRITE_RESPONSE> {
            Ok(FDEOR_DEFAULT)
        }
    }

    impl IFileDialogControlEvents_Impl for Events_Impl {
        fn OnItemSelected(&self, c: Option<&IFileDialogCustomize>, id: u32, item: u32) -> windows::core::Result<()> {
            println!("event=OnItemSelected control={id} item={item}");
            if id == MENU && item == 1 {
                let ms = self.tool_ui.get();
                if ms > 0 {
                    // what Office's Tools > General Options does: the app's own modal window, from inside the event
                    let started = Instant::now();
                    unsafe extern "system" {
                        fn MessageBoxTimeoutW(hwnd: HWND, text: PCWSTR, caption: PCWSTR, kind: u32, language: u16, ms: u32) -> i32;
                    }
                    #[link(name = "user32")]
                    unsafe extern "system" {}
                    unsafe { MessageBoxTimeoutW(HWND::default(), w!("The app's own window, from a Tools menu item"), w!("App tool"), 0x1000, 0, ms) };
                    println!("event=tool window closed after {}ms", started.elapsed().as_millis());
                }
                // and what an app can do to its own controls as a result
                if let Some(c) = c {
                    let _ = unsafe { c.SetControlLabel(CHECK, w!("Save Thumbnail (changed by the app)")) };
                }
            }
            Ok(())
        }
        fn OnButtonClicked(&self, c: Option<&IFileDialogCustomize>, id: u32) -> windows::core::Result<()> {
            println!("event=OnButtonClicked control={id}");
            if let Some(c) = c {
                let _ = unsafe { c.SetControlState(DISABLED, CDCS_ENABLEDVISIBLE) };
            }
            Ok(())
        }
        fn OnCheckButtonToggled(&self, _: Option<&IFileDialogCustomize>, id: u32, on: BOOL) -> windows::core::Result<()> {
            println!("event=OnCheckButtonToggled control={id} on={}", on.as_bool());
            Ok(())
        }
        fn OnControlActivating(&self, c: Option<&IFileDialogCustomize>, id: u32) -> windows::core::Result<()> {
            println!("event=OnControlActivating control={id}");
            // Office fills its Tools menu as it opens
            if let (MENU, Some(c)) = (id, c) {
                unsafe {
                    let _ = c.RemoveAllControlItems(MENU);
                    let _ = c.AddControlItem(MENU, 1, w!("&General Options..."));
                    let _ = c.AddControlItem(MENU, 2, w!("Web Options (added when opened)..."));
                }
            }
            Ok(())
        }
    }

    const MENU: u32 = 1001;
    const COMBO: u32 = 1002;
    const CHECK: u32 = 1003;
    const EDIT: u32 = 1005;
    const RADIO: u32 = 1006;
    const BUTTON: u32 = 1007;
    const DISABLED: u32 = 1009;
    const HIDDEN: u32 = 1011;

    /// A Save dialog set up the way Office's is documented and reported to be. An imitation, for what can be checked
    /// without Office: real Office may do more than this, in other orders.
    pub fn office_save(args: &[String]) -> i32 {
        use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyDescriptionList, PSGetPropertyDescriptionListFromString};
        let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
        let dialog: IFileSaveDialog = unsafe { CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER) }.expect("create the dialog");
        let common: IFileDialog = dialog.cast().unwrap();
        println!("object_from={} daleris_dll_loaded={}", module_of(&common), ours_loaded());
        let custom: IFileDialogCustomize = dialog.cast().unwrap();
        unsafe {
            common.SetOptions(common.GetOptions().unwrap() | FOS_OVERWRITEPROMPT | FOS_FORCEFILESYSTEM).unwrap();
            common.SetTitle(w!("Save As (imitation of Office)")).unwrap();
            let specs = [COMDLG_FILTERSPEC { pszName: w!("Word Document"), pszSpec: w!("*.docx") }, COMDLG_FILTERSPEC { pszName: w!("Plain Text"), pszSpec: w!("*.txt") }];
            common.SetFileTypes(&specs).unwrap();
            common.SetFileTypeIndex(1).unwrap();
            common.SetDefaultExtension(w!("docx")).unwrap();
            let n = wide(&value("--name").unwrap_or_else(|| "Report".to_string()));
            common.SetFileName(PCWSTR(n.as_ptr())).unwrap();
            if let Some(start) = value("--start") {
                let w = wide(&start);
                let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(w.as_ptr()), None).unwrap();
                common.SetFolder(&item).unwrap();
            }
            custom.AddMenu(MENU, w!("&Tools")).unwrap();
            custom.AddControlItem(MENU, 1, w!("&General Options...")).unwrap();
            custom.AddComboBox(COMBO).unwrap();
            custom.AddControlItem(COMBO, 1, w!("Maintain compatibility")).unwrap();
            custom.AddControlItem(COMBO, 2, w!("Newest format")).unwrap();
            custom.SetSelectedControlItem(COMBO, 2).unwrap();
            custom.AddCheckButton(CHECK, w!("Save &Thumbnail"), true).unwrap();
            custom.StartVisualGroup(1010, w!("Details")).unwrap();
            custom.AddText(1004, w!("Authors:")).unwrap();
            custom.AddEditBox(EDIT, w!("Someone")).unwrap();
            custom.EndVisualGroup().unwrap();
            custom.AddRadioButtonList(RADIO).unwrap();
            custom.AddControlItem(RADIO, 1, w!("Whole document")).unwrap();
            custom.AddControlItem(RADIO, 2, w!("Selection")).unwrap();
            custom.SetSelectedControlItem(RADIO, 1).unwrap();
            custom.AddPushButton(BUTTON, w!("Options...")).unwrap();
            custom.AddSeparator(1008).unwrap();
            custom.AddCheckButton(DISABLED, w!("Disabled until Options is pressed"), false).unwrap();
            custom.SetControlState(DISABLED, CDCS_VISIBLE).unwrap();
            custom.AddCheckButton(HIDDEN, w!("Never shown"), false).unwrap();
            custom.SetControlState(HIDDEN, CDCS_ENABLED).unwrap();
            custom.MakeProminent(COMBO).unwrap();
            let mut list = std::ptr::null_mut();
            PSGetPropertyDescriptionListFromString(w!("prop:System.Author;System.Keywords"), &IPropertyDescriptionList::IID, &mut list).unwrap();
            let list = IPropertyDescriptionList::from_raw(list);
            println!("SetCollectedProperties={:?}", dialog.SetCollectedProperties(&list, true).map_err(|e| e.code()));
        }
        // what an app that goes looking gets, for interfaces nobody implements here
        for (name, iid) in [("a made-up one", windows::core::GUID::from_u128(0x12345678_1234_1234_1234_123456789ABC)), ("IPersist", windows::core::GUID::from_u128(0x0000010C_0000_0000_C000_000000000046)), ("IServiceProvider", IServiceProvider::IID)] {
            let mut out = std::ptr::null_mut();
            let hr = unsafe { dialog.query(&iid, &mut out) };
            println!("query {name}: {hr:?}");
            if hr.is_ok() {
                drop(unsafe { windows::core::IUnknown::from_raw(out) });
            }
        }
        let tool_ui = value("--tool-ui").and_then(|m| m.parse().ok()).unwrap_or(0);
        let events: IFileDialogEvents = Events { reject: Cell::new(1), ok_calls: Cell::new(0), tool_ui: Cell::new(tool_ui) }.into();
        let cookie = unsafe { common.Advise(&events) }.unwrap();
        if let Some(ms) = value("--close-windows-dialog-after").and_then(|m| m.parse::<u64>().ok()) {
            close_windows_dialog_after(ms);
        }
        let shown = unsafe { common.Show(HWND::default()) };
        let _ = unsafe { common.Unadvise(cookie) };
        // everything an app reads back afterwards
        unsafe {
            println!("check={:?}", custom.GetCheckButtonState(CHECK).map(|b| b.as_bool()).map_err(|e| e.code()));
            println!("combo={:?}", custom.GetSelectedControlItem(COMBO).map_err(|e| e.code()));
            println!("radio={:?}", custom.GetSelectedControlItem(RADIO).map_err(|e| e.code()));
            match custom.GetEditBoxText(EDIT) {
                Ok(p) => {
                    println!("edit={:?}", PCWSTR(p).to_string());
                    CoTaskMemFree(Some(p as *const c_void));
                }
                Err(e) => println!("edit error {:?}", e.code()),
            }
            println!("disabled_state={:?}", custom.GetControlState(DISABLED).map(|s| s.0).map_err(|e| e.code()));
        }
        println!("type_index={:?}", unsafe { common.GetFileTypeIndex() }.map_err(|e| e.code()));
        match shown {
            Ok(()) => {
                let item = unsafe { common.GetResult() }.expect("a result");
                let path = path_of(&item);
                println!("result=ok path={path}");
                let store = unsafe { dialog.GetProperties() };
                println!("GetProperties={:?}", store.as_ref().map(|s| unsafe { s.GetCount() }.unwrap_or(0)).map_err(|e| e.code()));
                if value("--write").is_some() {
                    let _ = std::fs::write(&path, b"saved by dialog-test\n");
                    if let Ok(store) = &store {
                        let applied = unsafe { dialog.ApplyProperties(&item, store, HWND::default(), None) };
                        println!("ApplyProperties={:?}", applied.map_err(|e| e.code()));
                    }
                }
                0
            }
            Err(e) if e.code() == HRESULT(0x800704C7u32 as i32) => {
                println!("result=cancelled");
                1
            }
            Err(e) => {
                println!("result=error code={:?}", e.code());
                3
            }
        }
    }

    pub fn dialog(save: bool, args: &[String]) -> i32 {

        let has = |flag: &str| args.iter().any(|a| a == flag);
        let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
        let started = Instant::now();
        let dialog: IFileDialog = unsafe {
            if save {
                CoCreateInstance::<_, IFileSaveDialog>(&FileSaveDialog, None, CLSCTX_INPROC_SERVER).and_then(|d| d.cast())
            } else {
                CoCreateInstance::<_, IFileOpenDialog>(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).and_then(|d| d.cast())
            }
        }
        .expect("create the dialog");
        println!("object_from={} daleris_dll_loaded={}", module_of(&dialog), ours_loaded());
        unsafe {
            let mut o = dialog.GetOptions().unwrap();
            if has("--multi") {
                o |= FOS_ALLOWMULTISELECT;
            }
            if has("--folder-pick") {
                o |= FOS_PICKFOLDERS;
            }
            dialog.SetOptions(o).unwrap();
            dialog.SetTitle(w!("dialog-test title")).unwrap();
            let specs = [
                COMDLG_FILTERSPEC { pszName: w!("Text"), pszSpec: w!("*.txt;*.md") },
                COMDLG_FILTERSPEC { pszName: w!("Everything"), pszSpec: w!("*.*") },
            ];
            // Windows' own dialog refuses these for a folder picker
            if !has("--folder-pick") {
                dialog.SetFileTypes(&specs).unwrap();
                dialog.SetFileTypeIndex(1).unwrap();
            }
            if let Some(name) = value("--name") {
                let n = wide(&name);
                dialog.SetFileName(PCWSTR(n.as_ptr())).unwrap();
            }
            if let Some(ext) = value("--ext") {
                let n = wide(&ext);
                dialog.SetDefaultExtension(PCWSTR(n.as_ptr())).unwrap();
            }
            if let Some(start) = value("--start") {
                let n = wide(&start);
                let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(n.as_ptr()), None).unwrap();
                dialog.SetFolder(&item).unwrap();
            }
            if has("--customize") {
                let c: IFileDialogCustomize = dialog.cast().unwrap();
                c.AddCheckButton(100, w!("An extra check box"), false).unwrap();
            }
        }
        let reject = if has("--reject-first") { 1 } else { 0 };
        if let Some(ms) = value("--close-windows-dialog-after").and_then(|m| m.parse::<u64>().ok()) {
            close_windows_dialog_after(ms);
        }
        let events: IFileDialogEvents = Events { reject: Cell::new(reject), ok_calls: Cell::new(0), tool_ui: Cell::new(0) }.into();
        let cookie = unsafe { dialog.Advise(&events) }.unwrap();
        let shown = unsafe { dialog.Show(HWND::default()) };
        let ms = started.elapsed().as_millis();
        let _ = unsafe { dialog.Unadvise(cookie) };
        if has("--print-name") {
            if let Ok(name) = unsafe { dialog.GetFileName() } {
                println!("name_after={}", unsafe { name.to_string() }.unwrap_or_default());
                unsafe { CoTaskMemFree(Some(name.0 as *const c_void)) };
            }
        }
        let code = match shown {
            Ok(()) => {
                let index = unsafe { dialog.GetFileTypeIndex() }.unwrap_or(0);
                let result = unsafe { dialog.GetResult() };
                println!("result=ok ms={ms} type_index={index} path={}", result.map(|i| path_of(&i)).unwrap_or_else(|e| format!("<{e}>")));
                if let Ok(open) = dialog.cast::<IFileOpenDialog>() {
                    if let Ok(items) = unsafe { open.GetResults() } {
                        let count = unsafe { items.GetCount() }.unwrap_or(0);
                        for i in 0..count {
                            if let Ok(item) = unsafe { items.GetItemAt(i) } {
                                println!("item={}", path_of(&item));
                            }
                        }
                    }
                }
                0
            }
            Err(e) if e.code() == HRESULT(0x800704C7u32 as i32) => {
                println!("result=cancelled ms={ms}");
                1
            }
            Err(e) => {
                println!("result=error ms={ms} code={:?}", e.code());
                3
            }
        };
        if has("--free") {
            drop(events);
            drop(dialog);
            // what an app that asks COM to let go right away gets; most only get it from COM's own timer, or at exit
            unsafe { CoFreeUnusedLibrariesEx(0, 0) };
            println!("daleris_dll_loaded_after_free={}", ours_loaded());
        }
        code
    }

    /// A test's stand-in for a person closing Windows' own dialog, which it can tell by its class.
    fn close_windows_dialog_after(ms: u64) {
        unsafe extern "system" fn each(hwnd: HWND, _: LPARAM) -> BOOL {
            let mut owner = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut owner));
            let mut class = [0u16; 16];
            let n = GetClassNameW(hwnd, &mut class) as usize;
            if owner == windows::Win32::System::Threading::GetCurrentProcessId() && IsWindowVisible(hwnd).as_bool() && String::from_utf16_lossy(&class[..n]) == "#32770" {
                println!("windows_dialog_seen=1");
                let _ = PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            true.into()
        }
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(ms));
            let _ = unsafe { EnumWindows(Some(each), LPARAM(0)) };
        });
    }

    /// Is any file of ours loaded in this process?
    fn ours_loaded() -> bool {
        use windows::Win32::System::ProcessStatus::{EnumProcessModules, GetModuleBaseNameW};
        use windows::Win32::System::Threading::GetCurrentProcess;
        let mut modules = vec![HMODULE::default(); 2048];
        let mut needed = 0;
        let size = std::mem::size_of_val(modules.as_slice()) as u32;
        unsafe {
            let process = GetCurrentProcess();
            if EnumProcessModules(process, modules.as_mut_ptr(), size, &mut needed).is_err() {
                return false;
            }
            modules[..(needed as usize / std::mem::size_of::<HMODULE>()).min(modules.len())].iter().any(|&m| {
                let mut name = [0u16; 260];
                let len = GetModuleBaseNameW(process, m, &mut name) as usize;
                String::from_utf16_lossy(&name[..len]).to_lowercase().contains("daleris")
            })
        }
    }

    pub fn show_in_folder(path: &str) -> i32 {
        let started = Instant::now();
        let p = wide(path);
        let mut pidl = std::ptr::null_mut();
        let parsed = unsafe { SHParseDisplayName(PCWSTR(p.as_ptr()), None, &mut pidl, 0, None) };
        if parsed.is_err() {
            println!("result=parse-failed");
            return 3;
        }
        let shown = unsafe { SHOpenFolderAndSelectItems(pidl, None, 0) };
        println!("result={:?} ms={}", shown.map_err(|e| e.code()), started.elapsed().as_millis());
        0
    }

    /// A simulated press: all a test has. Real presses go the same way once Windows has them, but not through this.
    pub fn press_win_e() -> i32 {
        use windows::Win32::UI::Input::KeyboardAndMouse::*;
        let key = |vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS| INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } } };
        let inputs = [key(VK_LWIN, KEYBD_EVENT_FLAGS(0)), key(VK_E, KEYBD_EVENT_FLAGS(0)), key(VK_E, KEYEVENTF_KEYUP), key(VK_LWIN, KEYEVENTF_KEYUP)];
        let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
        println!("result=sent {sent}");
        0
    }

    /// What's in front, as a Start menu that opened by mistake would be.
    pub fn foreground() -> i32 {
        let hwnd = unsafe { GetForegroundWindow() };
        let (mut class, mut title) = ([0u16; 128], [0u16; 128]);
        let c = unsafe { GetClassNameW(hwnd, &mut class) } as usize;
        let t = unsafe { GetWindowTextW(hwnd, &mut title) } as usize;
        println!("result=class:{} title:{}", String::from_utf16_lossy(&class[..c]), String::from_utf16_lossy(&title[..t]));
        0
    }

    /// Ctrl + G goes to whatever window is in front.
    pub fn press_ctrl_g() -> i32 {
        use windows::Win32::UI::Input::KeyboardAndMouse::*;
        let key = |vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS| INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } } };
        let inputs = [key(VK_CONTROL, KEYBD_EVENT_FLAGS(0)), key(VK_G, KEYBD_EVENT_FLAGS(0)), key(VK_G, KEYEVENTF_KEYUP), key(VK_CONTROL, KEYEVENTF_KEYUP)];
        let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
        println!("result=sent {sent}");
        0
    }

    /// Runs a program with a path Windows allows and Unicode doesn't, which used to panic before anything else happened.
    pub fn odd_arg(exe: &str, shot: &str) -> i32 {
        use std::os::windows::ffi::OsStringExt;
        let odd = std::ffi::OsString::from_wide(&[0x43, 0x3a, 0x5c, 0xd800, 0x41]);
        let status = std::process::Command::new(exe).arg("--screenshot").arg(shot).arg(odd).status();
        println!("result={:?}", status.map(|s| s.code()));
        0
    }

    pub fn probe() -> Option<i32> {
        let me = std::env::current_exe().ok()?;
        if me.file_stem()?.to_str()? != "daleris-probe" {
            return None;
        }
        let cfg = std::fs::read_to_string(me.with_extension("cfg")).ok()?;
        let lines: Vec<&str> = cfg.lines().collect();
        let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let line = format!("{now} {args:?}\n");
        let mut log = std::fs::OpenOptions::new().create(true).append(true).open(lines[0]).ok()?;
        std::io::Write::write_all(&mut log, line.as_bytes()).ok()?;
        if lines.get(1).is_some_and(|l| !l.is_empty() && *l != "-") {
            let _ = std::process::Command::new(lines[1]).args(&args).env("APPDATA", lines[2]).env("LOCALAPPDATA", lines[3]).spawn();
        }
        Some(0)
    }

    pub fn shell_open(path: &str) -> i32 {
        let p = wide(path);
        let rc = unsafe { ShellExecuteW(None, w!("open"), PCWSTR(p.as_ptr()), None, None, SW_SHOWNORMAL) };
        println!("result={}", rc.0 as isize);
        0
    }

}
