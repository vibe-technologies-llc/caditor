use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use caditor_document::{Document, Editor};
use caditor_file::{JournalEntry, Start, Storage, StorageConfig};
use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Constraint, EntityId, Sketch};
use egui::{Event, Modifiers, PointerButton};
use tempfile::TempDir;

use super::{
    FILE_TIMEOUT, Harness, canonical, damage_chunk, edit_free_sketch, fail_a_frame, line_ends,
    sample_document,
};
use crate::{
    app::{self, Workspace},
    export::ExportCommand,
    files::FileCommand,
    model::Action,
    selection::Pickable,
};

#[test]
fn opening_a_damaged_file_reports_what_was_lost_and_keeps_the_original_on_save() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("damaged.caditor");
    let bytes = caditor_file::encode(&sample_document().unwrap()).unwrap();
    let side_sketch_record = 4;
    let damaged = damage_chunk(&bytes, side_sketch_record);
    std::fs::write(&path, &damaged).unwrap();

    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.command(FileCommand::OpenPath(path.clone()));
    harness.wait_until("the file is open", |harness| harness.model.path().is_some());
    assert!(harness.shows("Parts of “damaged.caditor” could not be read"));
    assert!(
        harness.shows("A damaged part of the file was skipped; anything it held was left out.")
    );
    assert_eq!(harness.model.document().features().len(), 1);
    harness.click("Close");
    assert!(!harness.shows("Parts of “damaged.caditor” could not be read"));

    harness.command(FileCommand::Save);
    harness.wait_until("the model is saved", |harness| !harness.model.is_saving());
    let backup = dir.path().join("damaged.damaged.caditor");
    assert_eq!(std::fs::read(backup).unwrap(), damaged);
    assert!(harness.shows("Saved. The damaged original was kept as “damaged.damaged.caditor”."));
    assert!(caditor_file::load(&path).unwrap().issues.is_empty());
}

#[test]
fn an_unreadable_journal_is_kept_aside_and_reported_when_its_model_opens() {
    let dir = TempDir::new().unwrap();
    let root = canonical(&dir);
    let path = root.as_path().join("plate.caditor");
    caditor_file::save(&sample_document().unwrap(), &path, false).unwrap();
    let journal = root.as_path().join(".plate.caditor.journal");
    std::fs::write(&journal, b"\x89CDJ\r\n\x1a\n\xff\xff\xff\xff").unwrap();

    let mut harness = Harness::with_directories(Some(root.as_path()));
    harness.command(FileCommand::OpenPath(path.clone()));
    harness.wait_until("the file is open", |harness| harness.model.path().is_some());
    assert!(harness.shows("Unsaved changes to “plate.caditor” could not be recovered"));
    let kept: Vec<String> = std::fs::read_dir(root.as_path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".unreadable"))
        .collect();
    assert_eq!(kept.len(), 1);
    let note = caditor_file::describe_set_aside(&root.as_path().join(&kept[0]));
    assert!(harness.shows(&note), "{note}");
    harness.click("Close");
    assert!(!harness.model.is_dirty());
}

fn crashed_with_a_width_change(dir: &Path) -> Editor {
    crashed_with_a_width_change_to(dir, None)
}

fn crashed_with_a_width_change_to(dir: &Path, file: Option<PathBuf>) -> Editor {
    let base = sample_document().unwrap();
    if let Some(file) = &file {
        caditor_file::save(&base, file, false).unwrap();
    }
    let mut editor = Editor::new(base.clone());
    let width = base.parameter_named("width").unwrap().id();
    let change = caditor_document::Transaction::single(
        "Edit width",
        caditor_document::Edit::SetParameterExpression {
            id: width,
            expression: base.parse("55 mm").unwrap(),
        },
    );
    editor.apply(change.clone()).unwrap();
    let crashed = Storage::spawn(
        StorageConfig {
            recovery_dir: Some(dir.join("recovery")),
            ..StorageConfig::default()
        },
        Start {
            file,
            on_disk: None,
            loaded_with_problems: false,
            base,
            folded: 0,
            entries: vec![JournalEntry::Apply(change)],
            replaces: None,
            after: None,
        },
        || {},
    )
    .unwrap();
    assert!(crashed.flusher().flush(FILE_TIMEOUT));
    assert!(crashed.close(false).wait(FILE_TIMEOUT));
    editor
}

