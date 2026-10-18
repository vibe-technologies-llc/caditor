use std::{
    borrow::Cow,
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
};

use caditor_cloud::{
    Client, Design, FolderRef, Fusion, Network, Onshape, ProjectId, Provider, SendOutcome, Service,
    Session, Transport, WorkspaceId,
};

use crate::{
    browser,
    commands::Reason,
    files::FileCommand,
    model::{Action, Notice, WakerFactory},
    preferences::CloudPreferences,
};

mod downloads;
mod failure;
mod jobs;
mod keyring;
mod state;
mod store;
mod thumbnails;

pub use self::{
    failure::Failure,
    jobs::Opener,
    state::{
        CloudCommand, Connection, Destination, Downloads, Import, ImportItem, Link, Linked, Load,
        Send, SendStage, Shipment, SigningIn, Stage, first_workspace,
    },
    thumbnails::Thumbnail,
};
use self::{
    jobs::{Courier, Event, ImportJob, Loaded, SendJob, SignInJob},
    keyring::Keyring,
    store::SessionStore,
    thumbnails::{AT_ONCE, ThumbnailCache},
};
#[cfg(test)]
pub use self::{keyring::FakeKeyring, thumbnails::sample_png};

pub const CALLBACK_PORT: u16 = 49617;
const DEFAULT_AUTODESK_CLIENT_ID: &str = "";
const DEFAULT_ONSHAPE_CLIENT_ID: &str = "";
pub const PROJECT_ROWS_ON_HOME: usize = 6;

pub fn outgoing_dir(state_dir: &Path) -> PathBuf {
    SessionStore::outgoing_dir(state_dir)
}

pub fn built_in_client_id(service: Service) -> &'static str {
    match service {
        Service::Fusion => {
            option_env!("CADITOR_AUTODESK_CLIENT_ID").unwrap_or(DEFAULT_AUTODESK_CLIENT_ID)
        }
        Service::Onshape => {
            option_env!("CADITOR_ONSHAPE_CLIENT_ID").unwrap_or(DEFAULT_ONSHAPE_CLIENT_ID)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    #[error("Cloud services are turned off. Turn them on in Preferences › General › Cloud.")]
    Off,
    #[error("Already signed in to {0}.")]
    AlreadySignedIn(Service),
    #[error("Signing in to {0} is waiting for the browser. Cancel it first to start again.")]
    SigningIn(Service),
    #[error("Sign in to {0} first, from Home or File › Cloud.")]
    NotSignedIn(Service),
    #[error("Sign in to a cloud service first, from Home or File › Cloud.")]
    NotSignedInAnywhere,
    #[error(
        "This build of caditor has no {0} app registered. Enter one in Preferences › General › Cloud."
    )]
    NoClientId(Service),
    #[error("Another design is still being imported from {0}.")]
    Importing(Service),
    #[error("Another model is still being sent to {0}.")]
    Sending(Service),
    #[error("Sign in to Fusion 360 or Onshape first, from Home or File › Cloud.")]
    NothingToSendTo,
}

impl Reason for Refusal {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

pub struct CloudServices {
    pub make_waker: WakerFactory,
    pub state_dir: Option<PathBuf>,
    pub keyring: Arc<dyn Keyring>,
}

impl CloudServices {
    pub fn on_this_system(make_waker: WakerFactory, state_dir: Option<PathBuf>) -> Self {
        Self {
            make_waker,
            state_dir,
            keyring: keyring::system(),
        }
    }

    pub fn without_files() -> Self {
        Self {
            make_waker: Box::new(|| Box::new(|| {})),
            state_dir: None,
            keyring: keyring::system(),
        }
    }
}

pub struct Cloud {
    preferences: CloudPreferences,
    connections: Vec<Connection>,
    generations: BTreeMap<Service, u64>,
    providers: BTreeMap<Service, Arc<dyn Provider>>,
    transport: Option<Arc<dyn Transport>>,
    pinned_backend: bool,
    courier: Courier,
    receiver: Receiver<Event>,
    state_dir: Option<PathBuf>,
    keyring: Arc<dyn Keyring>,
    opener: Opener,
    callback_port: u16,
    next_attempt: u64,
    next_import: u64,
    restoring: bool,
    use_built_in_client_ids: bool,
    pub downloads: Downloads,
}

impl Cloud {
    pub fn new(preferences: &CloudPreferences, services: CloudServices) -> Self {
        let (sender, receiver) = mpsc::channel();
        let courier = Courier::new(sender, (services.make_waker)());
        let connections = Service::ALL
            .into_iter()
            .map(|service| Connection {
                service,
                link: if preferences.enabled {
                    Link::Starting
                } else {
                    Link::Off
                },
                last_failure: None,
            })
            .collect();
        let downloads = Downloads {
            dir: services
                .state_dir
                .as_deref()
                .map(SessionStore::downloads_dir),
            ..Downloads::default()
        };
        Self {
            preferences: preferences.clone(),
            connections,
            generations: BTreeMap::new(),
            providers: BTreeMap::new(),
            transport: None,
            pinned_backend: false,
            courier,
            receiver,
            state_dir: services.state_dir,
            keyring: services.keyring,
            opener: Arc::new(browser::open),
            callback_port: CALLBACK_PORT,
            next_attempt: 0,
            next_import: 0,
            restoring: false,
            use_built_in_client_ids: true,
            downloads,
        }
    }

