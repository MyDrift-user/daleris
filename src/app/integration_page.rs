//! The Windows Explorer switches: how Settings holds them, keeping the system in step with them, the page they're on,
//! and the same thing from the command line (`--restore-explorer`, for uninstalling).

use daleris_integration::deny;

use super::settings_page::{card, managed_note, managed_toggle, toggle};
use super::*;
use crate::integration::{self, Feature, Live};

/// What the command line asks the first window to show.
#[derive(Default)]
pub struct Wanted {
    pub dir: Option<PathBuf>,
    /// the name in `dir` to select
    pub select: Option<String>,
    pub this_pc: bool,
}

impl Settings {
    /// Every switch as it's set now, which policy has already had its say in.
    pub(super) fn explorer_switches(&self) -> Vec<(Feature, bool)> {
        vec![
            (Feature::Folders, self.exp_folders),
            (Feature::WinE, self.exp_win_e),
            (Feature::WinEHook, self.exp_win_e_hook),
            (Feature::Select, self.exp_select),
            (Feature::SingleWindow, self.exp_single),
            (Feature::Dialogs, self.exp_dialogs),
            (Feature::DialogJump, self.exp_jump),
            (Feature::DialogLog, self.exp_dialog_log),
        ]
    }

    fn switch_mut(&mut self, f: Feature) -> &mut bool {
        match f {
            Feature::Folders => &mut self.exp_folders,
            Feature::WinE => &mut self.exp_win_e,
            Feature::WinEHook => &mut self.exp_win_e_hook,
            Feature::Select => &mut self.exp_select,
            Feature::SingleWindow => &mut self.exp_single,
            Feature::Dialogs => &mut self.exp_dialogs,
            Feature::DialogJump => &mut self.exp_jump,
            Feature::DialogLog => &mut self.exp_dialog_log,
        }
    }
}

/// The settings file key of a switch.
pub(super) fn switch_key(f: Feature) -> &'static str {
    match f {
        Feature::Folders => "exp_folders",
        Feature::WinE => "exp_win_e",
        Feature::WinEHook => "exp_win_e_hook",
        Feature::Select => "exp_select",
        Feature::SingleWindow => "exp_single",
        Feature::Dialogs => "exp_dialogs",
        Feature::DialogJump => "exp_jump",
        Feature::DialogLog => "exp_dialog_log",
    }
}

impl App {
    /// Once per frame: keeps the system matching the switches, however they changed (here, in another window, or by
    /// a policy, which is how a switch forced off gets removed while running and at the next start).
    pub(super) fn explorer_tick(&mut self, ctx: &egui::Context) {
        if self.picker.is_some() || !self.writes_settings() || integration::is_elevated() {
            return;
        }
        let wanted = (self.settings.explorer_switches(), self.settings.exp_exclude.clone());
        if self.explorer_applied.as_ref() != Some(&wanted) {
            if let Err(e) = integration::sync(&wanted.0, &wanted.1) {
                self.toast(e, true);
                // what just got turned on doesn't stay on, so that the switch says what's true
                let before = self.explorer_applied.as_ref().map(|(w, _)| w.clone()).unwrap_or_default();
                for (f, on) in wanted.0.iter().copied() {
                    let was_on = before.iter().any(|(g, o)| *g == f && *o);
                    if on && !was_on && !policy::is_fixed(switch_key(f)) {
                        *self.settings.switch_mut(f) = false;
                    }
                }
                self.save_settings();
            }
            self.explorer_applied = Some((self.settings.explorer_switches(), self.settings.exp_exclude.clone()));
            self.explorer_status_at = None;
            self.explorer_processes(ctx);
        }
        self.take_requests();
        // the window in front is the one Ctrl + G goes by, when several are open
        if self.settings.exp_jump && ctx.input(|i| i.viewport().focused).unwrap_or(true) {
            let dir = self.tabs[self.active].dir().map(Path::to_path_buf);
            if self.jump_told != Some(dir.clone()) {
                integration::set_jump_folder(dir.as_deref());
                self.jump_told = Some(dir);
            }
        }
    }

