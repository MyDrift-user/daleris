//! Which apps the file dialog DLL steps aside for. Windows' COM decides when the DLL loads and it can't choose by app,
//! so these checks run as the first thing in the DLL, before any object of ours exists: the host gets Windows' own
//! dialog straight from `comdlg32` and nothing of ours is left to run.

/// Never touched, whatever the person's list says: games, launchers and anti-cheat (it treats a loaded DLL as cheating,
/// which can cost an account), security software (which can treat it as an attack), and Windows' own shell.
pub const ALWAYS_LEFT_ALONE: &[&str] = &[
    // Roblox
    "robloxplayerbeta.exe", "robloxplayerlauncher.exe", "robloxstudiobeta.exe", "robloxstudiolauncherbeta.exe", "robloxcrashhandler.exe", "bloxstrap.exe", "fishstrap.exe",
    // launchers
    "steam.exe", "steamwebhelper.exe", "steamservice.exe", "epicgameslauncher.exe", "epicwebhelper.exe", "epiconlineservicesuserhelper.exe", "battle.net.exe", "agent.exe",
    "riotclientservices.exe", "riotclientux.exe", "riotclientuxrender.exe", "leagueclient.exe", "leagueclientux.exe", "origin.exe", "eadesktop.exe", "eabackgroundservice.exe",
    "upc.exe", "uplay.exe", "ubisoftconnect.exe", "ubisoftgamelauncher.exe", "galaxyclient.exe", "goggalaxy.exe", "minecraftlauncher.exe", "playnite.desktopapp.exe",
    "xboxpcapp.exe", "gamingservices.exe", "rockstarlauncher.exe", "launcher.exe", "launcherpatcher.exe",
    // games known to run anti-cheat
    "valorant.exe", "valorant-win64-shipping.exe", "league of legends.exe", "fortniteclient-win64-shipping.exe", "fortnitelauncher.exe", "cs2.exe", "csgo.exe", "dota2.exe", "r5apex.exe",
    "r5apex_dx12.exe", "overwatch.exe", "wow.exe", "wowclassic.exe", "gta5.exe", "gta5_enhanced.exe", "fivem.exe", "rainbowsix.exe", "rainbowsix_be.exe", "destiny2.exe", "pubg.exe", "tslgame.exe",
    "rustclient.exe", "eldenring.exe", "start_protected_game.exe", "cod.exe", "modernwarfare.exe", "blackops6.exe", "escapefromtarkov.exe", "bf2042.exe", "thefinals.exe", "marvel-win64-shipping.exe",
    "minecraft.windows.exe", "javaw.exe",
    // anti-cheat itself
    "easyanticheat.exe", "easyanticheat_eos.exe", "easyanticheat_setup.exe", "beservice.exe", "beservice_x64.exe", "vgc.exe", "vgtray.exe", "faceit.exe", "faceitservice.exe",
    "faceitclient.exe", "esea client.exe", "gameguard.exe", "gamemon.exe", "gamemon64.exe", "xigncode.exe", "nprotect.exe", "ricochet.exe", "atvi-randgrid.exe",
    // security software
    "msmpeng.exe", "mpcmdrun.exe", "nissrv.exe", "securityhealthservice.exe", "securityhealthsystray.exe", "smartscreen.exe", "avp.exe", "avpui.exe", "avgui.exe", "avastui.exe", "avastsvc.exe",
    "bdagent.exe", "bdservicehost.exe", "egui.exe", "ekrn.exe", "mbam.exe", "mbamservice.exe", "mcshield.exe", "mfemms.exe", "ntrtscan.exe", "sophosui.exe", "csfalconservice.exe", "sentinelagent.exe",
    "norton.exe", "nortonsecurity.exe", "ccsvchst.exe", "wrsa.exe",
    // Windows' shell and system
    "explorer.exe", "dllhost.exe", "rundll32.exe", "regsvr32.exe", "svchost.exe", "lsass.exe", "csrss.exe", "wininit.exe", "winlogon.exe", "services.exe", "smss.exe", "taskhostw.exe", "sihost.exe",
    "ctfmon.exe", "shellexperiencehost.exe", "startmenuexperiencehost.exe", "searchhost.exe", "searchapp.exe", "runtimebroker.exe", "applicationframehost.exe", "systemsettings.exe", "msiexec.exe",
    "consent.exe", "logonui.exe", "dwm.exe", "fontdrvhost.exe", "spoolsv.exe", "wuauclt.exe", "pickerhost.exe", "filepicker.exe", "mmc.exe", "taskmgr.exe", "regedit.exe",
    // ourselves
    "daleris.exe", "daleris-open.exe",
];

/// Left alone until the person takes one off with `!name.exe` in their list. Browsers run their dialogs in sandboxed or
/// tightly watched processes and block outside code, so a wrong guess there is more likely to cost a working dialog.
pub const LEFT_ALONE_BY_DEFAULT: &[&str] = &[
    "chrome.exe", "msedge.exe", "msedgewebview2.exe", "firefox.exe", "brave.exe", "opera.exe", "opera_gx.exe", "vivaldi.exe", "arc.exe", "iexplore.exe", "waterfox.exe", "librewolf.exe", "zen.exe", "thorium.exe",
];

