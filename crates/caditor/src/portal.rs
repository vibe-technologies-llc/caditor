use std::{io, path::PathBuf};

#[cfg(windows)]
mod windows;
#[cfg(unix)]
mod xdg;

#[cfg(windows)]
pub use self::windows::{choose, own_dialogs};
#[cfg(unix)]
pub use self::xdg::choose;

const TRY_AGAIN: &str = " Try again, or choose a file on this computer.";

#[derive(Debug, thiserror::Error)]
pub enum DialogError {
    #[cfg(unix)]
    #[error("there is no desktop session bus to ask for a file dialog ({0})")]
    NoSessionBus(zbus::Error),
    #[cfg(unix)]
    #[error("no desktop portal offers a file dialog, and zenity is not installed ({0})")]
    NoPortal(zbus::Error),
    #[cfg(unix)]
    #[error("the desktop portal's file dialog failed ({0})")]
    Portal(zbus::Error),
    #[cfg(unix)]
    #[error("the desktop portal chose “{0}”, which is not a local file")]
    NotALocalFile(String),
    #[cfg(unix)]
    #[error("zenity could not show the file dialog ({0})")]
    Zenity(io::Error),
    #[cfg(unix)]
    #[error("zenity's file dialog failed with exit status {0}")]
    ZenityFailed(i32),
    #[error("could not start the file dialog's thread ({0})")]
    Thread(io::Error),
}

impl DialogError {
    pub fn notice(&self) -> String {
        let remedy = match self {
            #[cfg(unix)]
            Self::NoSessionBus(_) | Self::NoPortal(_) => {
                " Install xdg-desktop-portal with the backend for your desktop (such as \
                 xdg-desktop-portal-gtk or xdg-desktop-portal-kde), or zenity, then try again."
            }
            #[cfg(unix)]
            Self::Portal(_) | Self::NotALocalFile(_) | Self::Zenity(_) | Self::ZenityFailed(_) => {
                TRY_AGAIN
            }
            Self::Thread(_) => TRY_AGAIN,
        };
        format!("The file dialog could not be shown: {self}.{remedy}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    pub name: String,
    pub extensions: Vec<String>,
}

impl Filter {
    pub fn new(name: &str, extensions: &[&str]) -> Self {
        Self {
            name: name.to_owned(),
            extensions: extensions
                .iter()
                .map(|&extension| extension.to_owned())
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Open,
    Save,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRequest {
    pub mode: Mode,
    pub title: String,
    pub directory: Option<PathBuf>,
    pub file_name: Option<String>,
    pub filters: Vec<Filter>,
}
