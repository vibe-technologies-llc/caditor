use std::io::Write;

use serde::Deserialize;
use serde_json::json;

use crate::{
    catalogue::{
        Account, Conversion, Design, DesignId, Download, Folder, FolderRef, Project, ProjectId,
        Translation, Workspace, WorkspaceId, WorkspaceKind,
    },
    encoding::{form, percent_encode},
    error::{Error, Subject},
    pkce::{self, Verifier},
    provider::{Callback, Provider, Redirect, SignIn},
    rest::{self, Token, checked},
    service::Service,
    session::Session,
    transport::{Body, Request, Response, Transport},
};

const SERVICE: Service = Service::Onshape;
const API: &str = "https://cad.onshape.com/api/v10";
const AUTHORIZE: &str = "https://oauth.onshape.com/oauth/authorize";
const TOKEN: &str = "https://oauth.onshape.com/oauth/token";
const DOCUMENT_PAGE: usize = 20;
const MAX_PAGES: usize = 20;
const DOCUMENTS: &str = "documents";
const STEP_FORMAT: &str = "STEP";
const DONE: &str = "DONE";
const FAILED: &str = "FAILED";

pub struct Onshape {
    client_id: String,
}

impl Onshape {
    pub fn new(client_id: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into().trim().to_owned(),
        }
    }

    fn get<T: serde::de::DeserializeOwned>(
        &self,
        net: &dyn Transport,
        session: &Session,
        url: String,
        subject: Subject,
    ) -> Result<T, Error> {
        rest::get(SERVICE, net, session, url, subject)
    }

    fn token(&self, net: &dyn Transport, fields: &[(&str, &str)]) -> Result<Token, Error> {
        rest::token(SERVICE, net, TOKEN, fields)
    }

    fn default_workspace(
        &self,
        net: &dyn Transport,
        session: &Session,
        project: &ProjectId,
    ) -> Result<String, Error> {
        let url = format!("{API}/documents/{}", percent_encode(project.as_str()));
        let document: DocumentDetail = self.get(net, session, url, Subject::Folder)?;
        Ok(document.default_workspace.id)
    }
}

impl Provider for Onshape {
    fn service(&self) -> Service {
        SERVICE
    }

    fn start_sign_in(&self, redirect: &Redirect) -> Result<SignIn, Error> {
        if self.client_id.is_empty() {
            return Err(Error::NoClientId { service: SERVICE });
        }
        let verifier = Verifier::generate()?;
        let state = pkce::state()?;
        let query = form(&[
            ("response_type", "code"),
            ("client_id", &self.client_id),
            ("redirect_uri", &redirect.uri),
            ("state", &state),
            ("code_challenge", &verifier.challenge()),
            ("code_challenge_method", "S256"),
        ]);
        Ok(SignIn {
            url: format!("{AUTHORIZE}?{query}"),
            state,
            verifier,
            redirect: redirect.clone(),
        })
    }

    fn finish_sign_in(
        &self,
        net: &dyn Transport,
        sign_in: &SignIn,
        callback: &Callback,
        now: u64,
    ) -> Result<Session, Error> {
        let token = self.token(
            net,
            &[
                ("grant_type", "authorization_code"),
                ("client_id", &self.client_id),
                ("code", &callback.code),
                ("redirect_uri", &sign_in.redirect.uri),
                ("code_verifier", sign_in.verifier.as_str()),
            ],
        )?;
        Ok(token.into_session(None, now))
    }

    fn refresh(&self, net: &dyn Transport, session: &Session, now: u64) -> Result<Session, Error> {
        let Some(refresh) = &session.refresh_token else {
            return Err(Error::SignedOut);
        };
        let token = self.token(
            net,
            &[
                ("grant_type", "refresh_token"),
                ("client_id", &self.client_id),
                ("refresh_token", refresh),
            ],
        )?;
        Ok(token.into_session(Some(refresh), now))
    }

    fn revoke(&self, _net: &dyn Transport, _session: &Session) -> Result<(), Error> {
        Ok(())
    }

