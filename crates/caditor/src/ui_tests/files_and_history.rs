use std::{fs, path::Path};

use caditor_document::Transaction;
use egui::{Event, Id, Key, Modifiers, PanelState, PointerButton, Pos2};
use tempfile::TempDir;

use super::{Harness, canonical, extruded_plate, run_from_palette, sample_document};
use crate::{
    app::Workspace,
    commands::{Command, RecentSlot},
    export::ExportCommand,
    files::{FileCommand, GuardChoice},
    layout::PanelLayout,
    model::{Action, NEVER_SAVED, NO_CHANGES_SINCE_SAVED},
    preferences::Preferences,
    undo_history,
};

fn notice_starts(harness: &Harness, text: &str) -> bool {
    harness
        .model
        .notice()
        .is_some_and(|notice| notice.text.starts_with(text))
}

fn offered(harness: &Harness, command: Command) -> Result<(), String> {
    harness
        .workspace
        .last_offers
        .iter()
        .find(|offer| offer.command == command)
        .map_or_else(
            || Err("not offered".to_owned()),
            |offer| offer.availability.clone(),
        )
}

fn offer_title(harness: &Harness, command: Command) -> Option<String> {
    harness
        .workspace
        .last_offers
        .iter()
        .find(|offer| offer.command == command)
        .map(crate::commands::Offer::title)
}

fn add_parameters(harness: &mut Harness, names: &[&str]) {
    for name in names {
        let length = harness.model.length_unit().default_length(10.0);
        let mut transaction = harness.document().transaction(format!("Add {name}"));
        transaction.add_parameter((*name).to_owned(), length);
        harness.perform(Action::Apply(transaction.finish()));
        harness.settle();
    }
}

fn exported(harness: &mut Harness, file: &str) {
    let text = format!("Exported 1 body to “{file}”");
    harness.wait_until("the export is written", |harness| {
        notice_starts(harness, &text)
    });
}

fn saved_model(dir: &Path, name: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    caditor_file::save(&sample_document().unwrap(), &path, false).unwrap();
    path
}

#[test]
fn the_undo_history_goes_back_to_before_every_change_and_marks_the_current_one() {
    let mut harness = Harness::new();
    let before = harness.document().parameters().len();
    add_parameters(&mut harness, &["first", "second"]);

    run_from_palette(&mut harness, "undo history");

    assert!(harness.workspace.undo_history_open);
    assert!(harness.shows(undo_history::NOW));
    assert!(harness.shows(undo_history::BEFORE));

    harness.click("Add second");
    harness.settle();

    assert_eq!(harness.document().parameters().len(), before + 2);
    assert_eq!(harness.model.redo_steps().count(), 0);

    harness.click(undo_history::BEFORE);
    harness.settle();

    assert_eq!(harness.document().parameters().len(), before);
    assert_eq!(harness.model.undo_steps().count(), 0);
    assert_eq!(
        harness
            .model
            .redo_steps()
            .map(Transaction::label)
            .collect::<Vec<_>>(),
        ["Add first", "Add second"]
    );
    assert!(!harness.shows(undo_history::BEFORE));
}

#[test]
fn the_edit_menu_names_the_step_undo_and_redo_would_take() {
    let mut harness = Harness::new();
    add_parameters(&mut harness, &["first"]);

    harness.click("Edit");

    assert!(harness.shows("Undo Add first"));
    assert!(harness.shows("Redo"));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();
    harness.perform(Action::Undo);
    harness.settle();
    harness.click("Edit");

    assert!(harness.shows("Undo"));
    assert!(harness.shows("Redo Add first"));
}

