use std::{borrow::Cow, fs, path::Path, time::Duration};

use caditor_document::{Edit, Transaction};
use egui::{Event, Id, Key, Modifiers, PanelState, PointerButton, Pos2};
use tempfile::TempDir;

use super::{
    FILE_TIMEOUT, Harness, Plane, Point2, Sketch, canonical, click_window_button, extruded_plate,
    rectangle, run_from_palette, sample_document,
};
use crate::{
    app::Workspace,
    commands::{Command, RecentSlot},
    export::ExportCommand,
    files::{FileCommand, GuardChoice},
    history::HistoryCommand,
    icons,
    layout::PanelLayout,
    model::{Action, NEVER_SAVED, NO_CHANGES_SINCE_SAVED},
    preferences::Preferences,
    samples::Sample,
    undo_history, version_preview,
};

fn notice_starts(harness: &Harness, text: &str) -> bool {
    harness
        .model
        .notice()
        .is_some_and(|notice| notice.text.starts_with(text))
}

fn offered(harness: &Harness, command: Command) -> Result<(), Cow<'static, str>> {
    harness
        .workspace
        .last_offers
        .iter()
        .find(|offer| offer.command == command)
        .map_or_else(
            || Err(Cow::Borrowed("not offered")),
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
        Err(NEVER_SAVED.into())
    );

    harness.command(FileCommand::OpenPath(path.clone()));
    harness.wait_until("the file is open", |harness| {
        harness.model.path() == Some(path.as_path())
    });
    harness.frame();

    assert_eq!(
        offered(&harness, Command::RevertToSaved),
        Err(NO_CHANGES_SINCE_SAVED.into())
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

    harness.wait_until("the removal is announced", |harness| {
        notice_starts(harness, "Removed “second.caditor” from the recent models.")
    });

    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();

    assert_eq!(
        offer_title(&harness, Command::ForgetRecent(RecentSlot::ALL[0])).as_deref(),
        Some("Remove the most recent model from the list: third.caditor")
    );

    run_from_palette(&mut harness, "remove the most recent model");

    assert_eq!(harness.files.recent(), std::slice::from_ref(&first));

    harness.wait_until("the second removal is announced", |harness| {
        notice_starts(harness, "Removed “third.caditor” from the recent models.")
    });
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

#[cfg(unix)]
#[test]
fn a_recent_model_that_cannot_be_reached_stays_in_the_recent_models() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    let root = canonical(&dir);
    let share = root.join("share");
    fs::create_dir(&share).unwrap();
    let model = saved_model(&share, "shared.caditor");
    let mut harness = Harness::with_directories(Some(root.as_path()));

    harness.command(FileCommand::OpenPath(model.clone()));
    harness.wait_until("the file is open", |harness| {
        harness.model.path() == Some(model.as_path())
    });
    harness.command(FileCommand::New);
    harness.settle();
    assert!(harness.files.wait_for_jobs(Duration::from_secs(10)));

    fs::set_permissions(&share, fs::Permissions::from_mode(0o000)).unwrap();
    harness.command(FileCommand::OpenPath(model.clone()));
    harness.wait_until("the failure is reported", |harness| {
        notice_starts(harness, "Could not open “shared.caditor”")
    });
    fs::set_permissions(&share, fs::Permissions::from_mode(0o700)).unwrap();

    assert!(!harness.model.notice().unwrap().text.contains("removed"));
    assert_eq!(harness.files.recent(), std::slice::from_ref(&model));
    assert!(!harness.files.is_missing(&model));
}

