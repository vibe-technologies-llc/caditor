use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use caditor_document::Document;
use tempfile::TempDir;

use super::{FILE_TIMEOUT, Harness, canonical, cube_stl, run_from_palette};
use crate::files::FileCommand;

struct SlowReader {
    started: Arc<AtomicBool>,
    saw_cancel: Arc<AtomicBool>,
    release: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
    obeys_cancel: bool,
}

impl SlowReader {
    fn new(obeys_cancel: bool) -> Self {
        Self {
            started: Arc::default(),
            saw_cancel: Arc::default(),
            release: Arc::default(),
            finished: Arc::default(),
            obeys_cancel,
        }
    }

    fn reader(&self) -> crate::files::ModelReader {
        let started = Arc::clone(&self.started);
        let saw_cancel = Arc::clone(&self.saw_cancel);
        let release = Arc::clone(&self.release);
        let finished = Arc::clone(&self.finished);
        let obeys_cancel = self.obeys_cancel;
        Arc::new(move |path, cancel, _| {
            started.store(true, Ordering::SeqCst);
            let deadline = Instant::now() + FILE_TIMEOUT;
            while !release.load(Ordering::SeqCst) && Instant::now() < deadline {
                if cancel.is_cancelled() {
                    saw_cancel.store(true, Ordering::SeqCst);
                    if obeys_cancel {
                        finished.store(true, Ordering::SeqCst);
                        return Err(caditor_file::ImportError::Cancelled);
                    }
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            let read = crate::import::read_model(
                path,
                &caditor_document::CancelToken::never(),
                &caditor_file::ReadingProgress::default(),
            );
            finished.store(true, Ordering::SeqCst);
            read
        })
    }

    fn has(flag: &Arc<AtomicBool>) -> bool {
        flag.load(Ordering::SeqCst)
    }
}

#[derive(Default)]
struct SlowLoader {
    started: Arc<AtomicBool>,
    saw_cancel: Arc<AtomicBool>,
    release: Arc<AtomicBool>,
}

impl SlowLoader {
    fn loader(&self) -> crate::files::ModelLoader {
        let started = Arc::clone(&self.started);
        let saw_cancel = Arc::clone(&self.saw_cancel);
        let release = Arc::clone(&self.release);
        Arc::new(move |path, cancel| {
            started.store(true, Ordering::SeqCst);
            let deadline = Instant::now() + FILE_TIMEOUT;
            while !release.load(Ordering::SeqCst) && Instant::now() < deadline {
                if cancel.is_cancelled() {
                    saw_cancel.store(true, Ordering::SeqCst);
                    return Err(caditor_file::LoadError::Cancelled);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            caditor_file::load_cancellable(path, cancel)
        })
    }
}

fn wait_until_delivered(harness: &mut Harness) {
    harness.wait_until("every file thread delivered its result", |harness| {
        harness.files.own_threads_delivered()
    });
    harness.frame();
}

fn start_slow_import(harness: &mut Harness, dir: &Path, slow: &SlowReader) -> PathBuf {
    let path = dir.join("cube.stl");
    std::fs::write(&path, cube_stl(10.0)).unwrap();
    harness.files.read_models_with(slow.reader());
    harness.answer_dialog(Some(path.clone()));
    harness.command(FileCommand::Import { into: None });
    harness.wait_until("the import is reading", |_| SlowReader::has(&slow.started));
    harness.wait_until("the import is shown", |harness| {
        harness.shows("Importing “cube.stl”…")
    });
    path
}

#[test]
fn a_slow_import_can_be_cancelled_from_the_status_bar_and_the_reader_is_told_to_stop() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let slow = SlowReader::new(true);
    let before = harness.document().features().len();
    start_slow_import(&mut harness, dir.path(), &slow);

    assert!(harness.files.is_importing());
    assert!(!harness.files.is_blocking());
    assert!(!SlowReader::has(&slow.saw_cancel));
    harness.hover("Cancel");
    assert!(harness.shows_containing("Stop reading the file and add nothing to the model"));
    harness.click("Cancel");

    harness.wait_until("the reader saw the cancel", |_| {
        SlowReader::has(&slow.saw_cancel) && SlowReader::has(&slow.finished)
    });
    harness.frame();
    assert!(!harness.files.is_importing());
    assert!(!harness.shows("Importing “cube.stl”…"));
    assert_eq!(
        harness.model.notice().unwrap().text,
        "Stopped importing “cube.stl”. Nothing was added."
    );
    assert_eq!(harness.document().features().len(), before);
    assert!(!harness.shows_containing("Could not import"));
}

#[test]
fn cancelling_an_import_from_the_palette_drops_what_a_reader_that_ignores_it_returns() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let slow = SlowReader::new(false);
    let before = harness.document().features().len();
    start_slow_import(&mut harness, dir.path(), &slow);

    run_from_palette(&mut harness, "cancel the import");
    assert!(!harness.files.is_importing());
    harness.wait_until("the reader saw the cancel", |_| {
        SlowReader::has(&slow.saw_cancel)
    });
    slow.release.store(true, Ordering::SeqCst);
    harness.wait_until("the reader returned", |_| SlowReader::has(&slow.finished));
    wait_until_delivered(&mut harness);

    assert_eq!(harness.document().features().len(), before);
    assert!(!harness.files.is_importing());
    assert_eq!(
        harness.model.notice().unwrap().text,
        "Stopped importing “cube.stl”. Nothing was added."
    );
}

#[test]
fn a_new_import_after_a_cancelled_one_is_not_cancelled_with_it() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let slow = SlowReader::new(false);
    let before = harness.document().features().len();
    let path = start_slow_import(&mut harness, dir.path(), &slow);
    harness.click("Cancel");

    harness
        .files
        .read_models_with(Arc::new(|path, cancel, progress| {
            crate::import::read_model(path, cancel, progress)
        }));
    harness.answer_dialog(Some(path));
    harness.command(FileCommand::Import { into: None });
    slow.release.store(true, Ordering::SeqCst);
    harness.wait_until("the second import adds its body", |harness| {
        harness.document().features().len() == before + 1
    });
    harness.wait_until("the first reader returned", |_| {
        SlowReader::has(&slow.finished)
    });
    wait_until_delivered(&mut harness);

    assert_eq!(harness.document().features().len(), before + 1);
    assert_eq!(
        harness.model.notice().unwrap().text,
        "Imported 1 body from “cube.stl”."
    );
}

#[test]
fn opening_a_model_does_not_wait_for_a_slow_import() {
    let dir = TempDir::new().unwrap();
    let root = canonical(&dir);
    let mut harness = Harness::with_directories(Some(root.as_path()));
    let slow = SlowReader::new(true);
    start_slow_import(&mut harness, root.as_path(), &slow);
    let model = root.as_path().join("plate.caditor");
    caditor_file::save(&Document::default(), &model, false).unwrap();

    harness.command(FileCommand::OpenPath(model.clone()));
    if harness.shows("Continue without saving") {
        harness.click("Continue without saving");
    }
    harness.wait_until("the model is open beside the running import", |harness| {
        harness.model.path() == Some(model.as_path())
    });

    assert!(harness.files.is_importing());
    assert!(!SlowReader::has(&slow.finished));
    harness.click("Cancel");
    harness.wait_until("the reader finished", |_| SlowReader::has(&slow.finished));
    assert!(!harness.files.is_importing());
}

#[test]
fn closing_the_window_during_a_load_asks_about_unsaved_changes_instead_of_hiding_the_prompt() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let other = dir.path().join("other.caditor");
    caditor_file::save(&Document::default(), &other, false).unwrap();
    harness.edit_width("45 mm");
    let slow = SlowLoader::default();
    harness.files.load_models_with(slow.loader());