#[test]
fn revert_to_saved_goes_back_to_the_file_in_one_step_that_undo_takes_back() {
    let dir = TempDir::new().unwrap();
    let root = canonical(&dir);
    let path = saved_model(&root, "kept.caditor");
    let mut harness = Harness::with_directories(Some(root.as_path()));
    harness.frame();

    assert_eq!(
        offered(&harness, Command::RevertToSaved),
        Err(NEVER_SAVED.to_owned())
    );

    harness.command(FileCommand::OpenPath(path.clone()));
    harness.wait_until("the file is open", |harness| {
        harness.model.path() == Some(path.as_path())
    });
    harness.frame();

    assert_eq!(
        offered(&harness, Command::RevertToSaved),
        Err(NO_CHANGES_SINCE_SAVED.to_owned())
    );

    let saved = harness.document().clone();
    add_parameters(&mut harness, &["first", "second"]);
    assert!(harness.model.is_dirty());

    run_from_palette(&mut harness, "revert to the saved");
    harness.settle();

    assert!(!harness.model.is_dirty());
    assert!(harness.document().same_content(&saved));
    assert_eq!(
        harness.model.undo_label(),
        Some("Revert to the saved version")
    );
    assert!(notice_starts(
        &harness,
        "Went back to “kept.caditor” as it was last saved. Undo brings your changes back."
    ));

    harness.perform(Action::Undo);
    harness.settle();

    assert!(harness.model.is_dirty());
    assert!(harness.document().parameter_named("second").is_some());
}

#[test]
fn file_dialogs_start_in_the_folder_last_chosen_for_the_same_purpose() {
    let dir = TempDir::new().unwrap();
    let exports = dir.path().join("exports");
    fs::create_dir(&exports).unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    extruded_plate(&mut harness);

    harness.command(FileCommand::Export(ExportCommand::Show));
    harness.click("STL");
    harness.answer_dialog(Some(exports.join("plate")));
    harness.click("Export…");
    exported(&mut harness, "plate.stl");

    harness.answer_dialog(None);
    harness.command(FileCommand::Export(ExportCommand::Show));
    harness.click("Export…");

    assert_eq!(
        harness.dialogs.asked_in.lock().last(),
        Some(&Some(exports.clone()))
    );

    harness.command(FileCommand::Export(ExportCommand::Hide));
    harness.command(FileCommand::Import { into: None });

    assert_eq!(harness.dialogs.asked_in.lock().last(), Some(&None));
}

#[test]
fn export_again_repeats_the_last_export_and_asks_only_once_the_file_changed_elsewhere() {
    let dir = TempDir::new().unwrap();
    let plate = dir.path().join("plate.stl");
    let mut harness = Harness::with_directories(Some(dir.path()));
    extruded_plate(&mut harness);
    harness.frame();

    assert!(offered(&harness, Command::ExportAgain).is_err());

    harness.command(FileCommand::Export(ExportCommand::Show));
    harness.click("STL");
    harness.answer_dialog(Some(dir.path().join("plate")));
    harness.click("Export…");
    exported(&mut harness, "plate.stl");
    harness.frame();

    assert_eq!(
        offer_title(&harness, Command::ExportAgain).as_deref(),
        Some("Export again to plate.stl")
    );

    harness.command(FileCommand::Export(ExportCommand::Show));
    harness.click("3MF");
    harness.click("Cancel");
    harness.perform(Action::DismissNotice);
    harness.answer_dialog(None);
    run_from_palette(&mut harness, "export again");
    exported(&mut harness, "plate.stl");

    assert!(!harness.shows("Replace “plate.stl”?"));
    assert!(!dir.path().join("plate.3mf").exists());

    fs::write(&plate, b"changed elsewhere").unwrap();
    harness.perform(Action::DismissNotice);
    run_from_palette(&mut harness, "export again");
    harness.wait_until("the replacement is asked about", |harness| {
        harness.shows("Replace “plate.stl”?")
    });
    harness.click("Cancel");

    assert_eq!(fs::read(&plate).unwrap(), b"changed elsewhere");

    run_from_palette(&mut harness, "export again");
    harness.wait_until("the replacement is asked about", |harness| {
        harness.shows("Replace “plate.stl”?")
    });
    harness.click("Replace");
    exported(&mut harness, "plate.stl");

    assert_ne!(fs::read(&plate).unwrap(), b"changed elsewhere");

    harness.command(FileCommand::New);
    harness.command(FileCommand::Guard(GuardChoice::DontSave));
    harness.settle();

    assert!(offered(&harness, Command::ExportAgain).is_err());
}