#[test]
fn a_recent_model_is_not_announced_as_removed_when_the_list_could_not_be_saved() {
    let dir = TempDir::new().unwrap();
    let root = canonical(&dir);
    let first = saved_model(&root, "first.caditor");
    let mut harness = Harness::with_directories(Some(root.as_path()));

    harness.command(FileCommand::OpenPath(first.clone()));
    harness.wait_until("the file is open", |harness| {
        harness.model.path() == Some(first.as_path())
    });
    assert!(harness.files.wait_for_jobs(Duration::from_secs(10)));

    fs::remove_dir_all(root.join("state")).unwrap();
    fs::write(root.join("state"), b"not a folder").unwrap();
    harness.perform(Action::DismissNotice);

    harness.command(FileCommand::ForgetRecent(first.clone()));

    assert!(harness.files.recent().is_empty());
    assert!(!notice_starts(&harness, "Removed"));

    harness.wait_until("the failure is shown", |harness| {
        notice_starts(harness, "Could not save the recent models: ")
    });

    let notice = harness.model.notice().unwrap();
    assert!(notice.outlasts_edits);
    assert!(notice.text.ends_with("next time it starts."));
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

fn set_width(harness: &mut Harness, width: &str) {
    let document = harness.document();
    let id = document.parameter_named("width").unwrap().id();
    let expression = document.parse(width).unwrap();
    harness.perform(Action::Apply(Transaction::single(
        "Width",
        Edit::SetParameterExpression { id, expression },
    )));
    harness.frame();
    harness.frame();
}

#[test]
fn a_kept_version_is_previewed_in_the_background_without_touching_the_model() {
    let dir = TempDir::new().unwrap();
    let plate = Sample::Plate.document().unwrap();
    let mut harness = Harness::starting(Some(dir.path()), plate, Workspace::new());
    let path = dir.path().join("plate.caditor");
    harness.answer_dialog(Some(path));
    harness.command(FileCommand::SaveAs);
    harness.wait_until("the model is saved", |harness| {
        harness.model.path().is_some() && !harness.model.is_saving()
    });
    harness.edit_width("45 mm");
    harness.command(FileCommand::Save);
    harness.wait_until("the change is saved", |harness| !harness.model.is_dirty());
    harness.edit_width("47 mm");
    let revision = harness.model.revision();
    let undo_label = harness.model.undo_label().map(str::to_owned);
    let features = harness.document().features().len();

    harness.command(FileCommand::History(HistoryCommand::Show));
    harness.wait_until("the versions are listed", |harness| {
        harness.shows(version_preview::SHOW)
    });
    harness.click(version_preview::SHOW);
    harness.wait_until("the preview is drawn", |harness| {
        harness
            .files
            .version_preview()
            .is_some_and(version_preview::Preview::is_drawn)
    });

    assert!(harness.shows_containing("Preview of the version saved"));
    assert!(harness.shows(&format!("{features} features · 1 body")));
    assert!(harness.shows("Compared with the model now:"));
    assert!(harness.shows("Parameters changed since: width"));
    assert!(harness.shows(version_preview::HIDE));
    let preview = harness.files.version_preview().unwrap();
    assert_eq!(preview.summary().unwrap().features, features);
    assert_eq!(preview.changes().unwrap().parameters, ["width"]);
    assert_eq!(harness.model.revision(), revision);
    assert_eq!(harness.model.undo_label().map(str::to_owned), undo_label);
    assert_eq!(harness.expression_text("width"), "47 mm");

    set_width(&mut harness, "50 mm");
    assert!(harness.shows(version_preview::SAME_AS_NOW));
    set_width(&mut harness, "47 mm");
    assert!(harness.shows("Parameters changed since: width"));

    harness.click(version_preview::HIDE);
    assert!(harness.files.version_preview().is_none());
    assert!(!harness.shows("Parameters changed since: width"));

    harness.click(version_preview::SHOW);
    harness.wait_until("the preview is ready again", |harness| {
        harness.shows("Restore this version")
    });
    harness.click("Restore this version");
    harness.wait_until("the version is restored", |harness| {
        notice_starts(harness, "Restored the version saved")
    });
    assert_eq!(harness.expression_text("width"), "50 mm");
    assert!(harness.files.version_preview().is_none());
}

fn start_image_export(harness: &mut Harness, target: std::path::PathBuf) {
    harness.key(Key::E, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.frame();
    harness.show_new_windows();
    harness.answer_dialog(Some(target));
    harness.click("Export…");
}

#[test]
fn closing_the_window_while_an_image_export_runs_waits_for_it_to_finish() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.renders_held = true;

    start_image_export(&mut harness, dir.path().join("plate"));
    assert!(harness.files.exports_running());

    click_window_button(&mut harness, icons::CLOSE);
    harness.frame();
    harness.frame();

    assert!(harness.shows("Closing…"));
    assert!(harness.shows("Exporting “plate.png”…"));
    assert!(harness.shows("Cancel export and quit"));
    assert!(!harness.files.should_quit());

    harness.renders_held = false;
    harness.wait_until("caditor quits", |harness| harness.files.should_quit());

    assert!(dir.path().join("plate.png").exists());
}

#[test]
fn quitting_while_an_export_runs_can_be_called_off_or_stop_the_export_and_quit() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    extruded_plate(&mut harness);
    harness.renders_held = true;

    harness.key(Key::E, Modifiers::COMMAND);
    harness.frame();
    harness.show_new_windows();
    harness.click("3MF");
    harness.answer_dialog(Some(dir.path().join("plate")));
    harness.click("Export…");
    assert!(harness.files.exports_running());

    harness.command(FileCommand::Quit);
    harness.click("Close without saving");
    harness.frame();

    assert!(harness.shows("Exporting “plate.3mf”…"));
    assert!(!harness.files.should_quit());

    harness.click("Don't quit");
    harness.frame();

    assert!(!harness.shows("Cancel export and quit"));
    assert!(harness.files.exports_running());
    assert!(!harness.files.should_quit());

    harness.command(FileCommand::Quit);
    harness.click("Close without saving");
    harness.click("Cancel export and quit");
    harness.frame();

    assert!(harness.shows("Stopping the export. caditor closes as soon as it has stopped."));
    assert!(!harness.files.should_quit());

    harness.renders_held = false;
    harness.wait_until("caditor quits", |harness| harness.files.should_quit());

    assert!(!dir.path().join("plate.3mf").exists());
}

