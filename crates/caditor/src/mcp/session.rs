use std::{
    cell::Cell,
    collections::BTreeSet,
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
    CancelToken, Document, EditError, Editor, Evaluation, FeatureId, ModelEvaluator,
    ParameterValues, Recompute, Transaction,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
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
    #[error("the model has no file yet; use save_as with a .caditor path in an allowed folder")]
    NoFilePath,
}

pub type SaveHook = Box<dyn FnMut(Option<PathBuf>) -> Result<Stored, SaveFailure> + Send>;

pub struct AttachedConfig {
    pub document: Document,
    pub evaluation: Evaluation,
    pub model: Option<PathBuf>,
    pub folders: Vec<PathBuf>,
    pub exported: BTreeSet<PathBuf>,
    pub undo_label: Option<String>,
    pub redo_label: Option<String>,
    pub is_unsaved: bool,
    pub load_issues: Vec<String>,
    pub on_save: Option<SaveHook>,
}

struct AttachedBacking {
    has_path: bool,
    recorded: Vec<Change>,
    undo_label: Option<String>,
    redo_label: Option<String>,
    unsaved: bool,
    on_save: Option<SaveHook>,
}

enum Backing {
    Headless(Box<Journal>),
    Attached(AttachedBacking),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    Apply(Transaction),
    Undo,
    Redo,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Performed {
    pub label: String,
    pub changed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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

struct Standing {
    document: Document,
    evaluation: Evaluation,
}

pub struct Session {
    model: PathBuf,
    folders: Vec<PathBuf>,
    exported: BTreeSet<PathBuf>,
    editor: Editor,
    saved: Option<Document>,
    on_disk: Option<FileDigest>,
    keep_original: bool,
    load_issues: Vec<String>,
    retention: Retention,
    recompute: Recompute,
    evaluation: Evaluation,
    evaluated: Option<u64>,
    before: Option<Standing>,
    backing: Backing,
}

impl Session {
    pub fn attached(config: AttachedConfig) -> Self {
        let AttachedConfig {
            document,
            evaluation,
            model,
            folders,
            exported,
            undo_label,
            redo_label,
            is_unsaved,
            load_issues,
            on_save,
        } = config;
        let has_path = model.is_some();
        let model_path = model.unwrap_or_else(|| PathBuf::from("Untitled.caditor"));
        let mut allowed = Vec::new();

        if has_path
            && let Some(parent) = model_path.parent()
            && let Ok(canonical) = dunce::canonicalize(parent)
            && canonical.is_dir()
        {
            allowed.push(canonical);
        }
        for folder in folders {
            if let Ok(canonical) = dunce::canonicalize(&folder)
                && canonical.is_dir()
                && !allowed.contains(&canonical)
            {
                allowed.push(canonical);
            }
        }

        let editor = Editor::new(document);
        let revision = editor.revision();
        Self {
            model: model_path,
            folders: allowed,
            exported,
            editor,
            saved: None,
            on_disk: None,
            keep_original: false,
            load_issues,
            retention: Retention::default(),
            recompute: Recompute::default(),
            evaluation,
            evaluated: Some(revision),
            before: None,
            backing: Backing::Attached(AttachedBacking {
                has_path,
                recorded: Vec::new(),
                undo_label,
                redo_label,
                unsaved: is_unsaved,
                on_save,
            }),
        }
    }

    pub fn take_changes(&mut self) -> Vec<Change> {
        match &mut self.backing {
            Backing::Headless(_) => Vec::new(),
            Backing::Attached(attached) => std::mem::take(&mut attached.recorded),
        }
    }

    pub fn exported_paths(&self) -> &BTreeSet<PathBuf> {
        &self.exported
    }

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
            exported: BTreeSet::new(),
            editor: Editor::new(document.clone()),
            saved,
            on_disk,
            load_issues,
            retention,
            recompute: Recompute::default(),
            evaluation: Evaluation::default(),
            evaluated: None,
            before: None,
            backing: Backing::Headless(Box::new(Journal {
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
            })),
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

    pub fn has_exported(&self, path: &Path) -> bool {
        self.exported.contains(path)
    }

    pub fn exported(&mut self, path: PathBuf) {
        self.exported.insert(path);
    }

    pub fn document(&self) -> &Document {
        self.before
            .as_ref()
            .map_or_else(|| self.editor.document(), |standing| &standing.document)
    }

    pub fn evaluation(&self) -> &Evaluation {
        self.before
            .as_ref()
            .map_or(&self.evaluation, |standing| &standing.evaluation)
    }

    pub fn looking_before<T>(
        &mut self,
        feature: FeatureId,
        run: impl FnOnce(&mut Self) -> T,
    ) -> Option<T> {
        if !self.is_evaluated() || self.before.is_some() {
            return None;
        }

        let document = self.editor.document();
        let standing = Standing {
            evaluation: self
                .evaluation
                .standing_before(document.feature(feature)?)?,
            document: document.rolled_back_before(feature),
        };

        self.before = Some(standing);

        let result = run(self);

        self.before = None;
        Some(result)
    }

    pub fn parameters(&self) -> ParameterValues {
        ParameterValues::evaluate(self.editor.document())
    }

    pub fn undo_label(&self) -> Option<&str> {
        match &self.backing {
            Backing::Headless(_) => self.editor.undo_label(),
            Backing::Attached(attached) => {
                self.editor.undo_label().or(attached.undo_label.as_deref())
            }
        }
    }

    pub fn redo_label(&self) -> Option<&str> {
        match &self.backing {
            Backing::Headless(_) => self.editor.redo_label(),
            Backing::Attached(attached) => {
                self.editor.redo_label().or(attached.redo_label.as_deref())
            }
        }
    }

    pub fn load_issues(&self) -> &[String] {
        &self.load_issues
    }

    pub fn is_unsaved(&self) -> bool {
        match &self.backing {
            Backing::Headless(_) => self
                .saved
                .as_ref()
                .is_none_or(|saved| !self.editor.document().same_content(saved)),
            Backing::Attached(attached) => attached.unsaved || !attached.recorded.is_empty(),
        }
    }

    pub fn unprotected(&mut self) -> Option<&JournalFailure> {
        self.poll_storage();
        match &self.backing {
            Backing::Headless(journal) => journal.unprotected.as_ref(),
            Backing::Attached(_) => None,
        }
    }

    pub fn forget_evaluation(&mut self) {
        self.before = None;
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
        if let Backing::Attached(attached) = &mut self.backing {
            return match change {
                Change::Apply(transaction) => {
                    let label = transaction.label().to_owned();
                    let changed = self.editor.apply(transaction.clone()).map_err(|source| {
                        ChangeError::Refused {
                            label: label.clone(),
                            source: Box::new(source),
                        }
                    })?;

                    if changed {
                        attached.recorded.push(Change::Apply(transaction));
                        attached.unsaved = true;
                    }
                    Ok(Performed { label, changed })
                }
                Change::Undo => {
                    let label = match self.editor.undo() {
                        Ok(Some(label)) => label,
                        Ok(None) => attached
                            .undo_label
                            .take()
                            .ok_or(ChangeError::NothingToUndo)?,
                        Err(error) => return Err(ChangeError::Undo(Box::new(error))),
                    };

                    attached.recorded.push(Change::Undo);
                    attached.unsaved = true;
                    Ok(Performed {
                        label,
                        changed: true,
                    })
                }
                Change::Redo => {
                    let label = match self.editor.redo() {
                        Ok(Some(label)) => label,
                        Ok(None) => attached
                            .redo_label
                            .take()
                            .ok_or(ChangeError::NothingToRedo)?,
                        Err(error) => return Err(ChangeError::Redo(Box::new(error))),
                    };

                    attached.recorded.push(Change::Redo);
                    attached.unsaved = true;
                    Ok(Performed {
                        label,
                        changed: true,
                    })
                }
            };
        }

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
        if let Backing::Attached(attached) = &mut self.backing {
            if !attached.has_path {
                return Err(SaveFailure::NoFilePath);
            }

            let Some(hook) = attached.on_save.as_mut() else {
                return Err(SaveFailure::WriterStopped);
            };
            let stored = hook(None)?;

            attached.unsaved = false;
            return Ok(stored);
        }
        self.save_to(self.model.clone())
    }

    pub fn save_to(&mut self, path: PathBuf) -> Result<Stored, SaveFailure> {
        if let Backing::Attached(attached) = &mut self.backing {
            let Some(hook) = attached.on_save.as_mut() else {
                return Err(SaveFailure::WriterStopped);
            };
            let stored = hook(Some(path.clone()))?;

            self.model = path;
            attached.has_path = true;
            attached.unsaved = false;
            return Ok(stored);
        }
        self.poll_storage();

        let needs_start =
            matches!(&self.backing, Backing::Headless(journal) if journal.storage.is_none());

        if needs_start {
            self.start_storage();
        }

        let document = self.editor.document().clone();
        let (ticket, request) = {
            let Backing::Headless(journal) = &mut self.backing else {
                return Err(SaveFailure::WriterStopped);
            };
            let ticket = journal.next_ticket;

            journal.next_ticket += 1;

            let request = SaveRequest {
                ticket,
                document: document.clone(),
                keep_original: self.keep_original && path == self.model,
                path,
                label: self.editor.undo_label().map(str::to_owned),
                replace_outside_changes: false,
                retention: self.retention,
                preview: None,
            };
            (ticket, request)
        };
        let sent = match &self.backing {
            Backing::Headless(journal) => journal
                .storage
                .as_ref()
                .is_some_and(|storage| storage.save(request).is_ok()),
            Backing::Attached(_) => false,
        };

        if !sent {
            self.restart_storage();
            return Err(SaveFailure::WriterStopped);
        }
        if let Backing::Headless(journal) = &mut self.backing {
            journal.pending = Some(PendingSave {
                ticket,
                document,
                entries: journal.entries.len(),
            });
        }
        loop {
            let reports = {
                let Backing::Headless(journal) = &mut self.backing else {
                    return Err(SaveFailure::WriterStopped);
                };
                let _ = journal.wakes.recv_timeout(STORAGE_POLL);
                journal.storage.as_ref().map(Storage::poll)
            };
            let reports = match reports {
                Some(Ok(reports)) => reports,
                Some(Err(_)) | None => {
                    if let Backing::Headless(journal) = &mut self.backing {
                        journal.pending = None;
                    }
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
        match self.backing {
            Backing::Headless(mut journal) => {
                *journal.panic_flush.lock() = None;
                journal.storage.take().map(|storage| storage.close(discard))
            }
            Backing::Attached(_) => None,
        }
    }

    fn record(&mut self, entry: JournalEntry) {
        let recorded = match &mut self.backing {
            Backing::Headless(journal) => {
                journal.entries.push(entry.clone());
                journal
                    .storage
                    .as_ref()
                    .is_some_and(|storage| storage.record(entry).is_ok())
            }
            Backing::Attached(_) => return,
        };
        if !recorded {
            self.restart_storage();
        }
    }

    fn start_storage(&mut self) {
        let Backing::Headless(journal) = &mut self.backing else {
            return;
        };
        let waker = journal.waker.clone();
        let start = Start {
            file: Some(self.model.clone()),
            on_disk: self.on_disk.clone(),
            loaded_with_problems: self.keep_original,
            base: journal.base.clone(),
            folded: journal.folded,
            entries: journal.entries.clone(),
            replaces: None,
            after: None,
        };
        let wake = move || {
            let _ = waker.send(());
        };
        match Storage::spawn(journal.config.clone(), start, wake) {
            Ok(storage) => {
                *journal.panic_flush.lock() = Some(storage.flusher());
                journal.storage = Some(storage);
            }
            Err(error) => {
                log::error!("could not start the storage worker: {error}");
                *journal.panic_flush.lock() = None;
                journal.unprotected = Some(JournalFailure::WriterUnavailable);
            }
        }
    }

    fn restart_storage(&mut self) {
        log::error!("the storage worker stopped; starting a new one");
        if let Backing::Headless(journal) = &mut self.backing {
            journal.storage = None;
            journal.unprotected = None;
        }
        self.start_storage();
    }

    fn poll_storage(&mut self) {
        let polled = match &self.backing {
            Backing::Headless(journal) => journal.storage.as_ref().map(Storage::poll),
            Backing::Attached(_) => return,
        };
        let Some(result) = polled else {
            return;
        };
        match result {
            Ok(reports) => {
                for report in reports {
                    self.handle_report(report, u64::MAX);
                }
            }
            Err(_) => self.restart_storage(),
        }
    }

    fn handle_report(&mut self, report: Report, waited: u64) -> Option<Settled> {
        let Backing::Headless(journal) = &mut self.backing else {
            return None;
        };
        match report {
            Report::Saved {
                ticket,
                path,
                backup,
                dropped_for_size,
                digest,
                ..
            } => {
                let pending = journal
                    .pending
                    .take_if(|pending| pending.ticket == ticket)?;

                journal.base = pending.document.clone();
                journal.folded = 0;
                journal
                    .entries
                    .drain(..pending.entries.min(journal.entries.len()));
                self.saved = Some(pending.document);
                self.model = path;
                self.on_disk = Some(digest);
                self.keep_original = false;
                (ticket == waited).then_some(Settled::Saved(Stored {
                    backup,
                    dropped_for_size,
                }))
            }
            Report::ChangedOnDisk { ticket, path } => {
                journal.pending.take_if(|pending| pending.ticket == ticket);
                (ticket == waited).then(|| {
                    Settled::Failed(SaveFailure::ChangedOnDisk {
                        file: display_name(Some(&path)),
                    })
                })
            }
            Report::SaveFailed {
                ticket,
                path,
                error,
            } => {
                journal.pending.take_if(|pending| pending.ticket == ticket);
                (ticket == waited).then(|| {
                    Settled::Failed(SaveFailure::Writing {
                        file: display_name(Some(&path)),
                        source: error,
                    })
                })
            }
            Report::JournalFailed { failure } => {
                let file = display_name(Some(&self.model));

                log::warn!(
                    "unsaved changes to {file} are not protected against a crash: {failure}"
                );
                journal.unprotected = Some(failure);
                None
            }
            Report::JournalRestored => {
                journal.unprotected = None;
                None
            }
            Report::Rebased { entries, base } => {
                journal.base = base;
                journal.folded = journal.folded.saturating_add(entries);
                journal.entries.drain(..entries.min(journal.entries.len()));
                if let Some(pending) = &mut journal.pending {
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