    fn account(&self, net: &dyn Transport, session: &Session) -> Result<Account, Error> {
        let url = format!("{API}/users/sessioninfo");
        let info: SessionInfo = self.get(net, session, url, Subject::Account)?;
        Ok(Account {
            name: info.display_name(),
        })
    }

    fn workspaces(
        &self,
        _net: &dyn Transport,
        _session: &Session,
    ) -> Result<Vec<Workspace>, Error> {
        Ok(vec![Workspace {
            id: WorkspaceId::new(DOCUMENTS),
            name: "Your documents".to_owned(),
            kind: WorkspaceKind::Personal,
        }])
    }

    fn projects(
        &self,
        net: &dyn Transport,
        session: &Session,
        workspace: &WorkspaceId,
    ) -> Result<Vec<Project>, Error> {
        let mut found = Vec::new();
        let mut next = Some(format!(
            "{API}/documents?sortColumn=modifiedAt&sortOrder=desc&limit={DOCUMENT_PAGE}"
        ));
        for _ in 0..MAX_PAGES {
            let Some(url) = next.take() else {
                break;
            };
            let page: DocumentPage = self.get(net, session, url, Subject::Projects)?;
            found.extend(page.items.into_iter().map(|document| {
                let name = document.title();
                Project {
                    id: ProjectId::new(document.id),
                    workspace: workspace.clone(),
                    name,
                }
            }));
            next = page.next.filter(|link| link.starts_with(API));
        }
        Ok(found)
    }

    fn root(
        &self,
        net: &dyn Transport,
        session: &Session,
        project: &Project,
    ) -> Result<Folder, Error> {
        let workspace = self.default_workspace(net, session, &project.id)?;
        let url = format!(
            "{API}/documents/d/{}/w/{}/elements",
            percent_encode(project.id.as_str()),
            percent_encode(&workspace)
        );
        let elements: Vec<Element> = self.get(net, session, url, Subject::Folder)?;
        let mut folder = Folder {
            folders: Vec::new(),
            designs: elements
                .iter()
                .filter_map(|element| element.design(&project.id, &workspace))
                .collect(),
        };
        folder.newest_first();
        Ok(folder)
    }

    fn folder(
        &self,
        _net: &dyn Transport,
        _session: &Session,
        _folder: &FolderRef,
    ) -> Result<Folder, Error> {
        Ok(Folder::default())
    }

    fn start_conversion(
        &self,
        net: &dyn Transport,
        session: &Session,
        design: &Design,
    ) -> Result<Translation, Error> {
        let place = DesignPlace::parse(&design.id).ok_or(Error::NotFound {
            subject: Subject::Design,
        })?;
        let url = format!(
            "{API}/{}/d/{}/w/{}/e/{}/translations",
            place.kind.path(),
            percent_encode(design.project.as_str()),
            percent_encode(place.workspace),
            percent_encode(place.element)
        );
        let job = json!({ "formatName": STEP_FORMAT, "storeInDocument": false });
        let request =
            Request::post(url, Body::Json(job.to_string())).with_bearer(&session.access_token);
        let response = checked(SERVICE, net.send(&request)?, Subject::Translation)?;
        let started: TranslationState = rest::read(SERVICE, &response, Subject::Translation)?;
        let id = started.id.ok_or(Error::Unreadable {
            service: SERVICE,
            subject: Subject::Translation,
        })?;
        Ok(Translation {
            source: format!("{}/{id}", design.project.as_str()),
        })
    }

    fn poll_conversion(
        &self,
        net: &dyn Transport,
        session: &Session,
        translation: &Translation,
    ) -> Result<Conversion, Error> {
        let (document, id) = translation.source.split_once('/').ok_or(Error::NotFound {
            subject: Subject::Translation,
        })?;
        let url = format!("{API}/translations/{}", percent_encode(id));
        let state: TranslationState = self.get(net, session, url, Subject::Translation)?;
        match state.request_state.as_deref() {
            Some(DONE) => state
                .result_external_data_ids
                .into_iter()
                .next()
                .map(|file| {
                    Conversion::Ready(Download {
                        source: document.to_owned(),
                        artifact: file,
                    })
                })
                .ok_or(Error::NoStepFile { service: SERVICE }),
            Some(FAILED) => Err(Error::ConversionFailed { service: SERVICE }),
            _ => Ok(Conversion::Working { percent: None }),
        }
    }