    /// What of the switches runs inside this process.
    #[cfg(windows)]
    fn explorer_processes(&mut self, ctx: &egui::Context) {
        if self.settings.exp_single {
            if self.requests.is_none() {
                self.serving.store(true, Ordering::SeqCst);
                self.requests = integration::serve_requests(ctx.clone(), self.serving.clone());
            }
        } else if self.requests.take().is_some() {
            self.serving.store(false, Ordering::SeqCst);
            integration::stop_serving();
        }
        // told again after any change of switches, as turning it off and on again wipes what it knew
        self.jump_told = None;
    }

    #[cfg(not(windows))]
    fn explorer_processes(&mut self, _: &egui::Context) {}

    /// Folders that new launches sent to this window.
    #[cfg(windows)]
    fn take_requests(&mut self) {
        use daleris_integration::pipe::Request;
        // new launches, through the shared pipe, and tabs other windows hand over, through this window's own
        let mut list: Vec<Request> = self.requests.as_ref().map(|rx| rx.try_iter().collect()).unwrap_or_default();
        list.extend(self.tab_requests.as_ref().map(|rx| rx.try_iter().collect::<Vec<_>>()).unwrap_or_default());
        for request in list {
            log::info!("a new launch asks this window for {request:?}");
            match request {
                Request::Open(p) => {
                    let p = PathBuf::from(p);
                    if !self.settings.open_new_drives && super::is_new_drive(&p) {
                        log::info!("{} was just connected, and new drives don't open a tab", p.display());
                        continue;
                    }
                    self.open_tab(Loc::Dir(p), true);
                }
                Request::Select(p) => {
                    let p = PathBuf::from(p);
                    if let Some(dir) = p.parent() {
                        self.open_tab(Loc::Dir(dir.to_path_buf()), true);
                        let tab = self.tabs.len() - 1;
                        self.tabs[tab].select_after_load = p.file_name().map(|n| n.to_string_lossy().into_owned());
                    }
                }
                Request::ThisPc => self.open_tab(Loc::ThisPc, true),
                Request::Show => {
                    let home = self.places.iter().find(|p| p.name == "Home").map(|p| p.path.clone()).unwrap_or_else(|| PathBuf::from("C:\\"));
                    self.open_tab(Loc::Dir(home), true);
                }
            }
        }
    }

    #[cfg(not(windows))]
    fn take_requests(&mut self) {}

    fn explorer_status(&mut self) -> &[(Feature, Live)] {
        if self.explorer_status_at.is_none_or(|t| t.elapsed() > Duration::from_secs(1)) {
            self.explorer_status_at = Some(Instant::now());
            self.explorer_live = Feature::ALL.iter().map(|&f| (f, integration::status(f))).collect();
        }
        &self.explorer_live
    }

