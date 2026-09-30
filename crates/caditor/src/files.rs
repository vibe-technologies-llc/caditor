use std::{
    collections::VecDeque,
    ffi::OsString,
    fs,
    panic::{self, AssertUnwindSafe},
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant},
};

use caditor_document::{Document, FeatureId};
use caditor_file::{
    Closing, DXF_EXTENSION, Drawing, ExportError, ExportFormat, Exported, FILE_EXTENSION,
    FileJournal, History, ImportError, LoadError, Loaded, ModelImport, PNG_EXTENSION, RecentChange,
    RecentFiles, Recovered, STEP_EXTENSIONS, STEP_IMPORT_EXTENSIONS, SavedState, Settings,
    describe_set_aside, journal_for, load, load_version, read_dxf, read_step_file, scan,
};
use caditor_render::{ImageError, SurfaceSize};
use egui::{Id, Modal, RichText, Ui};
use parking_lot::Mutex;

use crate::{
    appearance,
    commands::{Command, CommandFrame, RecentSlot},
    editing::SketchEditing,
    export::{self, ExportCommand, Exporter},
    history::{self, HistoryCommand, VersionHistory},
    icons,
    image_export::{
        self, IMAGE_HINT, ImageCommand, ImageExporter, ImageFailure, ReadPixels, RenderJob,
    },
    import::{self, DrawingPlan, IMPORT_HINT, Placement},
    model::{Action, FileEvent, Model, Notice, WakerFactory, display_name},
    preferences::PreferencesCommand,
    samples::Sample,
    widgets::{self, DialogWidth, Tone},
};

