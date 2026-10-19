use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};

use caditor_cloud::{
    Account, Client, Design, DesignId, Folder, FolderRef, Project, ProjectId, SendAs, Service,
    Upload, Version, Workspace, WorkspaceId, WorkspaceKind,
};

use super::{downloads::Usage, failure::Failure, thumbnails::Thumbnail};

#[derive(Debug, Clone)]
pub enum Load<T> {
    Idle,
    Loading,
    Ready(T),
    Failed(Arc<Failure>),
}

impl<T> Load<T> {
    pub fn ready(&self) -> Option<&T> {
        match self {
            Self::Ready(value) => Some(value),
            _ => None,
        }
    }

    pub fn is_loading(&self) -> bool {
        matches!(self, Self::Loading)
    }

    pub fn failure(&self) -> Option<&Arc<Failure>> {
        match self {
            Self::Failed(failure) => Some(failure),
            _ => None,
        }
    }
}

pub const DESIGN_EXTENSION: &str = "step";

pub static IDLE_FOLDER: Load<Folder> = Load::Idle;
pub static IDLE_VERSIONS: Load<Vec<Version>> = Load::Idle;

#[derive(Debug, Clone)]
pub enum Stage {
    Starting,
    Converting(Option<u8>),
    Downloading { received: u64, total: Option<u64> },
    Failed(Arc<Failure>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportItem {
    Design(Design),
    Version(Design, Version),
    Upload(Upload),
}

impl ImportItem {
    pub fn name(&self) -> String {
        match self {
            Self::Design(design) => design.name.clone(),
            Self::Version(design, version) => {
                format!("{} (version {})", design.name, version.number)
            }
            Self::Upload(upload) => upload.name.clone(),
        }
    }

    pub fn stem(&self) -> String {
        match self {
            Self::Design(design) => design.name.clone(),
            Self::Version(design, version) => format!("{} v{}", design.name, version.number),
            Self::Upload(upload) => upload.stem().to_owned(),
        }
    }

    pub fn extension(&self) -> &'static str {
        match self {
            Self::Design(_) | Self::Version(..) => DESIGN_EXTENSION,
            Self::Upload(upload) => upload.format.extension(),
        }
    }

    pub fn project(&self) -> &ProjectId {
        match self {
            Self::Design(design) | Self::Version(design, _) => &design.project,
            Self::Upload(upload) => &upload.project,
        }
    }

    pub fn key(&self) -> String {
        match self {
            Self::Design(design) => design.id.as_str().to_owned(),
            Self::Version(design, version) => {
                format!("{}\0{}", design.id.as_str(), version.id.as_str())
            }
            Self::Upload(upload) => upload.id.as_str().to_owned(),
        }
    }

    pub fn is_converted(&self) -> bool {
        !matches!(self, Self::Upload(_))
    }

    pub fn command(&self, service: Service) -> CloudCommand {
        match self {
            Self::Design(design) => CloudCommand::Import(service, design.clone()),
            Self::Version(design, version) => {
                CloudCommand::ImportVersion(service, design.clone(), version.clone())
            }
            Self::Upload(upload) => CloudCommand::ImportUpload(service, upload.clone()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AfterSend {
    #[default]
    Nothing,
    OpenOnTheWeb,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destination {
    pub service: Service,
    pub folder: FolderRef,
    pub folder_name: String,
    pub how: SendAs,
    pub after: AfterSend,
}

impl Destination {
    pub fn opening(&self) -> bool {
        self.after == AfterSend::OpenOnTheWeb
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remembered {
    pub workspace: WorkspaceId,
    pub folder: FolderRef,
    pub folder_name: String,
}

impl Remembered {
    pub fn destination(&self, service: Service) -> Destination {
        Destination {
            service,
            folder: self.folder.clone(),
            folder_name: self.folder_name.clone(),
            how: SendAs::File,
            after: AfterSend::OpenOnTheWeb,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unopened {
    pub name: String,
    pub link: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shipment {
    pub destination: Destination,
    pub name: String,
    pub file: PathBuf,
}

#[derive(Debug, Clone)]
pub enum SendStage {
    Starting,
    Uploading { sent: u64, total: Option<u64> },
    Translating,
    Failed(Arc<Failure>),
}

#[derive(Debug, Clone)]
pub struct Send {
    pub id: u64,
    pub shipment: Shipment,
    pub stage: SendStage,
    pub(super) stop: Arc<AtomicBool>,
}

impl Send {
    pub fn is_running(&self) -> bool {
        !matches!(self.stage, SendStage::Failed(_))
    }
}

#[derive(Debug, Clone)]
pub struct Import {
    pub id: u64,
    pub item: ImportItem,
    pub stage: Stage,
    pub(super) stop: Arc<AtomicBool>,
}

impl Import {
    pub fn is_running(&self) -> bool {
        !matches!(self.stage, Stage::Failed(_))
    }
}

pub struct SigningIn {
    pub url: Option<String>,
    pub(super) attempt: u64,
    pub(super) stop: Arc<AtomicBool>,
}

pub struct Linked {
    pub(super) client: Arc<Client>,
    pub account: Load<Account>,
    pub workspaces: Load<Vec<Workspace>>,
    pub workspace: Option<WorkspaceId>,
    pub projects: Load<Vec<Project>>,
    pub(super) roots: BTreeMap<ProjectId, Load<Folder>>,
    pub(super) folders: BTreeMap<FolderRef, Load<Folder>>,
    pub(super) thumbnails: BTreeMap<DesignId, Load<Option<Arc<Thumbnail>>>>,
    pub(super) versions: BTreeMap<DesignId, Load<Vec<Version>>>,
    pub(super) destinations: Vec<Remembered>,
    pub import: Option<Import>,
    pub sending: Option<Send>,
    pub unopened: Option<Unopened>,
}

impl Linked {
    pub(super) fn new(client: Arc<Client>) -> Self {
        Self {
            client,
            account: Load::Idle,
            workspaces: Load::Idle,
            workspace: None,
            projects: Load::Idle,
            roots: BTreeMap::new(),
            folders: BTreeMap::new(),
            thumbnails: BTreeMap::new(),
            versions: BTreeMap::new(),
            destinations: Vec::new(),
            import: None,
            sending: None,
            unopened: None,
        }
    }

    pub fn remembered_destination(&self) -> Option<&Remembered> {
        let hub = self.workspace.as_ref()?;
        self.destinations
            .iter()
            .find(|remembered| remembered.workspace == *hub)
    }

    pub fn root(&self, project: &ProjectId) -> &Load<Folder> {
        self.roots.get(project).unwrap_or(&IDLE_FOLDER)
    }

    pub fn folder(&self, folder: &FolderRef) -> &Load<Folder> {
        self.folders.get(folder).unwrap_or(&IDLE_FOLDER)
    }

    pub fn versions(&self, design: &DesignId) -> &Load<Vec<Version>> {
        self.versions.get(design).unwrap_or(&IDLE_VERSIONS)
    }

    pub fn picture(&self, design: &DesignId) -> Option<&Arc<Thumbnail>> {
        match self.thumbnails.get(design)? {
            Load::Ready(picture) => picture.as_ref(),
            _ => None,
        }
    }

    pub fn wants_picture(&self, design: &DesignId) -> bool {
        !self.thumbnails.contains_key(design)
    }

    pub fn project(&self, id: &ProjectId) -> Option<&Project> {
        self.projects
            .ready()
            .and_then(|projects| projects.iter().find(|project| project.id == *id))
    }

    pub fn chosen_workspace(&self) -> Option<&Workspace> {
        let chosen = self.workspace.as_ref()?;
        self.workspaces
            .ready()?
            .iter()
            .find(|workspace| workspace.id == *chosen)
    }

    pub fn is_importing(&self) -> bool {
        self.import.as_ref().is_some_and(Import::is_running)
    }

    pub fn is_sending(&self) -> bool {
        self.sending.as_ref().is_some_and(Send::is_running)
    }
}

pub fn first_workspace(workspaces: &[Workspace]) -> Option<&Workspace> {
    [
        WorkspaceKind::Team,
        WorkspaceKind::Personal,
        WorkspaceKind::Other,
    ]
    .into_iter()
    .find_map(|kind| workspaces.iter().find(|workspace| workspace.kind == kind))
}

pub enum Link {
    Off,
    Starting,
    SignedOut,
    SigningIn(SigningIn),
    Linked(Box<Linked>),
}

pub struct Connection {
    pub service: Service,
    pub link: Link,
    pub last_failure: Option<Arc<Failure>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloudCommand {
    SignIn(Service),
    CancelSignIn(Service),
    SignOut(Service),
    Refresh(Service),
    ChooseWorkspace(Service, WorkspaceId),
    LoadProject(Service, ProjectId),
    LoadThumbnail(Service, Design),
    ReloadProject(Service, ProjectId),
    OpenFolder(Service, FolderRef),
    ReloadFolder(Service, FolderRef),
    Import(Service, Design),
    ImportUpload(Service, Upload),
    ImportVersion(Service, Design, Version),
    LoadVersions(Service, Design),
    ReloadVersions(Service, Design),
    CancelImport(Service),
    DismissImport(Service),
    Send(Shipment),
    CancelSend(Service),
    DismissSend(Service),
    OpenInFusion,
    DismissUnopened(Service),
    MeasureDownloads,
    ClearDownloads,
}

#[derive(Debug, Clone, Default)]
pub struct Downloads {
    pub dir: Option<PathBuf>,
    pub usage: Option<Usage>,
    pub measuring: bool,
}
