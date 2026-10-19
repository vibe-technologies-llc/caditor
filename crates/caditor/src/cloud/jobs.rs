use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
    thread,
    time::{Duration, Instant},
};

use caditor_cloud::{
    Account, Client, Conversion, Design, DesignId, Error, Folder, FolderRef, Listener, Outgoing,
    Placement, Project, ProjectId, Provider, SendAs, SendOutcome, Sent, Service, Session,
    Transport, Version, Workspace, WorkspaceId,
};
use parking_lot::Mutex;

use super::{
    downloads::{self, KEPT_DESIGNS, Usage},
    failure::Failure,
    state::{ImportItem, SendStage, Shipment, Stage},
    store::SessionStore,
    thumbnails::{self, Thumbnail},
};
use crate::{browser::OpenError, model::Waker};

const SIGN_IN_PATIENCE: Duration = Duration::from_secs(5 * 60);
const CONVERSION_PATIENCE: Duration = Duration::from_secs(10 * 60);
const POLL_EVERY: Duration = Duration::from_secs(2);
const STOP_CHECK: Duration = Duration::from_millis(100);
const PROGRESS_EVERY: Duration = Duration::from_millis(120);

pub type Opener = Arc<dyn Fn(&str) -> Result<(), OpenError> + Send + Sync>;

pub enum Loaded {
    Account(Result<Account, Arc<Failure>>),
    Workspaces(Result<(Vec<Workspace>, Option<WorkspaceId>), Arc<Failure>>),
    Projects(WorkspaceId, Result<Vec<Project>, Arc<Failure>>),
    Root(ProjectId, Result<Folder, Arc<Failure>>),
    Folder(FolderRef, Result<Folder, Arc<Failure>>),
    Thumbnail(DesignId, Result<Option<Arc<Thumbnail>>, Arc<Failure>>),
    Versions(DesignId, Result<Vec<Version>, Arc<Failure>>),
}

pub enum Event {
    Restored {
        service: Service,
        outcome: Result<Option<Session>, Failure>,
    },
    SignInUrl {
        service: Service,
        attempt: u64,
        url: String,
    },
    SignedIn {
        service: Service,
        attempt: u64,
        outcome: Result<Session, Failure>,
    },
    Loaded {
        service: Service,
        generation: u64,
        loaded: Loaded,
    },
    Progress {
        service: Service,
        generation: u64,
        import: u64,
        stage: Stage,
    },
    Imported {
        service: Service,
        generation: u64,
        import: u64,
        outcome: Result<PathBuf, Failure>,
    },
    SendProgress {
        service: Service,
        generation: u64,
        send: u64,
        stage: SendStage,
    },
    Delivered {
        service: Service,
        generation: u64,
        send: u64,
        outcome: Result<Sent, Failure>,
    },
    Left,
    Measured(Result<Usage, Failure>),
    Cleared(Result<(), Failure>),
}

#[derive(Clone)]
pub struct Courier {
    sender: Sender<Event>,
    waker: Arc<Mutex<Waker>>,
}

impl Courier {
    pub fn new(sender: Sender<Event>, waker: Waker) -> Self {
        Self {
            sender,
            waker: Arc::new(Mutex::new(waker)),
        }
    }

    pub fn send(&self, event: Event) {
        if self.sender.send(event).is_ok() {
            (self.waker.lock())();
        }
    }

    pub fn spawn<T: Send + 'static>(
        &self,
        service: Service,
        work: impl FnOnce() -> Result<T, Failure> + Send + 'static,
        event: impl Fn(Result<T, Failure>) -> Event + Send + Sync + 'static,
    ) {
        let event = Arc::new(event);
        let courier = self.clone();
        let on_thread = Arc::clone(&event);
        let spawned = thread::Builder::new()
            .name("cloud".to_owned())
            .spawn(move || {
                let result =
                    catch_unwind(AssertUnwindSafe(work)).unwrap_or(Err(Failure::Crashed(service)));
                courier.send(on_thread(result));
            });
        if let Err(error) = spawned {
            log::warn!("could not start a cloud job: {error}");
            self.send(event(Err(Failure::Spawn(error.kind()))));
        }
    }
}

pub struct SignInJob {
    pub service: Service,
    pub attempt: u64,
    pub provider: Arc<dyn Provider>,
    pub transport: Arc<dyn Transport>,
    pub store: Option<SessionStore>,
    pub opener: Opener,
    pub port: u16,
    pub stop: Arc<AtomicBool>,
    pub courier: Courier,
}

