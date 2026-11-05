//! Windows Explorer integration, as the program sees it: what the switches in Settings do to the system, how to tell if
//! each one is really in effect, and the parts that live in this process (the channel for tabs in the running window,
//! the hotkey for ordinary file dialogs). The shared details are in `crates/integration`.
//!
//! Each switch is off until someone turns it on. On other systems there's nothing to switch, and it all does nothing.

pub use daleris_integration::Feature;

/// What Settings shows under a switch.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum Live {
    /// off, and nothing of it is in place
    Off,
    /// in effect, with what that means right now
    Active(String),
    /// on but not doing what it says, with why
    Problem(String),
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn sync(_: &[(Feature, bool)], _: &str) -> Result<(), String> {
        Ok(())
    }
    pub fn status(_: Feature) -> Live {
        Live::Off
    }
    pub fn restore() {}
    pub fn set_jump_folder(_: Option<&std::path::Path>) {}
    pub fn clear_jump_folder() {}
    pub fn dialog_log_file() -> Option<std::path::PathBuf> {
        None
    }
    pub fn is_elevated() -> bool {
        false
    }
}

#[cfg(windows)]
#[path = "integration_windows.rs"]
mod imp;

pub use imp::*;
