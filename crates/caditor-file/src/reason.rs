use std::io::{Error, ErrorKind};

pub(crate) fn reading(error: &Error) -> String {
    match error.kind() {
        ErrorKind::NotFound => "it no longer exists".to_owned(),
        ErrorKind::PermissionDenied => "you do not have permission to read it".to_owned(),
        ErrorKind::IsADirectory => "it is a folder, not a file".to_owned(),
        _ => format!("the system reported an error ({error})"),
    }
}

pub(crate) fn writing(error: &Error) -> String {
    match error.kind() {
        ErrorKind::NotFound => "its folder no longer exists".to_owned(),
        ErrorKind::PermissionDenied => {
            "you do not have permission to write to its folder".to_owned()
        }
        ErrorKind::ReadOnlyFilesystem => "its folder is on a read-only drive".to_owned(),
        ErrorKind::StorageFull | ErrorKind::QuotaExceeded => "the disk is full".to_owned(),
        ErrorKind::IsADirectory => "a folder with that name already exists".to_owned(),
        _ => format!("the system reported an error ({error})"),
    }
}