fn quit_while_the_replacing_write_is_held(harness: &mut Harness, running: &str) {
    let release = harness.files.hold_jobs();

    harness.click("Replace");
    harness.command(FileCommand::Quit);
    if harness.shows("Close without saving") {
        harness.click("Close without saving");
    }
    harness.frame();
    harness.frame();

    assert!(harness.shows("Closing…"));
    assert!(harness.shows(running));
    assert!(!harness.shows("Cancel export and quit"));
    assert!(!harness.files.should_quit());

    release.send(()).unwrap();
    harness.wait_until("caditor quits", |harness| harness.files.should_quit());
}

#[test]
fn quitting_while_the_parameters_are_being_written_waits_for_the_file() {
    let dir = TempDir::new().unwrap();
    let written = dir.path().join("dimensions.csv");
    fs::write(&written, b"precious").unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));

    harness.answer_dialog(Some(dir.path().join("dimensions")));
    run_from_palette(&mut harness, "Export parameters");
    harness.wait_until("the replacement is confirmed", |harness| {
        harness.shows("Replace “dimensions.csv”?")
    });
    quit_while_the_replacing_write_is_held(
        &mut harness,
        "Writing the parameters “dimensions.csv”…",
    );

    assert!(
        fs::read_to_string(&written)
            .unwrap()
            .starts_with("caditor-parameters,")
    );
}

#[test]
fn quitting_while_a_template_is_being_saved_waits_for_the_file() {
    let dir = TempDir::new().unwrap();
    let folder = dir.path().join("config").join("templates");
    let written = folder.join("Bracket.caditor");
    fs::create_dir_all(&folder).unwrap();
    fs::write(&written, b"precious").unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));

    harness.answer_dialog(Some(folder.join("Bracket")));
    run_from_palette(&mut harness, "Save as template");
    harness.wait_until("the replacement is confirmed", |harness| {
        harness.shows("Replace “Bracket.caditor”?")
    });
    quit_while_the_replacing_write_is_held(&mut harness, "Saving the template “Bracket.caditor”…");

    assert_ne!(fs::read(&written).unwrap(), b"precious");
}

