use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{browser, slicer};

pub const FOLDER: &str = "fusion";
const SCHEME_LINK: &str = "fusion360://host/?command=open&file=";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PrepareError {
    #[error("caditor has no folder of its own to keep the file for Fusion 360 in")]
    NoFolder,
    #[error("caditor could not make its folder for the file for Fusion 360: {0}")]
    Folder(io::ErrorKind),
    #[error(
        "Nothing on this computer opens Fusion 360 links. Install Fusion 360 and start it once, \
         then try again"
    )]
    NoHandler,
    #[error("caditor ran into an internal error while preparing the file for Fusion 360")]
    Crashed,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LaunchError {
    #[error("The file’s place “{}” cannot be written in a link for Fusion 360", .0.display())]
    NotText(PathBuf),
    #[error("caditor could not hand the file to Fusion 360: {0}")]
    Unavailable(io::ErrorKind),
}

pub type Launcher = Arc<dyn Fn(&Path) -> Result<(), LaunchError> + Send + Sync>;
pub type LinkHandler = Arc<dyn Fn() -> bool + Send + Sync>;

pub fn opens_links() -> bool {
    os::opens_links()
}

pub fn prepare(
    folder: Option<&Path>,
    opens_links: &dyn Fn() -> bool,
) -> Result<PathBuf, PrepareError> {
    let folder = folder.ok_or(PrepareError::NoFolder)?;
    slicer::make_folder(folder).map_err(PrepareError::Folder)?;
    if !opens_links() {
        return Err(PrepareError::NoHandler);
    }
    Ok(folder.to_path_buf())
}

pub fn address(file: &Path) -> Result<String, LaunchError> {
    let text = file
        .to_str()
        .ok_or_else(|| LaunchError::NotText(file.to_path_buf()))?;
    let mut address = String::from(SCHEME_LINK);
    for byte in text.replace('\\', "/").bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            address.push(char::from(byte));
        } else {
            address.push_str(&format!("%{byte:02X}"));
        }
    }
    Ok(address)
}

pub fn launch(file: &Path) -> Result<(), LaunchError> {
    browser::launch(&address(file)?).map_err(|error| LaunchError::Unavailable(error.kind()))
}

#[cfg(unix)]
mod os {
    use std::process::{Command, Stdio};

    const SCHEME_HANDLER: &str = "x-scheme-handler/fusion360";

    pub fn opens_links() -> bool {
        Command::new("xdg-mime")
            .args(["query", "default", SCHEME_HANDLER])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .is_ok_and(|output| output.status.success() && !output.stdout.trim_ascii().is_empty())
    }
}

#[cfg(windows)]
mod os {
    pub fn opens_links() -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_windows_path_is_written_with_forward_slashes_and_encoded() {
        assert_eq!(
            address(Path::new(r"C:\Users\Ada Lovelace\caditor\Plate (2).step")).unwrap(),
            "fusion360://host/?command=open&file=C%3A%2FUsers%2FAda%20Lovelace%2Fcaditor%2FPlate%20%282%29.step"
        );
    }

    #[test]
    fn a_unix_path_and_a_non_ascii_name_are_encoded_byte_by_byte() {
        assert_eq!(
            address(Path::new("/home/ola/Zębatka-1_a~b.step")).unwrap(),
            "fusion360://host/?command=open&file=%2Fhome%2Fola%2FZ%C4%99batka-1_a~b.step"
        );
    }

    #[test]
    fn a_missing_folder_is_refused_before_anything_is_made() {
        assert_eq!(prepare(None, &|| true), Err(PrepareError::NoFolder));
    }

    #[cfg(unix)]
    #[test]
    fn a_path_that_is_not_text_is_refused() {
        use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

        let path = Path::new(OsStr::from_bytes(b"/tmp/\xff.step"));

        assert_eq!(address(path), Err(LaunchError::NotText(path.to_path_buf())));
    }
}
