use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

use caditor_document::{Document, Editor};
use tempfile::TempDir;

use super::{
    JobGate, WAIT, config, crash, edit_width, only_file_in, sample, save_request, untitled,
};
use crate::{
    Inspection, JournalEntry, Recovered, Report, SaveError, Storage, StorageConfig, inspect, load,
    scan,
};

struct Session {
    dir: TempDir,
    gate: JobGate,
    storage: Storage,
    editor: Editor,
    base: Document,
}

impl Session {
    fn untitled() -> Self {
        let dir = TempDir::new().unwrap();
        let gate = JobGate::default();
        let base = sample();

        let config = StorageConfig {
            jobs: gate.clone(),
            ..config(&dir)
        };
        let storage = Storage::spawn(config, untitled(&base), || {}).unwrap();
        let editor = Editor::new(base.clone());

        Self {
            dir,
            gate,
            storage,
            editor,
            base,
        }
    }

    fn model(&self) -> PathBuf {
        self.dir.path().join("model.caditor")
    }

    fn recovery(&self) -> PathBuf {
        self.dir.path().join("recovery")
    }

    fn change_width(&mut self, text: &str) {
        let change = edit_width(self.editor.document(), text);

        self.editor.apply(change.clone()).unwrap();
        self.storage.record(JournalEntry::Apply(change)).unwrap();
    }

    fn undo(&mut self) {
        let undone = self.editor.next_undo().cloned().unwrap();

        self.editor.undo().unwrap();
        self.storage.record(JournalEntry::Undo(undone)).unwrap();
    }

    fn save(&self, ticket: u64, path: &Path) {
        self.storage
            .save(save_request(ticket, self.editor.document(), path, false))
            .unwrap();
    }

    fn reports(&self, count: usize) -> Vec<Report> {
        let deadline = Instant::now() + WAIT;
        let mut reports = Vec::new();

        while reports.len() < count {
            reports.extend(self.storage.poll().unwrap());

            assert!(Instant::now() < deadline, "only {reports:?} were reported");

            std::thread::yield_now();
        }
        reports
    }

    fn crash(self) -> (TempDir, Vec<Recovered>) {
        let recovery = self.recovery();

        crash(self.storage);

        let recovered = scan(Some(&recovery), &[]);
        (self.dir, recovered)
    }
}

fn saved(report: &Report, expected: u64) -> bool {
    matches!(report, Report::Saved { ticket, .. } if *ticket == expected)
}

#[test]
fn a_crash_flush_during_a_save_writes_the_changes_made_while_it_runs() {
    let mut session = Session::untitled();

    let copied = session.dir.path().join("copied");

    fs::create_dir(&copied).unwrap();
    session.change_width("50 mm");

    session.gate.hold();
    session.save(1, &session.model());
    session.change_width("60 mm");
    session.change_width("70 mm");

    let flushed = session.storage.flusher().flush(WAIT);
    let journal = only_file_in(&session.recovery());
    let crashed = copied.join(journal.file_name().unwrap());

    session.storage.copy_journal(&crashed).unwrap();
    session.gate.release();

    assert!(flushed);

    let Inspection::Recoverable(recovered) = inspect(&crashed).unwrap() else {
        panic!("the journal copied during the save should be recoverable");
    };

    assert_eq!(recovered.changes(), 3);
    assert_eq!(recovered.base, session.base);
    assert_eq!(recovered.editor.document(), session.editor.document());
    assert!(saved(&session.reports(1)[0], 1));
}

#[test]
fn the_journal_rewritten_after_a_save_keeps_the_changes_made_while_it_ran() {
    let mut session = Session::untitled();

    let path = session.model();

    session.change_width("50 mm");

    let at_save = session.editor.document().clone();

    session.gate.hold();
    session.save(1, &path);
    session.change_width("60 mm");
    session.change_width("70 mm");
    session.gate.release();

    let reports = session.reports(1);
    let edited = session.editor.document().clone();
    let (_dir, recovered) = session.crash();

    assert!(saved(&reports[0], 1), "{reports:?}");
    assert_eq!(load(&path).unwrap().document, at_save);
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].file.as_deref(), Some(path.as_path()));
    assert_eq!(recovered[0].base, at_save);
    assert_eq!(recovered[0].changes(), 2);
    assert_eq!(recovered[0].editor.document(), &edited);
    assert!(recovered[0].issues.is_empty(), "{:?}", recovered[0].issues);
}