    #[cfg(test)]
    pub fn without_built_in_client_id(mut self) -> Self {
        self.use_built_in_client_ids = false;
        self
    }

    #[cfg(test)]
    pub fn with_backend(
        mut self,
        provider: Arc<dyn Provider>,
        transport: Arc<dyn Transport>,
        opener: Opener,
    ) -> Self {
        self.providers.insert(provider.service(), provider);
        self.transport = Some(transport);
        self.opener = opener;
        self.callback_port = 0;
        self.pinned_backend = true;
        self
    }

    pub fn is_enabled(&self) -> bool {
        self.preferences.enabled
    }

    pub fn shows_on_home(&self) -> bool {
        self.preferences.shows_on_home()
    }

    pub fn connections(&self) -> &[Connection] {
        &self.connections
    }

    pub fn connection(&self, service: Service) -> Option<&Connection> {
        self.connections
            .iter()
            .find(|connection| connection.service == service)
    }

    pub fn linked(&self, service: Service) -> Option<&Linked> {
        match &self.connection(service)?.link {
            Link::Linked(linked) => Some(linked),
            _ => None,
        }
    }

    pub fn client_id(&self, service: Service) -> Option<String> {
        let own = self.preferences.client_id(service);
        let chosen = if own.is_empty() && self.use_built_in_client_ids {
            built_in_client_id(service).trim()
        } else {
            own
        };
        (!chosen.is_empty()).then(|| chosen.to_owned())
    }

    pub fn can_sign_in(&self, service: Service) -> Result<(), Refusal> {
        if !self.preferences.enabled {
            return Err(Refusal::Off);
        }
        match self.connection(service).map(|connection| &connection.link) {
            Some(Link::Linked(_)) => return Err(Refusal::AlreadySignedIn(service)),
            Some(Link::SigningIn(_)) => return Err(Refusal::SigningIn(service)),
            _ => {}
        }
        if self.pinned_backend || self.client_id(service).is_some() {
            Ok(())
        } else {
            Err(Refusal::NoClientId(service))
        }
    }

    pub fn any_signed_in(&self) -> Result<(), Refusal> {
        if !self.preferences.enabled {
            return Err(Refusal::Off);
        }
        Service::ALL
            .into_iter()
            .any(|service| self.linked(service).is_some())
            .then_some(())
            .ok_or(Refusal::NotSignedInAnywhere)
    }

    pub fn can_send_anywhere(&self) -> Result<(), Refusal> {
        if !self.preferences.enabled {
            return Err(Refusal::Off);
        }
        Service::ALL
            .into_iter()
            .any(|service| self.linked(service).is_some())
            .then_some(())
            .ok_or(Refusal::NothingToSendTo)
    }

    pub fn is_signed_in(&self, service: Service) -> Result<(), Refusal> {
        if !self.preferences.enabled {
            return Err(Refusal::Off);
        }
        self.linked(service)
            .map(|_| ())
            .ok_or(Refusal::NotSignedIn(service))
    }

    pub fn configure(&mut self, preferences: &CloudPreferences) {
        let before = std::mem::replace(&mut self.preferences, preferences.clone());
        if before.enabled && !preferences.enabled {
            for service in Service::ALL {
                self.leave(service, true);
                self.set_link(service, Link::Off);
            }
            self.restoring = false;
            return;
        }
        if !before.enabled && preferences.enabled {
            for service in Service::ALL {
                self.set_link(service, Link::Starting);
            }
            self.restoring = false;
        }
        for service in Service::ALL {
            if before.client_id(service) == preferences.client_id(service) {
                continue;
            }
            if !self.pinned_backend {
                self.providers.remove(&service);
            }
            if self.linked(service).is_some() || self.is_signing_in(service) {
                self.leave(service, false);
                self.set_link(service, Link::SignedOut);
            }
        }
    }

