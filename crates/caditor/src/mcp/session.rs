use std::{
    cell::Cell,
    io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    time::Duration,
};

use caditor_document::{
    CancelToken, Document, EditError, Editor, Evaluation, ModelEvaluator, ParameterValues,
    Recompute, Transaction,
};
use caditor_file::{
    FILE_EXTENSION, FileDigest, FileJournal, JournalEntry, JournalFailure, LoadError, Report,
    Retention, SaveError, SaveRequest, Start, Storage, StorageConfig,
};
use caditor_kernel::Interrupt;

use crate::model::{PanicFlush, display_name};

const STORAGE_POLL: Duration = Duration::from_millis(50);

pub struct Work<'a> {
    cancelled: Arc<AtomicBool>,
    report: &'a dyn Fn(&str),
}

impl<'a> Work<'a> {
    pub fn new(cancelled: Arc<AtomicBool>, report: &'a dyn Fn(&str)) -> Self {
        Self { cancelled, report }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn cancel_token(&self) -> CancelToken {
        let cancelled = Arc::clone(&self.cancelled);
        CancelToken::new(move || cancelled.load(Ordering::Acquire))
    }

    pub fn interrupt(&self) -> Interrupt {
        self.cancel_token().interrupt()
    }

    pub fn progress(&self, message: &str) {
        (self.report)(message);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the call was cancelled")]
pub struct Cancelled;

#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("“{file}” is not a .caditor model; give the server a model file, ending in .caditor")]
    NotAModelFile { file: String },
    #[error("the folder of “{file}” does not exist")]
    NoFolder { file: String },
    #[error("the folder “{folder}” cannot be used: {source}")]
    UnusableFolder {
        folder: String,
        #[source]
        source: io::Error,
    },
    #[error("“{folder}” is not a folder")]
    NotAFolder { folder: String },
    #[error("could not open “{file}”: {source}")]
    Unreadable {
        file: String,
        #[source]
        source: LoadError,
    },
    #[error(
        "“{file}” is open in a caditor window; close it there first, since two programs \
         changing one model would lose the work of one"
    )]
    InUseElsewhere { file: String },
    #[error(
        "“{file}” has unsaved changes from an earlier session in its recovery journal; open it \
         in caditor to restore or discard them first"
    )]
    Unrecovered { file: String },
}

