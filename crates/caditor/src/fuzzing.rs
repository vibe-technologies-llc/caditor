use caditor_file::Settings;
use egui::KeyboardShortcut;

use crate::{commands, preferences::Preferences};

pub struct Settled {
    pub written: Settings,
    pub rewritten: Settings,
}

pub fn preferences(settings: Settings) -> Settled {
    let written = Preferences::from_settings(settings).settings();
    let rewritten = Preferences::from_settings(written.clone()).settings();
    Settled { written, rewritten }
}

pub fn parse_stored(text: &str) -> Option<KeyboardShortcut> {
    commands::parse_stored(text)
}

pub fn stored_text(shortcut: &KeyboardShortcut) -> String {
    commands::stored_text(shortcut)
}
