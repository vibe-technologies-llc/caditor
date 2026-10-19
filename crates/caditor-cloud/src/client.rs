use std::{io::Write, sync::Arc};

use parking_lot::Mutex;

use crate::{
    catalogue::{
        Account, Conversion, Design, Download, Folder, FolderRef, Outgoing, Placement, Project,
        Sent, Translation, Upload, Version, Workspace, WorkspaceId,
    },
    error::Error,
    provider::{Callback, Provider, SignIn},
    service::Service,
    session::{Session, now_seconds},
    transport::Transport,
};

pub type Saved = Arc<dyn Fn(&Session) + Send + Sync>;

pub fn finish_sign_in(
    provider: &dyn Provider,
    net: &dyn Transport,
    sign_in: &SignIn,
    callback: &Callback,
) -> Result<Session, Error> {
    if callback.state != sign_in.state {
        return Err(Error::SignInMismatch);
    }
    provider.finish_sign_in(net, sign_in, callback, now_seconds())
}

pub struct Client {
    provider: Arc<dyn Provider>,
    transport: Arc<dyn Transport>,
    session: Mutex<Session>,
    renewing: Mutex<()>,
    clock: fn() -> u64,
    saved: Option<Saved>,
}

impl Client {
    pub fn new(
        provider: Arc<dyn Provider>,
        transport: Arc<dyn Transport>,
        session: Session,
    ) -> Self {
        Self {
            provider,
            transport,
            session: Mutex::new(session),
            renewing: Mutex::new(()),
            clock: now_seconds,
            saved: None,
        }
    }

    pub fn saving_with(mut self, saved: Saved) -> Self {
        self.saved = Some(saved);
        self
    }

    pub fn with_clock(mut self, clock: fn() -> u64) -> Self {
        self.clock = clock;
        self
    }

    pub fn service(&self) -> Service {
        self.provider.service()
    }

    pub fn session(&self) -> Session {
        self.session.lock().clone()
    }

    pub fn revoke(&self) -> Result<(), Error> {
        let session = self.session();
        self.provider.revoke(&*self.transport, &session)
    }

    pub fn account(&self) -> Result<Account, Error> {
        self.authorised(|session| self.provider.account(&*self.transport, session))
    }

    pub fn workspaces(&self) -> Result<Vec<Workspace>, Error> {
        self.authorised(|session| self.provider.workspaces(&*self.transport, session))
    }

    pub fn projects(&self, workspace: &WorkspaceId) -> Result<Vec<Project>, Error> {
        self.authorised(|session| self.provider.projects(&*self.transport, session, workspace))
    }

    pub fn root(&self, project: &Project) -> Result<Folder, Error> {
        self.authorised(|session| self.provider.root(&*self.transport, session, project))
    }

    pub fn folder(&self, folder: &FolderRef) -> Result<Folder, Error> {
        self.authorised(|session| self.provider.folder(&*self.transport, session, folder))
    }

    pub fn versions(&self, design: &Design) -> Result<Vec<Version>, Error> {
        self.authorised(|session| self.provider.versions(&*self.transport, session, design))
    }

    pub fn start_conversion(
        &self,
        design: &Design,
        version: Option<&Version>,
    ) -> Result<Translation, Error> {
        self.authorised(|session| {
            self.provider
                .start_conversion(&*self.transport, session, design, version)
        })
    }

    pub fn poll_conversion(&self, translation: &Translation) -> Result<Conversion, Error> {
        self.authorised(|session| {
            self.provider
                .poll_conversion(&*self.transport, session, translation)
        })
    }

    pub fn thumbnail(&self, design: &Design) -> Result<Option<Vec<u8>>, Error> {
        self.authorised(|session| self.provider.thumbnail(&*self.transport, session, design))
    }

    pub fn fetch(
        &self,
        download: &Download,
        sink: &mut dyn Write,
        progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
    ) -> Result<u64, Error> {
        let session = self.fresh()?;
        self.provider
            .fetch(&*self.transport, &session, download, sink, progress)
    }