#[derive(Debug, thiserror::Error)]
pub enum ChangeError {
    #[error("{label} was not applied: {source}")]
    Refused {
        label: String,
        #[source]
        source: Box<EditError>,
    },
    #[error("there is nothing to undo")]
    NothingToUndo,
    #[error("there is nothing to redo")]
    NothingToRedo,
    #[error("undo failed: {0}")]
    Undo(#[source] Box<EditError>),
    #[error("redo failed: {0}")]
    Redo(#[source] Box<EditError>),
}

#[derive(Debug, thiserror::Error)]
pub enum SaveFailure {
    #[error(
        "“{file}” was changed by another program since it was opened; nothing was saved, so \
         neither change is lost"
    )]
    ChangedOnDisk { file: String },
    #[error("could not save “{file}”: {source}")]
    Writing {
        file: String,
        #[source]
        source: SaveError,
    },
    #[error("the save did not finish, because the background writer stopped; save again")]
    WriterStopped,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Apply(Transaction),
    Undo,
    Redo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Performed {
    pub label: String,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    pub backup: Option<PathBuf>,
    pub dropped_for_size: usize,
}

pub struct Opening {
    pub model: PathBuf,
    pub folders: Vec<PathBuf>,
    pub storage: StorageConfig,
    pub panic_flush: PanicFlush,
    pub retention: Retention,
}

struct PendingSave {
    ticket: u64,
    document: Document,
    entries: usize,
}

enum Settled {
    Saved(Stored),
    Failed(SaveFailure),
}

struct Journal {
    config: StorageConfig,
    storage: Option<Storage>,
    wakes: Receiver<()>,
    waker: Sender<()>,
    panic_flush: PanicFlush,
    base: Document,
    folded: usize,
    entries: Vec<JournalEntry>,
    unprotected: Option<JournalFailure>,
    next_ticket: u64,
    pending: Option<PendingSave>,
}

pub struct Session {
    model: PathBuf,
    folders: Vec<PathBuf>,
    editor: Editor,
    saved: Option<Document>,
    on_disk: Option<FileDigest>,
    keep_original: bool,
    load_issues: Vec<String>,
    retention: Retention,
    recompute: Recompute,
    evaluation: Evaluation,
    evaluated: Option<u64>,
    journal: Journal,
}

impl Session {
    pub fn open(opening: Opening) -> Result<Self, OpenError> {
        let Opening {
            model,
            folders,
            storage,
            panic_flush,
            retention,
        } = opening;
        let file = display_name(Some(&model));
        if !has_model_extension(&model) {
            return Err(OpenError::NotAModelFile { file });
        }
        let model = placed(&model).ok_or_else(|| OpenError::NoFolder { file: file.clone() })?;
        let folders = allowed_folders(&model, &folders)?;
        match caditor_file::journal_for(&model, storage.recovery_dir.as_deref()) {
            FileJournal::Recoverable(_) => return Err(OpenError::Unrecovered { file }),
            FileJournal::InUse => return Err(OpenError::InUseElsewhere { file }),
            FileJournal::SetAside(kept) => {
                for journal in kept {
                    log::warn!(
                        "a damaged recovery journal of {} was set aside at {}",
                        model.display(),
                        journal.display()
                    );
                }
            }
            FileJournal::None => {}
        }
        let (document, saved, on_disk, load_issues) = if model.exists() {
            let loaded = caditor_file::load(&model).map_err(|source| OpenError::Unreadable {
                file: file.clone(),
                source,
            })?;
            (
                loaded.document.clone(),
                Some(loaded.document),
                loaded.digest,
                loaded.issues,
            )
        } else {
            (Document::default(), None, None, Vec::new())
        };
        let (waker, wakes) = mpsc::channel();
        let mut session = Self {
            keep_original: !load_issues.is_empty(),
            model,
            folders,
            editor: Editor::new(document.clone()),
            saved,
            on_disk,
            load_issues,
            retention,
            recompute: Recompute::default(),
            evaluation: Evaluation::default(),
            evaluated: None,
            journal: Journal {
                config: storage,
                storage: None,
                wakes,
                waker,
                panic_flush,
                base: document,
                folded: 0,
                entries: Vec::new(),
                unprotected: None,
                next_ticket: 0,
                pending: None,
            },
        };
        session.start_storage();
        Ok(session)
    }

    pub fn model(&self) -> &Path {
        &self.model
    }

    pub fn folders(&self) -> &[PathBuf] {
        &self.folders
    }

    pub fn document(&self) -> &Document {
        self.editor.document()
    }

    pub fn evaluation(&self) -> &Evaluation {
        &self.evaluation
    }

    pub fn parameters(&self) -> ParameterValues {
        ParameterValues::evaluate(self.editor.document())
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.editor.undo_label()
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.editor.redo_label()
    }

    pub fn load_issues(&self) -> &[String] {
        &self.load_issues
    }

    pub fn is_unsaved(&self) -> bool {
        self.saved
            .as_ref()
            .is_none_or(|saved| !self.editor.document().same_content(saved))
    }

    pub fn unprotected(&mut self) -> Option<&JournalFailure> {
        self.poll_storage();
        self.journal.unprotected.as_ref()
    }

    pub fn forget_evaluation(&mut self) {
        self.evaluated = None;
    }

    pub fn is_evaluated(&self) -> bool {
        self.evaluated == Some(self.editor.revision())
    }

    pub fn evaluate(&mut self, work: &Work<'_>) -> Result<&Evaluation, Cancelled> {
        if self.is_evaluated() {
            return Ok(&self.evaluation);
        }
        let cancel = work.cancel_token();
        let last = Cell::new(None);
        let progress = |done: usize, total: usize| {
            if last.replace(Some(done)) != Some(done) {
                work.progress(&format!("Recomputing: {done} of {total} features"));
            }
        };
        self.evaluation =
            self.recompute
                .run(self.editor.document(), &ModelEvaluator, &cancel, &progress);
        if work.is_cancelled() {
            return Err(Cancelled);
        }
        self.editor.follow_readings(&self.evaluation);
        self.evaluated = Some(self.editor.revision());
        Ok(&self.evaluation)
    }

