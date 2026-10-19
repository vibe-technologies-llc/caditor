use std::io;

use caditor_cloud::{Error, Service, Subject};

use crate::browser::OpenError;

#[derive(Debug, thiserror::Error)]
pub enum Failure {
    #[error(transparent)]
    Cloud(#[from] Error),
    #[error("Something went wrong inside caditor while it talked to {0}. Try again.")]
    Crashed(Service),
    #[error("caditor could not start a background job: {0}. Try again.")]
    Spawn(io::ErrorKind),
    #[error(
        "caditor has no folder to keep downloaded designs in, so it cannot import from the cloud."
    )]
    NoDownloadFolder,
    #[error("caditor could not save the downloaded design: {0}.")]
    Save(io::ErrorKind),
    #[error("caditor could not read the exported file it was to send: {0}. Try sending again.")]
    Read(io::ErrorKind),
    #[error(transparent)]
    Browser(OpenError),
}

impl Failure {
    pub fn is_stop(&self) -> bool {
        matches!(self, Self::Cloud(error) if error.is_stop())
    }

    pub fn is_folder_unusable(&self) -> bool {
        matches!(
            self,
            Self::Cloud(
                Error::NotFound {
                    subject: Subject::Folder
                } | Error::SendForbidden { .. }
            )
        )
    }

    pub fn is_sign_in_needed(&self) -> bool {
        matches!(self, Self::Cloud(error) if error.is_sign_in_needed())
    }
}
