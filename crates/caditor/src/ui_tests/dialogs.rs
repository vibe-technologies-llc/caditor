use std::path::PathBuf;

use egui::{Key, Modifiers};

use super::{Harness, extruded_plate, feature_names};
use crate::{
    export::ExportCommand, files::FileCommand, guide::Page, model::Action,
    preferences::PreferencesCommand, widgets,
};

#[test]
fn a_file_dialog_left_open_says_so_and_can_be_stopped_from_the_window() {
    let mut harness = Harness::new();
    *harness.dialogs.held.lock() = Some(Vec::new());

    harness.perform(Action::File(FileCommand::Import { into: None }));
    harness.frame();
    let waiting = harness.shows("Waiting for the file dialog…");
    let blocked = harness.files.is_blocking();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let freed = !harness.files.is_blocking() && !harness.files.is_importing();
    let late = harness.dialogs.held.lock().take().unwrap_or_default();
    for respond in late {
        respond(Ok(Some(PathBuf::from("/tmp/late.step"))));
    }
    harness.frame();
    harness.frame();

    assert!(waiting);
    assert!(blocked);
    assert!(freed);
    assert!(!harness.files.is_picking());
    assert!(!harness.files.is_importing());
    assert!(!harness.shows("Waiting for the file dialog…"));
}

#[test]
fn a_dialog_s_help_button_closes_it_and_opens_its_page_of_the_guide() {
    let mut harness = Harness::new();
    harness.perform(Action::Preferences(PreferencesCommand::ShowMessages));
    harness.show_new_windows();
    let shown = harness.workspace.messages_open;

    harness.click_button(&widgets::dialog_help_hover(Page::Window));
    harness.frame();
    harness.frame();

    assert!(shown);
    assert!(!harness.workspace.messages_open);
    assert!(harness.workspace.guide.open);
    assert_eq!(harness.workspace.guide.page(), Some(Page::Window));
}

#[test]
fn the_guide_key_in_a_dialog_opens_its_page_and_does_nothing_in_one_without_a_page() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    extruded_plate(&mut harness);

    harness.perform(Action::Preferences(PreferencesCommand::ShowAbout));
    harness.show_new_windows();
    harness.key(Key::F1, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let about_kept = harness.shows("About caditor") && !harness.workspace.guide.open;
    harness.perform(Action::Preferences(PreferencesCommand::CloseAbout));
    harness.frame();

    harness.command(FileCommand::Export(ExportCommand::Show));
    harness.show_new_windows();
    let exporting = harness.files.is_blocking();
    harness.key(Key::F1, Modifiers::NONE);
    harness.frame();
    harness.frame();
    harness.frame();

    assert!(about_kept);
    assert!(exporting);
    assert!(!harness.files.is_blocking());
    assert_eq!(harness.workspace.guide.page(), Some(Page::Exporting));
}

#[test]
fn help_in_keyboard_shortcuts_closes_preferences_under_it_but_not_while_a_key_is_recorded() {
    let mut harness = Harness::new();
    harness.perform(Action::Preferences(PreferencesCommand::Show));
    harness.show_new_windows();
    harness.perform(Action::Preferences(PreferencesCommand::ShowShortcuts));
    harness.show_new_windows();
    let both = harness.workspace.preferences_open && harness.workspace.shortcut_editor.is_some();

    harness.key(Key::F1, Modifiers::NONE);
    harness.frame();
    harness.frame();
    harness.frame();

    assert!(both);
    assert!(harness.workspace.shortcut_editor.is_none());
    assert!(!harness.workspace.preferences_open);
    assert_eq!(harness.workspace.guide.page(), Some(Page::Keyboard));
}

#[test]
fn a_dialog_focuses_its_primary_action_and_enter_runs_it() {
    let mut harness = Harness::new();

    harness.perform(Action::Preferences(PreferencesCommand::ShowAbout));
    harness.show_new_windows();
    assert!(harness.shows("About caditor"));
    assert!(harness.focused().is_some());

    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert!(!harness.shows("About caditor"));
}

#[test]
fn enter_acts_on_the_widget_holding_focus_in_a_dialog_not_on_its_primary_action() {
    let mut harness = Harness::new();
    harness.perform(Action::Preferences(PreferencesCommand::Show));
    harness.show_new_windows();
    assert!(harness.shows("Preferences"));

    harness.key(Key::Tab, Modifiers::NONE);
    harness.frame();
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert!(harness.shows("Preferences"));
}

#[test]
fn the_delete_dialog_starts_on_cancel_so_enter_deletes_nothing() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    let everything = feature_names(&harness);

    harness.click("Plate");
    harness.key(Key::Delete, Modifiers::NONE);
    harness.show_new_windows();
    let asked = harness.shows("Delete “Plate”?");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();

    assert!(asked);
    assert!(!harness.shows("Delete “Plate”?"));
    assert_eq!(feature_names(&harness), everything);
    assert!(harness.workspace.panels.deleting.is_none());
}
