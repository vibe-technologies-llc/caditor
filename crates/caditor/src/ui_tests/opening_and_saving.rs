use caditor_document::Document;
use caditor_file::{Start, Storage, StorageConfig};
use caditor_sketch::Entity;
use egui::{Key, Modifiers};
use tempfile::TempDir;

use super::{FILE_TIMEOUT, Harness, canonical, sample_document};
use crate::{app, export::ExportCommand, files::FileCommand, panels::Focus, selection::Pickable};

#[test]
fn closing_with_unsaved_changes_asks_first_and_the_title_marks_them() {
    let mut harness = Harness::new();
    assert_eq!(app::window_title(&harness.model), "Untitled — caditor");
    harness.command(FileCommand::Quit);
    harness.wait_until("caditor quits", |harness| harness.files.should_quit());

    let mut harness = Harness::new();
    harness.edit_width("45 mm");
    assert_eq!(app::window_title(&harness.model), "*Untitled — caditor");
    harness.key(Key::Q, Modifiers::COMMAND);
    harness.frame();
    harness.show_new_windows();
    assert!(harness.shows("Save changes to “Untitled”?"));
    harness.click("Cancel");
    assert!(!harness.shows("Save changes to “Untitled”?"));
    assert!(!harness.files.should_quit());

    harness.command(FileCommand::Quit);
    harness.click("Close without saving");
    harness.wait_until("caditor quits", |harness| harness.files.should_quit());
}

