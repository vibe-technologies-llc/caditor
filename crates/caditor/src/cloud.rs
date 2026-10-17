use std::{
    borrow::Cow,
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
};

use caditor_cloud::{
    Client, Design, FolderRef, Fusion, Network, Onshape, ProjectId, Provider, Service, Session,
    Transport, WorkspaceId,
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
mod state;
mod store;

pub use self::{
    failure::Failure,
    jobs::Opener,
    state::{
        CloudCommand, Connection, Downloads, Import, Link, Linked, Load, SigningIn, Stage,
        first_workspace,
    },
};
use self::{
    jobs::{Courier, Event, ImportJob, Loaded, SignInJob},
    store::SessionStore,
};

pub const CALLBACK_PORT: u16 = 49617;
const DEFAULT_AUTODESK_CLIENT_ID: &str = "";
const DEFAULT_ONSHAPE_CLIENT_ID: &str = "";
pub const PROJECT_ROWS_ON_HOME: usize = 6;

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
}

impl Reason for Refusal {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

pub struct CloudServices {
    pub make_waker: WakerFactory,
    pub state_dir: Option<PathBuf>,
}

impl CloudServices {
    pub fn without_files() -> Self {
        Self {
            make_waker: Box::new(|| Box::new(|| {})),
            state_dir: None,
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
                self.start_import(service, design, &mut actions)
            }
            CloudCommand::CancelImport(service) => self.cancel_import(service),
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
            .map(|dir| SessionStore::in_state_dir(dir, service))
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
        match left {
            Link::SigningIn(signing) => signing.stop.store(true, Ordering::Relaxed),
            Link::Linked(linked) => {
                linked.import.iter().for_each(|import| {
                    import.stop.store(true, Ordering::Relaxed);
                });
                let client = Arc::clone(&linked.client);
                self.courier.spawn(
                    service,
                    move || {
                        if revoke {
                            jobs::sign_out(&client, store.as_ref());
                        } else {
                            jobs::forget(store.as_ref());
                        }
                        Ok(())
                    },
                    |_: Result<(), Failure>| Event::Left,
                );
                return;
            }
            Link::Off | Link::Starting | Link::SignedOut => {}
        }
        if !revoke {
            jobs::forget(store.as_ref());
        }
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
        linked.projects = Load::Idle;
        self.load_projects(service, workspace);
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
        self.courier.spawn(
            service,
            move || client.workspaces().map_err(Failure::from),
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

    fn start_import(&mut self, service: Service, design: Design, actions: &mut Vec<Action>) {
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
            design: design.clone(),
            stage: Stage::Starting,
            stop: Arc::clone(&stop),
        });
        let job = ImportJob {
            service,
            generation,
            id,
            client: Arc::clone(&linked.client),
            design,
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
            | Loaded::Folder(_, Err(failure)) => Some(Arc::clone(failure)),
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
                if let Ok(workspaces) = &result {
                    let kept = linked
                        .workspace
                        .as_ref()
                        .filter(|id| workspaces.iter().any(|workspace| workspace.id == **id))
                        .cloned();
                    choose = kept.or_else(|| {
                        first_workspace(workspaces).map(|workspace| workspace.id.clone())
                    });
                }
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
        }
        if let Some(workspace) = choose {
            linked.workspace = Some(workspace.clone());
            if matches!(linked.projects, Load::Idle) {
                self.load_projects(service, workspace);
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
        let name = current.design.name.clone();
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

fn settle<T>(result: Result<T, Arc<Failure>>) -> Load<T> {
    match result {
        Ok(value) => Load::Ready(value),
        Err(failure) => Load::Failed(failure),
    }
}