    harness.command(FileCommand::OpenPath(other));
    harness.click("Continue without saving");
    harness.frame();
    assert!(harness.files.is_opening());
    assert!(harness.shows_containing("Opening “other.caditor”"));

    harness.command(FileCommand::Quit);
    harness.frame();

    assert!(!harness.files.is_opening());
    assert!(!harness.shows_containing("Opening “other.caditor”"));
    assert!(harness.shows("Save changes to “Untitled”?"));
    assert!(harness.shows_containing("Stopped opening “other.caditor”"));

    harness.click("Cancel");
    harness.wait_until("the abandoned load saw the cancel", |_| {
        SlowReader::has(&slow.saw_cancel)
    });
    wait_until_delivered(&mut harness);

    assert_eq!(harness.model.path(), None);
    assert!(!harness.files.should_quit());
    assert!(!harness.shows("Save changes to “Untitled”?"));
}

#[test]
fn cancelling_a_slow_open_stops_its_read_and_leaves_the_files_worker_free() {
    let dir = TempDir::new().unwrap();
    let root = canonical(&dir);
    let mut harness = Harness::with_directories(Some(root.as_path()));
    let slow_model = root.as_path().join("slow.caditor");
    let next_model = root.as_path().join("next.caditor");
    caditor_file::save(&Document::default(), &slow_model, false).unwrap();
    caditor_file::save(&Document::default(), &next_model, false).unwrap();
    let slow = SlowLoader::default();
    harness.files.load_models_with(slow.loader());

    harness.command(FileCommand::OpenPath(slow_model));
    harness.wait_until("the load is reading", |_| SlowReader::has(&slow.started));
    let worker_free = harness.files.wait_for_jobs(FILE_TIMEOUT);
    harness.frame();
    let shown = harness.shows_containing("Opening “slow.caditor”");
    harness.command(FileCommand::CancelOpen);
    harness.wait_until("the load saw the cancel", |_| {
        SlowReader::has(&slow.saw_cancel)
    });
    harness
        .files
        .load_models_with(Arc::new(caditor_file::load_cancellable));
    harness.command(FileCommand::OpenPath(next_model.clone()));
    harness.wait_until("the next model opens", |harness| {
        harness.model.path() == Some(next_model.as_path())
    });

    assert!(worker_free);
    assert!(shown);
    assert!(!harness.files.is_opening());
    assert!(!harness.shows_containing("Could not open"));
}

