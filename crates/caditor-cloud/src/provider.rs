use std::io::Write;

use crate::{
    catalogue::{
        Account, Conversion, Design, Download, Folder, FolderRef, Project, Translation, Upload,
        Version, Workspace, WorkspaceId,
    },
    error::Error,
    pkce::Verifier,
    service::Service,
    session::Session,
    transport::Transport,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redirect {
    pub uri: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Callback {
    pub code: String,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignIn {
    pub url: String,
    pub state: String,
    pub verifier: Verifier,
    pub redirect: Redirect,
}

pub trait Provider: Send + Sync {
    fn service(&self) -> Service;

    fn start_sign_in(&self, redirect: &Redirect) -> Result<SignIn, Error>;

    fn finish_sign_in(
        &self,
        net: &dyn Transport,
        sign_in: &SignIn,
        callback: &Callback,
        now: u64,
    ) -> Result<Session, Error>;

    fn refresh(&self, net: &dyn Transport, session: &Session, now: u64) -> Result<Session, Error>;

    fn revoke(&self, net: &dyn Transport, session: &Session) -> Result<(), Error>;

    fn account(&self, net: &dyn Transport, session: &Session) -> Result<Account, Error>;

    fn workspaces(&self, net: &dyn Transport, session: &Session) -> Result<Vec<Workspace>, Error>;

    fn projects(
        &self,
        net: &dyn Transport,
        session: &Session,
        workspace: &WorkspaceId,
    ) -> Result<Vec<Project>, Error>;

    fn root(
        &self,
        net: &dyn Transport,
        session: &Session,
        project: &Project,
    ) -> Result<Folder, Error>;

    fn folder(
        &self,
        net: &dyn Transport,
        session: &Session,
        folder: &FolderRef,
    ) -> Result<Folder, Error>;

    fn versions(
        &self,
        net: &dyn Transport,
        session: &Session,
        design: &Design,
    ) -> Result<Vec<Version>, Error>;

    fn start_conversion(
        &self,
        net: &dyn Transport,
        session: &Session,
        design: &Design,
        version: Option<&Version>,
    ) -> Result<Translation, Error>;

    fn poll_conversion(
        &self,
        net: &dyn Transport,
        session: &Session,
        translation: &Translation,
    ) -> Result<Conversion, Error>;

    fn fetch(
        &self,
        net: &dyn Transport,
        session: &Session,
        download: &Download,
        sink: &mut dyn Write,
        progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
    ) -> Result<u64, Error>;

    fn fetch_upload(
        &self,
        net: &dyn Transport,
        session: &Session,
        upload: &Upload,
        sink: &mut dyn Write,
        progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
    ) -> Result<u64, Error>;

    fn thumbnail(
        &self,
        _net: &dyn Transport,
        _session: &Session,
        _design: &Design,
    ) -> Result<Option<Vec<u8>>, Error> {
        Ok(None)
    }
}
