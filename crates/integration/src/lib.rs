//! What the program, the launcher (`daleris-open`) and the file dialog DLL share about Windows Explorer integration:
//! the registry changes each switch makes and how they're undone, how a command line is read, the channel to a
//! running window, and which apps the DLL leaves alone.
//!
//! Only `target` and `deny` hold anything that isn't Windows' own, so those are the two that also build elsewhere,
//! which lets their tests run on every system.

pub mod deny;
pub mod pick;
pub mod target;

#[cfg(windows)]
pub mod explorer;
#[cfg(windows)]
pub mod hookctl;
#[cfg(windows)]
pub mod pipe;
#[cfg(windows)]
pub mod reg;

/// One switch in Settings. Each is off until the person turns it on, and works on its own.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Feature {
    /// Folders open in Daleris: the Folder class's open and explore commands.
    Folders,
    /// Win + E opens Daleris through the registry.
    WinE,
    /// Win + E opens Daleris through a low-level keyboard hook in the launcher, for builds that ignore the registry.
    WinEHook,
    /// `SHOpenFolderAndSelectItems` selects the file: a shell window the launcher registers. Needs `Folders`.
    Select,
    /// New folders open as tabs in the running window.
    SingleWindow,
    /// Apps' Open and Save dialogs show Daleris: the per-user COM override.
    Dialogs,
    /// A hotkey sends an ordinary Windows file dialog to the folder Daleris shows. Nothing loads into the other app.
    DialogJump,
    /// The file dialog DLL writes down what it decides and why, for finding out why an app kept the Windows dialog.
    DialogLog,
}

impl Feature {
    pub const ALL: [Feature; 8] = [Feature::Folders, Feature::WinE, Feature::WinEHook, Feature::Select, Feature::SingleWindow, Feature::Dialogs, Feature::DialogJump, Feature::DialogLog];

    /// Name on the command line and in the registry.
    pub fn id(self) -> &'static str {
        match self {
            Feature::Folders => "folders",
            Feature::WinE => "win-e",
            Feature::WinEHook => "win-e-hook",
            Feature::Select => "select",
            Feature::SingleWindow => "single-window",
            Feature::Dialogs => "dialogs",
            Feature::DialogJump => "dialog-jump",
            Feature::DialogLog => "dialog-log",
        }
    }

    pub fn from_id(id: &str) -> Option<Feature> {
        Feature::ALL.into_iter().find(|f| f.id() == id)
    }

    /// The value under `Software\Policies\Daleris` that fixes this switch. See `src/policy.rs`.
    pub fn policy_name(self) -> &'static str {
        match self {
            Feature::Folders => "ExplorerOpenFolders",
            Feature::WinE => "ExplorerWinE",
            Feature::WinEHook => "ExplorerWinEHook",
            Feature::Select => "ExplorerSelectFile",
            Feature::SingleWindow => "ExplorerSingleWindow",
            Feature::Dialogs => "ExplorerFileDialogs",
            Feature::DialogJump => "ExplorerDialogJump",
            Feature::DialogLog => "ExplorerDialogLog",
        }
    }
}
