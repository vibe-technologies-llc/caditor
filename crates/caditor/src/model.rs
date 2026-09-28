use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use caditor_document::{
    Document, Editor, Evaluation, FeatureId, FeatureState, ModelEvaluator, Outcome,
    ParameterValues, Progress, Recomputer, Transaction,
};
use caditor_file::{
    Closing, Flusher, JournalEntry, Recovered, Report, SaveRequest, Start, Storage, StorageConfig,
};
use caditor_sketch::Sketch;
use parking_lot::Mutex;

use crate::{
    editing::EditingCommand, files::FileCommand, preferences::PreferencesCommand, units::LengthUnit,
};

pub type Waker = Box<dyn Fn() + Send>;
pub type WakerFactory = Box<dyn Fn() -> Waker>;
pub type PanicFlush = Arc<Mutex<Option<Flusher>>>;

pub const UNTITLED: &str = "Untitled";

#[derive(Debug, Clone)]
pub enum Action {
    Apply(Transaction),
    Undo,
    Redo,
    Recompute,
    CancelRecompute,
    DismissNotice,
    Inform(Notice),
    File(FileCommand),
    Editing(EditingCommand),
    Preferences(PreferencesCommand),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecomputeStatus {
    UpToDate,
    Running { since: Instant },
    Cancelled,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    Info,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub kind: NoticeKind,
    pub text: String,
}

impl Notice {
    pub fn info(text: impl Into<String>) -> Self {
        Self {
            kind: NoticeKind::Info,
            text: text.into(),
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            kind: NoticeKind::Error,
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileEvent {
    Saved(PathBuf),
    SaveFailed,
}

pub struct Services {
    pub make_waker: WakerFactory,
    pub storage: StorageConfig,
    pub panic_flush: PanicFlush,
}

struct PendingSave {
    ticket: u64,
    document: Document,
    entries: usize,
}

pub struct Model {
    editor: Editor,
    parameters: ParameterValues,
    evaluation: Evaluation,
    recomputer: Option<Recomputer>,
    services: Services,
    status: RecomputeStatus,
    notice: Option<Notice>,
    revision_offset: u64,
    storage: Option<Storage>,
    path: Option<PathBuf>,
    saved: Document,
    entries: Vec<JournalEntry>,
    keep_original: bool,
    dirty: bool,
    pending_save: Option<PendingSave>,
    next_ticket: u64,
    file_events: Vec<FileEvent>,
    length_unit: LengthUnit,
}

impl Model {
    pub fn new(document: Document, services: Services) -> Self {
        let mut model = Self {
            parameters: ParameterValues::evaluate(&document),
            editor: Editor::new(document.clone()),
            evaluation: Evaluation::default(),
            recomputer: None,
            services,
            status: RecomputeStatus::UpToDate,
            notice: None,
            revision_offset: 0,
            storage: None,
            path: None,
            saved: document,
            entries: Vec::new(),
            keep_original: false,
            dirty: false,
            pending_save: None,
            next_ticket: 0,
            file_events: Vec::new(),
            length_unit: LengthUnit::default(),
        };
        model.start_storage(None, None);
        model.recompute();
        model
    }

    pub fn length_unit(&self) -> LengthUnit {
        self.length_unit
    }

    pub fn set_length_unit(&mut self, unit: LengthUnit) {
        self.length_unit = unit;
    }

    pub fn document(&self) -> &Document {
        self.editor.document()
    }

    pub fn parameters(&self) -> &ParameterValues {
        &self.parameters
    }

    pub fn evaluation(&self) -> &Evaluation {
        &self.evaluation
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.editor.undo_label()
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.editor.redo_label()
    }

    pub fn status(&self) -> RecomputeStatus {
        self.status
    }

    pub fn progress(&self) -> Option<Progress> {
        self.recomputer.as_ref().and_then(Recomputer::progress)
    }

    pub fn notice(&self) -> Option<&Notice> {
        self.notice.as_ref()
    }

    pub fn set_notice(&mut self, notice: Notice) {
        match notice.kind {
            NoticeKind::Info => log::info!("{}", notice.text),
            NoticeKind::Error => log::warn!("{}", notice.text),
        }
        self.notice = Some(notice);
    }

    pub fn revision(&self) -> u64 {
        self.revision_offset + self.editor.revision()
    }

    pub fn session(&self) -> u64 {
        self.revision_offset
    }

    pub fn settled_sketch(&self, feature: FeatureId) -> Option<&Sketch> {
        if self.status != RecomputeStatus::UpToDate {
            return None;
        }
        let status = self.evaluation.feature(feature)?;
        if status.state != FeatureState::UpToDate {
            return None;
        }
        status
            .result
            .as_deref()?
            .sketch()
            .map(|result| &result.geometry)
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn display_name(&self) -> String {
        display_name(self.path())
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn is_saving(&self) -> bool {
        self.pending_save.is_some()
    }

    pub fn take_file_events(&mut self) -> Vec<FileEvent> {
        std::mem::take(&mut self.file_events)
    }

    pub fn perform(&mut self, action: Action) {
        let changed = match action {
            Action::Apply(transaction) => {
                let label = transaction.label().to_owned();
                let entry =
                    (!transaction.is_empty()).then(|| JournalEntry::Apply(transaction.clone()));
                self.editor
                    .apply(transaction)
                    .map(|()| entry)
                    .map_err(|error| format!("{label}: {error}"))
            }
            Action::Undo => {
                let entry = self.editor.next_undo().cloned().map(JournalEntry::Undo);
                self.editor
                    .undo()
                    .map(|undone| undone.and(entry))
                    .map_err(|error| format!("Undo failed: {error}"))
            }
            Action::Redo => {
                let entry = self.editor.next_redo().cloned().map(JournalEntry::Redo);
                self.editor
                    .redo()
                    .map(|redone| redone.and(entry))
                    .map_err(|error| format!("Redo failed: {error}"))
            }
            Action::Recompute => {
                self.recompute();
                Ok(None)
            }
            Action::CancelRecompute => {
                if let Some(recomputer) = &self.recomputer {
                    recomputer.cancel();
                }
                Ok(None)
            }
            Action::DismissNotice => {
                self.notice = None;
                Ok(None)
            }
            Action::Inform(notice) => {
                self.set_notice(notice);
                Ok(None)
            }
            Action::File(command) => {
                log::warn!("{command:?} reached the model instead of the file workflow");
                Ok(None)
            }
            Action::Editing(command) => {
                log::warn!("{command:?} reached the model instead of the sketch editor");
                Ok(None)
            }
            Action::Preferences(command) => {
                log::warn!("{command:?} reached the model instead of the preferences");
                Ok(None)
            }
        };
        match changed {
            Ok(Some(entry)) => {
                self.notice = None;
                self.record(entry);
                self.dirty = *self.editor.document() != self.saved;
                self.parameters = ParameterValues::evaluate(self.editor.document());
                self.recompute();
            }
            Ok(None) => {}
            Err(message) => self.set_notice(Notice::error(message)),
        }
    }

    pub fn poll(&mut self) -> bool {
        let stored = self.poll_storage();
        let Some(recomputer) = &self.recomputer else {
            return stored;
        };
        match recomputer.poll() {
            Ok(Some(update)) if update.revision < self.revision_offset => stored,
            Ok(Some(update)) => {
                if update.revision == self.revision() {
                    self.status = match update.outcome {
                        Outcome::Finished => RecomputeStatus::UpToDate,
                        Outcome::Cancelled => RecomputeStatus::Cancelled,
                    };
                }
                self.evaluation = update.evaluation;
                true
            }
            Ok(None) => stored,
            Err(error) => {
                log::error!("{error}");
                self.recomputer = None;
                self.status = RecomputeStatus::Stopped;
                true
            }
        }
    }

    pub fn save_to(&mut self, path: PathBuf) {
        if self.is_saving() {
            self.set_notice(Notice::info("A save is already in progress."));
            return;
        }
        if self.storage.is_none() {
            self.start_storage(None, None);
        }
        let ticket = self.next_ticket;
        self.next_ticket += 1;
        let document = self.editor.document().clone();
        let request = SaveRequest {
            ticket,
            document: document.clone(),
            keep_original: self.keep_original && self.path.as_ref() == Some(&path),
            label: self.editor.undo_label().map(str::to_owned),
            path,
        };
        let sent = self
            .storage
            .as_ref()
            .is_some_and(|storage| storage.save(request).is_ok());
        if sent {
            self.pending_save = Some(PendingSave {
                ticket,
                document,
                entries: self.entries.len(),
            });
        } else {
            self.storage = None;
            self.set_notice(Notice::error(
                "Could not save, because the background writer stopped. Try saving again.",
            ));
            self.file_events.push(FileEvent::SaveFailed);
        }
    }

    pub fn replace(&mut self, document: Document, path: Option<PathBuf>, damaged: bool) {
        self.switch_to(
            Editor::new(document.clone()),
            document,
            path,
            Vec::new(),
            None,
        );
        self.keep_original = damaged;
    }

    pub fn restore(&mut self, recovered: Recovered) {
        let Recovered {
            journal,
            file,
            base,
            entries,
            editor,
            ..
        } = recovered;
        self.switch_to(editor, base, file, entries, Some(journal));
    }

    pub fn close(&mut self) -> Option<Closing> {
        *self.services.panic_flush.lock() = None;
        self.storage.take().map(|storage| storage.close(true))
    }

    fn switch_to(
        &mut self,
        editor: Editor,
        saved: Document,
        path: Option<PathBuf>,
        entries: Vec<JournalEntry>,
        replaces: Option<PathBuf>,
    ) {
        let predecessor = self.storage.take().map(|storage| storage.close(true));
        self.revision_offset = self.revision() + 1;
        self.editor = editor;
        self.dirty = *self.editor.document() != saved;
        self.saved = saved;
        self.path = path;
        self.entries = entries;
        self.keep_original = false;
        self.pending_save = None;
        self.notice = None;
        self.parameters = ParameterValues::evaluate(self.editor.document());
        self.evaluation = Evaluation::default();
        self.start_storage(replaces, predecessor);
        self.recompute();
    }

    fn start_storage(&mut self, replaces: Option<PathBuf>, after: Option<Closing>) {
        let start = Start {
            file: self.path.clone(),
            base: self.saved.clone(),
            entries: self.entries.clone(),
            replaces,
            after,
        };
        let wake = (self.services.make_waker)();
        match Storage::spawn(self.services.storage.clone(), start, wake) {
            Ok(storage) => {
                *self.services.panic_flush.lock() = Some(storage.flusher());
                self.storage = Some(storage);
            }
            Err(error) => {
                log::error!("could not start the storage worker: {error}");
                *self.services.panic_flush.lock() = None;
                self.set_notice(Notice::error(
                    "Unsaved changes are not protected against a crash, because the background \
                     writer could not start. Save your work often.",
                ));
            }
        }
    }

    fn record(&mut self, entry: JournalEntry) {
        self.entries.push(entry.clone());
        let recorded = self
            .storage
            .as_ref()
            .is_some_and(|storage| storage.record(entry).is_ok());
        if !recorded && self.storage.is_some() {
            self.restart_storage();
        }
    }

    fn restart_storage(&mut self) {
        log::error!("the storage worker stopped; starting a new one");
        self.storage = None;
        if self.pending_save.take().is_some() {
            self.set_notice(Notice::error(
                "The save did not finish, because the background writer stopped. Try saving \
                 again.",
            ));
            self.file_events.push(FileEvent::SaveFailed);
        }
        self.start_storage(None, None);
    }

    fn poll_storage(&mut self) -> bool {
        let Some(storage) = &self.storage else {
            return false;
        };
        let reports = match storage.poll() {
            Ok(reports) => reports,
            Err(_) => {
                self.restart_storage();
                return true;
            }
        };
        let any = !reports.is_empty();
        for report in reports {
            self.handle_report(report);
        }
        any
    }

    fn handle_report(&mut self, report: Report) {
        match report {
            Report::Saved {
                ticket,
                path,
                backup,
            } => {
                if let Some(pending) = self
                    .pending_save
                    .take_if(|pending| pending.ticket == ticket)
                {
                    self.saved = pending.document;
                    self.entries
                        .drain(..pending.entries.min(self.entries.len()));
                }
                self.path = Some(path.clone());
                self.keep_original = false;
                self.dirty = *self.editor.document() != self.saved;
                if let Some(backup) = backup {
                    self.set_notice(Notice::info(format!(
                        "Saved. The damaged original was kept as “{}”.",
                        display_name(Some(&backup))
                    )));
                }
                self.file_events.push(FileEvent::Saved(path));
            }
            Report::SaveFailed {
                ticket,
                path,
                reason,
            } => {
                self.pending_save
                    .take_if(|pending| pending.ticket == ticket);
                self.set_notice(Notice::error(format!(
                    "Could not save “{}”: {reason}. Use Save As to choose another location.",
                    display_name(Some(&path))
                )));
                self.file_events.push(FileEvent::SaveFailed);
            }
            Report::JournalFailed { reason } => self.set_notice(Notice::error(format!(
                "Unsaved changes are not protected against a crash: {reason}. Save your work to \
                 keep it safe."
            ))),
        }
    }

    fn recompute(&mut self) {
        if self.recomputer.is_none() {
            match Recomputer::spawn(ModelEvaluator, (self.services.make_waker)()) {
                Ok(recomputer) => self.recomputer = Some(recomputer),
                Err(error) => {
                    log::error!("could not start the recompute worker: {error}");
                    self.status = RecomputeStatus::Stopped;
                    return;
                }
            }
        }
        let document = self.editor.document().clone();
        let revision = self.revision();
        let submitted = self
            .recomputer
            .as_mut()
            .map(|recomputer| recomputer.submit(document, revision));
        self.status = match submitted {
            Some(Ok(())) => RecomputeStatus::Running {
                since: Instant::now(),
            },
            Some(Err(error)) => {
                log::error!("{error}");
                self.recomputer = None;
                RecomputeStatus::Stopped
            }
            None => RecomputeStatus::Stopped,
        };
    }
}

pub fn display_name(path: Option<&Path>) -> String {
    path.and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| UNTITLED.to_owned())
}
