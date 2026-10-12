use std::path::PathBuf;

use egui::{Key, Modifiers};

use super::{Harness, extruded_plate, feature_names};
use crate::{files::FileCommand, model::Action, preferences::PreferencesCommand};

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
