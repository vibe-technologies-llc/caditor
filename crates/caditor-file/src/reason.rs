use std::io::{Error, ErrorKind};

use crate::{
    read::{NotAFile, TooLarge},
    save::{NotReadBack, ReadOnly},
};

pub(crate) fn reading(error: &Error) -> String {
    if let Some(inner) = error
        .get_ref()
        .filter(|inner| inner.is::<NotAFile>() || inner.is::<TooLarge>())
    {
        return inner.to_string();
    }
    match error.kind() {
        ErrorKind::NotFound => "it no longer exists".to_owned(),
        ErrorKind::PermissionDenied => "you do not have permission to read it".to_owned(),
        ErrorKind::IsADirectory => "it is a folder, not a file".to_owned(),
        ErrorKind::OutOfMemory => "there is not enough memory to read it".to_owned(),
        ErrorKind::InvalidFilename => "its name is longer than the system allows".to_owned(),
        _ => format!("the system reported an error ({error})"),
    }
}

pub(crate) fn writing(error: &Error) -> String {
    if error.get_ref().is_some_and(|inner| inner.is::<NotAFile>()) {
        return "a device, pipe or socket with that name already exists".to_owned();
    }
    if let Some(inner) = error
        .get_ref()
        .filter(|inner| inner.is::<NotReadBack>() || inner.is::<ReadOnly>())
    {
        return inner.to_string();
    }
    match error.kind() {
        ErrorKind::NotFound => "its folder no longer exists".to_owned(),
        ErrorKind::PermissionDenied => {
            "you do not have permission to write to its folder".to_owned()
        }
        ErrorKind::ReadOnlyFilesystem => "its folder is on a read-only drive".to_owned(),
        ErrorKind::StorageFull | ErrorKind::QuotaExceeded => "the disk is full".to_owned(),
        ErrorKind::IsADirectory => "a folder with that name already exists".to_owned(),
        ErrorKind::InvalidFilename => "its name is longer than the system allows".to_owned(),
        _ => format!("the system reported an error ({error})"),
    }
}
