//! German text, keyed by the English in the code. One file per part of the app; see `crate::i18n`.

mod settings;
mod chrome;
mod menu;
mod list;
mod base;
mod system;

pub const ALL: &[&[(&str, &str)]] = &[settings::T, chrome::T, menu::T, list::T, base::T, system::T];
