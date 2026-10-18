use std::{
    collections::{BTreeMap, VecDeque},
    io::Write,
    sync::atomic::{AtomicUsize, Ordering},
};

use parking_lot::Mutex;

use crate::{
    catalogue::{
        Account, Conversion, Design, DesignId, Download, Folder, FolderRef, Project, ProjectId,
        Translation, Upload, UploadId, Version, Workspace, WorkspaceId,
    },
    error::Error,
    pkce::Verifier,
    provider::{Callback, Provider, Redirect, SignIn},
    service::Service,
    session::Session,
    transport::{Fetched, Method, Request, Response, Transport, TransportError},
};

#[derive(Clone)]
enum Reply {
    Answer { status: u16, body: Vec<u8> },
    Fail(TransportError),
}

struct Route {
    method: Method,
    url: String,
    replies: VecDeque<Reply>,
}

#[derive(Default)]
pub struct ScriptedTransport {
    routes: Mutex<Vec<Route>>,
    requests: Mutex<Vec<Request>>,
}

impl ScriptedTransport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn route(self, method: Method, url: &str, status: u16, body: &str) -> Self {
        self.queue(
            method,
            url,
            Reply::Answer {
                status,
                body: body.as_bytes().to_vec(),
            },
        )
    }

    pub fn route_bytes(self, method: Method, url: &str, status: u16, body: &[u8]) -> Self {
        self.queue(
            method,
            url,
            Reply::Answer {
                status,
                body: body.to_vec(),
            },
        )
    }

    pub fn failing(self, method: Method, url: &str, error: TransportError) -> Self {
        self.queue(method, url, Reply::Fail(error))
    }

    fn queue(self, method: Method, url: &str, reply: Reply) -> Self {
        {
            let mut routes = self.routes.lock();
            match routes
                .iter_mut()
                .find(|route| route.method == method && route.url == url)
            {
                Some(route) => route.replies.push_back(reply),
                None => routes.push(Route {
                    method,
                    url: url.to_owned(),
                    replies: VecDeque::from([reply]),
                }),
            }
        }
        self
    }

    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().clone()
    }

    fn answer(&self, request: &Request) -> Result<(u16, Vec<u8>), TransportError> {
        self.requests.lock().push(request.clone());
        let mut routes = self.routes.lock();
        let route = routes
            .iter_mut()
            .find(|route| route.method == request.method && route.url == request.url)
            .ok_or(TransportError::Unreachable)?;
        let reply = if route.replies.len() > 1 {
            route.replies.pop_front()
        } else {
            route.replies.front().cloned()
        };
        match reply {
            Some(Reply::Answer { status, body }) => Ok((status, body)),
            Some(Reply::Fail(error)) => Err(error),
            None => Err(TransportError::Unreachable),
        }
    }
}

impl Transport for ScriptedTransport {
    fn send(&self, request: &Request) -> Result<Response, TransportError> {
        let (status, body) = self.answer(request)?;
        Ok(Response {
            status,
            body,
            retry_after_seconds: None,
        })
    }

