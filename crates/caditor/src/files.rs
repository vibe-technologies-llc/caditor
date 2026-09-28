use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::Duration,
};

use caditor_document::{Document, FeatureId};
use caditor_file::{
    DXF_EXTENSION, Drawing, ExportError, ExportFormat, Exported, FILE_EXTENSION, FileJournal,
    History, ImportError, LoadError, Loaded, ModelImport, RecentFiles, Recovered, STEP_EXTENSIONS,
    STEP_IMPORT_EXTENSIONS, SavedState, Settings, journal_for, load, load_version, read_dxf,
    read_step_file, scan,
};
use egui::{Button, Id, Modal, RichText, Ui};
use parking_lot::Mutex;

use crate::{
    commands::{Command, CommandFrame},
    editing::SketchEditing,
    export::{self, ExportCommand, Exporter},
    history::{self, HistoryCommand, VersionHistory},
    import::{self, IMPORT_HINT},
    model::{Action, FileEvent, Model, Notice, WakerFactory, display_name},
    preferences::PreferencesCommand,
};

const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
const DIALOG_WIDTH: f32 = 420.0;
const MODEL_KIND: &str = "caditor model";
const DRAWING_KIND: &str = "DXF drawing";
const MODEL_EXCHANGE_KIND: &str = "STEP model";
const IMPORTABLE_KIND: &str = "Drawings and models";
const FILE_COMMANDS: [Command; 9] = [
    Command::New,
    Command::Open,
    Command::Save,
    Command::SaveAs,
    Command::VersionHistory,
    Command::Import,
    Command::Export,
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
    Export(ExportCommand),
    History(HistoryCommand),
    Import { into: Option<FeatureId> },
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
    Import,
}

enum OpenOutcome {
    Loaded(Loaded),
    Recoverable(Box<Recovered>),
    AlreadyOpen,
    InUse,
    Failed { error: LoadError, missing: bool },
}

struct Opened {
    path: PathBuf,
    loaded: Loaded,
}

enum Event {
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
        result: Result<Drawing, ImportError>,
    },
    ImportedModel {
        path: PathBuf,
        session: u64,
        result: Result<ModelImport, ImportError>,
    },
}