/// Parts of a program's folder that mean it's a game or belongs to a game store, however it's called.
/// Lower case, with the backslashes on both sides.
pub const GAME_FOLDERS: &[&str] = &[
    r"\steamapps\", r"\steam\", r"\steamlibrary\", r"\epic games\", r"\epicgames\", r"\riot games\", r"\riot client\", r"\riot vanguard\", r"\battle.net\", r"\blizzard\", r"\origin games\", r"\ea games\", r"\ea app\",
    r"\electronic arts\", r"\ubisoft\", r"\ubisoft game launcher\", r"\gog galaxy\", r"\gog games\", r"\rockstar games\", r"\roblox\", r"\bloxstrap\", r"\fishstrap\", r"\xboxgames\", r"\modifiablewindowsapps\",
    r"\easyanticheat\", r"\battleye\", r"\faceit\", r"\gameguard\",
];

/// Modules that give a game or anti-cheat away inside a process, in lower case.
pub const WATCHFUL_MODULES: &[&str] = &[
    "easyanticheat.dll", "easyanticheat_x64.dll", "easyanticheat_x86.dll", "easyanticheat_eos.dll", "easyanticheat_eos_x64.dll", "beclient.dll", "beclient_x64.dll", "beclient2.dll", "battleye.dll",
    "vgk.dll", "vgc.dll", "xigncode.dll", "x3.xem", "npgame.dll", "npggnt.dll", "npggnt64.dll", "ricochet.dll", "hyperion.dll", "robloxplayerbeta.dll", "gameoverlayrenderer.dll", "gameoverlayrenderer64.dll",
    "eosovh-win64-shipping.dll", "eossdk-win64-shipping.dll", "eossdk-win32-shipping.dll", "steam_api.dll", "steam_api64.dll", "unityplayer.dll",
];

/// Why the DLL steps aside, for the log, or None if Daleris may show the dialog here.
pub fn reason_to_leave_alone(exe_path: &str, person_list: &str) -> Option<&'static str> {
    let lower = exe_path.to_lowercase().replace('/', "\\");
    let name = lower.rsplit('\\').next().unwrap_or(&lower);
    if ALWAYS_LEFT_ALONE.contains(&name) {
        return Some("built-in list");
    }
    let folder = &lower[..lower.len() - name.len()];
    if GAME_FOLDERS.iter().any(|g| folder.contains(g)) {
        return Some("a game's or store's folder");
    }
    let mut allowed = false;
    for entry in person_list.split(['|', '\n', ',', ';']).map(|e| e.trim().to_lowercase()).filter(|e| !e.is_empty()) {
        if let Some(allow) = entry.strip_prefix('!') {
            allowed |= allow == name;
        } else if entry == name || (entry.contains('\\') && lower.contains(&entry)) {
            return Some("your list");
        }
    }
    if !allowed && LEFT_ALONE_BY_DEFAULT.contains(&name) {
        return Some("browser");
    }
    None
}

/// Does a module loaded in the host give away a game or anti-cheat?
pub fn is_watchful_module(file_name: &str) -> bool {
    let lower = file_name.to_lowercase();
    WATCHFUL_MODULES.contains(&lower.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn games_and_anticheat_are_left_alone_whatever_the_person_says() {
        for exe in [r"C:\x\RobloxPlayerBeta.exe", r"C:\x\valorant.exe", r"C:\x\EasyAntiCheat.exe", r"C:\x\steam.exe", r"C:\x\MsMpEng.exe", r"C:\Windows\explorer.exe"] {
            assert!(reason_to_leave_alone(exe, "").is_some(), "{exe}");
            let name = exe.rsplit('\\').next().unwrap();
            assert!(reason_to_leave_alone(exe, &format!("!{name}")).is_some(), "{exe} with an exception");
        }
    }

    #[test]
    fn anything_in_a_games_folder_is_left_alone_however_its_called() {
        assert!(reason_to_leave_alone(r"D:\SteamLibrary\steamapps\common\Foo\foo.exe", "").is_some());
        assert!(reason_to_leave_alone(r"C:\Users\me\AppData\Local\Roblox\Versions\v1\thing.exe", "").is_some());
        assert!(reason_to_leave_alone(r"C:\Program Files\Epic Games\Foo\foo.exe", "!foo.exe").is_some());
    }

    #[test]
    fn browsers_are_left_alone_until_taken_off_the_list() {
        let chrome = r"C:\Program Files\Google\Chrome\Application\chrome.exe";
        assert_eq!(reason_to_leave_alone(chrome, ""), Some("browser"));
        assert_eq!(reason_to_leave_alone(chrome, "!chrome.exe"), None);
    }

    #[test]
    fn the_persons_own_list_counts_by_name_or_folder() {
        let app = r"C:\Tools\Foo\foo.exe";
        assert_eq!(reason_to_leave_alone(app, ""), None);
        assert_eq!(reason_to_leave_alone(app, "bar.exe|FOO.EXE"), Some("your list"));
        assert_eq!(reason_to_leave_alone(app, r"c:\tools\foo\"), Some("your list"));
    }

    #[test]
    fn modules_are_compared_without_regard_to_case() {
        assert!(is_watchful_module("BEClient_x64.dll"));
        assert!(!is_watchful_module("kernel32.dll"));
    }
}