    fn fetch(
        &self,
        request: &Request,
        sink: &mut dyn Write,
        progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
    ) -> Result<Fetched, TransportError> {
        let (status, body) = self.answer(request)?;
        if !(200..300).contains(&status) {
            return Ok(Fetched {
                status,
                retry_after_seconds: None,
                bytes: 0,
            });
        }
        sink.write_all(&body)
            .map_err(|error| TransportError::Io(error.kind()))?;
        let bytes = u64::try_from(body.len()).unwrap_or(u64::MAX);
        if !progress(bytes, Some(bytes)) {
            return Err(TransportError::Stopped);
        }
        Ok(Fetched {
            status,
            retry_after_seconds: None,
            bytes,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    SignIn,
    Revoke,
    Account,
    Workspaces,
    Projects,
    Root,
    Folder,
    Conversion,
    Fetch,
    Thumbnail,
    Versions,
}

type Failure = fn() -> Error;

pub struct FakeProvider {
    account: String,
    workspaces: Vec<Workspace>,
    projects: Vec<Project>,
    roots: BTreeMap<ProjectId, Folder>,
    folders: BTreeMap<FolderRef, Folder>,
    step: Vec<u8>,
    thumbnails: BTreeMap<DesignId, Vec<u8>>,
    stored: BTreeMap<UploadId, Vec<u8>>,
    versions: BTreeMap<DesignId, Vec<Version>>,
    working_polls: usize,
    failures: Mutex<Vec<(Step, Failure)>>,
    calls: Mutex<Vec<String>>,
    polled: AtomicUsize,
}

impl Default for FakeProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeProvider {
    pub fn new() -> Self {
        Self {
            account: "Ada Lovelace".to_owned(),
            workspaces: Vec::new(),
            projects: Vec::new(),
            roots: BTreeMap::new(),
            folders: BTreeMap::new(),
            step: Vec::new(),
            thumbnails: BTreeMap::new(),
            stored: BTreeMap::new(),
            versions: BTreeMap::new(),
            working_polls: 0,
            failures: Mutex::new(Vec::new()),
            calls: Mutex::new(Vec::new()),
            polled: AtomicUsize::new(0),
        }
    }

    pub fn account(mut self, name: &str) -> Self {
        self.account = name.to_owned();
        self
    }

    pub fn workspace(mut self, workspace: Workspace) -> Self {
        self.workspaces.push(workspace);
        self
    }

    pub fn project(mut self, project: Project, root: Folder) -> Self {
        self.roots.insert(project.id.clone(), root);
        self.projects.push(project);
        self
    }

    pub fn folder(mut self, reference: FolderRef, contents: Folder) -> Self {
        self.folders.insert(reference, contents);
        self
    }

    pub fn step_file(mut self, bytes: &[u8]) -> Self {
        self.step = bytes.to_vec();
        self
    }

    pub fn thumbnail(mut self, design: DesignId, picture: &[u8]) -> Self {
        self.thumbnails.insert(design, picture.to_vec());
        self
    }

    pub fn stored_file(mut self, upload: UploadId, bytes: &[u8]) -> Self {
        self.stored.insert(upload, bytes.to_vec());
        self
    }

    pub fn versions(mut self, design: DesignId, versions: Vec<Version>) -> Self {
        self.versions.insert(design, versions);
        self
    }

    pub fn working_polls(mut self, polls: usize) -> Self {
        self.working_polls = polls;
        self
    }

    pub fn failing(self, step: Step, failure: Failure) -> Self {
        self.failures.lock().push((step, failure));
        self
    }

    pub fn recover(&self, step: Step) {
        self.failures.lock().retain(|(failing, _)| *failing != step);
    }

    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().clone()
    }

    fn enter(&self, step: Step, call: String) -> Result<(), Error> {
        self.calls.lock().push(call);
        let failure = self
            .failures
            .lock()
            .iter()
            .find(|(failing, _)| *failing == step)
            .map(|(_, failure)| *failure);
        failure.map_or(Ok(()), |failure| Err(failure()))
    }

    fn session(now: u64) -> Session {
        Session {
            access_token: "fake-access".to_owned(),
            refresh_token: Some("fake-refresh".to_owned()),
            expires_at: now.saturating_add(3600),
        }
    }
}

impl Provider for FakeProvider {
    fn service(&self) -> Service {
        Service::Fusion
    }

    fn start_sign_in(&self, redirect: &Redirect) -> Result<SignIn, Error> {
        self.enter(Step::SignIn, "start sign in".to_owned())?;
        Ok(SignIn {
            url: format!(
                "https://example.invalid/authorize?redirect_uri={}&state=fake-state",
                redirect.uri
            ),
            state: "fake-state".to_owned(),
            verifier: Verifier::generate()?,
            redirect: redirect.clone(),
        })
    }

    fn finish_sign_in(
        &self,
        _net: &dyn Transport,
        _sign_in: &SignIn,
        _callback: &Callback,
        now: u64,
    ) -> Result<Session, Error> {
        self.enter(Step::SignIn, "finish sign in".to_owned())?;
        Ok(Self::session(now))
    }

    fn refresh(
        &self,
        _net: &dyn Transport,
        _session: &Session,
        now: u64,
    ) -> Result<Session, Error> {
        self.calls.lock().push("refresh".to_owned());
        Ok(Self::session(now))
    }

    fn revoke(&self, _net: &dyn Transport, _session: &Session) -> Result<(), Error> {
        self.enter(Step::Revoke, "revoke".to_owned())
    }

    fn account(&self, _net: &dyn Transport, _session: &Session) -> Result<Account, Error> {
        self.enter(Step::Account, "account".to_owned())?;
        Ok(Account {
            name: self.account.clone(),
        })
    }

    fn workspaces(
        &self,
        _net: &dyn Transport,
        _session: &Session,
    ) -> Result<Vec<Workspace>, Error> {
        self.enter(Step::Workspaces, "workspaces".to_owned())?;
        Ok(self.workspaces.clone())
    }

    fn projects(
        &self,
        _net: &dyn Transport,
        _session: &Session,
        workspace: &WorkspaceId,
    ) -> Result<Vec<Project>, Error> {
        self.enter(Step::Projects, format!("projects {}", workspace.as_str()))?;
        Ok(self
            .projects
            .iter()
            .filter(|project| project.workspace == *workspace)
            .cloned()
            .collect())
    }

    fn root(
        &self,
        _net: &dyn Transport,
        _session: &Session,
        project: &Project,
    ) -> Result<Folder, Error> {
        self.enter(Step::Root, format!("root {}", project.id.as_str()))?;
        Ok(self.roots.get(&project.id).cloned().unwrap_or_default())
    }

    fn folder(
        &self,
        _net: &dyn Transport,
        _session: &Session,
        folder: &FolderRef,
    ) -> Result<Folder, Error> {
        self.enter(Step::Folder, format!("folder {}", folder.folder.as_str()))?;
        Ok(self.folders.get(folder).cloned().unwrap_or_default())
    }

    fn versions(
        &self,
        _net: &dyn Transport,
        _session: &Session,
        design: &Design,
    ) -> Result<Vec<Version>, Error> {
        self.enter(Step::Versions, format!("versions {}", design.id.as_str()))?;
        Ok(self.versions.get(&design.id).cloned().unwrap_or_default())
    }

    fn start_conversion(
        &self,
        _net: &dyn Transport,
        _session: &Session,
        design: &Design,
        version: Option<&Version>,
    ) -> Result<Translation, Error> {
        let call = match version {
            Some(version) => format!("convert {} {}", design.id.as_str(), version.id.as_str()),
            None => format!("convert {}", design.id.as_str()),
        };
        self.enter(Step::Conversion, call)?;
        self.polled.store(0, Ordering::Relaxed);
        Ok(Translation {
            source: design.id.as_str().to_owned(),
        })
    }

    fn poll_conversion(
        &self,
        _net: &dyn Transport,
        _session: &Session,
        translation: &Translation,
    ) -> Result<Conversion, Error> {
        self.enter(Step::Conversion, "poll".to_owned())?;
        if self.polled.fetch_add(1, Ordering::Relaxed) < self.working_polls {
            return Ok(Conversion::Working { percent: Some(40) });
        }
        Ok(Conversion::Ready(Download {
            source: translation.source.clone(),
            artifact: "fake.step".to_owned(),
        }))
    }

    fn fetch(
        &self,
        _net: &dyn Transport,
        _session: &Session,
        _download: &Download,
        sink: &mut dyn Write,
        progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
    ) -> Result<u64, Error> {
        self.enter(Step::Fetch, "fetch".to_owned())?;
        sink.write_all(&self.step)
            .map_err(|error| Error::Write(error.kind()))?;
        let bytes = u64::try_from(self.step.len()).unwrap_or(u64::MAX);
        if !progress(bytes, Some(bytes)) {
            return Err(Error::Stopped);
        }
        Ok(bytes)
    }

    fn fetch_upload(
        &self,
        _net: &dyn Transport,
        _session: &Session,
        upload: &Upload,
        sink: &mut dyn Write,
        progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
    ) -> Result<u64, Error> {
        self.enter(Step::Fetch, format!("fetch upload {}", upload.id.as_str()))?;
        let bytes = self.stored.get(&upload.id).ok_or(Error::NoStoredFile {
            service: Service::Fusion,
        })?;
        sink.write_all(bytes)
            .map_err(|error| Error::Write(error.kind()))?;
        let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if !progress(length, Some(length)) {
            return Err(Error::Stopped);
        }
        Ok(length)
    }

    fn thumbnail(
        &self,
        _net: &dyn Transport,
        _session: &Session,
        design: &Design,
    ) -> Result<Option<Vec<u8>>, Error> {
        self.enter(Step::Thumbnail, format!("thumbnail {}", design.id.as_str()))?;
        Ok(self.thumbnails.get(&design.id).cloned())
    }
}
