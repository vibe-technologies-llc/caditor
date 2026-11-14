use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::browser;

pub type Revealer = Arc<dyn Fn(&Path) -> Result<(), RevealError> + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RevealError {
    #[error("“{}” is in no folder that could be opened", .0.display())]
    NoFolder(PathBuf),
    #[error("the file manager could not be started ({0})")]
    Unavailable(io::ErrorKind),
}

fn folder_of(path: &Path) -> Result<&Path, RevealError> {
    path.parent()
        .filter(|folder| folder.is_dir())
        .ok_or_else(|| RevealError::NoFolder(path.to_owned()))
}

fn open_folder(path: &Path) -> Result<(), RevealError> {
    browser::launch(folder_of(path)?).map_err(|error| RevealError::Unavailable(error.kind()))
}

#[cfg(unix)]
pub fn show(path: &Path) -> Result<(), RevealError> {
    if !path.exists() {
        return open_folder(path);
    }
    freedesktop::show_item(path).or_else(|error| {
        log::debug!(
            "no file manager took the request to show a file ({error}); opening its folder"
        );
        open_folder(path)
    })
}

#[cfg(windows)]
pub fn show(path: &Path) -> Result<(), RevealError> {
    use std::{ffi::OsString, os::windows::process::CommandExt, process::Command};

    if !path.exists() {
        return open_folder(path);
    }
    let mut selecting = OsString::from("/select,\"");
    selecting.push(path);
    selecting.push("\"");
    let mut explorer = Command::new("explorer.exe");
    explorer.raw_arg(selecting);
    browser::detach(explorer).map_err(|error| RevealError::Unavailable(error.kind()))
}

#[cfg(unix)]
mod freedesktop {
    use std::{os::unix::ffi::OsStrExt, path::Path};

    use zbus::{Connection, Proxy, block_on};

    const FILE_MANAGER: &str = "org.freedesktop.FileManager1";
    const FILE_MANAGER_PATH: &str = "/org/freedesktop/FileManager1";
    const SHOW_ITEMS: &str = "ShowItems";
    const NO_STARTUP_ID: &str = "";

    pub fn file_uri(path: &Path) -> String {
        let mut uri = String::from("file://");
        for byte in path.as_os_str().as_bytes() {
            let plain = byte.is_ascii_alphanumeric() || b"-._~/".contains(byte);
            if plain {
                uri.push(char::from(*byte));
            } else {
                uri.push_str(&format!("%{byte:02X}"));
            }
        }
        uri
    }

    pub fn show_item(path: &Path) -> zbus::Result<()> {
        let uri = file_uri(path);
        block_on(async {
            let connection = Connection::session().await?;
            let manager =
                Proxy::new(&connection, FILE_MANAGER, FILE_MANAGER_PATH, FILE_MANAGER).await?;
            manager
                .call_method(SHOW_ITEMS, &(vec![uri.as_str()], NO_STARTUP_ID))
                .await?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_in_no_folder_is_refused_in_words_naming_it() {
        let nowhere = Path::new("/no-such-folder-for-caditor/part.step");

        assert_eq!(
            folder_of(nowhere),
            Err(RevealError::NoFolder(nowhere.to_owned()))
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_path_becomes_a_file_uri_with_everything_but_plain_characters_escaped() {
        let uri = freedesktop::file_uri(Path::new("/home/someone/My parts/gehäuse #2.step"));

        assert_eq!(
            uri,
            "file:///home/someone/My%20parts/geh%C3%A4use%20%232.step"
        );
    }
}