#[test]
fn an_undo_during_a_save_of_a_change_made_before_it_survives_the_journal_rewrite() {
    let mut session = Session::untitled();

    let path = session.model();

    session.change_width("50 mm");

    session.gate.hold();
    session.save(1, &path);
    session.undo();
    session.gate.release();

    let reports = session.reports(1);
    let base = session.base.clone();
    let (_dir, recovered) = session.crash();

    assert!(saved(&reports[0], 1), "{reports:?}");
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].changes(), 1);
    assert_eq!(recovered[0].editor.document(), &base);
    assert!(recovered[0].issues.is_empty(), "{:?}", recovered[0].issues);
}

#[test]
fn a_save_queued_behind_one_in_flight_leaves_the_journal_only_the_changes_after_it() {
    let mut session = Session::untitled();

    let path = session.model();

    session.gate.hold();
    session.save(1, &path);
    session.change_width("50 mm");

    let at_second_save = session.editor.document().clone();

    session.save(2, &path);
    session.change_width("60 mm");
    session.gate.release();

    let reports = session.reports(2);
    let edited = session.editor.document().clone();
    let (_dir, recovered) = session.crash();

    assert!(saved(&reports[0], 1), "{reports:?}");
    assert!(saved(&reports[1], 2), "{reports:?}");
    assert_eq!(load(&path).unwrap().document, at_second_save);
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].base, at_second_save);
    assert_eq!(recovered[0].changes(), 1);
    assert_eq!(recovered[0].editor.document(), &edited);
}

#[test]
fn a_failed_save_leaves_the_journal_every_change_including_those_made_while_it_ran() {
    let mut session = Session::untitled();

    let unreachable = session.dir.path().join("missing").join("model.caditor");

    session.change_width("50 mm");

    session.gate.hold();
    session.save(1, &unreachable);
    session.change_width("60 mm");
    session.gate.release();

    let reports = session.reports(1);
    let base = session.base.clone();
    let edited = session.editor.document().clone();
    let (_dir, recovered) = session.crash();

    assert!(
        matches!(reports[0], Report::SaveFailed { ticket: 1, .. }),
        "{reports:?}"
    );
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].file, None);
    assert_eq!(recovered[0].base, base);
    assert_eq!(recovered[0].changes(), 2);
    assert_eq!(recovered[0].editor.document(), &edited);
}

#[test]
fn a_save_that_panics_fails_alone_and_the_journal_and_later_saves_carry_on() {
    let mut session = Session::untitled();

    let path = session.model();
    let copied = session.dir.path().join("copied");

    fs::create_dir(&copied).unwrap();
    session.change_width("50 mm");

    session.gate.panic_in_next(1);
    session.save(1, &path);
    session.change_width("60 mm");

    let failed = session.reports(1);
    let flushed = session.storage.flusher().flush(WAIT);
    let journal = only_file_in(&session.recovery());
    let crashed = copied.join(journal.file_name().unwrap());

    session.storage.copy_journal(&crashed).unwrap();

    let before_failure = session.editor.document().clone();

    session.save(2, &path);

    let at_second_save = session.editor.document().clone();

    session.change_width("70 mm");

    let saved_again = session.reports(1);
    let edited = session.editor.document().clone();
    let (_dir, recovered) = session.crash();

    assert!(
        matches!(
            &failed[0],
            Report::SaveFailed {
                ticket: 1,
                error: SaveError::StoppedUnexpectedly,
                ..
            }
        ),
        "{failed:?}"
    );
    assert!(flushed);

    let Inspection::Recoverable(kept) = inspect(&crashed).unwrap() else {
        panic!("the journal after the failed save should be recoverable");
    };

    assert_eq!(kept.changes(), 2);
    assert_eq!(kept.editor.document(), &before_failure);
    assert!(saved(&saved_again[0], 2), "{saved_again:?}");
    assert_eq!(load(&path).unwrap().document, at_second_save);
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].base, at_second_save);
    assert_eq!(recovered[0].changes(), 1);
    assert_eq!(recovered[0].editor.document(), &edited);
}
