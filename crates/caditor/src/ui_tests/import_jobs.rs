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
        Arc::new(move |path, cancel| {
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
            let read = crate::import::read_model(path, &caditor_document::CancelToken::never());
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
    std::thread::sleep(Duration::from_millis(100));
    harness.frame();
    harness.frame();

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

    harness.files.read_models_with(Arc::new(|path, cancel| {
        crate::import::read_model(path, cancel)
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
    std::thread::sleep(Duration::from_millis(100));
    harness.frame();

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
    std::thread::sleep(Duration::from_millis(200));
    harness.frame();
    harness.frame();

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
