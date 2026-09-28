use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, SystemTime},
};

use caditor_document::Document;
use caditor_file::{
    ExportError, Exported, FILE_EXTENSION, FileJournal, LoadError, Loaded, MeshFormat, RecentFiles,
    Recovered, journal_for, load, scan,
};
use egui::{Button, Id, KeyboardShortcut, Modal, Modifiers, RichText, Ui};
use parking_lot::Mutex;

use crate::{
    export::{self, EXPORT, ExportCommand, Exporter},
    model::{Action, FileEvent, Model, Notice, WakerFactory, display_name},
};

const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
const DIALOG_WIDTH: f32 = 420.0;
const MODEL_KIND: &str = "caditor model";
pub const NEW: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, egui::Key::N);
pub const OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, egui::Key::O);
pub const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, egui::Key::S);
pub const SAVE_AS: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), egui::Key::S);
pub const QUIT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, egui::Key::Q);

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
        format: MeshFormat,
        respond: Respond,
    );
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

    fn dialog(directory: Option<PathBuf>, kind: &str, extension: &str) -> rfd::FileDialog {
        let dialog = rfd::FileDialog::new().add_filter(kind, &[extension]);
        match directory {
            Some(directory) => dialog.set_directory(directory),
            None => dialog,
        }
    }
}

impl Dialogs for NativeDialogs {
    fn pick_model(&self, directory: Option<PathBuf>, respond: Respond) {
        Self::spawn(respond, move || {
            Self::dialog(directory, MODEL_KIND, FILE_EXTENSION)
                .set_title("Open Model")
                .pick_file()
        });
    }

    fn pick_save_path(&self, directory: Option<PathBuf>, file_name: String, respond: Respond) {
        Self::spawn(respond, move || {
            Self::dialog(directory, MODEL_KIND, FILE_EXTENSION)
                .set_title("Save Model")
                .set_file_name(file_name)
                .save_file()
        });
    }

    fn pick_export_path(
        &self,
        directory: Option<PathBuf>,
        file_name: String,
        format: MeshFormat,
        respond: Respond,
    ) {
        Self::spawn(respond, move || {
            Self::dialog(directory, format.name(), format.extension())
                .set_title(format!("Export {}", format.name()))
                .set_file_name(file_name)
                .save_file()
        });
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilesConfig {
    pub state_dir: Option<PathBuf>,
    pub recovery_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Purpose {
    Open,
    SaveAs,
    Export(MeshFormat),
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

struct LoadReport {
    name: String,
    issues: Vec<String>,
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
    report: Option<LoadReport>,
    opening: Option<PathBuf>,
    exporter: Exporter,
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
            exporter: Exporter::default(),
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
        }
    }

    pub fn poll(&mut self, model: &mut Model) -> bool {
        let mut changed = false;
        for event in model.take_file_events() {
            changed = true;
            match event {
                FileEvent::Saved(path) => {
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
            self.handle(event, model);
        }
        changed
    }

    fn handle(&mut self, event: Event, model: &mut Model) {
        match event {
            Event::Picked { purpose, path } => {
                self.picking = false;
                match (purpose, path) {
                    (Purpose::Open, Some(path)) => self.open(path, model),
                    (Purpose::SaveAs, Some(path)) => model.save_to(with_extension(path)),
                    (Purpose::Export(format), Some(path)) => self.export(path, format, model),
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
        }
    }

    fn export(&mut self, path: PathBuf, format: MeshFormat, model: &mut Model) {
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
            self.report = Some(LoadReport {
                name: display_name(Some(&path)),
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

pub fn menu(ui: &mut Ui, model: &Model, files: &Files, actions: &mut Vec<Action>) {
    ui.menu_button("File", |ui| {
        let mut command = None;
        let mut item = |ui: &mut Ui, text: &str, shortcut: Option<KeyboardShortcut>, chosen| {
            if menu_item(ui, text, shortcut) {
                command = Some(chosen);
            }
        };
        item(ui, "New", Some(NEW), FileCommand::New);
        item(ui, "Open…", Some(OPEN), FileCommand::Open);
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
        item(ui, "Save", Some(SAVE), FileCommand::Save);
        item(ui, "Save As…", Some(SAVE_AS), FileCommand::SaveAs);
        item(
            ui,
            "Export…",
            Some(EXPORT),
            FileCommand::Export(ExportCommand::Show),
        );
        if files.has_recoverable() {
            ui.separator();
            item(ui, "Recover Unsaved Work…", None, FileCommand::ShowRecovery);
        }
        ui.separator();
        item(ui, "Quit", Some(QUIT), FileCommand::Quit);
        if let Some(command) = command {
            actions.push(Action::File(command));
        }
    });
    if model.is_saving() {
        ui.spinner();
        ui.label("Saving…");
    }
    export::menu_status(ui, &files.exporter, actions);
}

fn menu_item(ui: &mut Ui, text: &str, shortcut: Option<KeyboardShortcut>) -> bool {
    let mut button = Button::new(text);
    if let Some(shortcut) = shortcut {
        button = button.shortcut_text(ui.ctx().format_shortcut(&shortcut));
    }
    ui.add(button).clicked()
}

pub fn shortcuts(ui: &mut Ui, actions: &mut Vec<Action>) {
    let commands = [
        (SAVE_AS, FileCommand::SaveAs),
        (SAVE, FileCommand::Save),
        (NEW, FileCommand::New),
        (OPEN, FileCommand::Open),
        (EXPORT, FileCommand::Export(ExportCommand::Show)),
        (QUIT, FileCommand::Quit),
    ];
    for (shortcut, command) in commands {
        if ui.input_mut(|input| input.consume_shortcut(&shortcut)) {
            actions.push(Action::File(command));
            return;
        }
    }
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
        command = load_report(&ctx, report);
    } else if files.showing_recovery() {
        command = recovery(&ctx, files);
    } else if files.exporter.is_open() {
        command = export::dialog(&ctx, model, &files.exporter).map(FileCommand::Export);
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

fn load_report(ctx: &egui::Context, report: &LoadReport) -> Option<FileCommand> {
    let response = Modal::new(Id::new("load-report")).show(ctx, |ui| {
        ui.set_max_width(DIALOG_WIDTH);
        ui.heading(format!("Parts of “{}” could not be read", report.name));
        ui.label(
            "The rest of the model was opened. When you save it, the original file is kept \
             next to it as a backup.",
        );
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
        .map(|modified| format!(", last one {}", ago(modified)))
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

fn ago(time: SystemTime) -> String {
    let seconds = SystemTime::now()
        .duration_since(time)
        .unwrap_or_default()
        .as_secs();
    let (count, unit) = match seconds {
        0..60 => return "just now".to_owned(),
        60..3600 => (seconds / 60, "minute"),
        3600..86400 => (seconds / 3600, "hour"),
        _ => (seconds / 86400, "day"),
    };
    match count {
        1 => format!("1 {unit} ago"),
        count => format!("{count} {unit}s ago"),
    }
}