#[test]
fn a_model_an_import_changed_while_the_open_dialog_was_up_is_not_replaced_without_asking() {
    let dir = TempDir::new().unwrap();
    let root = canonical(&dir);
    let mut harness = Harness::with_directories(Some(root.as_path()));
    let other = root.as_path().join("other.caditor");
    caditor_file::save(&Document::default(), &other, false).unwrap();
    let slow = SlowReader::new(true);
    start_slow_import(&mut harness, root.as_path(), &slow);

    harness.hold_dialogs();
    harness.command(FileCommand::Open);
    let clean_when_asked = !harness.model.is_dirty();
    let features_before = harness.document().features().count();
    slow.release.store(true, Ordering::SeqCst);
    harness.wait_until("the import lands", |harness| harness.model.is_dirty());
    harness.answer_held_dialogs(Some(other.clone()));
    harness.wait_until("the unsaved changes are asked about", |harness| {
        harness.shows("Save changes to “Untitled”?")
    });

    assert!(clean_when_asked);
    assert_eq!(harness.model.path(), None);

    harness.click("Cancel");

    assert_eq!(harness.model.path(), None);
    assert!(harness.model.is_dirty());
    assert_eq!(harness.document().features().count(), features_before + 1);
}

fn quit_while_an_import_dialog_is_open(harness: &mut Harness) {
    harness.edit_width("45 mm");
    harness.hold_dialogs();
    harness.command(FileCommand::Import { into: None });

    assert!(harness.files.is_picking());

    harness.command(FileCommand::Quit);
    harness.click("Save as…");

    assert_eq!(harness.dialogs.held.lock().as_ref().map(Vec::len), Some(2));
    assert!(!harness.files.is_importing());
    assert!(harness.files.is_picking());
}

#[test]
fn saving_from_the_close_prompt_while_a_file_dialog_is_open_asks_where_and_then_quits() {
    let dir = TempDir::new().unwrap();
    let root = canonical(&dir);
    let mut harness = Harness::with_directories(Some(root.as_path()));
    let path = root.as_path().join("bracket.caditor");
    quit_while_an_import_dialog_is_open(&mut harness);

    harness.answer_held_dialogs(Some(path.clone()));
    harness.wait_until("the model is saved and caditor quits", |harness| {
        harness.files.should_quit()
    });

    assert_eq!(harness.model.path(), Some(path.as_path()));
    assert_eq!(
        caditor_file::load(&path).unwrap().document,
        *harness.model.document()
    );
    assert!(!harness.files.is_importing());
}

#[test]
fn a_save_from_the_close_prompt_that_is_cancelled_does_not_quit_on_a_later_save() {
    let dir = TempDir::new().unwrap();
    let root = canonical(&dir);
    let mut harness = Harness::with_directories(Some(root.as_path()));
    let path = root.as_path().join("bracket.caditor");
    quit_while_an_import_dialog_is_open(&mut harness);

    harness.answer_held_dialogs(None);
    harness.frame();
    harness.frame();

    assert!(!harness.files.is_picking());
    assert!(!harness.files.should_quit());

    harness.answer_dialog(Some(path.clone()));
    harness.command(FileCommand::Save);
    harness.wait_until("the model is saved", |harness| {
        harness.model.path().is_some() && !harness.model.is_dirty()
    });
    harness.frame();
    harness.frame();

    assert_eq!(harness.model.path(), Some(path.as_path()));
    assert!(!harness.files.should_quit());
    assert!(!harness.shows("Closing…"));
}