const OPEN_RECENT: &str = "Open Recent";
const OPEN_SAMPLE: &str = "Open Sample";
const QUIT_ANYWAY_AFTER: Duration = Duration::from_secs(5);
const INTERNAL_ERROR: &str = "caditor ran into an internal error while reading it";
const REPORT_HEIGHT: f32 = 280.0;
const MODEL_KIND: &str = "caditor model";
const DRAWING_KIND: &str = "DXF drawing";
const MODEL_EXCHANGE_KIND: &str = "STEP model";
const IMPORTABLE_KIND: &str = "Drawings and models";
const IMAGE_KIND: &str = "PNG image";
const FILE_COMMANDS: [Command; 10] = [
    Command::New,
    Command::Open,
    Command::Save,
    Command::SaveAs,
    Command::VersionHistory,
    Command::Import,
    Command::Export,
    Command::ExportImage,
    Command::Preferences,
    Command::Quit,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileCommand {
    New,
    Open,
    OpenPath(PathBuf),
    Save,
    SaveAs,
    Quit,
    Guard(GuardChoice),
    ShowRecovery,
    HideRecovery,
    Restore(PathBuf),
    AskDiscard(PathBuf),
    KeepRecovered,
    Discard(PathBuf),
    DismissReport,
    Replace(bool),
    QuitAnyway,
    Export(ExportCommand),
    ExportImage(ImageCommand),
    History(HistoryCommand),
    Import {
        into: Option<FeatureId>,
    },
    Drop {
        paths: Vec<PathBuf>,
        into: Option<FeatureId>,
    },
    OpenSample(Sample),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardChoice {
    Save,
    DontSave,
    Cancel,
}

pub type Respond = Box<dyn FnOnce(Option<PathBuf>) + Send>;

pub trait Dialogs {
    fn pick_model(&self, directory: Option<PathBuf>, respond: Respond);
    fn pick_save_path(&self, directory: Option<PathBuf>, file_name: String, respond: Respond);
    fn pick_export_path(
        &self,
        directory: Option<PathBuf>,
        file_name: String,
        format: ExportFormat,
        respond: Respond,
    );
    fn pick_import(&self, directory: Option<PathBuf>, respond: Respond);
    fn pick_image_path(&self, directory: Option<PathBuf>, file_name: String, respond: Respond);
}

pub struct NativeDialogs;

impl NativeDialogs {
    fn spawn(respond: Respond, pick: impl FnOnce() -> Option<PathBuf> + Send + 'static) {
        let slot = Arc::new(Mutex::new(Some(respond)));
        let worker_slot = Arc::clone(&slot);
        let spawned = thread::Builder::new()
            .name("file-dialog".to_owned())
            .spawn(move || {
                let picked = pick();
                if let Some(respond) = worker_slot.lock().take() {
                    respond(picked);
                }
            });
        if let Err(error) = spawned {
            log::error!("could not open the file dialog: {error}");
            if let Some(respond) = slot.lock().take() {
                respond(None);
            }
        }
    }

    fn dialog(directory: Option<PathBuf>, kind: &str, extensions: &[&str]) -> rfd::FileDialog {
        let dialog = rfd::FileDialog::new().add_filter(kind, extensions);
        match directory {
            Some(directory) => dialog.set_directory(directory),
            None => dialog,
        }
    }
}

impl Dialogs for NativeDialogs {
    fn pick_model(&self, directory: Option<PathBuf>, respond: Respond) {
        Self::spawn(respond, move || {
            Self::dialog(directory, MODEL_KIND, &[FILE_EXTENSION])
                .set_title("Open Model")
                .pick_file()
        });
    }

    fn pick_save_path(&self, directory: Option<PathBuf>, file_name: String, respond: Respond) {
        Self::spawn(respond, move || {
            Self::dialog(directory, MODEL_KIND, &[FILE_EXTENSION])
                .set_title("Save Model")
                .set_file_name(file_name)
                .save_file()
        });
    }

    fn pick_export_path(
        &self,
        directory: Option<PathBuf>,
        file_name: String,
        format: ExportFormat,
        respond: Respond,
    ) {
        Self::spawn(respond, move || {
            let extensions: &[&str] = match format {
                ExportFormat::Step => &STEP_EXTENSIONS,
                ExportFormat::Stl | ExportFormat::ThreeMf => &[format.extension()],
            };
            Self::dialog(directory, format.name(), extensions)
                .set_title(format!("Export {}", format.name()))
                .set_file_name(file_name)
                .save_file()
        });
    }

    fn pick_import(&self, directory: Option<PathBuf>, respond: Respond) {
        Self::spawn(respond, move || {
            let every: Vec<&str> = std::iter::once(DXF_EXTENSION)
                .chain(STEP_IMPORT_EXTENSIONS)
                .collect();
            Self::dialog(directory, IMPORTABLE_KIND, &every)
                .add_filter(DRAWING_KIND, &[DXF_EXTENSION])
                .add_filter(MODEL_EXCHANGE_KIND, &STEP_IMPORT_EXTENSIONS)
                .set_title("Import")
                .pick_file()
        });
    }

    fn pick_image_path(&self, directory: Option<PathBuf>, file_name: String, respond: Respond) {
        Self::spawn(respond, move || {
            Self::dialog(directory, IMAGE_KIND, &[PNG_EXTENSION])
                .set_title("Export Image")
                .set_file_name(file_name)
                .save_file()
        });
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilesConfig {
    pub state_dir: Option<PathBuf>,
    pub recovery_dir: Option<PathBuf>,
    pub config_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Purpose {
    Open,
    SaveAs,
    Export(ExportFormat),
    Image,
    Import,
}

enum OpenOutcome {
    Loaded { loaded: Loaded, notes: Vec<String> },
    Recoverable(Box<Recovered>),
    AlreadyOpen,
    InUse,
    Failed { error: LoadError, missing: bool },
}

struct Opened {
    path: PathBuf,
    loaded: Loaded,
    notes: Vec<String>,
}

enum SaveTarget {
    Ready(PathBuf),
    Confirm(PathBuf),
    InUse(PathBuf),
    HasRecovery(PathBuf),
    Failed(PathBuf),
}

enum Event {
    ScanFailed,
    SaveTargetChecked(SaveTarget),
    Picked {
        purpose: Purpose,
        path: Option<PathBuf>,
    },
    Started {
        recent: RecentFiles,
        recovered: Vec<Recovered>,
    },
    Opened {
        path: PathBuf,
        revision: u64,
        outcome: OpenOutcome,
    },
    Discarded {
        journal: PathBuf,
        error: Option<String>,
    },
    Exported {
        path: PathBuf,
        result: Result<Exported, ExportError>,
    },
    ImageExported {
        path: PathBuf,
        result: Result<SurfaceSize, ImageFailure>,
    },
    HistoryListed {
        path: PathBuf,
        result: Result<History, LoadError>,
    },
    VersionLoaded {
        path: PathBuf,
        state: SavedState,
        result: Result<Loaded, LoadError>,
    },
    Imported {
        path: PathBuf,
        session: u64,
        into: Option<FeatureId>,
        result: Result<DrawingPlan, ImportError>,
    },
    ImportedModel {
        path: PathBuf,
        session: u64,
        result: Result<ModelImport, ImportError>,
    },
}

enum Intent {
    New,
    Sample(Sample),
    Open(Option<PathBuf>),
    Restore(PathBuf),
    Replace(Box<Opened>),
    Quit,
}

struct Candidate {
    recovered: Recovered,
    open_file_on_discard: bool,
}

struct Report {
    heading: String,
    intro: Option<&'static str>,
    issues: Vec<String>,
}

struct Importing {
    path: Option<PathBuf>,
    into: Option<FeatureId>,
}

struct Queued {
    path: PathBuf,
    into: Option<FeatureId>,
    session: u64,
}

type Job = Box<dyn FnOnce() + Send>;

pub struct Files {
    config: FilesConfig,
    dialogs: Box<dyn Dialogs>,
    make_waker: WakerFactory,
    events: Sender<Event>,
    inbox: Receiver<Event>,
    jobs: Option<Sender<Job>>,
    recent: RecentFiles,
    recoverable: Vec<Candidate>,
    recovery_open: bool,
    confirm_discard: Option<PathBuf>,
    guard: Option<Intent>,
    after_save: Option<Intent>,
    report: Option<Report>,
    opening: Option<PathBuf>,
    importing: Option<Importing>,
    queued_imports: VecDeque<Queued>,
    exporter: Exporter,
    image: ImageExporter,
    history: VersionHistory,
    picking: bool,
    confirm_replace: Option<PathBuf>,
    closing: Option<(Closing, Instant)>,
    stored_settings: Option<Settings>,
    quit: bool,
}

impl Files {
    pub fn new(config: FilesConfig, dialogs: Box<dyn Dialogs>, make_waker: WakerFactory) -> Self {
        let (events, inbox) = mpsc::channel();
        Self {
            config,
            dialogs,
            make_waker,
            events,
            inbox,
            jobs: None,
            recent: RecentFiles::default(),
            recoverable: Vec::new(),
            recovery_open: false,
            confirm_discard: None,
            guard: None,
            after_save: None,
            report: None,
            opening: None,
            importing: None,
            queued_imports: VecDeque::new(),
            exporter: Exporter::default(),
            image: ImageExporter::default(),
            history: VersionHistory::default(),
            picking: false,
            confirm_replace: None,
            closing: None,
            stored_settings: None,
            quit: false,
        }
    }

    pub fn start(&mut self, open: Option<PathBuf>, model: &mut Model) {
        let state_dir = self.config.state_dir.clone();
        let recovery_dir = self.config.recovery_dir.clone();
        self.spawn(
            move || {
                let recent = state_dir
                    .as_deref()
                    .map(RecentFiles::load)
                    .unwrap_or_default();
                let recovered = scan(recovery_dir.as_deref(), recent.paths());
                Event::Started { recent, recovered }
            },
            || Event::ScanFailed,
        );
        if let Some(path) = open {
            self.perform(FileCommand::OpenPath(path), model);
        }
    }

    pub fn should_quit(&self) -> bool {
        self.quit
    }

    pub fn is_blocking(&self) -> bool {
        self.guard.is_some()
            || self.closing.is_some()
            || self.confirm_replace.is_some()
            || self.opening.is_some()
            || self.report.is_some()
            || self.showing_recovery()
            || self.exporter.is_open()
            || self.image.is_open()
            || self.history.is_open()
            || self.picking
    }

    pub fn recent(&self) -> &[PathBuf] {
        self.recent.paths()
    }

    pub fn has_recoverable(&self) -> bool {
        !self.recoverable.is_empty()
    }

    fn showing_recovery(&self) -> bool {
        self.recovery_open && !self.recoverable.is_empty()
    }

    pub fn perform(&mut self, command: FileCommand, model: &mut Model) {
        match command {
            FileCommand::New => self.request(Intent::New, model),
            FileCommand::OpenSample(sample) => self.request(Intent::Sample(sample), model),
            FileCommand::Open => self.request(Intent::Open(None), model),
            FileCommand::OpenPath(path) => self.request(Intent::Open(Some(path)), model),
            FileCommand::Quit if self.closing.is_some() => {}
            FileCommand::Quit => self.request(Intent::Quit, model),
            FileCommand::Restore(journal) => self.request(Intent::Restore(journal), model),
            FileCommand::Save => self.save(model),
            FileCommand::SaveAs => self.pick(Purpose::SaveAs, model),
            FileCommand::Guard(choice) => {
                let Some(intent) = self.guard.take() else {
                    return;
                };
                match choice {
                    GuardChoice::Save => {
                        self.after_save = Some(intent);
                        self.save(model);
                    }
                    GuardChoice::DontSave => self.run(intent, model),
                    GuardChoice::Cancel => {}
                }
            }
            FileCommand::ShowRecovery => self.recovery_open = true,
            FileCommand::HideRecovery => {
                self.recovery_open = false;
                self.confirm_discard = None;
            }
            FileCommand::AskDiscard(journal) => self.confirm_discard = Some(journal),
            FileCommand::KeepRecovered => self.confirm_discard = None,
            FileCommand::Discard(journal) => {
                self.confirm_discard = None;
                let failed = journal.clone();
                self.spawn(
                    move || Event::Discarded {
                        error: caditor_file::discard(&journal)
                            .err()
                            .map(|error| error.to_string()),
                        journal,
                    },
                    move || Event::Discarded {
                        journal: failed,
                        error: Some(INTERNAL_ERROR.to_owned()),
                    },
                );
            }
            FileCommand::DismissReport => self.report = None,
            FileCommand::Replace(confirmed) => {
                let Some(path) = self.confirm_replace.take() else {
                    return;
                };
                if confirmed {
                    model.save_to(path);
                } else {
                    self.after_save = None;
                }
            }
            FileCommand::QuitAnyway => {
                if self.closing.take().is_some() {
                    log::warn!("quitting before the storage worker finished");
                    self.quit = true;
                }
            }
            FileCommand::Export(command) => {
                self.exporter.perform(command);
                if command == ExportCommand::Choose {
                    self.pick(Purpose::Export(self.exporter.format()), model);
                }
            }
            FileCommand::ExportImage(command) => {
                self.image.perform(command);
                if matches!(command, ImageCommand::Choose(_)) {
                    self.pick(Purpose::Image, model);
                }
            }
            FileCommand::History(command) => self.history_command(command, model),
            FileCommand::Import { into } => {
                if self.importing.is_some() {
                    model.set_notice(Notice::info("An import is already running."));
                    return;
                }
                if self.picking {
                    return;
                }
                self.importing = Some(Importing { path: None, into });
                self.pick(Purpose::Import, model);
            }
            FileCommand::Drop { paths, into } => self.dropped(paths, into, model),
        }
    }

    fn dropped(&mut self, paths: Vec<PathBuf>, into: Option<FeatureId>, model: &mut Model) {
        if self.is_blocking() {
            model.set_notice(Notice::info(
                "Finish with the open dialog before dropping files on caditor.",
            ));
            return;
        }
        let models = paths.iter().filter(|path| is_model_file(path)).count();
        match (models, paths.as_slice()) {
            (0, _) if self.importing.is_some() || !self.queued_imports.is_empty() => {
                model.set_notice(Notice::info(
                    "An import is already running. Drop the files again once it has finished.",
                ));
            }
            (0, _) => {
                let session = model.session();
                self.queued_imports
                    .extend(paths.into_iter().map(|path| Queued {
                        path,
                        into,
                        session,
                    }));
                self.import_next(model);
            }
            (1, [path]) => self.request(Intent::Open(Some(path.clone())), model),
            _ => model.set_notice(Notice::info(
                "Drop a single model to open it, or drawings and STEP files to import them.",
            )),
        }
    }

    fn import_next(&mut self, model: &Model) {
        if self.importing.is_some() || self.report.is_some() || self.picking {
            return;
        }
        let session = model.session();
        while let Some(queued) = self.queued_imports.pop_front() {
            if queued.session == session {
                self.import(queued.path, queued.into, model);
                return;
            }
        }
    }

    fn history_command(&mut self, command: HistoryCommand, model: &mut Model) {
        match command {
            HistoryCommand::Show => match model.path() {
                Some(path) => {
                    self.history.open(path.to_path_buf());
                    self.list_versions();
                }
                None => model.set_notice(Notice::info(
                    "Save the model first; from then on every save keeps the version before it.",
                )),
            },
            HistoryCommand::Hide => self.history.close(),
            HistoryCommand::Restore(index) => {
                if let Some((path, state)) = self.history.start_restoring(index) {
                    let (failed_path, failed_state) = (path.clone(), state.clone());
                    self.spawn(
                        move || Event::VersionLoaded {
                            result: load_version(&path, index),
                            path,
                            state,
                        },
                        move || Event::VersionLoaded {
                            path: failed_path,
                            state: failed_state,
                            result: Err(internal_load_error()),
                        },
                    );
                }
            }
        }
    }

    fn list_versions(&mut self) {
        if let Some(path) = self.history.path().cloned() {
            let failed = path.clone();
            self.spawn(
                move || Event::HistoryListed {
                    result: caditor_file::history(&path),
                    path,
                },
                move || Event::HistoryListed {
                    path: failed,
                    result: Err(internal_load_error()),
                },
            );
        }
    }

    fn version_loaded(
        &mut self,
        path: &Path,
        state: &SavedState,
        result: Result<Loaded, LoadError>,
        model: &mut Model,
    ) {
        self.history.finish_restoring();
        if model.path() != Some(path) {
            model.set_notice(Notice::info(format!(
                "The earlier version of “{}” was not restored, because another model is open now.",
                display_name(Some(path))
            )));
            return;
        }
        match result {
            Ok(loaded) if loaded.document.same_content(model.document()) => {
                model.set_notice(Notice::info("That version is the same as the model now."));
            }
            Ok(loaded) => {
                let transaction = model
                    .document()
                    .transaction_to(&loaded.document, "Restore earlier version");
                let revision = model.revision();
                model.perform(Action::Apply(transaction));
                if model.revision() == revision {
                    return;
                }
                self.history.close();
                model.set_notice(Notice::info(format!(
                    "Restored the version saved {}. Undo brings back what you had.",
                    history::when_saved(state)
                )));
            }
            Err(error) => model.set_notice(Notice::failure(format!(
                "Could not restore that version: {error}."
            ))),
        }
    }

    #[cfg(test)]
    pub fn exporter_includes(&self, body: FeatureId) -> bool {
        self.exporter.includes(body)
    }

    pub fn image_job(&mut self) -> Option<RenderJob> {
        self.image.render_job()
    }

    #[cfg(test)]
    pub fn is_exporting_image(&self) -> bool {
        self.image.is_running()
    }

    pub fn image_rendered(&mut self, pixels: Result<ReadPixels, ImageError>, model: &mut Model) {
        let events = self.events.clone();
        let wake = (self.make_waker)();
        let refused = self.image.rendered(
            pixels,
            Box::new(move |path, result| {
                if events.send(Event::ImageExported { path, result }).is_ok() {
                    wake();
                }
            }),
        );
        if let Some(notice) = refused {
            model.set_notice(notice);
        }
    }

    pub fn poll(&mut self, model: &mut Model, editing: &mut SketchEditing) -> bool {
        self.exporter.sync(model);
        let mut changed = false;
        if self
            .closing
            .take_if(|(closing, _)| closing.finished())
            .is_some()
        {
            self.quit = true;
            changed = true;
        }
        for event in model.take_file_events() {
            changed = true;
            match event {
                FileEvent::Saved(path) => {
                    if self.history.path() == Some(&path) {
                        self.list_versions();
                    }
                    self.remember(path);
                    if let Some(intent) = self.after_save.take() {
                        self.request(intent, model);
                    }
                }
                FileEvent::SaveFailed => self.after_save = None,
            }
        }
        while let Ok(event) = self.inbox.try_recv() {
            changed = true;
            self.handle(event, model, editing);
        }
        self.import_next(model);
        changed
    }

    fn handle(&mut self, event: Event, model: &mut Model, editing: &mut SketchEditing) {
        match event {
            Event::ScanFailed => model.set_notice(Notice::failure(
                "caditor could not look for unsaved work from an earlier session. It will look \
                 again the next time it starts.",
            )),
            Event::SaveTargetChecked(target) => self.save_target_checked(target, model),
            Event::Picked { purpose, path } => {
                self.picking = false;
                match (purpose, path) {
                    (Purpose::Open, Some(path)) => self.open(path, model),
                    (Purpose::SaveAs, Some(path)) => self.save_as(path, model),
                    (Purpose::Export(format), Some(path)) => self.export(path, format, model),
                    (Purpose::Import, Some(path)) => {
                        let into = self.importing.as_ref().and_then(|importing| importing.into);
                        self.import(path, into, model);
                    }
                    (Purpose::Image, path) => self.image.picked(path),
                    (Purpose::Import, None) => self.importing = None,
                    (_, None) => self.after_save = None,
                }
            }
            Event::Started { recent, recovered } => {
                let added_meanwhile = std::mem::replace(&mut self.recent, recent);
                for path in added_meanwhile.paths().iter().rev() {
                    self.recent.add(path.clone());
                }
                for recovered in recovered {
                    self.offer(recovered, false);
                }
            }
            Event::Opened {
                path,
                revision,
                outcome,
            } => {
                self.opening = None;
                self.opened(path, revision, outcome, model);
            }
            Event::Discarded { journal, error } => {
                let index = self
                    .recoverable
                    .iter()
                    .position(|candidate| candidate.recovered.journal == journal);
                let candidate = index.map(|index| self.recoverable.remove(index));
                match (error, candidate) {
                    (Some(error), _) => model.set_notice(Notice::failure(format!(
                        "Could not discard the recovered changes: {error}."
                    ))),
                    (None, Some(candidate)) if candidate.open_file_on_discard => {
                        if let Some(file) = candidate.recovered.file {
                            self.request(Intent::Open(Some(file)), model);
                        }
                    }
                    (None, _) => {}
                }
            }
            Event::Exported { path, result } => {
                let notice = self.exporter.finished(&path, result);
                model.set_notice(notice);
            }
            Event::ImageExported { path, result } => {
                let notice = self.image.finished(&path, result);
                model.set_notice(notice);
            }
            Event::HistoryListed { path, result } => self.history.listed(&path, result),
            Event::VersionLoaded {
                path,
                state,
                result,
            } => self.version_loaded(&path, &state, result, model),
            Event::Imported {
                path,
                session,
                into,
                result,
            } => {
                self.importing = None;
                if session != model.session() {
                    return;
                }
                match import::place_drawing(model, editing, &path, result) {
                    Placement::Done(Some(report)) => {
                        self.report = Some(Report {
                            heading: report.heading,
                            intro: None,
                            issues: report.notes,
                        });
                    }
                    Placement::Done(None) => {}
                    Placement::Stale(drawing) => self.plan_again(path, into, drawing, model),
                }
            }
            Event::ImportedModel {
                path,
                session,
                result,
            } => {
                self.importing = None;
                if session != model.session() {
                    return;
                }
                if let Some(report) = import::place_bodies(model, &path, result) {
                    self.report = Some(Report {
                        heading: report.heading,
                        intro: None,
                        issues: report.notes,
                    });
                }
            }
        }
    }

    fn import(&mut self, path: PathBuf, into: Option<FeatureId>, model: &Model) {
        self.importing = Some(Importing {
            path: Some(path.clone()),
            into,
        });
        let session = model.session();
        let base = model.base();
        let failed = path.clone();
        self.spawn(
            move || {
                if import::is_model(&path) {
                    Event::ImportedModel {
                        result: read_step_file(&path),
                        path,
                        session,
                    }
                } else {
                    Event::Imported {
                        result: read_dxf(&path)
                            .map(|drawing| import::plan_drawing(base, &path, into, drawing)),
                        path,
                        session,
                        into,
                    }
                }
            },
            move || Event::Imported {
                path: failed,
                session,
                into,
                result: Err(ImportError::Reading(INTERNAL_ERROR.to_owned())),
            },
        );
    }

    fn plan_again(
        &mut self,
        path: PathBuf,
        into: Option<FeatureId>,
        drawing: Drawing,
        model: &Model,
    ) {
        self.importing = Some(Importing {
            path: Some(path.clone()),
            into,
        });
        let session = model.session();
        let base = model.base();
        let failed = path.clone();
        self.spawn(
            move || Event::Imported {
                result: Ok(import::plan_drawing(base, &path, into, drawing)),
                path,
                session,
                into,
            },
            move || Event::Imported {
                path: failed,
                session,
                into,
                result: Err(ImportError::Reading(INTERNAL_ERROR.to_owned())),
            },
        );
    }

    fn export(&mut self, path: PathBuf, format: ExportFormat, model: &mut Model) {
        if self.exporter.is_running() {
            model.set_notice(Notice::info("An export is already running."));
            return;
        }
        let events = self.events.clone();
        let wake = (self.make_waker)();
        self.exporter.start(
            path,
            format,
            model,
            Box::new(move |path, result| {
                if events.send(Event::Exported { path, result }).is_ok() {
                    wake();
                }
            }),
        );
    }

    fn opened(&mut self, path: PathBuf, revision: u64, outcome: OpenOutcome, model: &mut Model) {
        let name = display_name(Some(&path));
        match outcome {
            OpenOutcome::Loaded { loaded, notes } => {
                let opened = Opened {
                    path,
                    loaded,
                    notes,
                };
                if model.revision() != revision && model.is_dirty() {
                    self.guard = Some(Intent::Replace(Box::new(opened)));
                } else {
                    self.finish_open(opened, model);
                }
            }
            OpenOutcome::Recoverable(recovered) => self.offer(*recovered, true),
            OpenOutcome::AlreadyOpen => {
                model.set_notice(Notice::info(format!("“{name}” is already open.")));
            }
            OpenOutcome::InUse => model.set_notice(Notice::failure(format!(
                "“{name}” is already open in another caditor window."
            ))),
            OpenOutcome::Failed { error, missing } => {
                if missing {
                    self.forget(&path);
                }
                model.set_notice(Notice::failure(format!(
                    "Could not open “{name}”: {error}."
                )));
            }
        }
    }

    fn offer(&mut self, recovered: Recovered, open_file_on_discard: bool) {
        self.recovery_open = true;
        if let Some(existing) = self
            .recoverable
            .iter_mut()
            .find(|candidate| candidate.recovered.journal == recovered.journal)
        {
            existing.open_file_on_discard |= open_file_on_discard;
            return;
        }
        self.recoverable.push(Candidate {
            recovered,
            open_file_on_discard,
        });
    }

    fn request(&mut self, intent: Intent, model: &mut Model) {
        if model.is_dirty() {
            self.guard = Some(intent);
        } else {
            self.run(intent, model);
        }
    }

    fn run(&mut self, intent: Intent, model: &mut Model) {
        match intent {
            Intent::New => model.replace(Document::default(), None, false),
            Intent::Sample(sample) => match sample.document() {
                Ok(document) => model.replace(document, None, false),
                Err(error) => {
                    log::error!("could not build a sample: {error:#}");
                    model.perform(Action::Inform(Notice::failure(format!(
                        "The {} sample could not be opened. Your model was not changed.",
                        sample.title()
                    ))));
                }
            },
            Intent::Open(None) => self.pick(Purpose::Open, model),
            Intent::Open(Some(path)) => self.open(path, model),
            Intent::Restore(journal) => {
                let Some(index) = self
                    .recoverable
                    .iter()
                    .position(|candidate| candidate.recovered.journal == journal)
                else {
                    return;
                };
                let candidate = self.recoverable.remove(index);
                self.confirm_discard = None;
                if let Some(file) = &candidate.recovered.file {
                    self.remember(file.clone());
                }
                model.restore(candidate.recovered);
            }
            Intent::Replace(opened) => self.finish_open(*opened, model),
            Intent::Quit => match model.close() {
                Some(closing) => self.closing = Some((closing, Instant::now())),
                None => self.quit = true,
            },
        }
    }

    fn save(&mut self, model: &mut Model) {
        match model.path() {
            Some(path) => model.save_to(path.to_path_buf()),
            None => self.pick(Purpose::SaveAs, model),
        }
    }

    fn pick(&mut self, purpose: Purpose, model: &Model) {
        if self.picking {
            return;
        }
        self.picking = true;
        let events = self.events.clone();
        let wake = (self.make_waker)();
        let respond: Respond = Box::new(move |path| {
            if events.send(Event::Picked { purpose, path }).is_ok() {
                wake();
            }
        });
        let directory = model
            .path()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .or_else(|| {
                self.recent
                    .paths()
                    .first()
                    .and_then(|recent| recent.parent().map(Path::to_path_buf))
            });
        match purpose {
            Purpose::Open => self.dialogs.pick_model(directory, respond),
            Purpose::SaveAs => {
                let file_name = match model.path() {
                    Some(_) => model.display_name(),
                    None => format!("{}.{FILE_EXTENSION}", model.display_name()),
                };
                self.dialogs.pick_save_path(directory, file_name, respond);
            }
            Purpose::Export(format) => {
                let file_name = self.exporter.file_name(model);
                self.dialogs
                    .pick_export_path(directory, file_name, format, respond);
            }
            Purpose::Image => {
                let directory = self.image.folder().or(directory);
                self.dialogs
                    .pick_image_path(directory, ImageExporter::file_name(model), respond);
            }
            Purpose::Import => self.dialogs.pick_import(directory, respond),
        }
    }

    fn open(&mut self, path: PathBuf, model: &mut Model) {
        self.opening = Some(path.clone());
        let revision = model.revision();
        let current = model.path().map(Path::to_path_buf);
        let recovery_dir = self.config.recovery_dir.clone();
        let failed = path.clone();
        self.spawn(
            move || {
                let path = fs::canonicalize(&path).unwrap_or(path);
                let outcome = open_file(&path, current.as_deref(), recovery_dir.as_deref());
                Event::Opened {
                    path,
                    revision,
                    outcome,
                }
            },
            move || Event::Opened {
                path: failed,
                revision,
                outcome: OpenOutcome::Failed {
                    error: internal_load_error(),
                    missing: false,
                },
            },
        );
    }

    fn save_as(&mut self, picked: PathBuf, model: &mut Model) {
        let current = model.path().map(Path::to_path_buf);
        let recovery_dir = self.config.recovery_dir.clone();
        let failed = picked.clone();
        self.spawn(
            move || {
                Event::SaveTargetChecked(check_save_target(
                    &picked,
                    current.as_deref(),
                    recovery_dir.as_deref(),
                ))
            },
            move || Event::SaveTargetChecked(SaveTarget::Failed(failed)),
        );
    }

    fn save_target_checked(&mut self, target: SaveTarget, model: &mut Model) {
        let refusal = match &target {
            SaveTarget::Ready(path) => {
                model.save_to(path.clone());
                return;
            }
            SaveTarget::Confirm(path) => {
                self.confirm_replace = Some(path.clone());
                return;
            }
            SaveTarget::InUse(path) => format!(
                "“{}” is open in another caditor window. Close it there first, or save under \
                 another name.",
                display_name(Some(path))
            ),
            SaveTarget::HasRecovery(path) => format!(
                "“{}” has unsaved changes from an earlier session. Open it to recover or discard \
                 them first, or save under another name.",
                display_name(Some(path))
            ),
            SaveTarget::Failed(path) => format!(
                "Could not save “{}”: caditor ran into an internal error while checking the \
                 location. Try again, or choose another name.",
                display_name(Some(path))
            ),
        };
        self.after_save = None;
        model.set_notice(Notice::failure(refusal));
    }

    fn finish_open(&mut self, opened: Opened, model: &mut Model) {
        let Opened {
            path,
            loaded,
            notes,
        } = opened;
        let damaged = !loaded.issues.is_empty();
        let name = display_name(Some(&path));
        if damaged {
            self.report = Some(Report {
                heading: format!("Parts of “{name}” could not be read"),
                intro: Some(
                    "The rest of the model was opened. When you save it, the original file is \
                     kept next to it as a backup.",
                ),
                issues: loaded.issues.into_iter().chain(notes).collect(),
            });
        } else if !notes.is_empty() {
            self.report = Some(Report {
                heading: format!("Unsaved changes to “{name}” could not be recovered"),
                intro: None,
                issues: notes,
            });
        }
        self.remember(path.clone());
        model.replace(loaded.document, Some(path), damaged);
    }

    fn remember(&mut self, path: PathBuf) {
        self.change_recent(RecentChange::Opened(path));
    }

    fn forget(&mut self, path: &Path) {
        self.change_recent(RecentChange::Forgotten(path.to_path_buf()));
    }

    pub fn settings_loaded(&mut self, settings: Settings) {
        self.stored_settings = Some(settings);
    }

    pub fn store_settings(&mut self, settings: Settings) {
        let Some(config_dir) = self.config.config_dir.clone() else {
            return;
        };
        let since = self
            .stored_settings
            .replace(settings.clone())
            .unwrap_or_default();
        self.run_job(Box::new(move || {
            if let Err(error) = settings.save_changes(&config_dir, &since) {
                log::warn!("could not save the preferences: {error}");
            }
        }));
    }

    pub fn wait_for_jobs(&mut self, timeout: Duration) -> bool {
        if self.jobs.is_none() {
            return true;
        }
        let (done, finished) = mpsc::channel();
        self.run_job(Box::new(move || {
            done.send(()).ok();
        }));
        finished.recv_timeout(timeout).is_ok()
    }

    fn change_recent(&mut self, change: RecentChange) {
        self.recent.apply(&change);
        let Some(state_dir) = self.config.state_dir.clone() else {
            return;
        };
        self.run_job(Box::new(move || {
            if let Err(error) = RecentFiles::save_changes(&state_dir, &[change]) {
                log::warn!("could not remember recent files: {error}");
            }
        }));
    }

    fn spawn(
        &mut self,
        task: impl FnOnce() -> Event + Send + 'static,
        failed: impl FnOnce() -> Event + Send + 'static,
    ) {
        let events = self.events.clone();
        let wake = (self.make_waker)();
        self.run_job(Box::new(move || {
            let event = panic::catch_unwind(AssertUnwindSafe(task)).unwrap_or_else(|_| {
                log::error!("a background file task panicked");
                failed()
            });
            if events.send(event).is_ok() {
                wake();
            }
        }));
    }

    fn run_job(&mut self, job: Job) {
        if self.jobs.is_none() {
            self.jobs = spawn_worker();
        }
        let Some(jobs) = &self.jobs else {
            log::error!("no background worker, so the file task runs on the UI thread");
            run_contained(job);
            return;
        };
        if let Err(mpsc::SendError(job)) = jobs.send(job) {
            self.jobs = None;
            log::error!("the background worker stopped, so the file task runs on the UI thread");
            run_contained(job);
        }
    }
}

fn open_file(path: &Path, current: Option<&Path>, recovery_dir: Option<&Path>) -> OpenOutcome {
    if current == Some(path) {
        return OpenOutcome::AlreadyOpen;
    }
    let set_aside = match journal_for(path, recovery_dir) {
        FileJournal::InUse => return OpenOutcome::InUse,
        FileJournal::Recoverable(recovered) => return OpenOutcome::Recoverable(recovered),
        FileJournal::None => Vec::new(),
        FileJournal::SetAside(kept) => kept,
    };
    match load(path) {
        Ok(loaded) => OpenOutcome::Loaded {
            loaded,
            notes: set_aside
                .iter()
                .map(|kept| describe_set_aside(kept))
                .collect(),
        },
        Err(error) => OpenOutcome::Failed {
            error,
            missing: !path.exists(),
        },
    }
}

fn spawn_worker() -> Option<Sender<Job>> {
    let (jobs, queue) = mpsc::channel::<Job>();
    let spawned = thread::Builder::new()
        .name("files".to_owned())
        .spawn(move || {
            while let Ok(job) = queue.recv() {
                run_contained(job);
            }
        });
    match spawned {
        Ok(_) => Some(jobs),
        Err(error) => {
            log::error!("could not start the background file worker: {error}");
            None
        }
    }
}

fn internal_load_error() -> LoadError {
    LoadError::Unreadable(INTERNAL_ERROR.to_owned())
}

fn run_contained(job: Job) {
    if panic::catch_unwind(AssertUnwindSafe(job)).is_err() {
        log::error!("a background file task panicked");
    }
}

fn with_extension(path: PathBuf) -> PathBuf {
    let is_model = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(FILE_EXTENSION));
    if is_model {
        return path;
    }
    let mut named = OsString::from(path.as_os_str());
    named.push(".");
    named.push(FILE_EXTENSION);
    PathBuf::from(named)
}

fn check_save_target(
    picked: &Path,
    current: Option<&Path>,
    recovery_dir: Option<&Path>,
) -> SaveTarget {
    let named = with_extension(picked.to_path_buf());
    let target = canonical_location(&named);
    if current == Some(target.as_path()) {
        return SaveTarget::Ready(target);
    }
    match journal_for(&target, recovery_dir) {
        FileJournal::InUse => return SaveTarget::InUse(target),
        FileJournal::Recoverable(_) => return SaveTarget::HasRecovery(target),
        FileJournal::None => {}
        FileJournal::SetAside(kept) => {
            for kept in kept {
                log::warn!("an unreadable journal was kept as {}", kept.display());
            }
        }
    }
    if named != picked && target.exists() {
        SaveTarget::Confirm(target)
    } else {
        SaveTarget::Ready(target)
    }
}

fn canonical_location(path: &Path) -> PathBuf {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return path.to_path_buf();
    };
    let parent = if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    };
    fs::canonicalize(parent).map_or_else(|_| path.to_path_buf(), |parent| parent.join(name))
}

pub fn menu(
    ui: &mut Ui,
    model: &Model,
    files: &Files,
    editing: &SketchEditing,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let history = model
        .path()
        .map(|_| ())
        .ok_or("Save the model to start keeping its versions");
    let mut chosen = Vec::new();
    ui.menu_button("File", |ui| {
        let item = |ui: &mut Ui, chosen: &mut Vec<Command>, command: Command| {
            if menu_item(ui, commands, command).clicked() {
                chosen.push(command);
            }
        };
        item(ui, &mut chosen, Command::New);
        item(ui, &mut chosen, Command::Open);
        ui.add_enabled_ui(!files.recent().is_empty(), |ui| {
            let recent = ui.menu_button(submenu_label(ui, icons::RECENT, OPEN_RECENT), |ui| {
                for path in files.recent() {
                    let response = ui
                        .button(display_name(Some(path)))
                        .on_hover_text(path.display().to_string());
                    if response.clicked() {
                        actions.push(Action::File(FileCommand::OpenPath(path.clone())));
                    }
                }
            });
            widgets::named(recent.response, OPEN_RECENT);
        });
        let samples = ui.menu_button(submenu_label(ui, icons::SAMPLE, OPEN_SAMPLE), |ui| {
            for sample in Sample::ALL {
                let response = ui
                    .button(sample.title())
                    .on_hover_text(sample.description());
                if response.clicked() {
                    chosen.push(Command::OpenSample(sample));
                }
            }
        });
        widgets::named(samples.response, OPEN_SAMPLE);
        ui.separator();
        item(ui, &mut chosen, Command::Save);
        item(ui, &mut chosen, Command::SaveAs);
        ui.add_enabled_ui(history.is_ok(), |ui| {
            item(ui, &mut chosen, Command::VersionHistory);
        })
        .response
        .on_disabled_hover_text("Save the model to start keeping its versions.");
        ui.separator();
        let hints = [
            (Command::Import, Some(IMPORT_HINT)),
            (Command::Export, None),
            (Command::ExportImage, Some(IMAGE_HINT)),
        ];
        for (command, hint) in hints {
            let response = menu_item(ui, commands, command);
            let response = match hint {
                Some(hint) => response.on_hover_text(hint),
                None => response,
            };
            if response.clicked() {
                chosen.push(command);
            }
        }
        if files.has_recoverable() {
            ui.separator();
            item(ui, &mut chosen, Command::RecoverUnsaved);
        }
        ui.separator();
        item(ui, &mut chosen, Command::Preferences);
        item(ui, &mut chosen, Command::KeyboardShortcuts);
        ui.separator();
        item(ui, &mut chosen, Command::Quit);
    });
    let recoverable = if files.has_recoverable() {
        Ok(())
    } else {
        Err("There is no unsaved work to recover")
    };
    if commands.invoke(Command::RecoverUnsaved, &recoverable)
        || chosen.contains(&Command::RecoverUnsaved)
    {
        actions.push(Action::File(FileCommand::ShowRecovery));
    }
    for slot in RecentSlot::ALL {
        let command = Command::OpenRecent(slot);
        match files.recent().get(slot.index()) {
            Some(path) => {
                let detail = Some(display_name(Some(path)));
                if commands.invoke_detailed(command, detail, &Ok::<(), String>(())) {
                    actions.push(Action::File(FileCommand::OpenPath(path.clone())));
                }
            }
            None => {
                if commands.take(command) {
                    actions.push(Action::Inform(Notice::info(format!(
                        "{}: there are not that many recent models",
                        command.title()
                    ))));
                }
            }
        }
    }
    for sample in Sample::ALL {
        let command = Command::OpenSample(sample);
        if commands.available(command) || chosen.contains(&command) {
            actions.push(Action::File(FileCommand::OpenSample(sample)));
        }
    }
    for command in FILE_COMMANDS {
        let availability = match command {
            Command::VersionHistory => history,
            _ => Ok(()),
        };
        let invoked = commands.invoke(command, &availability);
        if !invoked && !chosen.contains(&command) {
            continue;
        }
        let action = match command {
            Command::New => Action::File(FileCommand::New),
            Command::Open => Action::File(FileCommand::Open),
            Command::Save => Action::File(FileCommand::Save),
            Command::SaveAs => Action::File(FileCommand::SaveAs),
            Command::VersionHistory => Action::File(FileCommand::History(HistoryCommand::Show)),
            Command::Import => Action::File(FileCommand::Import {
                into: editing.feature(),
            }),
            Command::Export => Action::File(FileCommand::Export(ExportCommand::Show)),
            Command::ExportImage => Action::File(FileCommand::ExportImage(ImageCommand::Show)),
            Command::Preferences => Action::Preferences(PreferencesCommand::Show),
            _ => Action::File(FileCommand::Quit),
        };
        actions.push(action);
    }
    if chosen.contains(&Command::KeyboardShortcuts) {
        commands.trigger(Command::KeyboardShortcuts);
    }
}

pub fn activity(
    ui: &mut Ui,
    model: &Model,
    files: &Files,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    if let Some(reason) = model.unprotected() {
        widgets::pill(ui, Tone::Warning, "Not protected").on_hover_text(format!(
            "Unsaved changes are not protected against a crash: {reason}. caditor keeps trying; \
             saving keeps your work safe."
        ));
    }
    if model.is_saving() {
        ui.spinner();
        ui.label("Saving…");
    }
    if let Some(Importing {
        path: Some(path), ..
    }) = &files.importing
    {
        ui.spinner();
        ui.label(format!("Importing “{}”…", display_name(Some(path))));
    }
    export::activity(ui, &files.exporter, commands, actions);
    image_export::activity(ui, &files.image, commands, actions);
}

fn is_model_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(FILE_EXTENSION))
}

