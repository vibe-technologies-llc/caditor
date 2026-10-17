use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};

use caditor_cloud::{
    Account, Client, Design, Folder, FolderRef, Project, ProjectId, Service, Workspace,
    WorkspaceId, WorkspaceKind,
};

use super::{downloads::Usage, failure::Failure};

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

pub static IDLE_FOLDER: Load<Folder> = Load::Idle;

#[derive(Debug, Clone)]
pub enum Stage {
    Starting,
    Converting(Option<u8>),
    Downloading { received: u64, total: Option<u64> },
    Failed(Arc<Failure>),
}

#[derive(Debug, Clone)]
pub struct Import {
    pub id: u64,
    pub design: Design,
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
    pub import: Option<Import>,
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
            import: None,
        }
    }

    pub fn root(&self, project: &ProjectId) -> &Load<Folder> {
        self.roots.get(project).unwrap_or(&IDLE_FOLDER)
    }

    pub fn folder(&self, folder: &FolderRef) -> &Load<Folder> {
        self.folders.get(folder).unwrap_or(&IDLE_FOLDER)
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
    ReloadProject(Service, ProjectId),
    OpenFolder(Service, FolderRef),
    ReloadFolder(Service, FolderRef),
    Import(Service, Design),
    CancelImport(Service),
    DismissImport(Service),
    MeasureDownloads,
    ClearDownloads,
}

#[derive(Debug, Clone, Default)]
pub struct Downloads {
    pub dir: Option<PathBuf>,
    pub usage: Option<Usage>,
    pub measuring: bool,
}