    fn fetch(
        &self,
        net: &dyn Transport,
        session: &Session,
        download: &Download,
        sink: &mut dyn Write,
        progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
    ) -> Result<u64, Error> {
        let url = format!(
            "{API}/documents/d/{}/externaldata/{}",
            percent_encode(&download.source),
            percent_encode(&download.artifact)
        );
        let request = Request::get(url).with_bearer(&session.access_token);
        let fetched = net.fetch(&request, sink, progress)?;
        let answer = Response {
            status: fetched.status,
            body: Vec::new(),
            retry_after_seconds: fetched.retry_after_seconds,
        };
        checked(SERVICE, answer, Subject::Download)?;
        Ok(fetched.bytes)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ElementKind {
    PartStudio,
    Assembly,
}

impl ElementKind {
    fn from_type(text: &str) -> Option<Self> {
        match text {
            "PARTSTUDIO" => Some(Self::PartStudio),
            "ASSEMBLY" => Some(Self::Assembly),
            _ => None,
        }
    }

    fn path(self) -> &'static str {
        match self {
            Self::PartStudio => "partstudios",
            Self::Assembly => "assemblies",
        }
    }

    fn from_path(text: &str) -> Option<Self> {
        [Self::PartStudio, Self::Assembly]
            .into_iter()
            .find(|kind| kind.path() == text)
    }
}

struct DesignPlace<'a> {
    kind: ElementKind,
    workspace: &'a str,
    element: &'a str,
}

impl<'a> DesignPlace<'a> {
    fn encode(kind: ElementKind, workspace: &str, element: &str) -> DesignId {
        DesignId::new(format!("{}/{workspace}/{element}", kind.path()))
    }

    fn parse(id: &'a DesignId) -> Option<Self> {
        let mut parts = id.as_str().split('/');
        let kind = ElementKind::from_path(parts.next()?)?;
        let workspace = parts.next()?;
        let element = parts.next()?;
        (parts.next().is_none() && !workspace.is_empty() && !element.is_empty()).then_some(Self {
            kind,
            workspace,
            element,
        })
    }
}

#[derive(Deserialize)]
struct SessionInfo {
    name: Option<String>,
    email: Option<String>,
}

impl SessionInfo {
    fn display_name(self) -> String {
        [self.name, self.email]
            .into_iter()
            .flatten()
            .map(|text| text.trim().to_owned())
            .find(|text| !text.is_empty())
            .unwrap_or_else(|| SERVICE.account_name().to_owned())
    }
}

#[derive(Deserialize)]
struct DocumentPage {
    #[serde(default)]
    items: Vec<DocumentSummary>,
    next: Option<String>,
}

#[derive(Deserialize)]
struct DocumentSummary {
    id: String,
    name: Option<String>,
}

impl DocumentSummary {
    fn title(&self) -> String {
        self.name
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map_or_else(|| "Untitled".to_owned(), str::to_owned)
    }
}

#[derive(Deserialize)]
struct DocumentDetail {
    #[serde(rename = "defaultWorkspace")]
    default_workspace: WorkspaceRef,
}

#[derive(Deserialize)]
struct WorkspaceRef {
    id: String,
}

#[derive(Deserialize)]
struct Element {
    id: String,
    name: Option<String>,
    #[serde(rename = "elementType")]
    element_type: Option<String>,
}

impl Element {
    fn design(&self, document: &ProjectId, workspace: &str) -> Option<Design> {
        let kind = ElementKind::from_type(self.element_type.as_deref()?)?;
        let name = self
            .name
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map_or_else(|| "Untitled".to_owned(), str::to_owned);
        Some(Design {
            id: DesignPlace::encode(kind, workspace, &self.id),
            project: document.clone(),
            name,
            modified: None,
        })
    }
}

#[derive(Deserialize)]
struct TranslationState {
    id: Option<String>,
    #[serde(rename = "requestState")]
    request_state: Option<String>,
    #[serde(default, rename = "resultExternalDataIds")]
    result_external_data_ids: Vec<String>,
}

#[cfg(test)]
mod tests;
