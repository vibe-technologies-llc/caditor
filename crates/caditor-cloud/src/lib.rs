mod catalogue;
mod client;
mod encoding;
mod error;
#[cfg(any(test, feature = "test-support"))]
pub mod fake;
mod fusion;
mod loopback;
mod network;
mod onshape;
mod pkce;
mod provider;
mod rest;
mod service;
mod session;
mod transport;

pub use crate::{
    catalogue::{
        Account, Conversion, Design, DesignId, Download, Folder, FolderEntry, FolderId, FolderRef,
        Outgoing, Project, ProjectId, SendOutcome, Sent, Translation, Upload, UploadFormat,
        UploadId, Version, VersionId, Workspace, WorkspaceId, WorkspaceKind,
    },
    client::{Client, Saved, finish_sign_in},
    error::{Error, Subject},
    fusion::Fusion,
    loopback::{CALLBACK_PATH, Listener},
    network::Network,
    onshape::Onshape,
    provider::{Callback, Provider, Redirect, SignIn},
    service::Service,
    session::{Session, now_seconds},
    transport::{Body, Fetched, Method, Request, Response, Transport, TransportError},
};
