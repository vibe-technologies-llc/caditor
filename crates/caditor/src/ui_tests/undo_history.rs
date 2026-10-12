use caditor_document::Transaction;
use caditor_geometry::Plane;
use caditor_sketch::Sketch;
use egui::{Key, Modifiers};
use tempfile::TempDir;

use super::{Harness, run_from_palette};
use crate::{app, files::FileCommand, history::HistoryCommand, icons, model::Action};

#[test]
fn every_save_keeps_a_version_that_can_be_restored_and_undone() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.command(FileCommand::History(HistoryCommand::Show));
    assert!(!harness.files.is_blocking());

    let path = dir.path().join("plate.caditor");
    harness.answer_dialog(Some(path.clone()));
    harness.command(FileCommand::SaveAs);
    harness.wait_until("the model is saved", |harness| {
        harness.model.path().is_some() && !harness.model.is_saving()
    });
    for width in ["45 mm", "50 mm"] {
        harness.edit_width(width);
        harness.command(FileCommand::Save);
        harness.wait_until("the change is saved", |harness| !harness.model.is_dirty());
    }
    let history = caditor_file::history(&path).unwrap();
    assert_eq!(history.versions.len(), 2);
    assert!(history.versions.iter().all(|version| version.available));

    harness.command(FileCommand::History(HistoryCommand::Show));
    harness.wait_until("the versions are listed", |harness| {
        harness.shows("Restore")
    });
    assert!(harness.shows("Version history"));
    harness.click("Restore");
    harness.wait_until("the version is restored", |harness| {
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.starts_with("Restored the version saved"))
    });
    assert_eq!(harness.expression_text("width"), "45 mm");
    assert!(harness.model.is_dirty());
    assert!(!harness.files.is_blocking());

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert_eq!(harness.expression_text("width"), "50 mm");
    assert!(!harness.model.is_dirty());
    harness.key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.frame();
    harness.command(FileCommand::Save);
    harness.wait_until("the restored model is saved", |harness| {
        !harness.model.is_dirty()
    });
    let reopened = caditor_file::load(&path).unwrap();
    assert_eq!(
        reopened.document.expression_text(
            &reopened
                .document
                .parameter_named("width")
                .unwrap()
                .expression
        ),
        "45 mm"
    );
    assert_eq!(caditor_file::history(&path).unwrap().versions.len(), 3);
}

#[test]
fn a_version_can_be_kept_from_the_history_and_stays_marked_in_the_file() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let path = dir.path().join("plate.caditor");
    harness.answer_dialog(Some(path.clone()));
    harness.command(FileCommand::SaveAs);
    harness.wait_until("the model is saved", |harness| {
        harness.model.path().is_some() && !harness.model.is_saving()
    });
    for width in ["45 mm", "50 mm"] {
        harness.edit_width(width);
        harness.command(FileCommand::Save);
        harness.wait_until("the change is saved", |harness| !harness.model.is_dirty());
    }
    let undo_label = harness.model.undo_label().map(str::to_owned);

    harness.command(FileCommand::History(HistoryCommand::Show));
    harness.wait_until("the versions are listed", |harness| {
        harness.shows("Restore")
    });
    assert!(!harness.shows("Stop keeping"));
    assert!(!harness.shows(icons::KEPT_VERSION));
    assert_eq!(harness.count_shown("Keep"), 2);

    harness.click("Keep");
    harness.wait_until("the version is marked kept", |harness| {
        harness.shows("Stop keeping")
    });
    assert!(harness.shows("Kept"));
    assert!(harness.shows(icons::KEPT_VERSION));
    assert_eq!(harness.count_shown("Keep"), 1);
    assert!(harness.files.is_blocking());
    let listed = caditor_file::history(&path).unwrap();
    assert_eq!(
        listed
            .versions
            .iter()
            .map(|version| version.kept)
            .collect::<Vec<_>>(),
        [true, false]
    );
    assert!(!harness.model.is_dirty());
    assert_eq!(harness.model.undo_label().map(str::to_owned), undo_label);

    harness.click("Stop keeping");
    harness.wait_until("the versions are listed again, none kept", |harness| {
        harness.count_shown("Keep") == 2
    });
    assert!(!harness.shows("Kept"));
    assert!(!harness.shows(icons::KEPT_VERSION));
    assert!(
        caditor_file::history(&path)
            .unwrap()
            .versions
            .iter()
            .all(|version| !version.kept)
    );
}

#[test]
fn the_undo_history_goes_back_and_forward_several_steps_at_once() {
    let mut harness = Harness::new();
    let before = harness.document().parameters().len();
    for name in ["first", "second", "third"] {
        let length = harness.model.length_unit().default_length(10.0);
        let mut transaction = harness.document().transaction(format!("Add {name}"));
        transaction.add_parameter(name.to_owned(), length);
        harness.perform(Action::Apply(transaction.finish()));
        harness.settle();
    }
    assert_eq!(harness.document().parameters().len(), before + 3);

    run_from_palette(&mut harness, "undo history");
    assert!(harness.workspace.undo_history_open);
    for label in ["Add first", "Add second", "Add third"] {
        assert!(harness.shows(label), "{label}");
    }

    harness.click("Add first");
    harness.settle();
    assert_eq!(harness.document().parameters().len(), before + 1);
    assert_eq!(
        harness
            .model
            .redo_steps()
            .map(Transaction::label)
            .collect::<Vec<_>>(),
        ["Add second", "Add third"]
    );
    assert!(harness.workspace.undo_history_open);
    harness.hover("Add third");
    assert!(harness.shows_containing("Parameters: third"));

    harness.click("Add third");
    harness.settle();
    assert_eq!(harness.document().parameters().len(), before + 3);
    assert_eq!(harness.model.redo_steps().count(), 0);

    harness.click("Close");
    assert!(!harness.workspace.undo_history_open);
}

#[test]
fn adding_a_feature_then_undoing_it_leaves_the_model_saved() {
    let mut harness = Harness::new();
    assert!(!harness.model.is_dirty());
    harness.add_sketch(Sketch::new(Plane::XY));
    assert!(harness.model.is_dirty());
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert!(!harness.model.is_dirty());
    assert_eq!(app::window_title(&harness.model), "Untitled — caditor");
}
