use std::io::Write;

use serde::{Deserialize, de::DeserializeOwned};
use serde_json::json;

use crate::{
    catalogue::{
        Account, Conversion, Design, DesignId, Download, Folder, FolderEntry, FolderId, FolderRef,
        Project, ProjectId, Translation, Workspace, WorkspaceId, WorkspaceKind,
    },
    encoding::{base64_url, form, percent_encode},
    error::{Error, Subject},
    pkce::{self, Verifier},
    provider::{Callback, Provider, Redirect, SignIn},
    rest::{self, Token, checked},
    service::Service,
    session::Session,
    transport::{Body, Request, Response, Transport},
};

const SERVICE: Service = Service::Fusion;
const API: &str = "https://developer.api.autodesk.com";
const AUTHORIZE: &str = "https://developer.api.autodesk.com/authentication/v2/authorize";
const TOKEN: &str = "https://developer.api.autodesk.com/authentication/v2/token";
const REVOKE: &str = "https://developer.api.autodesk.com/authentication/v2/revoke";
const USER_INFO: &str = "https://api.userprofile.autodesk.com/userinfo";
const PROJECT_API: &str = "https://developer.api.autodesk.com/project/v1";
const DATA_API: &str = "https://developer.api.autodesk.com/data/v1";
const DERIVATIVE_API: &str = "https://developer.api.autodesk.com/modelderivative/v2/designdata";
const SCOPES: &str = "data:read data:write user-profile:read";
const FUSION_DESIGN: &str = "items:autodesk.fusion360:Design";
const TEAM_HUB: &str = "hubs:autodesk.core:Hub";
const PERSONAL_HUB: &str = "hubs:autodesk.a360:PersonalHub";
const STEP_OUTPUT: &str = "step";
const STEP_ROLE: &str = "step";
const THUMBNAIL_SIDE: u32 = 200;
const MAX_PAGES: usize = 20;

pub struct Fusion {
    client_id: String,
}

impl Fusion {
    pub fn new(client_id: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into().trim().to_owned(),
        }
    }

    fn get<T: DeserializeOwned>(
        &self,
        net: &dyn Transport,
        session: &Session,
        url: String,
        subject: Subject,
    ) -> Result<T, Error> {
        rest::get(SERVICE, net, session, url, subject)
    }

    fn pages<T>(
        &self,
        net: &dyn Transport,
        session: &Session,
        first: String,
        subject: Subject,
        each: impl Fn(&Page) -> Vec<T>,
    ) -> Result<Vec<T>, Error> {
        let mut found = Vec::new();
        let mut next = Some(first);
        for _ in 0..MAX_PAGES {
            let Some(url) = next.take() else {
                break;
            };
            let page: Page = self.get(net, session, url, subject)?;
            found.extend(each(&page));
            next = page
                .links
                .and_then(|links| links.next)
                .map(|link| link.href)
                .filter(|href| href.starts_with(API));
        }
        Ok(found)
    }

    fn token(&self, net: &dyn Transport, fields: &[(&str, &str)]) -> Result<Token, Error> {
        rest::token(SERVICE, net, TOKEN, fields)
    }

    fn version_urn(
        &self,
        net: &dyn Transport,
        session: &Session,
        design: &Design,
    ) -> Result<String, Error> {
        let url = format!(
            "{DATA_API}/projects/{}/items/{}/tip",
            percent_encode(design.project.as_str()),
            percent_encode(design.id.as_str())
        );
        let tip: Single = self.get(net, session, url, Subject::Design)?;
        Ok(base64_url(tip.data.id.as_bytes()))
    }

    fn contents(
        &self,
        net: &dyn Transport,
        session: &Session,
        folder: &FolderRef,
    ) -> Result<Folder, Error> {
        let url = format!(
            "{DATA_API}/projects/{}/folders/{}/contents",
            percent_encode(folder.project.as_str()),
            percent_encode(folder.folder.as_str())
        );
        let project = folder.project.clone();
        let parts = self.pages(net, session, url, Subject::Folder, |page| {
            page.data
                .iter()
                .filter_map(|resource| resource.entry(&project))
                .collect()
        })?;
        let mut contents = Folder::default();
        for part in parts {
            match part {
                Entry::Folder(entry) => contents.folders.push(entry),
                Entry::Design(design) => contents.designs.push(design),
            }
        }
        contents.newest_first();
        Ok(contents)
    }
}