impl SignInJob {
    pub fn run(self) {
        let service = self.service;
        let attempt = self.attempt;
        let courier = self.courier.clone();
        let outcome = catch_unwind(AssertUnwindSafe(|| self.sign_in()))
            .unwrap_or(Err(Failure::Crashed(service)));
        courier.send(Event::SignedIn {
            service,
            attempt,
            outcome,
        });
    }

    fn sign_in(&self) -> Result<Session, Failure> {
        let listener = Listener::bind(self.port)?;
        let sign_in = self.provider.start_sign_in(&listener.redirect())?;
        self.courier.send(Event::SignInUrl {
            service: self.service,
            attempt: self.attempt,
            url: sign_in.url.clone(),
        });
        if let Err(error) = (self.opener)(&sign_in.url) {
            log::warn!("the browser did not open by itself: {error}");
        }
        let callback = listener.wait(&self.stop, SIGN_IN_PATIENCE)?;
        let session =
            caditor_cloud::finish_sign_in(&*self.provider, &*self.transport, &sign_in, &callback)?;
        if let Some(store) = &self.store
            && let Err(error) = store.save(&session)
        {
            log::warn!("the sign-in could not be kept for the next start: {error}");
        }
        Ok(session)
    }
}

pub fn restore(store: &SessionStore) -> Result<Option<Session>, Failure> {
    store.load().map_err(|error| Failure::Save(error.kind()))
}

pub fn sign_out(client: &Client, store: Option<&SessionStore>) {
    if let Err(error) = client.revoke() {
        log::info!("the sign-in could not be revoked at the service: {error}");
    }
    forget(store);
    if let Some(store) = store
        && let Err(error) = store.forget_hub()
    {
        log::warn!("the remembered hub could not be removed: {error}");
    }
    if let Some(store) = store
        && let Err(error) = thumbnails::forget(store.thumbnails_dir())
    {
        log::warn!("the kept pictures of designs could not be removed: {error}");
    }
}

pub fn remember_hub(store: &SessionStore, workspace: &WorkspaceId) -> Result<(), Failure> {
    store
        .remember_hub(workspace)
        .map_err(|error| Failure::Save(error.kind()))
}

pub fn list_workspaces(
    client: &Client,
    store: Option<&SessionStore>,
) -> Result<(Vec<Workspace>, Option<WorkspaceId>), Failure> {
    let workspaces = client.workspaces()?;
    let remembered = store.and_then(SessionStore::remembered_hub);
    Ok((workspaces, remembered))
}

pub fn forget(store: Option<&SessionStore>) {
    if let Some(store) = store
        && let Err(error) = store.forget()
    {
        log::warn!("the saved sign-in could not be removed: {error}");
    }
}

pub struct ImportJob {
    pub service: Service,
    pub generation: u64,
    pub id: u64,
    pub client: Arc<Client>,
    pub item: ImportItem,
    pub dir: PathBuf,
    pub stop: Arc<AtomicBool>,
    pub courier: Courier,
}

impl ImportJob {
    pub fn run(&self) -> Result<PathBuf, Failure> {
        let target = downloads::path_for(&self.dir, &self.item);
        let temporary = downloads::temporary_for(&target);
        let outcome = match &self.item {
            ImportItem::Design(design) => {
                self.convert_and_download(design, None, &temporary, &target)
            }
            ImportItem::Version(design, version) => {
                self.convert_and_download(design, Some(version), &temporary, &target)
            }
            ImportItem::Upload(upload) => self.write(&temporary, &target, |writer, progress| {
                self.client.fetch_upload(upload, writer, progress)
            }),
        };
        if outcome.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        outcome?;
        if let Err(error) = downloads::prune(&self.dir, KEPT_DESIGNS) {
            log::warn!("old downloaded designs could not be tidied: {error}");
        }
        Ok(target)
    }

    fn convert_and_download(
        &self,
        design: &Design,
        version: Option<&Version>,
        temporary: &Path,
        target: &Path,
    ) -> Result<(), Failure> {
        let service = self.service;
        let translation = self.client.start_conversion(design, version)?;
        let deadline = Instant::now() + CONVERSION_PATIENCE;
        let download = loop {
            self.check_stopped()?;
            match self.client.poll_conversion(&translation)? {
                Conversion::Ready(download) => break download,
                Conversion::Working { percent } => self.report(Stage::Converting(percent)),
            }
            if Instant::now() >= deadline {
                return Err(Error::ConversionTimedOut { service }.into());
            }
            self.pause(POLL_EVERY)?;
        };
        self.write(temporary, target, |writer, progress| {
            self.client.fetch(&download, writer, progress)
        })
    }

