use std::io::Write;

use jiff::Timestamp;
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::json;

use crate::{
    catalogue::{
        Account, Conversion, Design, DesignId, Download, Folder, FolderEntry, FolderId, FolderRef,
        ItemId, Outgoing, Project, ProjectId, SendOutcome, Sent, Translation, Upload, UploadFormat,
        UploadId, Version, VersionId, Workspace, WorkspaceId, WorkspaceKind,
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
const STORAGE_API: &str = "https://developer.api.autodesk.com/oss/v2";
const DERIVATIVE_API: &str = "https://developer.api.autodesk.com/modelderivative/v2/designdata";
const SCOPES: &str = "data:read data:write data:create user-profile:read";
const FUSION_DESIGN: &str = "items:autodesk.fusion360:Design";
const TEAM_HUB: &str = "hubs:autodesk.core:Hub";
const PERSONAL_HUB: &str = "hubs:autodesk.a360:PersonalHub";
const STEP_OUTPUT: &str = "step";
const STEP_ROLE: &str = "step";
const THUMBNAIL_SIDE: u32 = 200;
const MAX_PAGES: usize = 20;
const STORAGE_OBJECT_PREFIX: &str = "urn:adsk.objects:os.object:";
const DOWNLOAD_LINK_MINUTES: u32 = 10;
const UPLOAD_LINK_MINUTES: u32 = 60;
const PART_BYTES: usize = 5 << 20;
const FILE_ITEM: &str = "items:autodesk.core:File";
const FILE_VERSION: &str = "versions:autodesk.core:File";
const EXTENSION_VERSION: &str = "1.0";
const FIRST_VERSION: &str = "1";

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
        version: Option<&Version>,
    ) -> Result<String, Error> {
        if let Some(version) = version {
            return Ok(base64_url(version.id.as_str().as_bytes()));
        }
        let url = format!(
            "{DATA_API}/projects/{}/items/{}/tip",
            percent_encode(design.project.as_str()),
            percent_encode(design.id.as_str())
        );
        let tip: Single = self.get(net, session, url, Subject::Design)?;
        Ok(base64_url(tip.data.id.as_bytes()))
    }

    fn download_link(
        &self,
        net: &dyn Transport,
        session: &Session,
        upload: &Upload,
    ) -> Result<String, Error> {
        let url = format!(
            "{DATA_API}/projects/{}/items/{}/tip",
            percent_encode(upload.project.as_str()),
            percent_encode(upload.id.as_str())
        );
        let tip: StoredTip = self.get(net, session, url, Subject::Upload)?;
        let stored = tip
            .data
            .relationships
            .and_then(|relationships| relationships.storage)
            .and_then(|storage| storage.data)
            .ok_or(Error::NoStoredFile { service: SERVICE })?;
        let (bucket, object) = stored
            .id
            .strip_prefix(STORAGE_OBJECT_PREFIX)
            .and_then(|location| location.split_once('/'))
            .filter(|(bucket, object)| !bucket.is_empty() && !object.is_empty())
            .ok_or(Error::NoStoredFile { service: SERVICE })?;
        let url = format!(
            "{STORAGE_API}/buckets/{}/objects/{}/signeds3download?minutesExpiration={DOWNLOAD_LINK_MINUTES}",
            percent_encode(bucket),
            percent_encode(object)
        );
        let signed: SignedDownload = self.get(net, session, url, Subject::StoredFile)?;
        signed
            .url
            .filter(|link| link.starts_with("https://"))
            .ok_or(Error::NoStoredFile { service: SERVICE })
    }

    fn existing_item(
        &self,
        net: &dyn Transport,
        session: &Session,
        folder: &FolderRef,
        name: &str,
    ) -> Result<Option<String>, Error> {
        let url = format!(
            "{DATA_API}/projects/{}/folders/{}/contents",
            percent_encode(folder.project.as_str()),
            percent_encode(folder.folder.as_str())
        );
        let wanted = name.trim();
        let found = self.pages(net, session, url, Subject::Folder, |page| {
            page.data
                .iter()
                .filter(|resource| {
                    resource.kind == "items"
                        && resource.extension() != Some(FUSION_DESIGN)
                        && resource.title().eq_ignore_ascii_case(wanted)
                })
                .map(|resource| resource.id.clone())
                .collect()
        })?;
        Ok(found.into_iter().next())
    }

    fn create_storage(
        &self,
        net: &dyn Transport,
        session: &Session,
        folder: &FolderRef,
        name: &str,
    ) -> Result<String, Error> {
        let body = json!({
            "jsonapi": { "version": "1.0" },
            "data": {
                "type": "objects",
                "attributes": { "name": name },
                "relationships": {
                    "target": { "data": { "type": "folders", "id": folder.folder.as_str() } }
                }
            }
        });
        let url = format!(
            "{DATA_API}/projects/{}/storage",
            percent_encode(folder.project.as_str())
        );
        let created: Created = self.write(net, session, url, body, Subject::Storage, name)?;
        Ok(created.data.id)
    }

    fn upload_bytes(
        &self,
        net: &dyn Transport,
        session: &Session,
        storage: &str,
        bytes: &[u8],
        progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
    ) -> Result<(), Error> {
        let (bucket, object) = storage
            .strip_prefix(STORAGE_OBJECT_PREFIX)
            .and_then(|location| location.split_once('/'))
            .filter(|(bucket, object)| !bucket.is_empty() && !object.is_empty())
            .ok_or(Error::Unreadable {
                service: SERVICE,
                subject: Subject::Storage,
            })?;
        let address = format!(
            "{STORAGE_API}/buckets/{}/objects/{}/signeds3upload",
            percent_encode(bucket),
            percent_encode(object)
        );
        let parts: Vec<&[u8]> = if bytes.is_empty() {
            vec![bytes]
        } else {
            bytes.chunks(PART_BYTES).collect()
        };
        let signed: SignedUpload = self.get(
            net,
            session,
            format!(
                "{address}?parts={}&firstPart=1&minutesExpiration={UPLOAD_LINK_MINUTES}",
                parts.len()
            ),
            Subject::SignedUpload,
        )?;
        let unreadable = Error::Unreadable {
            service: SERVICE,
            subject: Subject::SignedUpload,
        };
        if signed.urls.len() != parts.len()
            || signed.urls.iter().any(|link| !link.starts_with("https://"))
        {
            return Err(unreadable);
        }
        let total = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let mut sent = 0_u64;
        if !progress(sent, Some(total)) {
            return Err(Error::Stopped);
        }
        for (part, link) in parts.iter().zip(signed.urls) {
            let request = Request::put(link, Body::Bytes(part.to_vec()));
            let response = net.send(&request)?;
            if !response.is_success() {
                return Err(Error::UploadIncomplete { service: SERVICE });
            }
            sent = sent.saturating_add(u64::try_from(part.len()).unwrap_or(u64::MAX));
            if !progress(sent, Some(total)) {
                return Err(Error::Stopped);
            }
        }
        let finish = Request::post(
            address,
            Body::Json(json!({ "uploadKey": signed.upload_key }).to_string()),
        )
        .with_bearer(&session.access_token);
        let answer = net.send(&finish)?;
        if !answer.is_success() {
            return Err(Error::UploadIncomplete { service: SERVICE });
        }
        Ok(())
    }

    fn create_item(
        &self,
        net: &dyn Transport,
        session: &Session,
        folder: &FolderRef,
        name: &str,
        storage: &str,
    ) -> Result<Sent, Error> {
        let body = json!({
            "jsonapi": { "version": "1.0" },
            "data": {
                "type": "items",
                "attributes": {
                    "displayName": name,
                    "extension": { "type": FILE_ITEM, "version": EXTENSION_VERSION }
                },
                "relationships": {
                    "tip": { "data": { "type": "versions", "id": FIRST_VERSION } },
                    "parent": { "data": { "type": "folders", "id": folder.folder.as_str() } }
                }
            },
            "included": [{
                "type": "versions",
                "id": FIRST_VERSION,
                "attributes": {
                    "name": name,
                    "extension": { "type": FILE_VERSION, "version": EXTENSION_VERSION }
                },
                "relationships": {
                    "storage": { "data": { "type": "objects", "id": storage } }
                }
            }]
        });
        let url = format!(
            "{DATA_API}/projects/{}/items",
            percent_encode(folder.project.as_str())
        );
        let created: Created = self.write(net, session, url, body, Subject::Item, name)?;
        let version = created
            .included
            .iter()
            .find(|resource| resource.kind == "versions");
        let web_link = created
            .data
            .web_link()
            .or_else(|| version.and_then(Resource::web_link));
        Ok(Sent {
            name: name.to_owned(),
            outcome: SendOutcome::Created,
            item: Some(ItemId::new(created.data.id)),
            version: version.map(|version| VersionId::new(version.id.clone())),
            web_link,
        })
    }

    fn create_version(
        &self,
        net: &dyn Transport,
        session: &Session,
        folder: &FolderRef,
        name: &str,
        item: &str,
        storage: &str,
    ) -> Result<Sent, Error> {
        let body = json!({
            "jsonapi": { "version": "1.0" },
            "data": {
                "type": "versions",
                "attributes": {
                    "name": name,
                    "extension": { "type": FILE_VERSION, "version": EXTENSION_VERSION }
                },
                "relationships": {
                    "item": { "data": { "type": "items", "id": item } },
                    "storage": { "data": { "type": "objects", "id": storage } }
                }
            }
        });
        let url = format!(
            "{DATA_API}/projects/{}/versions",
            percent_encode(folder.project.as_str())
        );
        let created: Created = self.write(net, session, url, body, Subject::Item, name)?;
        Ok(Sent {
            name: name.to_owned(),
            outcome: SendOutcome::NewVersion {
                number: created.data.attributes.version_number,
            },
            item: Some(ItemId::new(item)),
            version: Some(VersionId::new(created.data.id.clone())),
            web_link: created.data.web_link(),
        })
    }

    fn write<T: DeserializeOwned>(
        &self,
        net: &dyn Transport,
        session: &Session,
        url: String,
        body: serde_json::Value,
        subject: Subject,
        name: &str,
    ) -> Result<T, Error> {
        let request =
            Request::post(url, Body::JsonApi(body.to_string())).with_bearer(&session.access_token);
        let response = net.send(&request)?;
        let response = match response.status {
            403 => return Err(Error::SendForbidden { service: SERVICE }),
            409 => {
                return Err(Error::NameTaken {
                    service: SERVICE,
                    name: name.to_owned(),
                });
            }
            _ => checked(SERVICE, response, subject)?,
        };
        rest::read(SERVICE, &response, subject)
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
        let mut contents = Folder {
            place: Some(folder.clone()),
            ..Folder::default()
        };
        for part in parts {
            match part {
                Entry::Folder(entry) => contents.folders.push(entry),
                Entry::Design(design) => contents.designs.push(design),
                Entry::Upload(upload) => contents.uploads.push(upload),
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
                    ..Folder::default()
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

    fn versions(
        &self,
        net: &dyn Transport,
        session: &Session,
        design: &Design,
    ) -> Result<Vec<Version>, Error> {
        let url = format!(
            "{DATA_API}/projects/{}/items/{}/versions",
            percent_encode(design.project.as_str()),
            percent_encode(design.id.as_str())
        );
        let mut versions = self.pages(net, session, url, Subject::Design, |page| {
            page.data.iter().filter_map(Resource::version).collect()
        })?;
        versions.sort_by_key(|version| std::cmp::Reverse(version.number));
        Ok(versions)
    }

    fn start_conversion(
        &self,
        net: &dyn Transport,
        session: &Session,
        design: &Design,
        version: Option<&Version>,
    ) -> Result<Translation, Error> {
        let source = self.version_urn(net, session, design, version)?;
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

    fn fetch_upload(
        &self,
        net: &dyn Transport,
        session: &Session,
        upload: &Upload,
        sink: &mut dyn Write,
        progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
    ) -> Result<u64, Error> {
        let link = self.download_link(net, session, upload)?;
        let fetched = net.fetch(&Request::get(link), sink, progress)?;
        let answer = Response {
            status: fetched.status,
            body: Vec::new(),
            retry_after_seconds: fetched.retry_after_seconds,
        };
        checked(SERVICE, answer, Subject::Upload)?;
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
        let existing = self.existing_item(net, session, outgoing.folder, name)?;
        let storage = self.create_storage(net, session, outgoing.folder, name)?;
        self.upload_bytes(net, session, &storage, outgoing.bytes, progress)?;
        match existing {
            Some(item) => self.create_version(net, session, outgoing.folder, name, &item, &storage),
            None => self.create_item(net, session, outgoing.folder, name, &storage),
        }
    }

    fn thumbnail(
        &self,
        net: &dyn Transport,
        session: &Session,
        design: &Design,
    ) -> Result<Option<Vec<u8>>, Error> {
        let urn = self.version_urn(net, session, design, None)?;
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
    Upload(Upload),
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
struct StoredTip {
    data: StoredVersion,
}

#[derive(Deserialize)]
struct StoredVersion {
    relationships: Option<StoredRelationships>,
}

#[derive(Deserialize)]
struct StoredRelationships {
    storage: Option<StoredLocation>,
}

#[derive(Deserialize)]
struct StoredLocation {
    data: Option<Identified>,
}

#[derive(Deserialize)]
struct Created {
    data: Resource,
    #[serde(default)]
    included: Vec<Resource>,
}

#[derive(Deserialize)]
struct SignedUpload {
    #[serde(rename = "uploadKey")]
    upload_key: String,
    #[serde(default)]
    urls: Vec<String>,
}

#[derive(Deserialize)]
struct SignedDownload {
    url: Option<String>,
}

#[derive(Deserialize)]
struct Resource {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    attributes: Attributes,
    links: Option<ResourceLinks>,
}

#[derive(Deserialize)]
struct ResourceLinks {
    #[serde(rename = "webView")]
    web_view: Option<Link>,
}

#[derive(Deserialize, Default)]
struct Attributes {
    name: Option<String>,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    #[serde(rename = "lastModifiedTime")]
    last_modified: Option<String>,
    #[serde(rename = "versionNumber")]
    version_number: Option<u32>,
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
    fn web_link(&self) -> Option<String> {
        let address = &self.links.as_ref()?.web_view.as_ref()?.href;
        (address.starts_with("https://") && !address.chars().any(char::is_whitespace))
            .then(|| address.clone())
    }

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

    fn version(&self) -> Option<Version> {
        (self.kind == "versions").then_some(())?;
        Some(Version {
            id: VersionId::new(&self.id),
            number: self.attributes.version_number?,
            modified: self.modified(),
        })
    }

    fn modified(&self) -> Option<Timestamp> {
        self.attributes
            .last_modified
            .as_deref()
            .and_then(|text| text.parse().ok())
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
                modified: self.modified(),
            })),
            "items" => {
                let name = self.title();
                UploadFormat::of_file_name(&name).map(|format| {
                    Entry::Upload(Upload {
                        id: UploadId::new(&self.id),
                        project: project.clone(),
                        name,
                        format,
                        modified: self.modified(),
                    })
                })
            }
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