    pub fn poll(&mut self) -> Vec<Action> {
        let mut actions = Vec::new();
        self.start_restoring();
        while let Ok(event) = self.receiver.try_recv() {
            self.handle(event, &mut actions);
        }
        actions
    }

    pub fn run(&mut self, command: CloudCommand) -> Vec<Action> {
        let mut actions = Vec::new();
        if !self.preferences.enabled {
            return actions;
        }
        match command {
            CloudCommand::SignIn(service) => self.start_sign_in(service, &mut actions),
            CloudCommand::CancelSignIn(service) => self.cancel_sign_in(service),
            CloudCommand::SignOut(service) => {
                self.leave(service, true);
                self.set_link(service, Link::SignedOut);
            }
            CloudCommand::Refresh(service) => self.refresh(service),
            CloudCommand::ChooseWorkspace(service, workspace) => {
                self.choose_workspace(service, workspace);
            }
            CloudCommand::LoadProject(service, project) => self.load_root(service, &project),
            CloudCommand::LoadThumbnail(service, design) => self.load_thumbnail(service, design),
            CloudCommand::OpenFolder(service, folder) => self.load_folder(service, folder),
            CloudCommand::ReloadProject(service, project) => {
                if let Some(linked) = self.linked_mut(service) {
                    linked.roots.remove(&project);
                }
                self.load_root(service, &project);
            }
            CloudCommand::ReloadFolder(service, folder) => {
                if let Some(linked) = self.linked_mut(service) {
                    linked.folders.remove(&folder);
                }
                self.load_folder(service, folder);
            }
            CloudCommand::Import(service, design) => {
                self.start_import(service, ImportItem::Design(design), &mut actions)
            }
            CloudCommand::ImportVersion(service, design, version) => {
                self.start_import(service, ImportItem::Version(design, version), &mut actions)
            }
            CloudCommand::LoadVersions(service, design) => self.load_versions(service, design),
            CloudCommand::ReloadVersions(service, design) => {
                if let Some(linked) = self.linked_mut(service) {
                    linked.versions.remove(&design.id);
                }
                self.load_versions(service, design);
            }
            CloudCommand::ImportUpload(service, upload) => {
                self.start_import(service, ImportItem::Upload(upload), &mut actions)
            }
            CloudCommand::CancelImport(service) => self.cancel_import(service),
            CloudCommand::Send(shipment) => self.start_send(shipment, &mut actions),
            CloudCommand::CancelSend(service) => self.cancel_send(service),
            CloudCommand::DismissSend(service) => {
                if let Some(linked) = self.linked_mut(service)
                    && linked
                        .sending
                        .as_ref()
                        .is_some_and(|send| !send.is_running())
                {
                    linked.sending = None;
                }
            }
            CloudCommand::DismissImport(service) => {
                if let Some(linked) = self.linked_mut(service)
                    && linked
                        .import
                        .as_ref()
                        .is_some_and(|import| !import.is_running())
                {
                    linked.import = None;
                }
            }
            CloudCommand::MeasureDownloads => self.measure_downloads(),
            CloudCommand::ClearDownloads => self.clear_downloads(),
        }
        actions
    }

    fn generation(&self, service: Service) -> u64 {
        self.generations.get(&service).copied().unwrap_or_default()
    }

    fn bump(&mut self, service: Service) -> u64 {
        let next = self.generation(service) + 1;
        self.generations.insert(service, next);
        next
    }

    fn provider(&mut self, service: Service) -> Arc<dyn Provider> {
        if let Some(provider) = self.providers.get(&service) {
            return Arc::clone(provider);
        }
        let client_id = self.client_id(service).unwrap_or_default();
        let provider: Arc<dyn Provider> = match service {
            Service::Fusion => Arc::new(Fusion::new(client_id)),
            Service::Onshape => Arc::new(Onshape::new(client_id)),
        };
        self.providers.insert(service, Arc::clone(&provider));
        provider
    }

    fn transport(&mut self) -> Arc<dyn Transport> {
        Arc::clone(
            self.transport
                .get_or_insert_with(|| Arc::new(Network::new())),
        )
    }

    fn store(&self, service: Service) -> Option<SessionStore> {
        self.state_dir
            .as_deref()
            .map(|dir| SessionStore::in_state_dir(dir, service, Arc::clone(&self.keyring)))
    }

    fn connection_mut(&mut self, service: Service) -> Option<&mut Connection> {
        self.connections
            .iter_mut()
            .find(|connection| connection.service == service)
    }

    fn linked_mut(&mut self, service: Service) -> Option<&mut Linked> {
        match &mut self.connection_mut(service)?.link {
            Link::Linked(linked) => Some(linked),
            _ => None,
        }
    }