    fn write(
        &self,
        temporary: &Path,
        target: &Path,
        fetch: impl FnOnce(
            &mut BufWriter<File>,
            &mut dyn FnMut(u64, Option<u64>) -> bool,
        ) -> Result<u64, Error>,
    ) -> Result<(), Failure> {
        let save = |error: std::io::Error| Failure::Save(error.kind());
        if let Some(folder) = target.parent() {
            fs::create_dir_all(folder).map_err(save)?;
        }
        let mut writer = BufWriter::new(File::create(temporary).map_err(save)?);
        let mut last = None::<Instant>;
        let mut progress = |received: u64, total: Option<u64>| {
            if last.is_none_or(|at| at.elapsed() >= PROGRESS_EVERY) {
                last = Some(Instant::now());
                self.report(Stage::Downloading { received, total });
            }
            !self.stop.load(Ordering::Relaxed)
        };
        fetch(&mut writer, &mut progress)?;
        writer.flush().map_err(save)?;
        let file = writer
            .into_inner()
            .map_err(|error| save(error.into_error()))?;
        file.sync_all().map_err(save)?;
        drop(file);
        fs::rename(temporary, target).map_err(save)
    }

    fn report(&self, stage: Stage) {
        self.courier.send(Event::Progress {
            service: self.service,
            generation: self.generation,
            import: self.id,
            stage,
        });
    }

    fn check_stopped(&self) -> Result<(), Failure> {
        if self.stop.load(Ordering::Relaxed) {
            Err(Error::Stopped.into())
        } else {
            Ok(())
        }
    }

    fn pause(&self, total: Duration) -> Result<(), Failure> {
        pause(&self.stop, total)
    }
}

fn pause(stop: &AtomicBool, total: Duration) -> Result<(), Failure> {
    let until = Instant::now() + total;
    while Instant::now() < until {
        if stop.load(Ordering::Relaxed) {
            return Err(Error::Stopped.into());
        }
        thread::sleep(STOP_CHECK);
    }
    Ok(())
}

pub struct SendJob {
    pub service: Service,
    pub generation: u64,
    pub id: u64,
    pub client: Arc<Client>,
    pub shipment: Shipment,
    pub stop: Arc<AtomicBool>,
    pub courier: Courier,
}

impl SendJob {
    pub fn run(&self) -> Result<Sent, Failure> {
        let outcome = self.deliver();
        if let Err(error) = fs::remove_file(&self.shipment.file) {
            log::info!("the exported file could not be removed after sending: {error}");
        }
        outcome
    }

    fn deliver(&self) -> Result<Sent, Failure> {
        if self.stop.load(Ordering::Relaxed) {
            return Err(Error::Stopped.into());
        }
        let bytes = fs::read(&self.shipment.file).map_err(|error| Failure::Read(error.kind()))?;
        let outgoing = Outgoing {
            folder: &self.shipment.destination.folder,
            name: &self.shipment.name,
            bytes: &bytes,
        };
        let mut last = None::<Instant>;
        let mut progress = |sent: u64, total: Option<u64>| {
            let finished = total == Some(sent);
            if finished || last.is_none_or(|at| at.elapsed() >= PROGRESS_EVERY) {
                last = Some(Instant::now());
                self.report(SendStage::Uploading { sent, total });
            }
            !self.stop.load(Ordering::Relaxed)
        };
        match self.shipment.destination.how {
            SendAs::File => Ok(self.client.send(&outgoing, &mut progress)?),
            SendAs::PartStudio => {
                let translation = self.client.send_part_studio(&outgoing, &mut progress)?;
                self.await_part_studio(&translation)?;
                Ok(Sent {
                    name: outgoing.name.trim().to_owned(),
                    outcome: SendOutcome::PartStudio,
                })
            }
        }
    }

    fn await_part_studio(&self, translation: &caditor_cloud::Translation) -> Result<(), Failure> {
        let service = self.service;
        let deadline = Instant::now() + CONVERSION_PATIENCE;
        loop {
            self.report(SendStage::Translating);
            if self.client.poll_part_studio(translation)? == Placement::Done {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(Error::PartStudioTimedOut { service }.into());
            }
            pause(&self.stop, POLL_EVERY)?;
        }
    }

    fn report(&self, stage: SendStage) {
        self.courier.send(Event::SendProgress {
            service: self.service,
            generation: self.generation,
            send: self.id,
            stage,
        });
    }
}

pub fn measure(dir: &std::path::Path) -> Result<Usage, Failure> {
    downloads::usage(dir).map_err(|error| Failure::Save(error.kind()))
}

pub fn clear(dir: &std::path::Path) -> Result<(), Failure> {
    downloads::clear(dir).map_err(|error| Failure::Save(error.kind()))
}