fn menu_item(ui: &mut Ui, commands: &CommandFrame<'_>, command: Command) -> egui::Response {
    widgets::menu_item(
        ui,
        icons::command(command),
        &command.title(),
        commands.keys(command),
    )
}

fn submenu_label(ui: &Ui, glyph: &str, title: &str) -> (egui::RichText, String) {
    (
        widgets::icon(glyph).color(appearance::tokens(ui).text_muted),
        title.to_owned(),
    )
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    files: &Files,
    view: Option<SurfaceSize>,
    actions: &mut Vec<Action>,
) {
    let ctx = ui.ctx().clone();
    let mut command = None;
    if let Some((_, since)) = &files.closing {
        command = closing(&ctx, *since);
    } else if let Some(path) = &files.opening {
        Modal::new(Id::new("opening"))
            .frame(widgets::dialog_frame(&ctx))
            .show(&ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!("Opening “{}”…", display_name(Some(path))));
                });
            });
    } else if let Some(intent) = &files.guard {
        command = guard(&ctx, model, intent).map(FileCommand::Guard);
    } else if let Some(path) = &files.confirm_replace {
        command = confirm_replace(&ctx, path).map(FileCommand::Replace);
    } else if let Some(report) = &files.report {
        command = show_report(&ctx, report);
    } else if files.showing_recovery() {
        command = recovery(&ctx, files);
    } else if files.exporter.is_open() {
        command = export::dialog(&ctx, model, &files.exporter).map(FileCommand::Export);
    } else if files.image.is_open() {
        command = image_export::dialog(&ctx, &files.image, view).map(FileCommand::ExportImage);
    } else if files.history.is_open() {
        command = history::dialog(&ctx, model, &files.history).map(FileCommand::History);
    }
    if let Some(command) = command {
        actions.push(Action::File(command));
    }
}