    fn is_signing_in(&self, service: Service) -> bool {
        matches!(
            self.connection(service).map(|connection| &connection.link),
            Some(Link::SigningIn(_))
        )
    }

    fn set_link(&mut self, service: Service, link: Link) {
        if let Some(connection) = self.connection_mut(service) {
            connection.link = link;
        }
    }

    fn leave(&mut self, service: Service, revoke: bool) {
        self.bump(service);
        let store = self.store(service);
        let Some(left) = self
            .connection_mut(service)
            .map(|connection| std::mem::replace(&mut connection.link, Link::SignedOut))
        else {
            return;
        };
        let client = match left {
            Link::SigningIn(signing) => {
                signing.stop.store(true, Ordering::Relaxed);
                None
            }
            Link::Linked(linked) => {
                linked.import.iter().for_each(|import| {
                    import.stop.store(true, Ordering::Relaxed);
                });
                linked.sending.iter().for_each(|send| {
                    send.stop.store(true, Ordering::Relaxed);
                });
                Some(Arc::clone(&linked.client))
            }
            Link::Off | Link::Starting | Link::SignedOut => None,
        };
        let signing_out = match (client, revoke) {
            (Some(client), true) => Some(client),
            (_, true) => return,
            (_, false) => None,
        };
        if signing_out.is_none() && store.is_none() {
            return;
        }
        self.courier.spawn(
            service,
            move || {
                match &signing_out {
                    Some(client) => jobs::sign_out(client, store.as_ref()),
                    None => jobs::forget(store.as_ref()),
                }
                Ok(())
            },
            |_: Result<(), Failure>| Event::Left,
        );
    }

    fn start_restoring(&mut self) {
        if self.restoring || !self.preferences.enabled {
            return;
        }
        self.restoring = true;
        for service in Service::ALL {
            if !matches!(
                self.connection(service).map(|connection| &connection.link),
                Some(Link::Starting)
            ) {
                continue;
            }
            match self.store(service) {
                Some(store) => self.courier.spawn(
                    service,
                    move || jobs::restore(&store),
                    move |outcome| Event::Restored { service, outcome },
                ),
                None => self.set_link(service, Link::SignedOut),
            }
        }
    }

    fn start_sign_in(&mut self, service: Service, actions: &mut Vec<Action>) {
        if let Err(refusal) = self.can_sign_in(service) {
            actions.push(Action::Inform(Notice::warning(refusal.to_string())));
            return;
        }
        self.next_attempt += 1;
        let attempt = self.next_attempt;
        let stop = Arc::new(AtomicBool::new(false));
        self.set_link(
            service,
            Link::SigningIn(SigningIn {
                url: None,
                attempt,
                stop: Arc::clone(&stop),
            }),
        );
        let job = SignInJob {
            service,
            attempt,
            provider: self.provider(service),
            transport: self.transport(),
            store: self.store(service),
            opener: Arc::clone(&self.opener),
            port: self.callback_port,
            stop,
            courier: self.courier.clone(),
        };
        let spawned = thread::Builder::new()
            .name("cloud sign-in".to_owned())
            .spawn(move || job.run());
        if let Err(error) = spawned {
            self.finish_sign_in(service, attempt, Err(Failure::Spawn(error.kind())), actions);
        }
    }

    fn cancel_sign_in(&mut self, service: Service) {
        if self.is_signing_in(service) {
            self.leave(service, false);
        }
    }

    fn finish_sign_in(
        &mut self,
        service: Service,
        attempt: u64,
        outcome: Result<Session, Failure>,
        actions: &mut Vec<Action>,
    ) {
        let current = matches!(
            self.connection(service).map(|connection| &connection.link),
            Some(Link::SigningIn(signing)) if signing.attempt == attempt
        );
        if !current {
            return;
        }
        match outcome {
            Ok(session) => self.link_with(service, session),
            Err(failure) => {
                self.set_link(service, Link::SignedOut);
                let failure = Arc::new(failure);
                if let Some(connection) = self.connection_mut(service) {
                    connection.last_failure = Some(Arc::clone(&failure));
                }
                if !failure.is_stop() {
                    actions.push(Action::Inform(Notice::warning(format!(
                        "Could not sign in to {service}. {failure}"
                    ))));
                }
            }
        }
    }

