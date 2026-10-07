use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Instant, SystemTime},
};

use caditor_document::{
    Base, Document, Editor, Evaluation, Feature, FeatureId, FeatureResult, FeatureState,
    ModelEvaluator, Outcome, ParameterValues, Prepared, Progress, Recomputer, SketchResult, Stale,
    Transaction,
};
use caditor_file::{
    Closing, FileDigest, Flusher, JournalEntry, JournalFailure, Recovered, Report, SaveRequest,
    Start, Storage, StorageConfig,
};
use caditor_kernel::MeshQuality;
use caditor_sketch::{Sketch, SketchSolution};
use parking_lot::Mutex;

use crate::{
    display::{Display, Displayed},
    drag_solver::{self, DragCommand, Finished, Polled},
    editing::EditingCommand,
    files::FileCommand,
    preferences::PreferencesCommand,
    selection::SelectionFilter,
    units::{AngleUnit, LengthUnit, Units},
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
    Drag(DragCommand),
    File(FileCommand),
    Editing(EditingCommand),
    Preferences(PreferencesCommand),
    Filter(SelectionFilter),
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
pub struct RecordedNotice {
    pub notice: Notice,
    pub at: SystemTime,
}

const NAMED_CONFLICTS: usize = 2;
const MAX_RECORDED_NOTICES: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub kind: NoticeKind,
    pub text: String,
    pub outlasts_edits: bool,
}

impl Notice {
    pub fn info(text: impl Into<String>) -> Self {
        Self {
            kind: NoticeKind::Info,
            text: text.into(),
            outlasts_edits: false,
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            kind: NoticeKind::Error,
            text: text.into(),
            outlasts_edits: false,
        }
    }