    pub fn fetch_upload(
        &self,
        upload: &Upload,
        sink: &mut dyn Write,
        progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
    ) -> Result<u64, Error> {
        let session = self.fresh()?;
        self.provider
            .fetch_upload(&*self.transport, &session, upload, sink, progress)
    }

    pub fn send(
        &self,
        outgoing: &Outgoing<'_>,
        progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
    ) -> Result<Sent, Error> {
        let session = self.fresh()?;
        self.provider
            .send(&*self.transport, &session, outgoing, progress)
    }

    pub fn send_part_studio(
        &self,
        outgoing: &Outgoing<'_>,
        progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
    ) -> Result<Translation, Error> {
        let session = self.fresh()?;
        self.provider
            .send_part_studio(&*self.transport, &session, outgoing, progress)
    }

    pub fn poll_part_studio(&self, translation: &Translation) -> Result<Placement, Error> {
        self.authorised(|session| {
            self.provider
                .poll_part_studio(&*self.transport, session, translation)
        })
    }

    fn fresh(&self) -> Result<Session, Error> {
        let session = self.session();
        if session.is_expiring((self.clock)()) {
            self.renew(&session)
        } else {
            Ok(session)
        }
    }

    fn authorised<T>(&self, call: impl Fn(&Session) -> Result<T, Error>) -> Result<T, Error> {
        let session = self.fresh()?;
        match call(&session) {
            Err(Error::SignedOut) => {
                let renewed = self.renew(&session)?;
                call(&renewed)
            }
            other => other,
        }
    }