    pub fn perform(&mut self, change: Change) -> Result<Performed, ChangeError> {
        self.poll_storage();
        let performed = match change {
            Change::Apply(transaction) => {
                let label = transaction.label().to_owned();
                let entry = JournalEntry::Apply(transaction.clone());
                let changed =
                    self.editor
                        .apply(transaction)
                        .map_err(|source| ChangeError::Refused {
                            label: label.clone(),
                            source: Box::new(source),
                        })?;
                if changed {
                    self.record(entry);
                }
                Performed { label, changed }
            }
            Change::Undo => {
                let entry = self.editor.next_undo().cloned().map(JournalEntry::Undo);
                let label = self
                    .editor
                    .undo()
                    .map_err(|error| ChangeError::Undo(Box::new(error)))?
                    .ok_or(ChangeError::NothingToUndo)?;
                if let Some(entry) = entry {
                    self.record(entry);
                }
                Performed {
                    label,
                    changed: true,
                }
            }
            Change::Redo => {
                let entry = self.editor.next_redo().cloned().map(JournalEntry::Redo);
                let label = self
                    .editor
                    .redo()
                    .map_err(|error| ChangeError::Redo(Box::new(error)))?
                    .ok_or(ChangeError::NothingToRedo)?;
                if let Some(entry) = entry {
                    self.record(entry);
                }
                Performed {
                    label,
                    changed: true,
                }
            }
        };
        Ok(performed)
    }

    pub fn save(&mut self) -> Result<Stored, SaveFailure> {
        self.poll_storage();
        if self.journal.storage.is_none() {
            self.start_storage();
        }
        let ticket = self.journal.next_ticket;
        self.journal.next_ticket += 1;
        let document = self.editor.document().clone();
        let request = SaveRequest {
            ticket,
            document: document.clone(),
            path: self.model.clone(),
            keep_original: self.keep_original,
            label: self.editor.undo_label().map(str::to_owned),
            replace_outside_changes: false,
            retention: self.retention,
        };
        let sent = self
            .journal
            .storage
            .as_ref()
            .is_some_and(|storage| storage.save(request).is_ok());
        if !sent {
            self.restart_storage();
            return Err(SaveFailure::WriterStopped);
        }
        self.journal.pending = Some(PendingSave {
            ticket,
            document,
            entries: self.journal.entries.len(),
        });
        loop {
            let _ = self.journal.wakes.recv_timeout(STORAGE_POLL);
            let reports = match self.journal.storage.as_ref().map(Storage::poll) {
                Some(Ok(reports)) => reports,
                Some(Err(_)) | None => {
                    self.journal.pending = None;
                    self.restart_storage();
                    return Err(SaveFailure::WriterStopped);
                }
            };
            let mut settled = None;
            for report in reports {
                if let Some(outcome) = self.handle_report(report, ticket) {
                    settled = Some(outcome);
                }
            }
            match settled {
                Some(Settled::Saved(stored)) => return Ok(stored),
                Some(Settled::Failed(failure)) => return Err(failure),
                None => {}
            }
        }
    }

    pub fn close(mut self) -> Option<caditor_file::Closing> {
        self.poll_storage();
        let discard = !self.is_unsaved();
        *self.journal.panic_flush.lock() = None;
        self.journal
            .storage
            .take()
            .map(|storage| storage.close(discard))
    }

    fn record(&mut self, entry: JournalEntry) {
        self.journal.entries.push(entry.clone());
        let recorded = self
            .journal
            .storage
            .as_ref()
            .is_some_and(|storage| storage.record(entry).is_ok());
        if !recorded {
            self.restart_storage();
        }
    }

