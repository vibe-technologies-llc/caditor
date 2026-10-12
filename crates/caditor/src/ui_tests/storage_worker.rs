use std::path::{Path, PathBuf};

use caditor_file::Inspection;
use egui::{Key, Modifiers};
use tempfile::TempDir;

use super::{FILE_TIMEOUT, Harness, canonical};
use crate::files::FileCommand;

const WRITER_STOPPED: &str =
    "Could not save, because the background writer stopped. Try saving again.";

fn journals_in(recovery: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(recovery)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect()
}

fn journal_in(recovery: &Path) -> PathBuf {
    let mut journals = journals_in(recovery);
    assert_eq!(journals.len(), 1);
    journals.remove(0)
}

fn journal_is_held(recovery: &Path) -> bool {
    matches!(
        caditor_file::inspect(&journal_in(recovery)).unwrap(),
        Inspection::InUse
    )
}

fn replacement_journal(recovery: &Path, replaced: &Path) -> bool {
    matches!(journals_in(recovery).as_slice(), [journal] if journal != replaced)
}

fn journalled_changes(recovery: &Path) -> usize {
    match caditor_file::inspect(&journal_in(recovery)).unwrap() {
        Inspection::Recoverable(recovered) => recovered.changes(),
        other => panic!("the journal should be recoverable, not {other:?}"),
    }
}

fn started(dir: &TempDir) -> (Harness, PathBuf) {
    let root = canonical(dir);
    let mut harness = Harness::with_directories(Some(root.as_path()));
    harness.edit_width("45 mm");
    harness.edit_width("46 mm");
    assert!(harness.model.flush_journal(FILE_TIMEOUT));
    (harness, root.join("recovery"))
}

fn wait_for_new_worker(harness: &mut Harness, recovery: &Path, replaced: &Path) {
    harness.wait_until("a new storage worker holds the journal", |_| {
        replacement_journal(recovery, replaced) && journal_is_held(recovery)
    });
}

#[test]
fn a_change_the_stopped_worker_cannot_take_keeps_every_entry_and_starts_a_new_worker() {
    let dir = TempDir::new().unwrap();
    let (mut harness, recovery) = started(&dir);
    let replaced = journal_in(&recovery);
    let before = harness.model.unsaved_journal_entries();
    assert!(before >= 2);

    harness.model.stop_storage_worker();
    assert!(!journal_is_held(&recovery));
    harness.edit_width("47 mm");

    assert!(harness.model.has_storage_worker());
    assert_eq!(harness.model.unsaved_journal_entries(), before + 1);
    assert!(harness.model.unprotected().is_none());

    wait_for_new_worker(&mut harness, &recovery, &replaced);
    assert!(harness.model.flush_journal(FILE_TIMEOUT));
    harness.model.stop_storage_worker();

    assert_eq!(journalled_changes(&recovery), before + 1);
    let recovered = caditor_file::scan(Some(&recovery), &[]);
    assert_eq!(recovered.len(), 1);
    assert_eq!(
        recovered[0].editor.document().expression_text(
            &recovered[0]
                .editor
                .document()
                .parameter_named("width")
                .unwrap()
                .expression
        ),
        "47 mm"
    );
}

#[test]
fn a_worker_found_stopped_while_polling_is_replaced_with_every_entry_kept() {
    let dir = TempDir::new().unwrap();
    let (mut harness, recovery) = started(&dir);
    let replaced = journal_in(&recovery);
    let before = harness.model.unsaved_journal_entries();

    harness.model.stop_storage_worker();
    assert!(!journal_is_held(&recovery));
    wait_for_new_worker(&mut harness, &recovery, &replaced);

    assert!(harness.model.has_storage_worker());
    assert_eq!(harness.model.unsaved_journal_entries(), before);
    assert!(harness.model.unprotected().is_none());

    harness.model.stop_storage_worker();

    assert_eq!(journalled_changes(&recovery), before);
}

#[test]
fn a_save_the_stopped_worker_cannot_take_is_reported_as_failed_and_the_next_try_writes_the_file() {
    let dir = TempDir::new().unwrap();
    let (mut harness, recovery) = started(&dir);
    let replaced = journal_in(&recovery);
    let path = canonical(&dir).join("plate.caditor");
    let before = harness.model.unsaved_journal_entries();

    harness.model.stop_storage_worker();
    harness.model.save_to(path.clone());

    assert!(!harness.model.is_saving());
    assert_eq!(harness.model.notice().unwrap().text, WRITER_STOPPED);
    assert!(!path.exists());
    assert!(harness.model.is_dirty());
    assert_eq!(harness.model.unsaved_journal_entries(), before);

    wait_for_new_worker(&mut harness, &recovery, &replaced);
    harness.edit_width("48 mm");
    harness.model.stop_storage_worker();

    assert_eq!(journalled_changes(&recovery), before + 1);

    let replaced = journal_in(&recovery);
    harness.model.save_to(path.clone());
    assert_eq!(harness.model.notice().unwrap().text, WRITER_STOPPED);
    wait_for_new_worker(&mut harness, &recovery, &replaced);
    harness.model.save_to(path.clone());
    harness.wait_until("the model is saved", |harness| {
        harness.model.path().is_some() && !harness.model.is_saving()
    });

    assert_eq!(
        caditor_file::load(&path).unwrap().document,
        *harness.model.document()
    );
}

#[test]
fn quitting_with_the_worker_gone_still_closes_without_saving() {
    let dir = TempDir::new().unwrap();
    let (mut harness, _) = started(&dir);

    harness.model.stop_storage_worker();
    harness.command(FileCommand::Quit);
    harness.click("Close without saving");
    harness.wait_until("caditor quits", |harness| harness.files.should_quit());
}

#[test]
fn saving_from_the_close_prompt_with_the_worker_gone_writes_the_file_through_a_new_worker_then_quits()
 {
    let dir = TempDir::new().unwrap();
    let (mut harness, _) = started(&dir);
    let path = canonical(&dir).join("bracket.caditor");

    harness.model.stop_storage_worker();
    harness.answer_dialog(Some(path.clone()));
    harness.command(FileCommand::Quit);
    harness.click("Save as…");
    harness.wait_until("caditor quits", |harness| harness.files.should_quit());
    assert!(harness.files.wait_for_jobs(FILE_TIMEOUT));

    assert_eq!(
        caditor_file::load(&path).unwrap().document,
        *harness.model.document()
    );
}

#[test]
fn quitting_an_unchanged_model_with_the_worker_gone_closes_at_once() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));

    harness.model.stop_storage_worker();
    harness.key(Key::Q, Modifiers::COMMAND);
    harness.command(FileCommand::Quit);
    harness.wait_until("caditor quits", |harness| harness.files.should_quit());
}
