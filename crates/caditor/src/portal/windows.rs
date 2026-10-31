use std::{num::NonZeroIsize, path::PathBuf, sync::OnceLock};

use caditor_windows::DialogParent;
use rfd::FileDialog;

use super::{DesktopSettings, DialogError, FileRequest, Mode, SettingError};

static OWNER: OnceLock<DialogParent> = OnceLock::new();

pub fn own_dialogs(window: NonZeroIsize) {
    if OWNER.set(DialogParent::new(window)).is_err() {
        log::debug!("file dialogs already belong to the first window");
    }
}

pub fn desktop_settings() -> Result<DesktopSettings, SettingError> {
    let animated = caditor_windows::client_area_animations().map_err(SettingError::Animations)?;
    let more_contrast = caditor_windows::high_contrast().map_err(SettingError::HighContrast)?;
    Ok(DesktopSettings {
        reduces_motion: !animated,
        more_contrast,
    })
}

pub fn watch_desktop(_changed: impl FnMut(DesktopSettings)) -> Result<(), SettingError> {
    Err(SettingError::NoChangeSignal)
}

pub fn choose(request: &FileRequest) -> Result<Option<PathBuf>, DialogError> {
    let mut dialog = FileDialog::new().set_title(request.title.as_str());
    if let Some(directory) = &request.directory {
        dialog = dialog.set_directory(directory);
    }
    if let Some(name) = &request.file_name {
        dialog = dialog.set_file_name(name.as_str());
    }
    for filter in &request.filters {
        dialog = dialog.add_filter(filter.name.as_str(), &filter.extensions);
    }
    if let Some(owner) = OWNER.get() {
        dialog = dialog.set_parent(owner);
    }
    Ok(match request.mode {
        Mode::Open => dialog.pick_file(),
        Mode::Save => dialog.save_file(),
    })
}