#[test]
fn unsaved_work_from_a_crash_is_offered_and_restored_with_its_history() {
    let dir = TempDir::new().unwrap();
    let editor = crashed_with_a_width_change(dir.path());

    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.wait_until("recovery is offered", |harness| {
        harness.shows("Recover unsaved work")
    });
    assert!(harness.shows("Untitled model"));
    assert!(harness.shows("1 unsaved change, last one just now"));
    harness.click("Restore");

    assert!(!harness.shows("Recover unsaved work"));
    assert_eq!(harness.model.document(), editor.document());
    assert!(harness.model.is_dirty());
    assert_eq!(harness.model.undo_label(), Some("Edit width"));
    assert_eq!(harness.expression_text("width"), "55 mm");
    let recovery = dir.path().join("recovery");
    harness.wait_until("the restored journal replaces the recovered one", |_| {
        std::fs::read_dir(&recovery).unwrap().count() == 1
    });
}

#[derive(Clone, Default)]
struct HeldScan {
    reached: Arc<std::sync::atomic::AtomicBool>,
    release: Arc<std::sync::atomic::AtomicBool>,
}

impl HeldScan {
    fn scanner(&self, holding: Option<PathBuf>) -> crate::files::Scanner {
        let held = self.clone();
        Arc::new(move |dir, recent| {
            let locked = holding.as_ref().map(|journal| {
                let file = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(journal)
                    .unwrap();
                file.lock().unwrap();
                file
            });
            let found = locked.is_none().then(|| caditor_file::scan(dir, recent));
            held.reached
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let deadline = Instant::now() + FILE_TIMEOUT;
            while !held.release.load(std::sync::atomic::Ordering::SeqCst)
                && Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(1));
            }
            drop(locked);
            found.unwrap_or_else(|| caditor_file::scan(dir, recent))
        })
    }

    fn has_reached(&self) -> bool {
        self.reached.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn let_go(&self) {
        self.release
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

fn starting_with_held_scan(root: &Path, held: &HeldScan, holding: Option<PathBuf>) -> Harness {
    let scanner = held.scanner(holding);
    let mut harness = Harness::starting_with(
        Some(root),
        sample_document().unwrap(),
        Workspace::new(),
        move |files| files.scan_with(scanner),
    );
    harness.wait_until("the startup scan has looked", |_| held.has_reached());
    harness
}

fn let_the_scan_report(harness: &mut Harness, held: &HeldScan) {
    held.let_go();
    assert!(harness.files.wait_for_jobs(FILE_TIMEOUT));
    harness.frame();
    harness.frame();
}

#[test]
fn work_restored_while_the_startup_scan_ran_is_not_offered_again_when_it_reports() {
    let dir = TempDir::new().unwrap();
    let root = canonical(&dir);
    let path = root.as_path().join("plate.caditor");
    let other = root.as_path().join("other.caditor");
    caditor_file::save(&Document::default(), &other, false).unwrap();
    let editor = crashed_with_a_width_change_to(root.as_path(), Some(path.clone()));
    let held = HeldScan::default();
    let mut harness = starting_with_held_scan(root.as_path(), &held, None);

    harness.command(FileCommand::OpenPath(path.clone()));
    harness.wait_until("the unsaved work is offered", |harness| {
        harness.shows("Recover unsaved work")
    });
    harness.click("Restore");
    assert_eq!(harness.model.document(), editor.document());
    harness.command(FileCommand::OpenPath(other.clone()));
    harness.click("Continue without saving");
    harness.wait_until("the other model opens", |harness| {
        harness.model.path() == Some(other.as_path())
    });

    let_the_scan_report(&mut harness, &held);

    assert!(!harness.files.has_recoverable());
    assert!(!harness.shows("Recover unsaved work"));
    assert_eq!(harness.model.path(), Some(other.as_path()));
}

#[test]
fn the_open_models_own_journal_found_by_the_startup_scan_is_not_offered() {
    let dir = TempDir::new().unwrap();
    let root = canonical(&dir);
    let path = root.as_path().join("plate.caditor");
    crashed_with_a_width_change_to(root.as_path(), Some(path.clone()));
    let held = HeldScan::default();
    let mut harness = starting_with_held_scan(root.as_path(), &held, None);

    caditor_file::discard(&root.as_path().join(".plate.caditor.journal")).unwrap();
    harness.command(FileCommand::OpenPath(path.clone()));
    harness.wait_until("the model opens", |harness| {
        harness.model.path() == Some(path.as_path())
    });
    harness.edit_width("60 mm");

    let_the_scan_report(&mut harness, &held);

    assert!(!harness.files.has_recoverable());
    assert!(!harness.shows("Recover unsaved work"));
    assert_eq!(harness.expression_text("width"), "60 mm");
}

#[test]
fn opening_a_model_whose_journal_the_startup_scan_holds_waits_for_the_scan() {
    let dir = TempDir::new().unwrap();
    let root = canonical(&dir);
    let path = root.as_path().join("plate.caditor");
    let editor = crashed_with_a_width_change_to(root.as_path(), Some(path.clone()));
    let held = HeldScan::default();
    let journal = root.as_path().join(".plate.caditor.journal");
    let mut harness = starting_with_held_scan(root.as_path(), &held, Some(journal));

    harness.command(FileCommand::OpenPath(path.clone()));
    harness.frame();

    assert!(harness.files.is_opening());
    assert!(!harness.shows_containing("already open in another caditor window"));

    let_the_scan_report(&mut harness, &held);
    harness.wait_until("the unsaved work is offered", |harness| {
        !harness.files.is_opening() && harness.shows("Recover unsaved work")
    });

    assert!(harness.model.notice().is_none_or(|notice| {
        !notice
            .text
            .contains("already open in another caditor window")
    }));
    harness.click("Restore");
    assert_eq!(harness.model.document(), editor.document());
    assert_eq!(harness.model.path(), Some(path.as_path()));
}

#[test]
fn unsaved_work_can_be_restored_with_every_feature_suppressed_and_brought_back_by_undo() {
    let dir = TempDir::new().unwrap();
    let editor = crashed_with_a_width_change(dir.path());
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.wait_until("recovery is offered", |harness| {
        harness.shows("Recover unsaved work")
    });

    harness.click("Restore suppressed");

    assert!(
        harness
            .model
            .document()
            .features()
            .all(|feature| feature.suppressed)
    );
    assert!(
        harness
            .model
            .notice()
            .unwrap()
            .text
            .starts_with("Restored with every feature suppressed.")
    );
    assert_eq!(harness.model.undo_label(), Some("Suppress every feature"));
    harness.perform(Action::Undo);
    assert_eq!(harness.model.document(), editor.document());
    assert_eq!(harness.model.undo_label(), Some("Edit width"));
}

#[cfg(unix)]
#[test]
fn a_discard_that_fails_keeps_the_recovery_offer_and_says_the_changes_can_still_be_restored() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    let editor = crashed_with_a_width_change(dir.path());
    let recovery = dir.path().join("recovery");
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.wait_until("recovery is offered", |harness| {
        harness.shows("Recover unsaved work")
    });
    assert!(harness.model.flush_journal(FILE_TIMEOUT));

    std::fs::set_permissions(&recovery, std::fs::Permissions::from_mode(0o500)).unwrap();
    harness.click("Discard…");
    harness.click("Discard");
    harness.wait_until("the failure is reported", |harness| {
        harness.model.notice().is_some_and(|notice| {
            notice
                .text
                .starts_with("Could not discard the recovered changes")
        })
    });
    std::fs::set_permissions(&recovery, std::fs::Permissions::from_mode(0o700)).unwrap();

    let notice = harness.model.notice().unwrap().text.clone();
    assert_eq!(
        notice,
        "Could not discard the recovered changes: you do not have permission to write to its \
         folder. They are still offered under Recover unsaved work, where you can restore them."
    );
    assert!(harness.model.unprotected().is_none());
    assert!(harness.files.has_recoverable());
    assert!(harness.shows("Recover unsaved work"));
    assert!(harness.shows("1 unsaved change, last one just now"));

    harness.click("Restore");

    assert_eq!(harness.model.document(), editor.document());
    assert!(!harness.files.has_recoverable());
}

#[test]
fn discarding_changes_another_window_already_removed_takes_them_off_the_list() {
    let dir = TempDir::new().unwrap();
    crashed_with_a_width_change(dir.path());
    let recovery = dir.path().join("recovery");
    let crashed = std::fs::read_dir(&recovery)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.wait_until("recovery is offered", |harness| {
        harness.shows("Recover unsaved work")
    });
    let before = harness.model.document().clone();

    for journal in &crashed {
        std::fs::remove_file(journal).unwrap();
    }
    harness.click("Discard…");
    harness.click("Discard");
    harness.wait_until("the offer is gone", |harness| {
        !harness.files.has_recoverable()
    });

    assert!(!harness.shows("Recover unsaved work"));
    assert!(
        harness
            .model
            .notice()
            .is_none_or(|notice| !notice.text.starts_with("Could not discard"))
    );
    assert_eq!(harness.model.document(), &before);
}

#[test]
fn a_failed_frame_during_a_sketch_drag_drops_the_drag_and_shows_the_committed_sketch() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
    let (start, end) = line_ends(&sketch, line);
    sketch
        .add_constraint(Constraint::Coincident(start, EntityId::ORIGIN))
        .unwrap();
    let feature = edit_free_sketch(&mut harness, sketch);
    let committed = harness.shown(feature);

    let grabbed = harness.hover_pickable(
        Plane::XY,
        Point2::new(40.0, 0.0),
        Pickable::SketchEntity {
            feature,
            entity: end,
        },
    );
    let target = harness.on_screen(Point2::new(30.0, 12.0));
    harness.events.push(Event::PointerButton {
        pos: grabbed,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    for step in 1..=4 {
        harness.events.push(Event::PointerMoved(
            grabbed + (target - grabbed) * (step as f32 / 4.0),
        ));
        harness.frame();
    }
    harness.wait_until("the drag is shown", |harness| {
        harness.shown(feature).point(end) != committed.point(end)
    });
    assert!(harness.model.is_dragging());

    let step = fail_a_frame(&mut harness);

    assert_eq!(step, app::AfterFailedFrame::ResetInterface);
    assert!(!harness.model.is_dragging());
    assert_eq!(harness.shown(feature), committed);
    assert_eq!(harness.sketch(feature), &committed);
}

#[test]
fn a_failed_frame_resets_the_interface_and_a_second_in_a_row_suppresses_every_feature() {
    let mut harness = Harness::new();
    let before = harness.model.document().clone();
    harness.workspace.preferences_open = true;
    harness.workspace.about_open = true;
    harness.command(FileCommand::Export(ExportCommand::Show));
    assert!(harness.files.is_blocking());

    let first = fail_a_frame(&mut harness);

    assert_eq!(first, app::AfterFailedFrame::ResetInterface);
    assert!(!harness.workspace.preferences_open);
    assert!(!harness.workspace.about_open);
    assert!(!harness.files.is_blocking());
    assert_eq!(harness.model.document(), &before);
    assert!(
        harness
            .model
            .notice()
            .unwrap()
            .text
            .starts_with("Something went wrong while drawing the window")
    );

    let second = fail_a_frame(&mut harness);

    assert_eq!(second, app::AfterFailedFrame::SuppressFeatures);
    assert!(
        harness
            .model
            .document()
            .features()
            .all(|feature| feature.suppressed)
    );
    harness.perform(Action::Undo);
    assert_eq!(harness.model.document(), &before);
    harness.frame();
    harness.workspace.frame_failures.drawn();
    assert_eq!(
        fail_a_frame(&mut harness),
        app::AfterFailedFrame::ResetInterface
    );
    for _ in 0..3 {
        fail_a_frame(&mut harness);
    }
    assert_eq!(fail_a_frame(&mut harness), app::AfterFailedFrame::GiveUp);
}