    fn renew(&self, stale: &Session) -> Result<Session, Error> {
        let _only_one = self.renewing.lock();
        let now = (self.clock)();
        let current = self.session();
        if current != *stale && !current.is_expiring(now) {
            return Ok(current);
        }
        if !current.can_refresh() {
            return Err(Error::SignedOut);
        }
        let renewed = self.provider.refresh(&*self.transport, &current, now)?;
        *self.session.lock() = renewed.clone();
        if let Some(saved) = &self.saved {
            saved(&renewed);
        }
        Ok(renewed)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::{
        catalogue::WorkspaceId,
        fake::{FakeProvider, ScriptedTransport, Step},
    };

    static NOW: AtomicU64 = AtomicU64::new(10_000);

    fn clock() -> u64 {
        NOW.load(Ordering::Relaxed)
    }

    fn session(expires_at: u64, refreshable: bool) -> Session {
        Session {
            access_token: "old".to_owned(),
            refresh_token: refreshable.then(|| "refresh".to_owned()),
            expires_at,
        }
    }

    fn client(provider: &Arc<FakeProvider>, session: Session) -> Client {
        Client::new(
            Arc::clone(provider) as Arc<dyn Provider>,
            Arc::new(ScriptedTransport::new()),
            session,
        )
        .with_clock(clock)
    }

    fn refreshes(provider: &FakeProvider) -> usize {
        provider
            .calls()
            .iter()
            .filter(|call| *call == "refresh")
            .count()
    }

    #[test]
    fn a_fresh_session_is_used_as_it_is() {
        let provider = Arc::new(FakeProvider::new());
        let client = client(&provider, session(clock() + 3600, true));

        client.account().unwrap();

        assert_eq!(refreshes(&provider), 0);
    }

    #[test]
    fn an_expiring_session_is_refreshed_first_and_saved() {
        let provider = Arc::new(FakeProvider::new());
        let kept = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let sink = Arc::clone(&kept);
        let client = client(&provider, session(clock() + 30, true))
            .saving_with(Arc::new(move |session| sink.lock().push(session.clone())));

        client.account().unwrap();

        assert_eq!(refreshes(&provider), 1);
        assert_eq!(kept.lock().len(), 1);
        assert_eq!(client.session().access_token, "fake-access");
    }

    #[test]
    fn an_expired_session_that_cannot_refresh_means_signing_in_again() {
        let provider = Arc::new(FakeProvider::new());
        let client = client(&provider, session(clock() - 5, false));

        assert!(client.account().unwrap_err().is_sign_in_needed());
        assert!(provider.calls().is_empty());
    }

    #[test]
    fn a_refusal_mid_session_refreshes_once_and_retries_once() {
        let provider = Arc::new(FakeProvider::new().failing(Step::Account, || Error::SignedOut));
        let client = client(&provider, session(clock() + 3600, true));

        let result = client.account();

        assert!(result.unwrap_err().is_sign_in_needed());
        assert_eq!(refreshes(&provider), 1);
        assert_eq!(
            provider
                .calls()
                .iter()
                .filter(|call| *call == "account")
                .count(),
            2
        );
    }

    #[test]
    fn the_session_stays_when_a_call_fails_for_another_reason() {
        let provider = Arc::new(
            FakeProvider::new().failing(Step::Workspaces, || Error::Refused {
                service: crate::service::Service::Fusion,
                status: 500,
            }),
        );
        let client = client(&provider, session(clock() + 3600, true));

        assert!(client.workspaces().is_err());
        assert_eq!(refreshes(&provider), 0);
        assert_eq!(client.session().access_token, "old");
    }

    #[test]
    fn projects_come_through_the_provider_for_the_workspace_asked() {
        let provider = Arc::new(FakeProvider::new());
        let client = client(&provider, session(clock() + 3600, true));

        client.projects(&WorkspaceId::new("hub")).unwrap();

        assert_eq!(provider.calls(), vec!["projects hub".to_owned()]);
    }

    #[test]
    fn a_callback_for_another_state_is_never_traded_for_a_session() {
        let provider = FakeProvider::new();
        let net = ScriptedTransport::new();
        let sign_in = provider
            .start_sign_in(&crate::provider::Redirect {
                uri: "http://localhost:1/callback".to_owned(),
            })
            .unwrap();
        let foreign = Callback {
            code: "c".to_owned(),
            state: "someone-else".to_owned(),
        };
        let own = Callback {
            code: "c".to_owned(),
            state: sign_in.state.clone(),
        };

        assert!(matches!(
            finish_sign_in(&provider, &net, &sign_in, &foreign),
            Err(Error::SignInMismatch)
        ));
        assert!(finish_sign_in(&provider, &net, &sign_in, &own).is_ok());
    }

    #[test]
    fn a_thumbnail_is_asked_through_the_refreshing_call() {
        let design = Design {
            id: crate::catalogue::DesignId::new("d1"),
            project: crate::catalogue::ProjectId::new("p"),
            name: "Frame".to_owned(),
            modified: None,
        };
        let provider = Arc::new(
            FakeProvider::new()
                .thumbnail(design.id.clone(), b"picture")
                .failing(Step::Thumbnail, || Error::SignedOut),
        );
        let client = client(&provider, session(clock() + 3600, true));

        assert!(client.thumbnail(&design).unwrap_err().is_sign_in_needed());
        assert_eq!(refreshes(&provider), 1);

        provider.recover(Step::Thumbnail);

        assert_eq!(
            client.thumbnail(&design).unwrap(),
            Some(b"picture".to_vec())
        );
    }

    #[test]
    fn a_send_goes_through_the_refreshing_session_and_reports_what_the_provider_did() {
        let top = crate::catalogue::FolderRef {
            project: crate::catalogue::ProjectId::new("p"),
            folder: crate::catalogue::FolderId::new("f"),
        };
        let provider = Arc::new(FakeProvider::new());
        let client = client(&provider, session(clock() + 30, true));
        let mut seen = Vec::new();

        let sent = client
            .send(
                &crate::catalogue::Outgoing {
                    folder: &top,
                    name: "plate.step",
                    bytes: b"abcd",
                },
                &mut |sent, total| {
                    seen.push((sent, total));
                    true
                },
            )
            .unwrap();

        assert_eq!(refreshes(&provider), 1);
        assert_eq!(sent.outcome, crate::catalogue::SendOutcome::Created);
        assert_eq!(seen, vec![(0, Some(4)), (2, Some(4)), (4, Some(4))]);
        assert_eq!(provider.received().len(), 1);
        assert_eq!(provider.calls().last().unwrap(), "send f plate.step");
    }
}