fn guard(ctx: &egui::Context, model: &Model, intent: &Intent) -> Option<GuardChoice> {
    let name = model.display_name();
    let consequence = match intent {
        Intent::Quit => "If you close without saving, your changes will be lost.",
        Intent::New
        | Intent::Sample(_)
        | Intent::Open(_)
        | Intent::Restore(_)
        | Intent::Replace(_) => "If you continue without saving, your changes will be lost.",
    };
    let (discard, keep) = match intent {
        Intent::Quit => ("Close Without Saving", "Cancel"),
        Intent::New
        | Intent::Sample(_)
        | Intent::Open(_)
        | Intent::Restore(_)
        | Intent::Replace(_) => ("Continue Without Saving", "Cancel"),
    };
    let save = if model.path().is_some() {
        "Save"
    } else {
        "Save As…"
    };
    let title = format!("Save changes to “{name}”?");
    let response = widgets::dialog(ctx, "unsaved-changes", &title, DialogWidth::Medium, |ui| {
        ui.label(consequence);
        widgets::footer(ui, |ui| {
            if ui.add(widgets::primary_button(ui, save)).clicked() {
                return Some(GuardChoice::Save);
            }
            if ui.button(discard).clicked() {
                return Some(GuardChoice::DontSave);
            }
            ui.button(keep).clicked().then_some(GuardChoice::Cancel)
        })
    });
    let closed = response.should_close().then_some(GuardChoice::Cancel);
    response.inner.or(closed)
}