    pub(super) fn page_explorer(&mut self, ui: &mut Ui) {
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new("Windows Explorer").size(15.0).strong());
            ui.label(egui::RichText::new("Make Daleris stand in for File Explorer. Each switch works on its own and is off until you turn it on. They only change your own account.").color(DIM()));
            if integration::is_elevated() {
                ui.add_space(6.0);
                ui.label(egui::RichText::new("Daleris is running as administrator, so these can't be changed here. Run it normally.").color(WARN_COLOR));
            }
        });
        ui.add_space(14.0);
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            // the whole look is on when every part below is set its way
            let mut look = self.settings.looks_like_explorer();
            if managed_toggle(ui, &mut look, "explorer_look", "Look like Windows Explorer") {
                self.settings.explorer_look(look);
                self.apply_theme();
            }
            ui.label(egui::RichText::new("Turns everything below on at once. Off puts it all back to how Daleris looks on its own.").color(DIM()));
            ui.add_space(8.0);
            let parts_free = !policy::is_fixed("explorer_look");
            ui.add_enabled_ui(parts_free, |ui| {
                ui.indent("explorer_parts", |ui| {
                    let s = &mut self.settings;
                    let mut changed = managed_toggle(ui, &mut s.explorer_colours, "explorer_colours", "Explorer's colours, following Windows's light or dark mode and accent");
                    changed |= managed_toggle(ui, &mut s.explorer_lines, "explorer_lines", "Explorer's lines between the panes and around the tab in front");
                    // Mica: Windows 11 only, and the window has to be made for it when it opens
                    if crate::shell::has_mica() {
                        changed |= managed_toggle(ui, &mut s.explorer_mica, "explorer_mica", "Mica behind the tab bar");
                        let mica = s.explorer_mica;
                        ui.indent("mica_parts", |ui| {
                            ui.add_enabled_ui(mica, |ui| {
                                changed |= managed_toggle(ui, &mut s.explorer_mica_bar, "explorer_mica_bar", "A little of it through the tab in front and the address bar");
                            });
                        });
                    }
                    if !prefers_bundled_icons() {
                        let mut system = s.icon_pack == "system";
                        if managed_toggle(ui, &mut system, "iconpack", "Windows's own icons") {
                            s.icon_pack = if system { "system".into() } else { "flat".into() };
                            changed = true;
                        }
                    }
                    let mut compact = s.density == 0;
                    if managed_toggle(ui, &mut compact, "density", "Compact rows") {
                        s.density = if compact { 0 } else { super::OWN_DENSITY };
                        changed = true;
                    }
                    changed |= managed_toggle(ui, &mut s.search_hide_sub, "search_hide_sub", "No subfolders switch in the search box");
                    if changed {
                        self.apply_theme();
                    }
                });
            });
            if !parts_free {
                managed_note(ui);
            }
            if self.settings.wants_mica() && crate::shell::has_mica() && !crate::theme::mica_window() {
                ui.add_space(4.0);
                ui.label(egui::RichText::new("Mica takes effect when Daleris starts again.").size(12.0).color(DIM()));
            }
        });
        ui.add_space(14.0);
        let enabled = !integration::is_elevated();
        let mut changed = false;
        ui.add_enabled_ui(enabled, |ui| {
            changed |= self.switch_card(ui, Feature::Folders, "Open folders in Daleris", "Double-clicking a folder on the desktop, opening one from Start, and what most apps do to open a folder, all come here.", Some("Apps that start explorer.exe themselves can't be redirected and still open File Explorer. Places only Explorer can show, such as the Recycle Bin and Control Panel, go to it too."));
            changed |= self.switch_card(ui, Feature::WinE, "Win + E opens Daleris", "Uses the setting Windows reads for this shortcut, like File Pilot's installer does.", Some("Recent Windows builds can skip that setting. If Win + E still opens File Explorer, turn on the next switch as well."));
            changed |= self.switch_card(ui, Feature::WinEHook, "Win + E opens Daleris even if Windows ignores the setting", "Runs a small background program from sign-in that watches for Win + E. Only that key combination is kept from Windows.", Some("It's a system-wide keyboard hook, in a process of its own: nothing loads into other apps, but some anti-cheat software looks for hooks like this. Turn it off if a game complains."));
            changed |= self.switch_card(ui, Feature::Select, "\"Show in folder\" selects the file", "When an app shows a file in its folder through the Windows call for that, the file is selected here too.", Some("Needs \"Open folders in Daleris\". It relies on undocumented Windows behaviour, so an update could stop it working: the folder would still open, with nothing selected. Apps that run explorer.exe /select still open File Explorer."));
            changed |= self.switch_card(ui, Feature::SingleWindow, "Open new folders as tabs in the running window", "With this off, each folder opened from outside gets its own window.", None);
            changed |= self.switch_card(ui, Feature::Dialogs, "Open and Save dialogs use Daleris", "When an app asks Windows for a modern file dialog, Daleris shows it instead, with any controls the app added, and goes back to the Windows dialog if anything goes wrong. Properties an app collects in the dialog, such as Office's Authors and Tags, can't be edited in Daleris.", Some("This loads a Daleris DLL into the app that opens the dialog, and only then. Games, game launchers, anti-cheat and security software get the Windows dialog straight away, but the DLL is loaded for a moment to find that out, and it isn't signed, so antivirus may question it. Don't turn this on while you play games with strict anti-cheat if you'd rather not take any chance."));
            changed |= self.exclusions_card(ui);
            changed |= self.switch_card(ui, Feature::DialogJump, "Ctrl + G sends a Windows file dialog to this folder", "While a Windows Open, Save or folder dialog is in front, Ctrl + G takes it to the folder Daleris is showing, and keeps the name you typed. Works in every app, and loads nothing into it.", Some("A small background program from sign-in watches which window is in front, and has Ctrl + G only while a file dialog is. It tells you if Daleris isn't open or the dialog can't be reached, as with apps running as administrator. Its log is dialog-jump.log, next to the file below."));
            changed |= self.switch_card(ui, Feature::DialogLog, "Log file dialog decisions", "Writes down every app that asks for a file dialog, whether Daleris showed it, and why not if it didn't: the controls the app added, and any interface it asked for that Daleris doesn't know.", Some("Use this to find out why an app, such as Office, still shows the Windows dialog. If an app doesn't appear in the log at all, it never asked Windows for this kind of dialog. Microsoft Store apps, including Store copies of Office, are packaged and always keep the Windows dialog; Windows may also keep what they write in their own copy of this folder, under %LOCALAPPDATA%\\Packages."));
            self.dialog_log_buttons(ui);
        });
        ui.add_space(14.0);
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new("Put File Explorer back").strong());
            ui.label(egui::RichText::new("Turns all of the above off and removes everything Daleris wrote, and nothing else. It's what uninstalling does, and it's the same as running daleris --restore-explorer.").color(DIM()));
            ui.add_space(6.0);
            if ui.button("Restore File Explorer").clicked() {
                self.restore_explorer();
                changed = true;
            }
        });
        if changed {
            self.save_settings();
        }
    }

    fn dialog_log_buttons(&mut self, ui: &mut Ui) {
        let Some(file) = integration::dialog_log_file() else { return };
        ui.horizontal(|ui| {
            if ui.add_enabled(file.is_file(), egui::Button::new("Open the log")).clicked() {
                shell::open(&file);
            }
            if ui.add_enabled(file.is_file(), egui::Button::new("Clear it")).clicked() {
                let _ = std::fs::remove_file(&file);
            }
            ui.label(egui::RichText::new(if file.is_file() { file.display().to_string() } else { "Nothing logged yet".to_string() }).small().color(DIM()));
        });
        ui.add_space(10.0);
    }

    fn restore_explorer(&mut self) {

        for f in Feature::ALL {
            if !policy::is_fixed(switch_key(f)) {
                *self.settings.switch_mut(f) = false;
            }
        }
        integration::restore();
        self.explorer_applied = None;
        self.toast("File Explorer is back", false);
    }

    fn switch_card(&mut self, ui: &mut Ui, f: Feature, title: &str, plain: &str, warning: Option<&str>) -> bool {
        let key = switch_key(f);
        let fixed = policy::is_fixed(key);
        let live = self.explorer_status().iter().find(|(g, _)| *g == f).map(|(_, l)| l.clone()).unwrap_or(Live::Off);
        let mut changed = false;
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let mut on = *self.settings.switch_mut(f);
                ui.add_enabled_ui(!fixed, |ui| {
                    if toggle(ui, &mut on, title) {
                        *self.settings.switch_mut(f) = on;
                        changed = true;
                    }
                });
                if fixed {
                    managed_note(ui);
                }
            });
            ui.label(egui::RichText::new(plain).color(DIM()));
            match &live {
                Live::Off => {}
                Live::Active(text) => {
                    ui.label(egui::RichText::new(text).color(ACTIVE_COLOR));
                }
                Live::Problem(text) => {
                    ui.label(egui::RichText::new(text).color(WARN_COLOR));
                }
            }
            if let Some(w) = warning {
                ui.label(egui::RichText::new(w).small().color(DIM()));
            }
        });
        ui.add_space(10.0);
        changed
    }

    fn exclusions_card(&mut self, ui: &mut Ui) -> bool {
        let fixed = policy::is_fixed("exp_exclude");
        let mut changed = false;
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new("Apps that keep the Windows dialog").strong());
            ui.label(egui::RichText::new("Program names or folders, separated by | such as notepad++.exe | C:\\Tools\\. To let Daleris show a browser's dialogs, add !chrome.exe.").color(DIM()));
            ui.add_enabled_ui(!fixed, |ui| {
                changed = ui.add(egui::TextEdit::singleline(&mut self.settings.exp_exclude).desired_width(f32::INFINITY)).changed();
            });
            if fixed {
                managed_note(ui);
            }
            egui::CollapsingHeader::new("Always left alone").id_salt("explorer_denied").show(ui, |ui| {
                ui.label(egui::RichText::new(format!("Programs: {}", deny::ALWAYS_LEFT_ALONE.join(", "))).small().color(DIM()));
                ui.add_space(4.0);
                ui.label(egui::RichText::new(format!("Anything in these folders: {}", deny::GAME_FOLDERS.join(" "))).small().color(DIM()));
                ui.add_space(4.0);
                ui.label(egui::RichText::new(format!("Anything with these loaded: {}", deny::WATCHFUL_MODULES.join(", "))).small().color(DIM()));
                ui.add_space(4.0);
                ui.label(egui::RichText::new(format!("Left alone unless you say otherwise: {}", deny::LEFT_ALONE_BY_DEFAULT.join(", "))).small().color(DIM()));
                ui.add_space(4.0);
                ui.label(egui::RichText::new("So is anything running as administrator, sandboxed, or installed from the Microsoft Store.").small().color(DIM()));
            });
        });
        ui.add_space(10.0);
        changed
    }
}

