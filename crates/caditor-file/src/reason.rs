use std::io::{Error, ErrorKind};

use crate::{
    binary::NotReadBack,
    read::{MAX_FILE_SIZE, NotAFile, TooLarge},
    save::ReadOnly,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReadFailure {
    #[error("it is a device, pipe or socket, not a file")]
    NotAFile,
    #[error("it is larger than the {} GiB caditor reads", MAX_FILE_SIZE >> 30)]
    TooLarge,
    #[error("it no longer exists")]
    NotFound,
    #[error("you do not have permission to read it")]
    PermissionDenied,
    #[error("it is a folder, not a file")]
    IsAFolder,
    #[error("there is not enough memory to read it")]
    OutOfMemory,
    #[error("its name is longer than the system allows")]
    NameTooLong,
    #[error("the system reported an error (os error {code})")]
    Os { code: i32 },
    #[error("the system reported an error ({kind})")]
    System { kind: ErrorKind },
}

impl ReadFailure {
    pub(crate) fn of(error: &Error) -> Self {
        if let Some(inner) = error.get_ref() {
            if inner.is::<NotAFile>() {
                return Self::NotAFile;
            }
            if inner.is::<TooLarge>() {
                return Self::TooLarge;
            }
        }
        match error.kind() {
            ErrorKind::NotFound => Self::NotFound,
            ErrorKind::PermissionDenied => Self::PermissionDenied,
            ErrorKind::IsADirectory => Self::IsAFolder,
            ErrorKind::OutOfMemory => Self::OutOfMemory,
            ErrorKind::InvalidFilename => Self::NameTooLong,
            kind => error
                .raw_os_error()
                .map_or(Self::System { kind }, |code| Self::Os { code }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WriteFailure {
    #[error("a device, pipe or socket with that name already exists")]
    NameTaken,
    #[error("the saved copy did not read back intact, so the file was left as it was")]
    NotReadBack,
    #[error(
        "an earlier version written into the saved copy did not read back intact, so the file \
         was left as it was"
    )]
    VersionNotReadBack,
    #[error("the file is read-only")]
    ReadOnly,
    #[error("its folder no longer exists")]
    FolderGone,
    #[error("you do not have permission to write to its folder")]
    PermissionDenied,
    #[error("its folder is on a read-only drive")]
    ReadOnlyDrive,
    #[error("the disk is full")]
    DiskFull,
    #[error("a folder with that name already exists")]
    IsAFolder,
    #[error("its name is longer than the system allows")]
    NameTooLong,
    #[error("the system reported an error (os error {code})")]
    Os { code: i32 },
    #[error("the system reported an error ({kind})")]
    System { kind: ErrorKind },
}

impl WriteFailure {
    pub(crate) fn of(error: &Error) -> Self {
        if let Some(inner) = error.get_ref() {
            if inner.is::<NotAFile>() {
                return Self::NameTaken;
            }
            if let Some(not_read_back) = inner.downcast_ref::<NotReadBack>() {
                return match not_read_back {
                    NotReadBack::Model => Self::NotReadBack,
                    NotReadBack::Version { .. } => Self::VersionNotReadBack,
                };
            }
            if inner.is::<ReadOnly>() {
                return Self::ReadOnly;
            }
        }
        match error.kind() {
            ErrorKind::NotFound => Self::FolderGone,
            ErrorKind::PermissionDenied => Self::PermissionDenied,
            ErrorKind::ReadOnlyFilesystem => Self::ReadOnlyDrive,
            ErrorKind::StorageFull | ErrorKind::QuotaExceeded => Self::DiskFull,
            ErrorKind::IsADirectory => Self::IsAFolder,
            ErrorKind::InvalidFilename => Self::NameTooLong,
            kind => error
                .raw_os_error()
                .map_or(Self::System { kind }, |code| Self::Os { code }),
        }
    }
}