fn closing(ctx: &egui::Context, since: Instant) -> Option<FileCommand> {
    let waited = since.elapsed();
    if waited < QUIT_ANYWAY_AFTER {
        ctx.request_repaint_after(QUIT_ANYWAY_AFTER.saturating_sub(waited));
    }
    Modal::new(Id::new("closing"))
        .frame(widgets::dialog_frame(ctx))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Finishing writing to disk…");
            });
            if waited < QUIT_ANYWAY_AFTER {
                return None;
            }
            ui.label("This is taking longer than usual, for example because the disk is slow.");
            widgets::footer(ui, |ui| {
                ui.button("Quit Anyway")
                    .on_hover_text(
                        "The changes you chose not to save may be offered for recovery the next \
                         time caditor starts.",
                    )
                    .clicked()
                    .then_some(FileCommand::QuitAnyway)
            })
        })
        .inner
}

fn confirm_replace(ctx: &egui::Context, path: &Path) -> Option<bool> {
    let name = display_name(Some(path));
    let title = format!("Replace “{name}”?");
    let response = widgets::dialog(ctx, "replace-model", &title, DialogWidth::Medium, |ui| {
        let folder = path
            .parent()
            .map(|folder| folder.display().to_string())
            .unwrap_or_default();
        ui.label(format!(
            "A model named “{name}” already exists in “{folder}”. Replacing it overwrites what \
             it holds."
        ));
        widgets::footer(ui, |ui| {
            if ui.add(widgets::primary_button(ui, "Replace")).clicked() {
                return Some(true);
            }
            ui.button("Cancel").clicked().then_some(false)
        })
    });
    let closed = response.should_close().then_some(false);
    response.inner.or(closed)
}