enum Intent {
    New,
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
    exporter: Exporter,
    history: VersionHistory,
    picking: bool,
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
            exporter: Exporter::default(),
            history: VersionHistory::default(),
            picking: false,
            quit: false,
        }
    }

    pub fn start(&mut self, open: Option<PathBuf>, model: &mut Model) {
        let state_dir = self.config.state_dir.clone();
        let recovery_dir = self.config.recovery_dir.clone();
        self.spawn(move || {
            let recent = state_dir
                .as_deref()
                .map(RecentFiles::load)
                .unwrap_or_default();
            let recovered = scan(recovery_dir.as_deref(), recent.paths());
            Event::Started { recent, recovered }
        });
        if let Some(path) = open {
            self.perform(FileCommand::OpenPath(path), model);
        }
    }

    pub fn should_quit(&self) -> bool {
        self.quit
    }

    pub fn is_blocking(&self) -> bool {
        self.guard.is_some()
            || self.opening.is_some()
            || self.report.is_some()
            || self.showing_recovery()
            || self.exporter.is_open()
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
            FileCommand::Open => self.request(Intent::Open(None), model),
            FileCommand::OpenPath(path) => self.request(Intent::Open(Some(path)), model),
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
                self.spawn(move || Event::Discarded {
                    error: caditor_file::discard(&journal)
                        .err()
                        .map(|error| error.to_string()),
                    journal,
                });
            }
            FileCommand::DismissReport => self.report = None,
            FileCommand::Export(command) => {
                self.exporter.perform(command);
                if command == ExportCommand::Choose {
                    self.pick(Purpose::Export(self.exporter.format()), model);
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
                    self.spawn(move || Event::VersionLoaded {
                        result: load_version(&path, index),
                        path,
                        state,
                    });
                }
            }
        }
    }

    fn list_versions(&mut self) {
        if let Some(path) = self.history.path().cloned() {
            self.spawn(move || Event::HistoryListed {
                result: caditor_file::history(&path),
                path,
            });
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
            return;
        }
        let described = history::describe(state);
        match result {
            Ok(loaded) if loaded.document.same_content(model.document()) => {
                model.set_notice(Notice::info("That version is the same as the model now."));
            }
            Ok(loaded) => {
                let transaction = model
                    .document()
                    .transaction_to(&loaded.document, "Restore earlier version");
                model.perform(Action::Apply(transaction));
                self.history.close();
                model.set_notice(Notice::info(format!(
                    "Restored the version {}. Undo brings back what you had.",
                    described.to_lowercase()
                )));
            }
            Err(error) => model.set_notice(Notice::error(format!(
                "Could not restore that version: {error}."
            ))),
        }
    }

    pub fn poll(&mut self, model: &mut Model, editing: &mut SketchEditing) -> bool {
        let mut changed = false;
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
        changed
    }

    fn handle(&mut self, event: Event, model: &mut Model, editing: &mut SketchEditing) {
        match event {
            Event::Picked { purpose, path } => {
                self.picking = false;
                match (purpose, path) {
                    (Purpose::Open, Some(path)) => self.open(path, model),
                    (Purpose::SaveAs, Some(path)) => model.save_to(with_extension(path)),
                    (Purpose::Export(format), Some(path)) => self.export(path, format, model),
                    (Purpose::Import, Some(path)) => self.import(path, model),
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
                    (Some(error), _) => model.set_notice(Notice::error(format!(
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
                if let Some(report) = import::place_drawing(model, editing, &path, into, result) {
                    self.report = Some(Report {
                        heading: report.heading,
                        intro: None,
                        issues: report.notes,
                    });
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

    fn import(&mut self, path: PathBuf, model: &Model) {
        let into = self.importing.as_ref().and_then(|importing| importing.into);
        self.importing = Some(Importing {
            path: Some(path.clone()),
            into,
        });
        let session = model.session();
        self.spawn(move || {
            if import::is_model(&path) {
                Event::ImportedModel {
                    result: read_step_file(&path),
                    path,
                    session,
                }
            } else {
                Event::Imported {
                    result: read_dxf(&path),
                    path,
                    session,
                    into,
                }
            }
        });
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
            OpenOutcome::Loaded(loaded) => {
                let opened = Opened { path, loaded };
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
            OpenOutcome::InUse => model.set_notice(Notice::error(format!(
                "“{name}” is already open in another caditor window."
            ))),
            OpenOutcome::Failed { error, missing } => {
                if missing {
                    self.forget(&path);
                }
                model.set_notice(Notice::error(format!("Could not open “{name}”: {error}.")));
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
            Intent::Quit => {
                if let Some(closing) = model.close()
                    && !closing.wait(CLOSE_TIMEOUT)
                {
                    log::warn!("the storage worker did not finish before quitting");
                }
                self.quit = true;
            }
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
            Purpose::Import => self.dialogs.pick_import(directory, respond),
        }
    }

    fn open(&mut self, path: PathBuf, model: &mut Model) {
        self.opening = Some(path.clone());
        let revision = model.revision();
        let current = model.path().map(Path::to_path_buf);
        let recovery_dir = self.config.recovery_dir.clone();
        self.spawn(move || {
            let path = fs::canonicalize(&path).unwrap_or(path);
            let outcome = open_file(&path, current.as_deref(), recovery_dir.as_deref());
            Event::Opened {
                path,
                revision,
                outcome,
            }
        });
    }

    fn finish_open(&mut self, opened: Opened, model: &mut Model) {
        let Opened { path, loaded } = opened;
        let damaged = !loaded.issues.is_empty();
        if damaged {
            self.report = Some(Report {
                heading: format!("Parts of “{}” could not be read", display_name(Some(&path))),
                intro: Some(
                    "The rest of the model was opened. When you save it, the original file is \
                     kept next to it as a backup.",
                ),
                issues: loaded.issues,
            });
        }
        self.remember(path.clone());
        model.replace(loaded.document, Some(path), damaged);
    }

    fn remember(&mut self, path: PathBuf) {
        self.recent.add(path);
        self.store_recent();
    }

    fn forget(&mut self, path: &Path) {
        self.recent.remove(path);
        self.store_recent();
    }

    pub fn store_settings(&mut self, settings: Settings) {
        let Some(config_dir) = self.config.config_dir.clone() else {
            return;
        };
        self.run_job(Box::new(move || {
            if let Err(error) = settings.save(&config_dir) {
                log::warn!("could not save the preferences: {error}");
            }
        }));
    }

    fn store_recent(&mut self) {
        let Some(state_dir) = self.config.state_dir.clone() else {
            return;
        };
        let recent = self.recent.clone();
        self.run_job(Box::new(move || {
            if let Err(error) = recent.save(&state_dir) {
                log::warn!("could not remember recent files: {error}");
            }
        }));
    }

    fn spawn(&mut self, task: impl FnOnce() -> Event + Send + 'static) {
        let events = self.events.clone();
        let wake = (self.make_waker)();
        self.run_job(Box::new(move || {
            if events.send(task()).is_ok() {
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
            job();
            return;
        };
        if let Err(mpsc::SendError(job)) = jobs.send(job) {
            self.jobs = None;
            log::error!("the background worker stopped, so the file task runs on the UI thread");
            job();
        }
    }
}

fn open_file(path: &Path, current: Option<&Path>, recovery_dir: Option<&Path>) -> OpenOutcome {
    if current == Some(path) {
        return OpenOutcome::AlreadyOpen;
    }
    match journal_for(path, recovery_dir) {
        FileJournal::InUse => OpenOutcome::InUse,
        FileJournal::Recoverable(recovered) => OpenOutcome::Recoverable(recovered),
        FileJournal::None => match load(path) {
            Ok(loaded) => OpenOutcome::Loaded(loaded),
            Err(error) => OpenOutcome::Failed {
                error,
                missing: !path.exists(),
            },
        },
    }
}

fn spawn_worker() -> Option<Sender<Job>> {
    let (jobs, queue) = mpsc::channel::<Job>();
    let spawned = thread::Builder::new()
        .name("files".to_owned())
        .spawn(move || {
            while let Ok(job) = queue.recv() {
                job();
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

fn with_extension(path: PathBuf) -> PathBuf {
    if path.extension().is_some() {
        path
    } else {
        path.with_extension(FILE_EXTENSION)
    }
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
            ui.menu_button("Open Recent", |ui| {
                for path in files.recent() {
                    let response = ui
                        .button(display_name(Some(path)))
                        .on_hover_text(path.display().to_string());
                    if response.clicked() {
                        actions.push(Action::File(FileCommand::OpenPath(path.clone())));
                    }
                }
            });
        });
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
            if ui.button("Recover Unsaved Work…").clicked() {
                actions.push(Action::File(FileCommand::ShowRecovery));
            }
        }
        ui.separator();
        item(ui, &mut chosen, Command::Preferences);
        item(ui, &mut chosen, Command::KeyboardShortcuts);
        ui.separator();
        item(ui, &mut chosen, Command::Quit);
    });
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
            Command::Preferences => Action::Preferences(PreferencesCommand::Show),
            _ => Action::File(FileCommand::Quit),
        };
        actions.push(action);
    }
    if chosen.contains(&Command::KeyboardShortcuts) {
        commands.trigger(Command::KeyboardShortcuts);
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
    export::menu_status(ui, &files.exporter, actions);
}

fn menu_item(ui: &mut Ui, commands: &CommandFrame<'_>, command: Command) -> egui::Response {
    let mut button = Button::new(command.title());
    if let Some(keys) = commands.keys(command) {
        button = button.shortcut_text(keys);
    }
    ui.add(button)
}

pub fn show(ui: &mut Ui, model: &Model, files: &Files, actions: &mut Vec<Action>) {
    let ctx = ui.ctx().clone();
    let mut command = None;
    if let Some(path) = &files.opening {
        Modal::new(Id::new("opening")).show(&ctx, |ui| {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!("Opening “{}”…", display_name(Some(path))));
            });
        });
    } else if let Some(intent) = &files.guard {
        command = guard(&ctx, model, intent).map(FileCommand::Guard);
    } else if let Some(report) = &files.report {
        command = show_report(&ctx, report);
    } else if files.showing_recovery() {
        command = recovery(&ctx, files);
    } else if files.exporter.is_open() {
        command = export::dialog(&ctx, model, &files.exporter).map(FileCommand::Export);
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
        Intent::New | Intent::Open(_) | Intent::Restore(_) | Intent::Replace(_) => {
            "If you continue without saving, your changes will be lost."
        }
    };
    let (discard, keep) = match intent {
        Intent::Quit => ("Close Without Saving", "Cancel"),
        Intent::New | Intent::Open(_) | Intent::Restore(_) | Intent::Replace(_) => {
            ("Continue Without Saving", "Cancel")
        }
    };
    let save = if model.path().is_some() {
        "Save"
    } else {
        "Save As…"
    };
    let response = Modal::new(Id::new("unsaved-changes")).show(ctx, |ui| {
        ui.set_max_width(DIALOG_WIDTH);
        ui.heading(format!("Save changes to “{name}”?"));
        ui.label(consequence);
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(save).clicked() {
                return Some(GuardChoice::Save);
            }
            if ui.button(discard).clicked() {
                return Some(GuardChoice::DontSave);
            }
            ui.button(keep).clicked().then_some(GuardChoice::Cancel)
        })
        .inner
    });
    let closed = response.should_close().then_some(GuardChoice::Cancel);
    response.inner.or(closed)
}

fn show_report(ctx: &egui::Context, report: &Report) -> Option<FileCommand> {
    let response = Modal::new(Id::new("file-report")).show(ctx, |ui| {
        ui.set_max_width(DIALOG_WIDTH);
        ui.heading(&report.heading);
        if let Some(intro) = report.intro {
            ui.label(intro);
        }
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .max_height(240.0)
            .show(ui, |ui| {
                for issue in &report.issues {
                    ui.label(format!("• {issue}"));
                }
            });
        ui.add_space(8.0);
        ui.button("OK").clicked()
    });
    (response.inner || response.should_close()).then_some(FileCommand::DismissReport)
}

fn recovery(ctx: &egui::Context, files: &Files) -> Option<FileCommand> {
    let response = Modal::new(Id::new("recovery")).show(ctx, |ui| {
        ui.set_max_width(DIALOG_WIDTH);
        ui.heading("Recover unsaved work");
        ui.label("caditor closed before these changes were saved.");
        let mut command = None;
        for candidate in &files.recoverable {
            ui.separator();
            if let Some(chosen) = recovery_row(ui, files, &candidate.recovered) {
                command = Some(chosen);
            }
        }
        ui.separator();
        if ui
            .button("Decide Later")
            .on_hover_text("These changes will be offered again the next time caditor starts.")
            .clicked()
        {
            command = Some(FileCommand::HideRecovery);
        }
        command
    });
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
    ui.weak(format!("{changes}{when}"));
    if let Some(file) = &recovered.file {
        ui.weak(file.display().to_string());
    }
    for issue in &recovered.issues {
        ui.colored_label(ui.visuals().warn_fg_color, issue);
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
        if ui.button("Restore").clicked() {
            return Some(FileCommand::Restore(journal));
        }
        ui.button("Discard…")
            .clicked()
            .then_some(FileCommand::AskDiscard(journal))
    })
    .inner
}
