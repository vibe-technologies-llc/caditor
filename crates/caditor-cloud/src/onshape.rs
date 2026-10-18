use std::io::Write;

use jiff::Timestamp;
use serde::Deserialize;
use serde_json::json;

use self::scope::{DesignPlace, ElementKind, Place, Scope, UploadPlace};
use crate::{
    catalogue::{
        Account, Conversion, Design, Download, Folder, FolderEntry, FolderRef, Outgoing, Project,
        ProjectId, SendOutcome, Sent, Translation, Upload, UploadFormat, Version as DesignVersion,
        Workspace, WorkspaceId, WorkspaceKind,
    },
    encoding::{form, percent_encode},
    error::{Error, Subject},
    pkce::{self, Verifier},
    provider::{Callback, Provider, Redirect, SignIn},
    rest::{self, Token, checked},
    service::Service,
    session::Session,
    transport::{Body, Multipart, Request, Response, Transport},
};

mod scope;

const SERVICE: Service = Service::Onshape;
const API: &str = "https://cad.onshape.com/api/v10";
const AUTHORIZE: &str = "https://oauth.onshape.com/oauth/authorize";
const TOKEN: &str = "https://oauth.onshape.com/oauth/token";
const API_PREFIX: &str = "https://cad.onshape.com/api/";
const DOCUMENT_PAGE: usize = 20;
const VERSION_LIMIT: usize = 50;
const MAX_PAGES: usize = 20;
const STEP_FORMAT: &str = "STEP";
const DONE: &str = "DONE";
const FAILED: &str = "FAILED";
const BLOB: &str = "BLOB";
const FILE_FIELD: &str = "file";

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

    fn elements(
        &self,
        net: &dyn Transport,
        session: &Session,
        document: &ProjectId,
        place: &Place,
    ) -> Result<Vec<Element>, Error> {
        let url = format!(
            "{API}/documents/d/{}/{}/elements",
            percent_encode(document.as_str()),
            place.path()
        );
        self.get(net, session, url, Subject::Folder)
    }

    fn contents(
        &self,
        net: &dyn Transport,
        session: &Session,
        document: &ProjectId,
        place: &Place,
    ) -> Result<Folder, Error> {
        let elements = self.elements(net, session, document, place)?;
        let mut folder = Folder {
            place: place.is_workspace().then(|| FolderRef {
                project: document.clone(),
                folder: place.folder_id(),
            }),
            designs: elements
                .iter()
                .filter_map(|element| element.design(document, place))
                .collect(),
            uploads: elements
                .iter()
                .filter_map(|element| element.upload(document, place))
                .collect(),
            ..Folder::default()
        };
        folder.newest_first();
        Ok(folder)
    }

    fn existing_blob(
        &self,
        net: &dyn Transport,
        session: &Session,
        document: &ProjectId,
        place: &Place,
        name: &str,
    ) -> Result<Option<String>, Error> {
        let wanted = name.to_lowercase();
        Ok(self
            .elements(net, session, document, place)?
            .into_iter()
            .find(|element| {
                element.element_type.as_deref() == Some(BLOB)
                    && element
                        .name
                        .as_deref()
                        .map(|found| found.trim().to_lowercase())
                        == Some(wanted.clone())
            })
            .map(|element| element.id))
    }

    fn upload_file(
        &self,
        net: &dyn Transport,
        session: &Session,
        url: String,
        name: &str,
        bytes: &[u8],
    ) -> Result<(), Error> {
        let fields = [
            ("encodedFilename", percent_encode(name)),
            ("translate", "false".to_owned()),
            ("storeInDocument", "true".to_owned()),
        ]
        .into_iter()
        .map(|(field, value)| (field.to_owned(), value))
        .collect();
        let multipart = Multipart::new(fields, FILE_FIELD, name, bytes.to_vec())?;
        let request =
            Request::post(url, Body::Multipart(multipart)).with_bearer(&session.access_token);
        let response = net.send(&request)?;
        let response = match response.status {
            403 => return Err(Error::SendForbidden { service: SERVICE }),
            _ => checked(SERVICE, response, Subject::Item)?,
        };
        let _: UploadedElement = rest::read(SERVICE, &response, Subject::Item)?;
        Ok(())
    }

    fn branches(
        &self,
        net: &dyn Transport,
        session: &Session,
        project: &Project,
        default: &str,
    ) -> Result<Vec<FolderEntry>, Error> {
        let url = format!(
            "{API}/documents/d/{}/workspaces",
            percent_encode(project.id.as_str())
        );
        let branches: Vec<Branch> = self.get(net, session, url, Subject::Folder)?;
        let mut entries: Vec<_> = branches
            .into_iter()
            .filter(|branch| branch.id != default)
            .map(|branch| {
                let place = Place::workspace(&branch.id);
                (
                    branch.label(),
                    FolderEntry {
                        reference: FolderRef {
                            project: project.id.clone(),
                            folder: place.folder_id(),
                        },
                        name: format!("Branch {}", branch.label()),
                    },
                )
            })
            .collect();
        entries.sort_by(|first, second| first.0.cmp(&second.0));
        Ok(entries.into_iter().map(|(_, entry)| entry).collect())
    }

    fn versions(
        &self,
        net: &dyn Transport,
        session: &Session,
        project: &Project,
    ) -> Result<Vec<FolderEntry>, Error> {
        let url = format!(
            "{API}/documents/d/{}/versions?limit={VERSION_LIMIT}",
            percent_encode(project.id.as_str())
        );
        let mut versions: Vec<Version> = self.get(net, session, url, Subject::Folder)?;
        versions.sort_by_key(|version| std::cmp::Reverse(version.created()));
        Ok(versions
            .into_iter()
            .map(|version| FolderEntry {
                reference: FolderRef {
                    project: project.id.clone(),
                    folder: Place::version(&version.id).folder_id(),
                },
                name: format!("Version {}", version.label()),
            })
            .collect())
    }

    fn listed(
        &self,
        net: &dyn Transport,
        session: &Session,
        first: String,
        subject: Subject,
    ) -> Result<Vec<Listed>, Error> {
        let mut found = Vec::new();
        let mut next = Some(first);
        for _ in 0..MAX_PAGES {
            let Some(url) = next.take() else {
                break;
            };
            let page: Page = self.get(net, session, url, subject)?;
            found.extend(page.items);
            next = page.next.filter(|link| link.starts_with(API_PREFIX));
        }
        Ok(found)
    }

    fn scopes(&self, net: &dyn Transport, session: &Session) -> Result<Vec<Workspace>, Error> {
        let fixed = |scope: Scope, name: &str| Workspace {
            id: WorkspaceId::new(scope.id()),
            name: name.to_owned(),
            kind: if scope == Scope::Mine {
                WorkspaceKind::Personal
            } else {
                WorkspaceKind::Other
            },
        };
        let companies = optional(self.listed(
            net,
            session,
            format!("{API}/companies"),
            Subject::Workspaces,
        ))?;
        let teams =
            optional(self.listed(net, session, format!("{API}/teams"), Subject::Workspaces))?;
        let mut scopes = vec![
            fixed(Scope::Mine, "My documents"),
            fixed(Scope::Recent, "Recently opened"),
            fixed(Scope::Shared, "Shared with me"),
        ];
        scopes.extend(companies.into_iter().map(|company| {
            fixed(
                Scope::Company(company.id),
                &format!("{} (company)", company.name.as_deref().unwrap_or("Company")),
            )
        }));
        scopes.extend(teams.into_iter().map(|team| {
            fixed(
                Scope::Team(team.id),
                &format!("{} (team)", team.name.as_deref().unwrap_or("Team")),
            )
        }));
        scopes.push(fixed(Scope::Public, "Public documents"));
        Ok(scopes)
    }
}