    fn link_with(&mut self, service: Service, session: Session) {
        let provider = self.provider(service);
        let transport = self.transport();
        let mut client = Client::new(provider, transport, session);
        if let Some(store) = self.store(service) {
            client = client.saving_with(Arc::new(move |session| {
                if let Err(error) = store.save(session) {
                    log::warn!("the renewed sign-in could not be kept: {error}");
                }
            }));
        }
        self.bump(service);
        if let Some(connection) = self.connection_mut(service) {
            connection.last_failure = None;
            connection.link = Link::Linked(Box::new(Linked::new(Arc::new(client))));
        }
        self.load_account(service);
        self.load_workspaces(service);
    }

    fn refresh(&mut self, service: Service) {
        self.bump(service);
        let Some(linked) = self.linked_mut(service) else {
            return;
        };
        linked.roots.clear();
        linked.folders.clear();
        linked.thumbnails.clear();
        linked.versions.clear();
        linked.projects = Load::Idle;
        linked.workspaces = Load::Idle;
        linked.account = Load::Idle;
        self.load_account(service);
        self.load_workspaces(service);
    }

    fn choose_workspace(&mut self, service: Service, workspace: WorkspaceId) {
        self.bump(service);
        let Some(linked) = self.linked_mut(service) else {
            return;
        };
        linked.workspace = Some(workspace.clone());
        linked.roots.clear();
        linked.folders.clear();
        linked.thumbnails.clear();
        linked.versions.clear();
        linked.projects = Load::Idle;
        self.load_projects(service, workspace.clone());
        self.remember_workspace(service, workspace);
    }

    fn remember_workspace(&mut self, service: Service, workspace: WorkspaceId) {
        let Some(store) = self.store(service) else {
            return;
        };
        self.courier.spawn(
            service,
            move || jobs::remember_hub(&store, &workspace),
            |outcome| {
                if let Err(failure) = outcome {
                    log::warn!("the chosen hub could not be remembered: {failure}");
                }
                Event::Left
            },
        );
    }

    fn load_account(&mut self, service: Service) {
        let generation = self.generation(service);
        let Some(linked) = self.linked_mut(service) else {
            return;
        };
        if linked.account.is_loading() {
            return;
        }
        linked.account = Load::Loading;
        let client = Arc::clone(&linked.client);
        self.courier.spawn(
            service,
            move || client.account().map_err(Failure::from),
            move |result| Event::Loaded {
                service,
                generation,
                loaded: Loaded::Account(result.map_err(Arc::new)),
            },
        );
    }

    fn load_workspaces(&mut self, service: Service) {
        let generation = self.generation(service);
        let Some(linked) = self.linked_mut(service) else {
            return;
        };
        if linked.workspaces.is_loading() {
            return;
        }
        linked.workspaces = Load::Loading;
        let client = Arc::clone(&linked.client);
        let store = self.store(service);
        self.courier.spawn(
            service,
            move || jobs::list_workspaces(&client, store.as_ref()),
            move |result| Event::Loaded {
                service,
                generation,
                loaded: Loaded::Workspaces(result.map_err(Arc::new)),
            },
        );
    }

    fn load_projects(&mut self, service: Service, workspace: WorkspaceId) {
        let generation = self.generation(service);
        let Some(linked) = self.linked_mut(service) else {
            return;
        };
        linked.projects = Load::Loading;
        let client = Arc::clone(&linked.client);
        let asked = workspace.clone();
        self.courier.spawn(
            service,
            move || client.projects(&asked).map_err(Failure::from),
            move |result| Event::Loaded {
                service,
                generation,
                loaded: Loaded::Projects(workspace.clone(), result.map_err(Arc::new)),
            },
        );
    }

    fn load_root(&mut self, service: Service, project: &ProjectId) {
        let generation = self.generation(service);
        let Some(linked) = self.linked_mut(service) else {
            return;
        };
        if !matches!(linked.root(project), Load::Idle) {
            return;
        }
        let Some(known) = linked.project(project).cloned() else {
            return;
        };
        linked.roots.insert(project.clone(), Load::Loading);
        let client = Arc::clone(&linked.client);
        let id = project.clone();
        self.courier.spawn(
            service,
            move || client.root(&known).map_err(Failure::from),
            move |result| Event::Loaded {
                service,
                generation,
                loaded: Loaded::Root(id.clone(), result.map_err(Arc::new)),
            },
        );
    }

    fn load_thumbnail(&mut self, service: Service, design: Design) {
        let generation = self.generation(service);
        let cache = self
            .store(service)
            .map(|store| ThumbnailCache::new(store.thumbnails_dir().to_owned()));
        let Some(linked) = self.linked_mut(service) else {
            return;
        };
        let loading = linked
            .thumbnails
            .values()
            .filter(|load| load.is_loading())
            .count();
        if !linked.wants_picture(&design.id) || loading >= AT_ONCE {
            return;
        }
        linked.thumbnails.insert(design.id.clone(), Load::Loading);
        let client = Arc::clone(&linked.client);
        let id = design.id.clone();
        self.courier.spawn(
            service,
            move || thumbnails::load(&client, cache.as_ref(), service, &design),
            move |result| Event::Loaded {
                service,
                generation,
                loaded: Loaded::Thumbnail(id.clone(), result.map_err(Arc::new)),
            },
        );
    }