    fn start_storage(&mut self) {
        let waker = self.journal.waker.clone();
        let start = Start {
            file: Some(self.model.clone()),
            on_disk: self.on_disk.clone(),
            loaded_with_problems: self.keep_original,
            base: self.journal.base.clone(),
            folded: self.journal.folded,
            entries: self.journal.entries.clone(),
            replaces: None,
            after: None,
        };
        let wake = move || {
            let _ = waker.send(());
        };
        match Storage::spawn(self.journal.config.clone(), start, wake) {
            Ok(storage) => {
                *self.journal.panic_flush.lock() = Some(storage.flusher());
                self.journal.storage = Some(storage);
            }
            Err(error) => {
                log::error!("could not start the storage worker: {error}");
                *self.journal.panic_flush.lock() = None;
                self.journal.unprotected = Some(JournalFailure::WriterUnavailable);
            }
        }
    }

    fn restart_storage(&mut self) {
        log::error!("the storage worker stopped; starting a new one");
        self.journal.storage = None;
        self.journal.unprotected = None;
        self.start_storage();
    }

    fn poll_storage(&mut self) {
        let Some(storage) = &self.journal.storage else {
            return;
        };
        match storage.poll() {
            Ok(reports) => {
                for report in reports {
                    self.handle_report(report, u64::MAX);
                }
            }
            Err(_) => self.restart_storage(),
        }
    }

    fn handle_report(&mut self, report: Report, waited: u64) -> Option<Settled> {
        let file = display_name(Some(&self.model));
        match report {
            Report::Saved {
                ticket,
                backup,
                dropped_for_size,
                digest,
                ..
            } => {
                let pending = self
                    .journal
                    .pending
                    .take_if(|pending| pending.ticket == ticket)?;
                self.journal.base = pending.document.clone();
                self.journal.folded = 0;
                self.journal
                    .entries
                    .drain(..pending.entries.min(self.journal.entries.len()));
                self.saved = Some(pending.document);
                self.on_disk = Some(digest);
                self.keep_original = false;
                (ticket == waited).then_some(Settled::Saved(Stored {
                    backup,
                    dropped_for_size,
                }))
            }
            Report::ChangedOnDisk { ticket, .. } => {
                self.journal
                    .pending
                    .take_if(|pending| pending.ticket == ticket);
                (ticket == waited).then_some(Settled::Failed(SaveFailure::ChangedOnDisk { file }))
            }
            Report::SaveFailed { ticket, error, .. } => {
                self.journal
                    .pending
                    .take_if(|pending| pending.ticket == ticket);
                (ticket == waited).then_some(Settled::Failed(SaveFailure::Writing {
                    file,
                    source: error,
                }))
            }
            Report::JournalFailed { failure } => {
                log::warn!(
                    "unsaved changes to {file} are not protected against a crash: {failure}"
                );
                self.journal.unprotected = Some(failure);
                None
            }
            Report::JournalRestored => {
                self.journal.unprotected = None;
                None
            }
            Report::Rebased { entries, base } => {
                self.journal.base = base;
                self.journal.folded = self.journal.folded.saturating_add(entries);
                self.journal
                    .entries
                    .drain(..entries.min(self.journal.entries.len()));
                if let Some(pending) = &mut self.journal.pending {
                    pending.entries = pending.entries.saturating_sub(entries);
                }
                None
            }
            Report::VersionKept { .. } | Report::KeepFailed { .. } => None,
        }
    }
}

fn placed(model: &Path) -> Option<PathBuf> {
    let name = model.file_name()?;
    let folder = match model.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let folder = dunce::canonicalize(folder).ok()?;
    folder.is_dir().then(|| folder.join(name))
}

fn allowed_folders(model: &Path, asked: &[PathBuf]) -> Result<Vec<PathBuf>, OpenError> {
    let own = model.parent().map(Path::to_path_buf).into_iter();
    let mut folders: Vec<PathBuf> = own.collect();
    for folder in asked {
        let shown = folder.display().to_string();
        let found = dunce::canonicalize(folder).map_err(|source| OpenError::UnusableFolder {
            folder: shown.clone(),
            source,
        })?;
        if !found.is_dir() {
            return Err(OpenError::NotAFolder { folder: shown });
        }
        if !folders.contains(&found) {
            folders.push(found);
        }
    }
    Ok(folders)
}

pub fn has_model_extension(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(FILE_EXTENSION))
}