fn show_report(ctx: &egui::Context, report: &Report) -> Option<FileCommand> {
    let response = widgets::dialog(
        ctx,
        "file-report",
        &report.heading,
        DialogWidth::Medium,
        |ui| {
            if let Some(intro) = report.intro {
                ui.label(intro);
            }
            egui::ScrollArea::vertical()
                .max_height(widgets::list_height(ui.ctx(), REPORT_HEIGHT))
                .min_scrolled_height(widgets::list_height(ui.ctx(), REPORT_HEIGHT))
                .show(ui, |ui| {
                    for issue in &report.issues {
                        widgets::callout(ui, Tone::Info, |ui| {
                            ui.label(issue);
                        });
                    }
                });
            widgets::footer(ui, |ui| ui.add(widgets::primary_button(ui, "OK")).clicked())
        },
    );
    (response.inner || response.should_close()).then_some(FileCommand::DismissReport)
}

fn recovery(ctx: &egui::Context, files: &Files) -> Option<FileCommand> {
    let response = widgets::dialog(
        ctx,
        "recovery",
        "Recover unsaved work",
        DialogWidth::Medium,
        |ui| {
            ui.label("caditor closed before these changes were saved.");
            let mut command = None;
            for candidate in &files.recoverable {
                widgets::card(ui, |ui| {
                    if let Some(chosen) = recovery_row(ui, files, &candidate.recovered) {
                        command = Some(chosen);
                    }
                });
            }
            widgets::footer(ui, |ui| {
                if ui
                    .button("Decide Later")
                    .on_hover_text(
                        "These changes will be offered again the next time caditor starts.",
                    )
                    .clicked()
                {
                    command = Some(FileCommand::HideRecovery);
                }
            });
            command
        },
    );
    let closed = response.should_close().then_some(FileCommand::HideRecovery);
    response.inner.or(closed)
}