    fn load_versions(&mut self, service: Service, design: Design) {
        let generation = self.generation(service);
        let Some(linked) = self.linked_mut(service) else {
            return;
        };
        if !matches!(linked.versions(&design.id), Load::Idle) {
            return;
        }
        linked.versions.insert(design.id.clone(), Load::Loading);
        let client = Arc::clone(&linked.client);
        let id = design.id.clone();
        self.courier.spawn(
            service,
            move || client.versions(&design).map_err(Failure::from),
            move |result| Event::Loaded {
                service,
                generation,
                loaded: Loaded::Versions(id.clone(), result.map_err(Arc::new)),
            },
        );
    }

    fn load_folder(&mut self, service: Service, folder: FolderRef) {
        let generation = self.generation(service);
        let Some(linked) = self.linked_mut(service) else {
            return;
        };
        if !matches!(linked.folder(&folder), Load::Idle) {
            return;
        }
        linked.folders.insert(folder.clone(), Load::Loading);
        let client = Arc::clone(&linked.client);
        let asked = folder.clone();
        self.courier.spawn(
            service,
            move || client.folder(&asked).map_err(Failure::from),
            move |result| Event::Loaded {
                service,
                generation,
                loaded: Loaded::Folder(folder.clone(), result.map_err(Arc::new)),
            },
        );
    }

    fn start_import(&mut self, service: Service, item: ImportItem, actions: &mut Vec<Action>) {
        let generation = self.generation(service);
        let dir = self.downloads.dir.clone();
        let courier = self.courier.clone();
        self.next_import += 1;
        let id = self.next_import;
        let Some(linked) = self.linked_mut(service) else {
            actions.push(Action::Inform(Notice::warning(
                Refusal::NotSignedIn(service).to_string(),
            )));
            return;
        };
        if linked.is_importing() {
            actions.push(Action::Inform(Notice::warning(
                Refusal::Importing(service).to_string(),
            )));
            return;
        }
        let Some(dir) = dir else {
            actions.push(Action::Inform(Notice::warning(
                Failure::NoDownloadFolder.to_string(),
            )));
            return;
        };
        let stop = Arc::new(AtomicBool::new(false));
        linked.import = Some(Import {
            id,
            item: item.clone(),
            stage: Stage::Starting,
            stop: Arc::clone(&stop),
        });
        let job = ImportJob {
            service,
            generation,
            id,
            client: Arc::clone(&linked.client),
            item,
            dir,
            stop,
            courier: courier.clone(),
        };
        courier.spawn(
            service,
            move || job.run(),
            move |outcome| Event::Imported {
                service,
                generation,
                import: id,
                outcome,
            },
        );
    }