#[test]
fn save_as_adds_the_model_extension_and_asks_before_replacing_what_the_dialog_did_not_name() {
    let dir = TempDir::new().unwrap();
    let existing = dir.path().join("Bracket v1.2.caditor");
    caditor_file::save(&Document::default(), &existing, false).unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.answer_dialog(Some(dir.path().join("Bracket v1.2")));

    harness.key(Key::S, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.wait_until("the replacement is confirmed", |harness| {
        harness.shows("Replace “Bracket v1.2.caditor”?")
    });
    harness.click("Cancel");
    assert_eq!(harness.model.path(), None);
    assert_eq!(
        caditor_file::load(&existing).unwrap().document,
        Document::default()
    );

    harness.key(Key::S, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.wait_until("the replacement is confirmed", |harness| {
        harness.shows("Replace “Bracket v1.2.caditor”?")
    });
    harness.click("Replace");
    harness.wait_until("the model is saved", |harness| {
        harness.model.path().is_some() && !harness.model.is_saving()
    });
    assert_eq!(
        harness.model.path(),
        Some(dunce::canonicalize(&existing).unwrap().as_path())
    );
    assert_eq!(
        caditor_file::load(&existing).unwrap().document,
        *harness.model.document()
    );
}

#[test]
fn saving_over_a_file_another_program_changed_asks_before_replacing_it() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("plate.caditor");
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.answer_dialog(Some(path.clone()));
    harness.key(Key::S, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.wait_until("the model is saved", |harness| {
        harness.model.path().is_some() && !harness.model.is_saving()
    });
    let mut outside = harness.model.document().clone();
    let mut transaction = outside.transaction("Elsewhere");
    transaction.add_parameter("elsewhere", transaction.parse("3 mm").unwrap());
    outside.apply(transaction.finish()).unwrap();
    caditor_file::save(&outside, &path, false).unwrap();
    harness.edit_width("41 mm");

    harness.key(Key::S, Modifiers::COMMAND);
    harness.wait_until("the outside change is shown", |harness| {
        harness.shows("“plate.caditor” was changed by another program")
    });
    harness.click("Cancel");

    assert!(harness.model.is_dirty());
    assert_eq!(caditor_file::load(&path).unwrap().document, outside);

    harness.key(Key::S, Modifiers::COMMAND);
    harness.wait_until("the outside change is shown", |harness| {
        harness.shows("“plate.caditor” was changed by another program")
    });
    harness.click("Replace");
    harness.wait_until("the change is saved", |harness| !harness.model.is_dirty());
    let listed = caditor_file::history(&path).unwrap();
    let kept = caditor_file::load_version(&path, listed.versions[0].index).unwrap();

    assert_eq!(
        caditor_file::load(&path).unwrap().document,
        *harness.model.document()
    );
    assert_eq!(kept.document, outside);

    harness.edit_width("42 mm");
    harness.key(Key::S, Modifiers::COMMAND);
    harness.wait_until("the next change is saved", |harness| {
        !harness.model.is_dirty()
    });
    assert!(!harness.shows("“plate.caditor” was changed by another program"));
}

#[test]
fn the_outside_change_prompt_of_a_model_left_without_saving_is_dropped_and_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("plate.caditor");
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.answer_dialog(Some(path.clone()));
    harness.key(Key::S, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.wait_until("the model is saved", |harness| {
        harness.model.path().is_some() && !harness.model.is_saving()
    });
    let mut outside = harness.model.document().clone();
    let mut transaction = outside.transaction("Elsewhere");
    transaction.add_parameter("elsewhere", transaction.parse("3 mm").unwrap());
    outside.apply(transaction.finish()).unwrap();
    caditor_file::save(&outside, &path, false).unwrap();
    let versions = caditor_file::history(&path).unwrap().versions.len();
    harness.edit_width("41 mm");

    harness.key(Key::S, Modifiers::COMMAND);
    harness.wait_until("the outside change is reported", |harness| {
        harness.shows("“plate.caditor” was changed by another program")
    });
    harness.command(FileCommand::New);

    assert!(harness.shows("Save changes to “plate.caditor”?"));

    harness.click("Continue without saving");
    harness.frame();

    assert_eq!(harness.model.path(), None);
    assert!(!harness.shows("“plate.caditor” was changed by another program"));
    assert!(!harness.files.is_blocking());
    assert!(
        harness
            .model
            .notice()
            .unwrap()
            .text
            .starts_with("“plate.caditor” was not saved: another program had changed it")
    );

    harness.command(FileCommand::OutsideChange(
        crate::files::OutsideChangeChoice::Replace,
    ));
    harness.wait_until("no save runs", |harness| !harness.model.is_saving());

    assert_eq!(caditor_file::load(&path).unwrap().document, outside);
    assert_eq!(
        caditor_file::history(&path).unwrap().versions.len(),
        versions
    );
}

#[test]
fn save_as_refuses_a_model_open_in_another_window() {
    let dir = TempDir::new().unwrap();
    let other_path = dir.path().join("other.caditor");
    caditor_file::save(&Document::default(), &other_path, false).unwrap();
    let other = Storage::spawn(
        StorageConfig {
            recovery_dir: Some(dir.path().join("recovery")),
            ..StorageConfig::default()
        },
        Start {
            file: Some(other_path.clone()),
            on_disk: None,
            loaded_with_problems: false,
            base: Document::default(),
            folded: 0,
            entries: Vec::new(),
            replaces: None,
            after: None,
        },
        || {},
    )
    .unwrap();
    assert!(other.flusher().flush(FILE_TIMEOUT));

    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.answer_dialog(Some(other_path.clone()));
    harness.key(Key::S, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.wait_until("the save is refused", |harness| {
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.contains("open in another caditor window"))
    });
    assert_eq!(harness.model.path(), None);
    assert_eq!(
        caditor_file::load(&other_path).unwrap().document,
        Document::default()
    );
    assert!(other.close(true).wait(FILE_TIMEOUT));
}

#[cfg(unix)]
#[test]
fn an_unwritable_recovery_folder_shows_that_changes_are_not_protected() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    let recovery = dir.path().join("recovery");
    std::fs::create_dir(&recovery).unwrap();
    std::fs::set_permissions(&recovery, std::fs::Permissions::from_mode(0o500)).unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.wait_until("the problem is shown", |harness| {
        harness.model.unprotected().is_some()
    });
    assert!(harness.shows("Not protected"));
    harness.edit_width("45 mm");
    assert!(harness.shows("Not protected"));
    std::fs::set_permissions(&recovery, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn saving_from_the_close_prompt_writes_the_file_then_quits() {
    let dir = TempDir::new().unwrap();
    let root = canonical(&dir);
    let mut harness = Harness::with_directories(Some(root.as_path()));
    harness.edit_width("45 mm");
    assert!(harness.model.flush_journal(FILE_TIMEOUT));
    let untitled_journals = std::fs::read_dir(root.as_path().join("recovery"))
        .unwrap()
        .count();
    assert_eq!(untitled_journals, 1);

    harness.answer_dialog(Some(root.as_path().join("bracket")));
    harness.command(FileCommand::Quit);
    harness.click("Save as…");
    harness.wait_until("the model is saved", |harness| harness.files.should_quit());
    assert!(harness.files.wait_for_jobs(FILE_TIMEOUT));

    let path = root.as_path().join("bracket.caditor");
    let saved = caditor_file::load(&path).unwrap();
    assert!(saved.issues.is_empty());
    assert_eq!(saved.document, *harness.model.document());
    assert_eq!(
        std::fs::read_dir(root.as_path().join("recovery"))
            .unwrap()
            .count(),
        0
    );
    assert!(!root.as_path().join(".bracket.caditor.journal").exists());
    let recent = caditor_file::RecentFiles::load(&root.as_path().join("state"));
    assert_eq!(recent.paths(), [path]);
}

#[test]
fn save_as_names_the_document_and_a_new_edit_marks_it_unsaved_again() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let path = dir.path().join("plate.caditor");
    harness.answer_dialog(Some(path.clone()));
    harness.key(Key::S, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.wait_until("the model is saved", |harness| {
        harness.model.path().is_some() && !harness.model.is_saving()
    });
    assert_eq!(app::window_title(&harness.model), "plate.caditor — caditor");

    harness.edit_width("41 mm");
    assert!(harness.model.is_dirty());
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert!(!harness.model.is_dirty());
    harness.key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.frame();
    harness.key(Key::S, Modifiers::COMMAND);
    harness.wait_until("the change is saved", |harness| !harness.model.is_dirty());
    let saved = caditor_file::load(&path).unwrap().document;
    assert_eq!(saved, *harness.model.document());
}

#[test]
fn saving_while_typing_in_a_field_commits_the_field_first() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let path = dir.path().join("plate.caditor");
    harness.answer_dialog(Some(path.clone()));
    harness.command(FileCommand::SaveAs);
    harness.wait_until("the model is saved", |harness| {
        harness.model.path().is_some() && !harness.model.is_saving()
    });

    let width = harness.parameter("width");
    harness.focus(Focus::ParameterValue(width));
    harness.replace_text("52 mm");
    assert_eq!(
        harness.focused(),
        Some(Focus::ParameterValue(width).field_id())
    );
    harness.key(Key::S, Modifiers::COMMAND);
    harness.frame();
    harness.frame();
    harness.wait_until("the typed width is saved", |harness| {
        !harness.model.is_dirty() && !harness.model.is_saving()
    });

    assert_eq!(harness.expression_text("width"), "52 mm");
    assert_eq!(harness.focused(), None);
    let reopened = caditor_file::load(&path).unwrap();
    let document = &reopened.document;
    assert_eq!(
        document.expression_text(&document.parameter_named("width").unwrap().expression),
        "52 mm"
    );
}

#[cfg(unix)]
#[test]
fn a_file_dialog_that_cannot_open_says_what_to_install_rather_than_doing_nothing() {
    let mut harness = Harness::new();
    *harness.dialogs.no_portal.lock() = true;

    harness.command(FileCommand::Open);
    harness.wait_until("the failure is reported", |harness| {
        harness.model.notice().is_some()
    });

    let notice = harness.model.notice().unwrap().text.clone();
    assert!(
        notice.starts_with("The file dialog could not be shown"),
        "{notice}"
    );
    assert!(notice.contains("xdg-desktop-portal"));
    *harness.dialogs.no_portal.lock() = false;
    harness.answer_dialog(None);
    harness.command(FileCommand::Open);
    harness.frame();
    assert!(!harness.files.is_blocking());
}

#[test]
fn a_recent_model_whose_folder_is_gone_too_stays_listed() {
    let dir = TempDir::new().unwrap();
    let root = canonical(&dir);
    let drive = root.as_path().join("drive");
    std::fs::create_dir(&drive).unwrap();
    let path = drive.join("kept.caditor");
    caditor_file::save(&sample_document().unwrap(), &path, false).unwrap();
    let mut harness = Harness::with_directories(Some(root.as_path()));
    harness.command(FileCommand::OpenPath(path.clone()));
    harness.wait_until("the file is open", |harness| harness.model.path().is_some());

    std::fs::rename(&drive, root.as_path().join("unplugged")).unwrap();
    harness.command(FileCommand::New);
    harness.command(FileCommand::OpenPath(path.clone()));
    harness.wait_until("the failure is reported", |harness| {
        harness.shows("Could not open “kept.caditor”: it no longer exists.")
    });

    assert_eq!(harness.files.recent(), std::slice::from_ref(&path));
}

#[test]
fn a_missing_file_is_reported_and_dropped_from_recent_files_and_reopening_is_harmless() {
    let dir = TempDir::new().unwrap();
    let root = canonical(&dir);
    let path = root.as_path().join("kept.caditor");
    caditor_file::save(&sample_document().unwrap(), &path, false).unwrap();
    let mut harness = Harness::with_directories(Some(root.as_path()));
    harness.command(FileCommand::OpenPath(path.clone()));
    harness.wait_until("the file is open", |harness| harness.model.path().is_some());
    assert_eq!(harness.files.recent(), std::slice::from_ref(&path));

    harness.command(FileCommand::OpenPath(path.clone()));
    harness.wait_until("the file is checked", |harness| {
        harness.shows("“kept.caditor” is already open.")
    });

    std::fs::rename(&path, root.as_path().join("moved.caditor")).unwrap();
    harness.command(FileCommand::New);
    harness.command(FileCommand::OpenPath(path));
    harness.wait_until("the failure is reported", |harness| {
        harness.shows(
            "Could not open “kept.caditor”: it no longer exists. It was removed from the recent \
             models.",
        )
    });
    assert!(harness.files.recent().is_empty());
    assert_eq!(harness.model.path(), None);
}

#[test]
fn a_newly_opened_model_starts_with_nothing_selected_or_left_out_of_export() {
    let mut harness = Harness::new();
    let base = harness.document().features().next().unwrap().id();
    let line = harness
        .sketch(base)
        .entities()
        .find_map(|(id, entity)| matches!(entity, Entity::Line { .. }).then_some(id))
        .unwrap();
    let chosen = Pickable::SketchEntity {
        feature: base,
        entity: line,
    };
    let session = harness.model.session();

    harness.select([chosen]);
    harness.workspace.panels.selected = Some(base);
    harness.command(FileCommand::Export(ExportCommand::Include {
        body: base,
        included: false,
    }));
    harness.command(FileCommand::OpenSample(crate::samples::Sample::Plate));
    if harness.shows("Continue without saving") {
        harness.click("Continue without saving");
    }
    harness.wait_until("the sample opens", |harness| {
        harness.model.session() != session
    });
    harness.settle();

    assert!(harness.document().feature(base).is_some());
    assert!(harness.workspace.viewport.selection().is_empty());
    assert_eq!(harness.workspace.panels.selected, None);
    assert!(harness.files.exporter_includes(base));
}

#[test]
fn a_newly_opened_model_forgets_the_coordinate_system_and_the_pull_the_last_one_chose() {
    let mut harness = Harness::new();
    let base = harness.document().features().next().unwrap().id();
    let line = harness
        .sketch(base)
        .entities()
        .find_map(|(id, entity)| matches!(entity, Entity::Line { .. }).then_some(id))
        .unwrap();
    let session = harness.model.session();

    harness.workspace.measure.relative_to = Some(base);
    harness.workspace.analysis.pull = crate::analysis::Pull::Picked(Pickable::SketchEntity {
        feature: base,
        entity: line,
    });
    harness.command(FileCommand::OpenSample(crate::samples::Sample::Plate));
    if harness.shows("Continue without saving") {
        harness.click("Continue without saving");
    }
    harness.wait_until("the sample opens", |harness| {
        harness.model.session() != session
    });
    harness.settle();

    assert_eq!(harness.workspace.measure.relative_to, None);
    assert_eq!(
        harness.workspace.analysis.pull,
        crate::analysis::Pull::Axis(crate::selection::Axis::Z)
    );
}