#[test]
fn quitting_while_a_drawing_is_being_written_waits_for_the_file() {
    let dir = TempDir::new().unwrap();
    let written = dir.path().join("outline.dxf");
    fs::write(&written, b"precious").unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::new(0.0, 0.0), Point2::new(40.0, 20.0));
    let id = harness.add_sketch(sketch);
    harness.settle();
    harness.workspace.panels.choose_only(id);
    harness.frame();

    harness.answer_dialog(Some(dir.path().join("outline")));
    run_from_palette(&mut harness, "export sketch");
    harness.click("Export…");
    harness.wait_until("the replacement is confirmed", |harness| {
        harness.shows("Replace “outline.dxf”?")
    });
    quit_while_the_replacing_write_is_held(&mut harness, "Writing the drawing “outline.dxf”…");

    assert_ne!(fs::read(&written).unwrap(), b"precious");
}

#[test]
fn a_model_changed_while_quit_waited_for_a_write_is_not_closed_without_asking() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("dimensions.csv"), b"precious").unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.answer_dialog(Some(dir.path().join("dimensions")));
    run_from_palette(&mut harness, "Export parameters");
    harness.wait_until("the replacement is confirmed", |harness| {
        harness.shows("Replace “dimensions.csv”?")
    });
    let release = harness.files.hold_jobs();
    harness.click("Replace");

    harness.command(FileCommand::Quit);
    let waiting = harness.shows("Closing…") && !harness.model.is_dirty();
    add_parameters(&mut harness, &["depth"]);
    release.send(()).unwrap();
    harness.wait_until("the unsaved changes are asked about", |harness| {
        harness.shows("Save changes to “Untitled”?")
    });

    assert!(waiting);
    assert!(!harness.files.should_quit());
    assert!(harness.document().parameter_named("depth").is_some());

    harness.click("Close without saving");
    harness.wait_until("caditor quits", |harness| harness.files.should_quit());
}

#[test]
fn a_save_as_checked_after_another_model_opened_writes_nothing_and_says_so() {
    let dir = TempDir::new().unwrap();
    let root = canonical(&dir);
    let other = saved_model(&root, "other.caditor");
    let copy = root.join("copy.caditor");
    let mut harness = Harness::with_directories(Some(root.as_path()));
    assert!(harness.files.wait_for_jobs(FILE_TIMEOUT));
    let release = harness.files.hold_jobs();

    harness.answer_dialog(Some(copy.clone()));
    harness.command(FileCommand::SaveAs);
    harness.command(FileCommand::OpenPath(other.clone()));
    harness.wait_until("the other model is open", |harness| {
        harness.model.path() == Some(other.as_path())
    });
    release.send(()).unwrap();
    harness.wait_until("the dropped save is announced", |harness| {
        notice_starts(
            harness,
            "Nothing was written to “copy.caditor”, because another model was opened after you \
             chose it.",
        )
    });
    harness.settle();

    assert!(!copy.exists());
    assert_eq!(harness.model.path(), Some(other.as_path()));
    assert!(!harness.model.is_dirty());
}

#[test]
fn an_export_checked_after_another_model_opened_writes_nothing_and_says_so() {
    let dir = TempDir::new().unwrap();
    let root = canonical(&dir);
    let other = saved_model(&root, "other.caditor");
    let mut harness = Harness::with_directories(Some(root.as_path()));
    extruded_plate(&mut harness);
    assert!(harness.files.wait_for_jobs(FILE_TIMEOUT));
    let release = harness.files.hold_jobs();

    harness.command(FileCommand::Export(ExportCommand::Show));
    harness.click("STL");
    harness.answer_dialog(Some(root.join("plate")));
    harness.click("Export…");
    harness.command(FileCommand::OpenPath(other.clone()));
    harness.command(FileCommand::Guard(GuardChoice::DontSave));
    harness.wait_until("the other model is open", |harness| {
        harness.model.path() == Some(other.as_path())
    });
    release.send(()).unwrap();
    harness.wait_until("the dropped export is announced", |harness| {
        notice_starts(harness, "Nothing was written to “plate.stl”")
    });

    assert!(!harness.files.exports_running());
    assert!(!root.join("plate.stl").exists());
}