    fn start_send(&mut self, shipment: Shipment, actions: &mut Vec<Action>) {
        let service = shipment.destination.service;
        let generation = self.generation(service);
        let courier = self.courier.clone();
        self.next_import += 1;
        let id = self.next_import;
        let refused = |refusal: Refusal| Action::Inform(Notice::warning(refusal.to_string()));
        let Some(linked) = self.linked_mut(service) else {
            actions.push(refused(Refusal::NotSignedIn(service)));
            return;
        };
        if linked.is_sending() {
            actions.push(refused(Refusal::Sending(service)));
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        linked.sending = Some(Send {
            id,
            shipment: shipment.clone(),
            stage: SendStage::Starting,
            stop: Arc::clone(&stop),
        });
        let job = SendJob {
            service,
            generation,
            id,
            client: Arc::clone(&linked.client),
            shipment,
            stop,
            courier: courier.clone(),
        };
        courier.spawn(
            service,
            move || job.run(),
            move |outcome| Event::Delivered {
                service,
                generation,
                send: id,
                outcome,
            },
        );
    }

    fn cancel_send(&mut self, service: Service) {
        if let Some(linked) = self.linked_mut(service)
            && let Some(send) = linked.sending.take()
        {
            send.stop.store(true, Ordering::Relaxed);
        }
    }

    fn cancel_import(&mut self, service: Service) {
        if let Some(linked) = self.linked_mut(service)
            && let Some(import) = linked.import.take()
        {
            import.stop.store(true, Ordering::Relaxed);
        }
    }

    pub fn measure_downloads(&mut self) {
        let Some(dir) = self.downloads.dir.clone() else {
            return;
        };
        if self.downloads.measuring {
            return;
        }
        self.downloads.measuring = true;
        self.courier.spawn(
            Service::Fusion,
            move || jobs::measure(&dir),
            Event::Measured,
        );
    }

    fn clear_downloads(&mut self) {
        let Some(dir) = self.downloads.dir.clone() else {
            return;
        };
        self.downloads.measuring = true;
        self.courier
            .spawn(Service::Fusion, move || jobs::clear(&dir), Event::Cleared);
    }

    fn handle(&mut self, event: Event, actions: &mut Vec<Action>) {
        match event {
            Event::Restored { service, outcome } => match outcome {
                Ok(Some(session)) if self.preferences.enabled => self.link_with(service, session),
                Ok(_) => self.set_link(service, Link::SignedOut),
                Err(failure) => {
                    log::warn!("the saved {service} sign-in could not be read: {failure}");
                    self.set_link(service, Link::SignedOut);
                }
            },
            Event::SignInUrl {
                service,
                attempt,
                url,
            } => {
                if let Some(Connection {
                    link: Link::SigningIn(signing),
                    ..
                }) = self.connection_mut(service)
                    && signing.attempt == attempt
                {
                    signing.url = Some(url);
                }
            }
            Event::SignedIn {
                service,
                attempt,
                outcome,
            } => self.finish_sign_in(service, attempt, outcome, actions),
            Event::Loaded {
                service,
                generation,
                loaded,
            } => {
                if generation == self.generation(service) {
                    self.loaded(service, loaded, actions);
                }
            }
            Event::Progress {
                service,
                generation,
                import,
                stage,
            } => {
                if generation == self.generation(service)
                    && let Some(linked) = self.linked_mut(service)
                    && let Some(current) = linked.import.as_mut()
                    && current.id == import
                    && current.is_running()
                {
                    current.stage = stage;
                }
            }
            Event::Imported {
                service,
                generation,
                import,
                outcome,
            } => {
                if generation == self.generation(service) {
                    self.imported(service, import, outcome, actions);
                }
            }
            Event::SendProgress {
                service,
                generation,
                send,
                stage,
            } => {
                if generation == self.generation(service)
                    && let Some(linked) = self.linked_mut(service)
                    && let Some(current) = linked.sending.as_mut()
                    && current.id == send
                    && current.is_running()
                {
                    current.stage = stage;
                }
            }
            Event::Delivered {
                service,
                generation,
                send,
                outcome,
            } => {
                if generation == self.generation(service) {
                    self.delivered(service, send, outcome, actions);
                }
            }
            Event::Left => {}
            Event::Measured(outcome) => {
                self.downloads.measuring = false;
                match outcome {
                    Ok(usage) => self.downloads.usage = Some(usage),
                    Err(failure) => {
                        log::warn!("downloaded designs could not be counted: {failure}")
                    }
                }
            }
            Event::Cleared(outcome) => {
                if let Err(failure) = outcome {
                    actions.push(Action::Inform(Notice::warning(format!(
                        "Downloaded designs could not be removed. {failure}"
                    ))));
                }
                if self.downloads.measuring {
                    self.downloads.measuring = false;
                    self.measure_downloads();
                }
            }
        }
    }

    fn loaded(&mut self, service: Service, loaded: Loaded, actions: &mut Vec<Action>) {
        let failed = match &loaded {
            Loaded::Account(Err(failure))
            | Loaded::Workspaces(Err(failure))
            | Loaded::Projects(_, Err(failure))
            | Loaded::Root(_, Err(failure))
            | Loaded::Folder(_, Err(failure))
            | Loaded::Thumbnail(_, Err(failure))
            | Loaded::Versions(_, Err(failure)) => Some(Arc::clone(failure)),
            _ => None,
        };
        if failed
            .as_ref()
            .is_some_and(|failure| failure.is_sign_in_needed())
        {
            self.leave(service, false);
            actions.push(Action::Inform(Notice::warning(format!(
                "Your {service} sign-in has ended. Sign in again from Home or File › Cloud."
            ))));
            return;
        }
        let Some(linked) = self.linked_mut(service) else {
            return;
        };
        let mut choose = None;
        match loaded {
            Loaded::Account(result) => linked.account = settle(result),
            Loaded::Workspaces(result) => {
                let result = result.map(|(workspaces, remembered)| {
                    choose = [linked.workspace.as_ref(), remembered.as_ref()]
                        .into_iter()
                        .flatten()
                        .find(|id| workspaces.iter().any(|workspace| workspace.id == **id))
                        .cloned()
                        .or_else(|| {
                            first_workspace(&workspaces).map(|workspace| workspace.id.clone())
                        });
                    workspaces
                });
                linked.workspaces = settle(result);
            }
            Loaded::Projects(workspace, result) => {
                if linked.workspace.as_ref() == Some(&workspace) {
                    linked.projects = settle(result);
                }
            }
            Loaded::Root(project, result) => {
                linked.roots.insert(project, settle(result));
            }
            Loaded::Folder(folder, result) => {
                linked.folders.insert(folder, settle(result));
            }
            Loaded::Thumbnail(design, result) => {
                linked.thumbnails.insert(design, settle(result));
            }
            Loaded::Versions(design, result) => {
                linked.versions.insert(design, settle(result));
            }
        }
        if let Some(workspace) = choose {
            linked.workspace = Some(workspace.clone());
            if matches!(linked.projects, Load::Idle) {
                self.load_projects(service, workspace);
            }
        }
    }

    fn delivered(
        &mut self,
        service: Service,
        send: u64,
        outcome: Result<caditor_cloud::Sent, Failure>,
        actions: &mut Vec<Action>,
    ) {
        let Some(linked) = self.linked_mut(service) else {
            return;
        };
        let Some(current) = linked.sending.as_ref().filter(|current| current.id == send) else {
            return;
        };
        let shipment = current.shipment.clone();
        let name = shipment.name.clone();
        match outcome {
            Ok(sent) => {
                linked.sending = None;
                linked.folders.remove(&shipment.destination.folder);
                linked.roots.remove(&shipment.destination.folder.project);
                actions.push(Action::Inform(Notice::success(delivery_words(
                    service, &shipment, &sent,
                ))));
            }
            Err(failure) if failure.is_stop() => linked.sending = None,
            Err(failure) if failure.is_sign_in_needed() => {
                self.leave(service, false);
                actions.push(Action::Inform(Notice::warning(format!(
                    "Your {service} sign-in has ended, so “{name}” was not sent. Sign in again \
                     from Home or File › Cloud."
                ))));
            }
            Err(failure) => {
                let failure = Arc::new(failure);
                if let Some(current) = linked.sending.as_mut() {
                    current.stage = SendStage::Failed(Arc::clone(&failure));
                }
                actions.push(Action::Inform(Notice::warning(format!(
                    "Could not send “{name}” to {service}. {failure}"
                ))));
            }
        }
    }

    fn imported(
        &mut self,
        service: Service,
        import: u64,
        outcome: Result<PathBuf, Failure>,
        actions: &mut Vec<Action>,
    ) {
        let Some(linked) = self.linked_mut(service) else {
            return;
        };
        let Some(current) = linked
            .import
            .as_ref()
            .filter(|current| current.id == import)
        else {
            return;
        };
        let name = current.item.name();
        match outcome {
            Ok(path) => {
                linked.import = None;
                actions.push(Action::File(FileCommand::Drop {
                    paths: vec![path],
                    into: None,
                }));
                self.measure_downloads();
            }
            Err(failure) if failure.is_stop() => linked.import = None,
            Err(failure) if failure.is_sign_in_needed() => {
                self.leave(service, false);
                actions.push(Action::Inform(Notice::warning(format!(
                    "Your {service} sign-in has ended, so “{name}” was not imported. Sign in \
                     again from Home or File › Cloud."
                ))));
            }
            Err(failure) => {
                let failure = Arc::new(failure);
                if let Some(current) = linked.import.as_mut() {
                    current.stage = Stage::Failed(Arc::clone(&failure));
                }
                actions.push(Action::Inform(Notice::warning(format!(
                    "Could not import “{name}” from {service}. {failure}"
                ))));
            }
        }
    }
}

fn delivery_words(service: Service, shipment: &Shipment, sent: &caditor_cloud::Sent) -> String {
    let folder = &shipment.destination.folder_name;
    match sent.outcome {
        SendOutcome::Created => format!(
            "Sent “{}” to {service}, in “{folder}”, as a new file.",
            sent.name
        ),
        SendOutcome::NewVersion {
            number: Some(number),
        } => format!(
            "Sent “{}” to {service}, in “{folder}”, as version {number} of the file of that name.",
            sent.name
        ),
        SendOutcome::NewVersion { number: None } => format!(
            "Sent “{}” to {service}, in “{folder}”, as a new version of the file of that name.",
            sent.name
        ),
    }
}

fn settle<T>(result: Result<T, Arc<Failure>>) -> Load<T> {
    match result {
        Ok(value) => Load::Ready(value),
        Err(failure) => Load::Failed(failure),
    }
}