fn recovery_row(ui: &mut Ui, files: &Files, recovered: &Recovered) -> Option<FileCommand> {
    let name = match &recovered.file {
        Some(file) => display_name(Some(file)),
        None => "Untitled model".to_owned(),
    };
    ui.label(RichText::new(name).strong());
    let changes = match recovered.changes() {
        1 => "1 unsaved change".to_owned(),
        count => format!("{count} unsaved changes"),
    };
    let when = recovered
        .modified
        .map(|modified| format!(", last one {}", history::ago(modified)))
        .unwrap_or_default();
    ui.label(widgets::muted(format!("{changes}{when}"), ui));
    if let Some(file) = &recovered.file {
        ui.label(widgets::muted(file.display().to_string(), ui));
    }
    for issue in &recovered.issues {
        widgets::callout(ui, Tone::Warning, |ui| {
            ui.label(issue);
        });
    }
    let journal = recovered.journal.clone();
    ui.horizontal(|ui| {
        if files.confirm_discard.as_ref() == Some(&journal) {
            ui.label("Discard these changes permanently?");
            if ui.button("Discard").clicked() {
                return Some(FileCommand::Discard(journal));
            }
            return ui
                .button("Keep")
                .clicked()
                .then_some(FileCommand::KeepRecovered);
        }
        if ui.add(widgets::primary_button(ui, "Restore")).clicked() {
            return Some(FileCommand::Restore(journal));
        }
        ui.button("Discard…")
            .clicked()
            .then_some(FileCommand::AskDiscard(journal))
    })
    .inner
}