impl Provider for Fusion {
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
            ("scope", SCOPES),
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
                ("scope", SCOPES),
            ],
        )?;
        Ok(token.into_session(Some(refresh), now))
    }

    fn revoke(&self, net: &dyn Transport, session: &Session) -> Result<(), Error> {
        let (token, hint) = match &session.refresh_token {
            Some(refresh) => (refresh, "refresh_token"),
            None => (&session.access_token, "access_token"),
        };
        let fields = form(&[
            ("token", token),
            ("token_type_hint", hint),
            ("client_id", &self.client_id),
        ]);
        let request = Request::post(REVOKE.to_owned(), Body::Form(fields));
        checked(SERVICE, net.send(&request)?, Subject::Token)?;
        Ok(())
    }

    fn account(&self, net: &dyn Transport, session: &Session) -> Result<Account, Error> {
        let profile: Profile = self.get(net, session, USER_INFO.to_owned(), Subject::Account)?;
        Ok(Account {
            name: profile.display_name(),
        })
    }

    fn workspaces(&self, net: &dyn Transport, session: &Session) -> Result<Vec<Workspace>, Error> {
        self.pages(
            net,
            session,
            format!("{PROJECT_API}/hubs"),
            Subject::Workspaces,
            |page| page.data.iter().filter_map(Resource::workspace).collect(),
        )
    }

    fn projects(
        &self,
        net: &dyn Transport,
        session: &Session,
        workspace: &WorkspaceId,
    ) -> Result<Vec<Project>, Error> {
        let url = format!(
            "{PROJECT_API}/hubs/{}/projects",
            percent_encode(workspace.as_str())
        );
        self.pages(net, session, url, Subject::Projects, |page| {
            page.data
                .iter()
                .filter_map(|resource| resource.project(workspace))
                .collect()
        })
    }

    fn root(
        &self,
        net: &dyn Transport,
        session: &Session,
        project: &Project,
    ) -> Result<Folder, Error> {
        let url = format!(
            "{PROJECT_API}/hubs/{}/projects/{}/topFolders",
            percent_encode(project.workspace.as_str()),
            percent_encode(project.id.as_str())
        );
        let top: Vec<FolderEntry> = self.pages(net, session, url, Subject::Folder, |page| {
            page.data
                .iter()
                .filter(|resource| resource.kind == "folders")
                .filter_map(|resource| match resource.entry(&project.id) {
                    Some(Entry::Folder(entry)) => Some(entry),
                    _ => None,
                })
                .collect()
        })?;
        match <[FolderEntry; 1]>::try_from(top) {
            Ok([only]) => self.contents(net, session, &only.reference),
            Err(several) => {
                let mut root = Folder {
                    folders: several,
                    designs: Vec::new(),
                };
                root.newest_first();
                Ok(root)
            }
        }
    }

    fn folder(
        &self,
        net: &dyn Transport,
        session: &Session,
        folder: &FolderRef,
    ) -> Result<Folder, Error> {
        self.contents(net, session, folder)
    }

    fn start_conversion(
        &self,
        net: &dyn Transport,
        session: &Session,
        design: &Design,
    ) -> Result<Translation, Error> {
        let source = self.version_urn(net, session, design)?;
        let job = json!({
            "input": { "urn": source },
            "output": { "formats": [{ "type": STEP_OUTPUT }] },
        });
        let request = Request::post(format!("{DERIVATIVE_API}/job"), Body::Json(job.to_string()))
            .with_bearer(&session.access_token);
        checked(SERVICE, net.send(&request)?, Subject::Translation)?;
        Ok(Translation { source })
    }

    fn poll_conversion(
        &self,
        net: &dyn Transport,
        session: &Session,
        translation: &Translation,
    ) -> Result<Conversion, Error> {
        let url = format!("{DERIVATIVE_API}/{}/manifest", translation.source);
        let manifest: Manifest = self.get(net, session, url, Subject::Translation)?;
        let step = manifest
            .derivatives
            .iter()
            .find(|derivative| derivative.output_type.as_deref() == Some(STEP_OUTPUT));
        match step {
            Some(derivative) => match derivative.status.as_deref() {
                Some("success") => derivative
                    .children
                    .iter()
                    .find(|child| child.role.as_deref() == Some(STEP_ROLE))
                    .map(|child| {
                        Conversion::Ready(Download {
                            source: translation.source.clone(),
                            artifact: child.urn.clone(),
                        })
                    })
                    .ok_or(Error::NoStepFile { service: SERVICE }),
                Some("failed" | "timeout") => Err(Error::ConversionFailed { service: SERVICE }),
                _ => Ok(Conversion::Working {
                    percent: percent_of(derivative.progress.as_deref()),
                }),
            },
            None => match manifest.status.as_deref() {
                Some("failed" | "timeout") => Err(Error::ConversionFailed { service: SERVICE }),
                _ => Ok(Conversion::Working {
                    percent: percent_of(manifest.progress.as_deref()),
                }),
            },
        }
    }

    fn thumbnail(
        &self,
        net: &dyn Transport,
        session: &Session,
        design: &Design,
    ) -> Result<Option<Vec<u8>>, Error> {
        let urn = self.version_urn(net, session, design)?;
        let url = format!(
            "{DERIVATIVE_API}/{urn}/thumbnail?width={THUMBNAIL_SIDE}&height={THUMBNAIL_SIDE}"
        );
        let request = Request::get(url).with_bearer(&session.access_token);
        let response = net.send(&request)?;
        if matches!(response.status, 202 | 404) {
            return Ok(None);
        }
        let response = checked(SERVICE, response, Subject::Thumbnail)?;
        Ok((!response.body.is_empty()).then_some(response.body))
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
            "{DERIVATIVE_API}/{}/manifest/{}",
            download.source,
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

fn percent_of(progress: Option<&str>) -> Option<u8> {
    let text = progress?.trim();
    if text == "complete" {
        return Some(100);
    }
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    digits.parse::<u8>().ok().filter(|percent| *percent <= 100)
}

enum Entry {
    Folder(FolderEntry),
    Design(Design),
}

#[derive(Deserialize)]
struct Profile {
    name: Option<String>,
    given_name: Option<String>,
    family_name: Option<String>,
    email: Option<String>,
    preferred_username: Option<String>,
}

impl Profile {
    fn display_name(self) -> String {
        let full = [self.given_name, self.family_name]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        [self.name, Some(full), self.email, self.preferred_username]
            .into_iter()
            .flatten()
            .map(|text| text.trim().to_owned())
            .find(|text| !text.is_empty())
            .unwrap_or_else(|| SERVICE.account_name().to_owned())
    }
}

#[derive(Deserialize)]
struct Page {
    data: Vec<Resource>,
    links: Option<Links>,
}

#[derive(Deserialize)]
struct Links {
    next: Option<Link>,
}

#[derive(Deserialize)]
struct Link {
    href: String,
}

#[derive(Deserialize)]
struct Single {
    data: Identified,
}

#[derive(Deserialize)]
struct Identified {
    id: String,
}

#[derive(Deserialize)]
struct Resource {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    attributes: Attributes,
}

#[derive(Deserialize, Default)]
struct Attributes {
    name: Option<String>,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    #[serde(rename = "lastModifiedTime")]
    last_modified: Option<String>,
    #[serde(default)]
    hidden: bool,
    extension: Option<Extension>,
}

#[derive(Deserialize)]
struct Extension {
    #[serde(rename = "type")]
    kind: Option<String>,
}

impl Resource {
    fn title(&self) -> String {
        [&self.attributes.display_name, &self.attributes.name]
            .into_iter()
            .flatten()
            .map(|text| text.trim())
            .find(|text| !text.is_empty())
            .map_or_else(|| "Untitled".to_owned(), str::to_owned)
    }

    fn extension(&self) -> Option<&str> {
        self.attributes
            .extension
            .as_ref()
            .and_then(|extension| extension.kind.as_deref())
    }

    fn workspace(&self) -> Option<Workspace> {
        if self.kind != "hubs" {
            return None;
        }
        let kind = match self.extension() {
            Some(TEAM_HUB) => WorkspaceKind::Team,
            Some(PERSONAL_HUB) => WorkspaceKind::Personal,
            _ => WorkspaceKind::Other,
        };
        Some(Workspace {
            id: WorkspaceId::new(&self.id),
            name: self.title(),
            kind,
        })
    }

    fn project(&self, workspace: &WorkspaceId) -> Option<Project> {
        (self.kind == "projects").then(|| Project {
            id: ProjectId::new(&self.id),
            workspace: workspace.clone(),
            name: self.title(),
        })
    }

    fn entry(&self, project: &ProjectId) -> Option<Entry> {
        if self.attributes.hidden {
            return None;
        }
        match self.kind.as_str() {
            "folders" => Some(Entry::Folder(FolderEntry {
                reference: FolderRef {
                    project: project.clone(),
                    folder: FolderId::new(&self.id),
                },
                name: self.title(),
            })),
            "items" if self.extension() == Some(FUSION_DESIGN) => Some(Entry::Design(Design {
                id: DesignId::new(&self.id),
                project: project.clone(),
                name: self.title(),
                modified: self
                    .attributes
                    .last_modified
                    .as_deref()
                    .and_then(|text| text.parse().ok()),
            })),
            _ => None,
        }
    }
}

#[derive(Deserialize)]
struct Manifest {
    status: Option<String>,
    progress: Option<String>,
    #[serde(default)]
    derivatives: Vec<Derivative>,
}

#[derive(Deserialize)]
struct Derivative {
    #[serde(rename = "outputType")]
    output_type: Option<String>,
    status: Option<String>,
    progress: Option<String>,
    #[serde(default)]
    children: Vec<Child>,
}

#[derive(Deserialize)]
struct Child {
    urn: String,
    role: Option<String>,
}

#[cfg(test)]
mod tests;