#[test]
fn a_recent_model_is_removed_on_its_own_and_a_missing_one_is_marked_and_said_to_be_removed() {
    let dir = TempDir::new().unwrap();
    let root = canonical(&dir);
    let first = saved_model(&root, "first.caditor");
    let second = saved_model(&root, "second.caditor");
    let third = saved_model(&root, "third.caditor");
    let mut harness = Harness::with_directories(Some(root.as_path()));
    for path in [&first, &second, &third] {
        harness.command(FileCommand::OpenPath(path.clone()));
        harness.wait_until("the file is open", |harness| {
            harness.model.path() == Some(path.as_path())
        });
    }
    harness.command(FileCommand::New);
    harness.settle();
    fs::remove_file(&first).unwrap();

    harness.click("File");
    harness.wait_until("the recent models are checked", |harness| {
        harness.files.is_missing(&first)
    });
    harness.click("Open recent");

    assert!(harness.shows("(not found)"));

    harness.click_button("Remove second.caditor from the recent models");

    assert_eq!(harness.files.recent(), [third.clone(), first.clone()]);
    assert!(notice_starts(
        &harness,
        "Removed “second.caditor” from the recent models."
    ));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();

    assert_eq!(
        offer_title(&harness, Command::ForgetRecent(RecentSlot::ALL[0])).as_deref(),
        Some("Remove the most recent model from the list: third.caditor")
    );

    run_from_palette(&mut harness, "remove the most recent model");

    assert_eq!(harness.files.recent(), std::slice::from_ref(&first));

    harness.command(FileCommand::OpenPath(first.clone()));
    harness.wait_until("the failure is reported", |harness| {
        notice_starts(harness, "Could not open “first.caditor”")
    });

    assert!(notice_starts(
        &harness,
        "Could not open “first.caditor”: it no longer exists. It was removed from the recent \
         models."
    ));
    assert!(harness.files.recent().is_empty());
}

fn measure_width(harness: &Harness) -> Option<f32> {
    PanelState::load(&harness.context, Id::new("measure")).map(|state| state.size().x)
}

fn drag_screen(harness: &mut Harness, from: Pos2, to: Pos2) {
    harness.events.push(Event::PointerMoved(from));
    harness.frame();
    harness.events.push(Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    for step in 1..=4 {
        let point = from + (to - from) * (step as f32 / 4.0);
        harness.events.push(Event::PointerMoved(point));
        harness.frame();
    }
    harness.events.push(Event::PointerButton {
        pos: to,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    harness.frame();
}

#[test]
fn right_hand_panels_open_as_wide_as_one_was_last_made() {
    let mut harness = Harness::new();
    harness.workspace.measure.toggle();
    harness.frame();
    harness.frame();
    let width = measure_width(&harness).unwrap();
    let edge = PanelState::load(&harness.context, Id::new("measure"))
        .unwrap()
        .outer_rect
        .left_center()
        + egui::vec2(2.0, 0.0);

    assert_eq!(harness.workspace.preferences.panels.right_width, None);

    drag_screen(&mut harness, edge, edge - egui::vec2(80.0, 0.0));
    let widened = measure_width(&harness).unwrap();

    assert!(widened > width + 40.0, "{width} → {widened}");
    assert_eq!(
        harness.workspace.preferences.panels.right_width,
        Some(widened.round())
    );

    let mut preferences = Preferences::from_settings(caditor_file::Settings::default());
    preferences.onboarding = crate::onboarding::Onboarding::finished();
    preferences.panels = PanelLayout {
        right_width: Some(widened.round()),
        ..PanelLayout::default()
    };
    let mut later = Harness::starting(
        None,
        sample_document().unwrap(),
        Workspace::with_preferences(preferences),
    );
    later.workspace.guide.open = true;
    later.frame();
    later.frame();

    let guide = PanelState::load(&later.context, Id::new("guide"))
        .unwrap()
        .size()
        .x;
    assert!((guide - widened.round()).abs() <= 1.0, "{guide}");
}
