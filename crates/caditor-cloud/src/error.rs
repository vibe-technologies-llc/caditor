use std::io;

use crate::{service::Service, transport::TransportError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subject {
    Token,
    Account,
    Workspaces,
    Projects,
    Folder,
    Design,
    Translation,
    Download,
    Thumbnail,
    Upload,
    StoredFile,
    Storage,
    SignedUpload,
    Item,
}

impl Subject {
    pub fn describe(self) -> &'static str {
        match self {
            Self::Token => "the sign-in",
            Self::Account => "the account",
            Self::Workspaces => "the list of workspaces",
            Self::Projects => "the list of projects",
            Self::Folder => "the folder",
            Self::Design => "the design",
            Self::Translation => "the conversion",
            Self::Download => "the converted file",
            Self::Thumbnail => "the picture of the design",
            Self::Upload => "the uploaded file",
            Self::StoredFile => "the download address of the file",
            Self::Storage => "the place to put the file",
            Self::SignedUpload => "the upload address of the file",
            Self::Item => "the file in the folder",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error("The sign-in has ended. Sign in again.")]
    SignedOut,
    #[error("{service} did not allow this. The hub or team may not have allowed caditor yet.")]
    Forbidden { service: Service },
    #[error("{} was not found. It may have been moved or deleted.", .subject.describe())]
    NotFound { subject: Subject },
    #[error("{service} is asking caditor to slow down. Try again in a minute.")]
    RateLimited {
        service: Service,
        retry_after_seconds: Option<u64>,
    },
    #[error("{service} refused the request (status {status}).")]
    Refused { service: Service, status: u16 },
    #[error("{service} sent {} caditor could not read.", .subject.describe())]
    Unreadable { service: Service, subject: Subject },
    #[error("The sign-in was declined in the browser.")]
    SignInDeclined,
    #[error("The browser answered a different sign-in than the one caditor started.")]
    SignInMismatch,
    #[error("The browser did not finish signing in in time.")]
    SignInTimedOut,
    #[error("The sign-in was stopped.")]
    Stopped,
    #[error("This build of caditor has no {service} app registered, so it cannot sign in.")]
    NoClientId { service: Service },
    #[error("caditor could not listen for the browser on port {port}: {kind}.")]
    ListenerUnavailable { port: u16, kind: io::ErrorKind },
    #[error("The system could not make random numbers for a secure sign-in.")]
    RandomUnavailable,
    #[error("{service} could not convert the design.")]
    ConversionFailed { service: Service },
    #[error("{service} is taking too long to convert the design.")]
    ConversionTimedOut { service: Service },
    #[error("{service} converted the design but gave no STEP file.")]
    NoStepFile { service: Service },
    #[error("{service} holds no file for this item that caditor can download.")]
    NoStoredFile { service: Service },
    #[error(
        "{service} does not let caditor add files to this folder. Ask for permission to edit it, \
         or choose another folder. If you were signed in before sending files was added, sign out \
         and in again."
    )]
    SendForbidden { service: Service },
    #[error(
        "{service} already holds something named “{name}” in this folder that caditor cannot \
         add a version to. Rename the model or choose another folder."
    )]
    NameTaken { service: Service, name: String },
    #[error(
        "A saved version of an Onshape document cannot be changed, so nothing can be sent into \
         it. Open the document's main workspace or one of its branches."
    )]
    VersionIsReadOnly,
    #[error("{service} did not take every part of the file. Try again.")]
    UploadIncomplete { service: Service },
    #[error("The file could not be written: {0}.")]
    Write(io::ErrorKind),
}

impl Error {
    pub fn is_sign_in_needed(&self) -> bool {
        matches!(self, Self::SignedOut)
    }

    pub fn is_stop(&self) -> bool {
        matches!(
            self,
            Self::Stopped | Self::Transport(TransportError::Stopped)
        )
    }
}