    pub fn failure(text: impl Into<String>) -> Self {
        Self {
            kind: NoticeKind::Error,
            text: text.into(),
            outlasts_edits: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileEvent {
    Saved(PathBuf),
    SaveFailed,
    ChangedOnDisk(PathBuf),
}

pub struct Services {
    pub make_waker: WakerFactory,
    pub storage: StorageConfig,
    pub panic_flush: PanicFlush,
}

#[derive(Debug, Clone)]
pub struct SessionBase {
    pub base: Base,
    pub session: u64,
}

#[derive(Clone, Copy)]
enum Retry {
    Nothing,
    Failures,
}

struct Session {
    editor: Editor,
    saved: Option<Document>,
    journal_base: Document,
    folded: usize,
    path: Option<PathBuf>,
    on_disk: Option<FileDigest>,
    keep_original: bool,
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
    drag_blocked: bool,
    recorded_notices: VecDeque<RecordedNotice>,
    revision_offset: u64,
    storage: Option<Storage>,
    path: Option<PathBuf>,
    on_disk: Option<FileDigest>,
    saved: Option<Document>,
    journal_base: Document,
    folded: usize,
    entries: Vec<JournalEntry>,
    keep_original: bool,
    unprotected: Option<JournalFailure>,
    dirty: bool,
    pending_save: Option<PendingSave>,
    next_ticket: u64,
    file_events: Vec<FileEvent>,
    length_unit: LengthUnit,
    angle_unit: AngleUnit,
    mesh_quality: MeshQuality,
    mesh_requested: Vec<Arc<FeatureResult>>,
    display: Display,
    shown_before: Vec<Arc<FeatureResult>>,
    evaluation_generation: u64,
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
            drag_blocked: false,
            recorded_notices: VecDeque::new(),
            revision_offset: 0,
            storage: None,
            path: None,
            on_disk: None,
            saved: Some(document.clone()),
            journal_base: document,
            folded: 0,
            entries: Vec::new(),
            keep_original: false,
            unprotected: None,
            dirty: false,
            pending_save: None,
            next_ticket: 0,
            file_events: Vec::new(),
            length_unit: LengthUnit::default(),
            angle_unit: AngleUnit::default(),
            mesh_quality: MeshQuality::default(),
            mesh_requested: Vec::new(),
            display: Display::default(),
            shown_before: Vec::new(),
            evaluation_generation: 0,
        };
        model.start_storage(None, None);
        model.recompute(Retry::Nothing);
        model
    }

    pub fn length_unit(&self) -> LengthUnit {
        self.length_unit
    }

    pub fn waker(&self) -> Waker {
        (self.services.make_waker)()
    }

    pub fn units(&self) -> Units {
        Units {
            length: self.length_unit,
            angle: self.angle_unit,
        }
    }

    pub fn set_angle_unit(&mut self, unit: AngleUnit) {
        self.angle_unit = unit;
    }

    pub fn set_length_unit(&mut self, unit: LengthUnit) {
        self.length_unit = unit;
    }

    pub fn mesh_quality(&self) -> MeshQuality {
        self.mesh_quality
    }

    pub fn set_mesh_quality(&mut self, quality: MeshQuality) {
        if self.mesh_quality == quality {
            return;
        }
        self.mesh_quality = quality;
        if let Some(recomputer) = &mut self.recomputer
            && let Err(error) = recomputer.set_mesh_quality(quality)
        {
            log::error!("{error}");
            self.recomputer = None;
        }
        self.recompute(Retry::Nothing);
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

    pub fn evaluation_generation(&self) -> u64 {
        self.evaluation_generation
    }

    pub fn display(&self) -> &Display {
        &self.display
    }

    pub fn displayed_sketch<'a>(&'a self, feature: &'a Feature) -> Option<Displayed<'a>> {
        self.display.sketches.get(&self.evaluation, feature)
    }

    pub fn bodies_pending(&self) -> bool {
        self.display.meshing.is_pending()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.editor.undo_label()
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.editor.redo_label()
    }

    pub fn undo_steps(&self) -> impl Iterator<Item = &Transaction> {
        self.editor.undo_steps()
    }

    pub fn redo_steps(&self) -> impl Iterator<Item = &Transaction> {
        self.editor.redo_steps()
    }

    pub fn status(&self) -> RecomputeStatus {
        self.status
    }

    #[cfg(test)]
    pub fn set_status(&mut self, status: RecomputeStatus) {
        self.status = status;
    }

    pub fn progress(&self) -> Option<Progress> {
        self.recomputer.as_ref().and_then(Recomputer::progress)
    }

    pub fn notice(&self) -> Option<&Notice> {
        self.notice.as_ref()
    }

    pub fn suppress_every_feature(&mut self) -> bool {
        let document = self.editor.document();
        let every: Vec<FeatureId> = document.features().map(Feature::id).collect();
        let transaction = document.suppression(&every, true, "Suppress every feature");
        if transaction.is_empty() {
            return false;
        }
        self.perform(Action::Apply(transaction));
        true
    }

    pub fn set_notice(&mut self, notice: Notice) {
        match notice.kind {
            NoticeKind::Info => log::info!("{}", notice.text),
            NoticeKind::Error => log::warn!("{}", notice.text),
        }
        let repeats = self
            .recorded_notices
            .front()
            .is_some_and(|recorded| recorded.notice == notice);
        if !repeats {
            self.recorded_notices.push_front(RecordedNotice {
                notice: notice.clone(),
                at: SystemTime::now(),
            });
            self.recorded_notices.truncate(MAX_RECORDED_NOTICES);
        }
        self.notice = Some(notice);
    }

    pub fn drag_blocked(&self) -> bool {
        self.drag_blocked
    }

    pub fn sketch_conflict(&self, feature: FeatureId) -> Option<String> {
        let FeatureState::Failed(error) = &self.evaluation.feature(feature)?.state else {
            return None;
        };
        let sketch = self.editor.document().feature(feature)?.kind.sketch()?;
        let named: Vec<String> = error
            .constraints
            .iter()
            .take(NAMED_CONFLICTS)
            .map(|constraint| sketch.describe_constraint(*constraint))
            .collect();
        let more = error.constraints.len().saturating_sub(NAMED_CONFLICTS);
        match (named.as_slice(), more) {
            ([], _) => None,
            ([only], _) => Some(only.clone()),
            ([first, second], 0) => Some(format!("{first} and {second}")),
            (named, more) => Some(format!("{} and {more} more", named.join(", "))),
        }
    }

    pub fn recorded_notices(&self) -> impl Iterator<Item = &RecordedNotice> {
        self.recorded_notices.iter()
    }

    pub fn revision(&self) -> u64 {
        self.revision_offset + self.editor.revision()
    }

    pub fn session(&self) -> u64 {
        self.revision_offset
    }

    pub fn settled_sketch(&self, feature: FeatureId) -> Option<&Sketch> {
        self.settled_result(feature).map(|result| &result.geometry)
    }

    pub fn settled_solution(&self, feature: FeatureId) -> Option<&SketchSolution> {
        self.settled_result(feature).map(|result| &result.solution)
    }

    fn settled_result(&self, feature: FeatureId) -> Option<&SketchResult> {
        if self.status != RecomputeStatus::UpToDate {
            return None;
        }
        let status = self.evaluation.feature(feature)?;
        if status.state != FeatureState::UpToDate {
            return None;
        }
        status.result.as_deref()?.sketch()
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

    pub fn is_empty_and_untitled(&self) -> bool {
        self.path.is_none() && self.document().same_content(&Document::default())
    }

    pub fn unprotected(&self) -> Option<&JournalFailure> {
        self.unprotected.as_ref()
    }

    pub fn is_saving(&self) -> bool {
        self.pending_save.is_some()
    }

    pub fn mesh_before(&mut self, feature: Option<FeatureId>) {
        let open: Vec<Arc<FeatureResult>> = feature
            .map(|feature| {
                self.evaluation
                    .body_before(feature)
                    .into_iter()
                    .chain(self.evaluation.cuts(feature))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        self.mesh_requested
            .retain(|requested| open.iter().any(|result| Arc::ptr_eq(result, requested)));
        self.shown_before = open.clone();
        let Some(feature) = feature else {
            return;
        };
        let name = self
            .document()
            .feature(feature)
            .map_or_else(String::new, |feature| feature.name.clone());
        for result in open {
            if result.solid().is_none_or(|solid| solid.is_meshed()) {
                self.display
                    .meshing
                    .request(&result, || (self.services.make_waker)());
                continue;
            }
            let requested = self
                .mesh_requested
                .iter()
                .any(|requested| Arc::ptr_eq(requested, &result));
            if requested {
                continue;
            }
            let sent = self.recomputer.as_ref().is_some_and(|recomputer| {
                recomputer.mesh(Arc::clone(&result), name.clone()).is_ok()
            });
            if sent {
                self.mesh_requested.push(result);
            }
        }
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
                    .map(|changed| entry.filter(|_| changed))
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
                self.recompute(Retry::Failures);
                Ok(None)
            }
            Action::CancelRecompute => {
                if let Some(recomputer) = &mut self.recomputer {
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
            Action::Drag(command) => {
                self.drag(command);
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
            Action::Filter(filter) => {
                log::warn!("{filter:?} reached the model instead of the viewport");
                Ok(None)
            }
        };
        match changed {
            Ok(Some(entry)) => self.changed(entry),
            Ok(None) => {}
            Err(message) => self.set_notice(Notice::error(message)),
        }
    }

    pub fn base(&self) -> SessionBase {
        SessionBase {
            base: self.editor.base(),
            session: self.session(),
        }
    }

    pub fn commit(&mut self, session: u64, prepared: Prepared) -> Result<(), Stale> {
        if session != self.session() {
            return Err(Stale);
        }
        let transaction = self.editor.commit(prepared)?;
        if !transaction.is_empty() {
            self.changed(JournalEntry::Apply(transaction));
        }
        Ok(())
    }

    fn drag(&mut self, command: DragCommand) {
        let revision = self.revision();
        let shown = |feature| {
            let owner = self.editor.document().feature(feature)?;
            let shown = self.display.sketches.get(&self.evaluation, owner)?;
            Some(Arc::new(Sketch::clone(&shown)))
        };
        let wake = || (self.services.make_waker)();
        let start = drag_solver::Start {
            revision,
            parameters: &self.parameters,
            shown: &shown,
            wake: &wake,
        };
        let polled = self.display.dragging.perform(command, &start);
        self.dragged(polled);
    }

    fn dragged(&mut self, polled: Polled) -> bool {
        let Polled {
            shown,
            finished,
            abandoned,
            blocked,
        } = polled;
        let cue_changed = blocked.is_some_and(|blocked| blocked != self.drag_blocked);
        let changed = shown.is_some() || finished.is_some() || abandoned || cue_changed;
        if let Some(blocked) = blocked {
            self.drag_blocked = blocked;
        }
        if abandoned || finished.is_some() {
            self.drag_blocked = false;
        }
        if abandoned {
            self.display.sketches.stop_showing_dragged();
        }
        if let Some((feature, sketch)) = shown {
            self.display.sketches.show_dragged(feature, sketch);
        }
        if let Some(finished) = finished {
            self.commit_drag(finished);
        }
        changed
    }

    fn commit_drag(&mut self, finished: Finished) {
        let Finished {
            feature,
            label,
            sketch,
        } = finished;
        let Some(sketch) = sketch else {
            self.display.sketches.stop_showing_dragged();
            let reason = match self.sketch_conflict(feature) {
                Some(conflict) => format!(
                    "the sketch's constraints conflict ({conflict}); remove or turn one of them off \
                     first"
                ),
                None => "the sketch could not be solved with it moved there".to_owned(),
            };
            self.set_notice(Notice::info(format!(
                "{label} did nothing, because {reason}."
            )));
            return;
        };
        let mut transaction = self.editor.document().transaction(label);
        transaction.settle_sketch(feature, &sketch);
        let transaction = transaction.finish();
        if transaction.is_empty() {
            self.display.sketches.stop_showing_dragged();
            return;
        }
        let before = self.revision();
        self.perform(Action::Apply(transaction));
        if self.revision() != before {
            self.display.sketches.show_dragged(feature, sketch);
            self.display.sketches.hold_dragged_until(self.revision());
        } else {
            self.display.sketches.stop_showing_dragged();
        }
    }

    fn changed(&mut self, entry: JournalEntry) {
        self.display.sketches.forget();
        self.display.sketches.stop_showing_dragged();
        self.notice.take_if(|notice| !notice.outlasts_edits);
        self.record(entry);
        self.dirty = self.differs_from_saved();
        self.parameters = ParameterValues::evaluate(self.editor.document());
        self.recompute(Retry::Nothing);
    }

    pub fn poll(&mut self) -> bool {
        let polled = self.display.dragging.poll();
        let stored = self.poll_storage() | self.display.meshing.poll() | self.dragged(polled);
        let Some(recomputer) = &mut self.recomputer else {
            return stored;
        };
        match recomputer.poll() {
            Ok(Some(update)) if update.revision < self.revision_offset => stored,
            Ok(Some(update)) => {
                if update.revision == self.revision() {
                    self.status = match update.outcome {
                        Outcome::FeaturesDone => self.status,
                        Outcome::Finished => RecomputeStatus::UpToDate,
                        Outcome::Cancelled => RecomputeStatus::Cancelled,
                        Outcome::Failed => RecomputeStatus::Stopped,
                    };
                }
                self.evaluation = update.evaluation;
                self.evaluation_generation += 1;
                self.display.sketches.forget();
                self.display
                    .sketches
                    .evaluated(update.revision, &self.evaluation);
                self.mesh_bodies();
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
        self.send_save(path, false);
    }

    pub fn save_replacing_outside_changes(&mut self, path: PathBuf) {
        self.send_save(path, true);
    }

    fn send_save(&mut self, path: PathBuf, replace_outside_changes: bool) {
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
            replace_outside_changes,
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
            self.set_notice(Notice::failure(
                "Could not save, because the background writer stopped. Try saving again.",
            ));
            self.file_events.push(FileEvent::SaveFailed);
        }
    }

    pub fn replace(
        &mut self,
        document: Document,
        path: Option<PathBuf>,
        on_disk: Option<FileDigest>,
        damaged: bool,
    ) {
        self.switch_to(
            Session {
                editor: Editor::new(document.clone()),
                saved: Some(document.clone()),
                journal_base: document,
                folded: 0,
                path,
                on_disk,
                keep_original: damaged,
            },
            Vec::new(),
            None,
        );
    }

    pub fn restore(&mut self, recovered: Recovered) {
        let Recovered {
            journal,
            file,
            on_disk,
            loaded_with_problems,
            base,
            folded,
            entries,
            editor,
            ..
        } = recovered;
        self.switch_to(
            Session {
                editor,
                saved: (folded == 0).then(|| base.clone()),
                journal_base: base,
                folded,
                path: file,
                on_disk,
                keep_original: loaded_with_problems,
            },
            entries,
            Some(journal),
        );
    }

    pub fn close(&mut self) -> Option<Closing> {
        *self.services.panic_flush.lock() = None;
        self.storage.take().map(|storage| storage.close(true))
    }

    fn switch_to(
        &mut self,
        session: Session,
        entries: Vec<JournalEntry>,
        replaces: Option<PathBuf>,
    ) {
        let predecessor = self.storage.take().map(|storage| storage.close(true));
        self.revision_offset = self.revision() + 1;
        self.editor = session.editor;
        self.saved = session.saved;
        self.journal_base = session.journal_base;
        self.folded = session.folded;
        self.dirty = self.differs_from_saved();
        self.path = session.path;
        self.on_disk = session.on_disk;
        self.entries = entries;
        self.keep_original = session.keep_original;
        self.unprotected = None;
        self.pending_save = None;
        self.notice = None;
        self.parameters = ParameterValues::evaluate(self.editor.document());
        self.evaluation = Evaluation::default();
        self.evaluation_generation += 1;
        self.display.sketches.forget();
        self.display.dragging.cancel();
        self.display.sketches.stop_showing_dragged();
        self.shown_before.clear();
        self.mesh_requested.clear();
        if let Some(recomputer) = &mut self.recomputer
            && let Err(error) = recomputer.forget()
        {
            log::error!("{error}");
            self.recomputer = None;
        }
        self.mesh_bodies();
        self.start_storage(replaces, predecessor);
        self.recompute(Retry::Nothing);
    }

    fn mesh_bodies(&mut self) {
        let shown: Vec<Arc<FeatureResult>> = self
            .evaluation
            .bodies()
            .filter_map(|(body, _)| self.evaluation.body_result(body))
            .chain(&self.shown_before)
            .cloned()
            .collect();
        self.display
            .meshing
            .retain(|source| shown.iter().any(|kept| Arc::ptr_eq(kept, source)));
        for source in &shown {
            self.display
                .meshing
                .request(source, || (self.services.make_waker)());
        }
    }

    fn start_storage(&mut self, replaces: Option<PathBuf>, after: Option<Closing>) {
        let start = Start {
            file: self.path.clone(),
            on_disk: self.on_disk.clone(),
            loaded_with_problems: self.keep_original,
            base: self.journal_base.clone(),
            folded: self.folded,
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
                self.unprotected = Some(JournalFailure::WriterUnavailable);
                self.set_notice(Notice::failure(
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
            self.set_notice(Notice::failure(
                "The save did not finish, because the background writer stopped. Try saving \
                 again.",
            ));
            self.file_events.push(FileEvent::SaveFailed);
        }
        self.unprotected = None;
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
                dropped_for_size,
                digest,
            } => {
                if let Some(pending) = self
                    .pending_save
                    .take_if(|pending| pending.ticket == ticket)
                {
                    self.saved = Some(pending.document.clone());
                    self.journal_base = pending.document;
                    self.folded = 0;
                    self.entries
                        .drain(..pending.entries.min(self.entries.len()));
                }
                self.path = Some(path.clone());
                self.on_disk = Some(digest);
                self.keep_original = false;
                self.dirty = self.differs_from_saved();
                if let Some(notice) = saved_notice(backup.as_deref(), dropped_for_size) {
                    self.set_notice(notice);
                }
                self.file_events.push(FileEvent::Saved(path));
            }
            Report::ChangedOnDisk { ticket, path } => {
                self.pending_save
                    .take_if(|pending| pending.ticket == ticket);
                self.file_events.push(FileEvent::ChangedOnDisk(path));
            }
            Report::SaveFailed {
                ticket,
                path,
                error,
            } => {
                self.pending_save
                    .take_if(|pending| pending.ticket == ticket);
                self.set_notice(Notice::failure(format!(
                    "Could not save “{}”: {error}. Use Save As to choose another location.",
                    display_name(Some(&path))
                )));
                self.file_events.push(FileEvent::SaveFailed);
            }
            Report::JournalFailed { failure } => {
                self.set_notice(Notice::failure(format!(
                    "Unsaved changes are not protected against a crash: {failure}. caditor keeps \
                     trying; save your work to keep it safe."
                )));
                self.unprotected = Some(failure);
            }
            Report::JournalRestored => {
                self.unprotected = None;
                self.set_notice(Notice::info(
                    "Unsaved changes are protected against a crash again.",
                ));
            }
            Report::Rebased { entries, base } => {
                self.journal_base = base;
                self.folded = self.folded.saturating_add(entries);
                self.entries.drain(..entries.min(self.entries.len()));
                if let Some(pending) = &mut self.pending_save {
                    pending.entries = pending.entries.saturating_sub(entries);
                }
            }
        }
    }

    fn differs_from_saved(&self) -> bool {
        self.saved
            .as_ref()
            .is_none_or(|saved| !self.editor.document().same_content(saved))
    }

    fn recompute(&mut self, retry: Retry) {
        if self.recomputer.is_none() {
            match Recomputer::spawn(ModelEvaluator, (self.services.make_waker)()) {
                Ok(mut recomputer) => {
                    if let Err(error) = recomputer.set_mesh_quality(self.mesh_quality) {
                        log::error!("{error}");
                    }
                    self.recomputer = Some(recomputer);
                }
                Err(error) => {
                    log::error!("could not start the recompute worker: {error}");
                    self.status = RecomputeStatus::Stopped;
                    return;
                }
            }
        }
        let document = self.editor.document().clone();
        let revision = self.revision();
        let submitted = self.recomputer.as_mut().map(|recomputer| match retry {
            Retry::Nothing => recomputer.submit(document, revision),
            Retry::Failures => recomputer.submit_retrying_failures(document, revision),
        });
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

fn saved_notice(backup: Option<&Path>, dropped_for_size: usize) -> Option<Notice> {
    let backup = backup.map(|backup| {
        format!(
            "The damaged original was kept as “{}”.",
            display_name(Some(backup))
        )
    });
    let dropped = match dropped_for_size {
        0 => None,
        1 => Some(
            "The oldest earlier version was removed to keep the file small enough to open again."
                .to_owned(),
        ),
        count => Some(format!(
            "The {count} oldest earlier versions were removed to keep the file small enough to \
             open again."
        )),
    };
    let sentences: Vec<String> = backup.into_iter().chain(dropped).collect();
    (!sentences.is_empty()).then(|| Notice::info(format!("Saved. {}", sentences.join(" "))))
}