fn optional<T: Default>(result: Result<T, Error>) -> Result<T, Error> {
    match result {
        Err(
            Error::Forbidden { .. }
            | Error::NotFound { .. }
            | Error::Refused { .. }
            | Error::Unreadable { .. },
        ) => Ok(T::default()),
        other => other,
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

    fn workspaces(&self, net: &dyn Transport, session: &Session) -> Result<Vec<Workspace>, Error> {
        self.scopes(net, session)
    }

    fn projects(
        &self,
        net: &dyn Transport,
        session: &Session,
        workspace: &WorkspaceId,
    ) -> Result<Vec<Project>, Error> {
        let scope = Scope::parse(workspace.as_str()).ok_or(Error::NotFound {
            subject: Subject::Projects,
        })?;
        let order = if scope.is_ordered_by_server() {
            String::new()
        } else {
            "&sortColumn=modifiedAt&sortOrder=desc".to_owned()
        };
        let first = format!(
            "{API}/documents?{}{order}&limit={DOCUMENT_PAGE}",
            scope.query()
        );
        Ok(self
            .listed(net, session, first, Subject::Projects)?
            .into_iter()
            .filter(Listed::is_document)
            .map(|document| {
                let name = document.title();
                Project {
                    id: ProjectId::new(document.id),
                    workspace: workspace.clone(),
                    name,
                }
            })
            .collect())
    }

    fn root(
        &self,
        net: &dyn Transport,
        session: &Session,
        project: &Project,
    ) -> Result<Folder, Error> {
        let default = self.default_workspace(net, session, &project.id)?;
        let main = self.contents(net, session, &project.id, &Place::workspace(&default))?;
        let mut folders = optional(self.branches(net, session, project, &default))?;
        folders.extend(optional(self.versions(net, session, project))?);
        Ok(Folder { folders, ..main })
    }

    fn folder(
        &self,
        net: &dyn Transport,
        session: &Session,
        folder: &FolderRef,
    ) -> Result<Folder, Error> {
        let place = Place::from_folder_id(&folder.folder).ok_or(Error::NotFound {
            subject: Subject::Folder,
        })?;
        self.contents(net, session, &folder.project, &place)
    }

    fn versions(
        &self,
        _net: &dyn Transport,
        _session: &Session,
        _design: &Design,
    ) -> Result<Vec<DesignVersion>, Error> {
        Ok(Vec::new())
    }

    fn start_conversion(
        &self,
        net: &dyn Transport,
        session: &Session,
        design: &Design,
        _version: Option<&DesignVersion>,
    ) -> Result<Translation, Error> {
        let place = DesignPlace::parse(&design.id).ok_or(Error::NotFound {
            subject: Subject::Design,
        })?;
        let url = format!(
            "{API}/{}/d/{}/{}/e/{}/translations",
            place.kind.path(),
            percent_encode(design.project.as_str()),
            place.place.path(),
            percent_encode(&place.element)
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

    fn send(
        &self,
        net: &dyn Transport,
        session: &Session,
        outgoing: &Outgoing<'_>,
        progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
    ) -> Result<Sent, Error> {
        let name = outgoing.name.trim();
        let place = Place::from_folder_id(&outgoing.folder.folder).ok_or(Error::NotFound {
            subject: Subject::Folder,
        })?;
        if !place.is_workspace() {
            return Err(Error::VersionIsReadOnly);
        }
        let document = &outgoing.folder.project;
        let existing = self.existing_blob(net, session, document, &place, name)?;
        let total = u64::try_from(outgoing.bytes.len()).unwrap_or(u64::MAX);
        if !progress(0, Some(total)) {
            return Err(Error::Stopped);
        }
        let base = format!(
            "{API}/blobelements/d/{}/{}",
            percent_encode(document.as_str()),
            place.path()
        );
        let (url, outcome) = match existing {
            Some(element) => (
                format!("{base}/e/{}", percent_encode(&element)),
                SendOutcome::NewVersion { number: None },
            ),
            None => (base, SendOutcome::Created),
        };
        self.upload_file(net, session, url, name, outgoing.bytes)?;
        progress(total, Some(total));
        Ok(Sent {
            name: name.to_owned(),
            outcome,
        })
    }

    fn fetch_upload(
        &self,
        net: &dyn Transport,
        session: &Session,
        upload: &Upload,
        sink: &mut dyn Write,
        progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
    ) -> Result<u64, Error> {
        let place = UploadPlace::parse(&upload.id).ok_or(Error::NotFound {
            subject: Subject::Upload,
        })?;
        let url = format!(
            "{API}/blobelements/d/{}/{}/e/{}",
            percent_encode(upload.project.as_str()),
            place.place.path(),
            percent_encode(&place.element)
        );
        let request = Request::get(url).with_bearer(&session.access_token);
        let fetched = net.fetch(&request, sink, progress)?;
        let answer = Response {
            status: fetched.status,
            body: Vec::new(),
            retry_after_seconds: fetched.retry_after_seconds,
        };
        checked(SERVICE, answer, Subject::Upload)?;
        Ok(fetched.bytes)
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
struct Page {
    #[serde(default)]
    items: Vec<Listed>,
    next: Option<String>,
}

#[derive(Deserialize)]
struct Listed {
    id: String,
    name: Option<String>,
    #[serde(rename = "jsonType")]
    json_type: Option<String>,
}

impl Listed {
    fn is_document(&self) -> bool {
        matches!(
            self.json_type.as_deref(),
            None | Some("document" | "document-summary" | "document-summary-search")
        )
    }

    fn title(&self) -> String {
        label(self.name.as_deref(), "Untitled")
    }
}

fn label(name: Option<&str>, fallback: &str) -> String {
    name.map(str::trim)
        .filter(|text| !text.is_empty())
        .map_or_else(|| fallback.to_owned(), str::to_owned)
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
    fn design(&self, document: &ProjectId, place: &Place) -> Option<Design> {
        let kind = ElementKind::from_type(self.element_type.as_deref()?)?;
        Some(Design {
            id: DesignPlace::encode(kind, place, &self.id),
            project: document.clone(),
            name: label(self.name.as_deref(), "Untitled"),
            modified: None,
        })
    }

    fn upload(&self, document: &ProjectId, place: &Place) -> Option<Upload> {
        if self.element_type.as_deref()? != BLOB {
            return None;
        }
        let name = label(self.name.as_deref(), "Untitled");
        Some(Upload {
            id: UploadPlace::encode(place, &self.id),
            project: document.clone(),
            format: UploadFormat::of_file_name(&name)?,
            name,
            modified: None,
        })
    }
}

#[derive(Deserialize)]
struct UploadedElement {}

#[derive(Deserialize)]
struct Branch {
    id: String,
    name: Option<String>,
}

impl Branch {
    fn label(&self) -> String {
        label(self.name.as_deref(), "Untitled")
    }
}

#[derive(Deserialize)]
struct Version {
    id: String,
    name: Option<String>,
    #[serde(rename = "createdAt")]
    created_at: Option<String>,
}

impl Version {
    fn label(&self) -> String {
        label(self.name.as_deref(), "Untitled")
    }

    fn created(&self) -> Option<Timestamp> {
        self.created_at.as_deref()?.parse().ok()
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