const ACTIVE_COLOR: Color32 = Color32::from_rgb(92, 184, 122);
const WARN_COLOR: Color32 = Color32::from_rgb(224, 164, 72);

/// Windows-side command-line runs, which do their job without a window. The exit code if this was one.
pub fn handle_integration_command() -> Option<i32> {
    let args: Vec<String> = std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    match args.first().map(String::as_str) {
        // Used by the installers, and for anyone whose folders got stuck. Removes what's ours, and only that.
        Some("--restore-explorer") => {
            let mut s = Settings::load();
            let any_on = s.explorer_switches().iter().any(|(_, on)| *on);
            integration::restore();
            // Otherwise the next start would put it all back. No settings file needs making for this.
            if any_on {
                for f in Feature::ALL {
                    *s.switch_mut(f) = false;
                }
                let tabs = s.tabs.clone();
                s.save(&tabs);
            }
            println!("restored");
            Some(0)
        }
        // For tests and scripts: the same as the switches in Settings.
        Some("--integration") => {
            let mut s = Settings::load();
            for pair in args[1..].chunks(2) {
                let (Some(f), Some(on)) = (Feature::from_id(&pair[0]), pair.get(1)) else {
                    eprintln!("usage: --integration <switch> on|off ...");
                    return Some(2);
                };
                *s.switch_mut(f) = on == "on";
            }
            s.enforce(&policy::current());
            let result = integration::sync(&s.explorer_switches(), &s.exp_exclude);
            let tabs = s.tabs.clone();
            s.save(&tabs);
            match result {
                Ok(()) => {
                    print_status();
                    Some(0)
                }
                Err(e) => {
                    println!("error: {e}");
                    Some(1)
                }
            }
        }
        Some("--integration-status") => {
            print_status();
            Some(0)
        }
        _ => None,
    }
}

fn print_status() {
    for f in Feature::ALL {
        println!("{}: {:?}", f.id(), integration::status(f));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_switch_is_off_until_someone_turns_it_on() {
        let s = Settings::defaults();
        assert!(s.explorer_switches().iter().all(|(_, on)| !on));
        assert!(s.exp_exclude.is_empty());
    }

    #[test]
    fn every_switch_has_a_settings_value_a_policy_can_fix() {
        let mut s = Settings::defaults();
        for f in Feature::ALL {
            let key = switch_key(f);
            assert!(policy::covers(key), "no policy for {key}");
            assert!(s.set_value(key, "1"), "{key} isn't a settings value");
            assert!(*s.switch_mut(f), "{key} doesn't set its switch");
            assert!(s.value_lines().iter().any(|(k, v)| *k == key && v == "1"), "{key} isn't written");
        }
        assert!(policy::covers("exp_exclude"));
        assert!(s.set_value("exp_exclude", "a.exe"));
        assert_eq!(s.exp_exclude, "a.exe");
    }

    #[test]
    fn the_policy_names_match_on_both_sides() {
        for f in Feature::ALL {
            assert_eq!(policy::name_of(switch_key(f)), Some(f.policy_name()));
        }
    }
}
