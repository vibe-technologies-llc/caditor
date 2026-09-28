use std::{
    ffi::OsString,
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::SystemTime,
};

use caditor_document::Document;

use crate::{binary, reason};

const BACKUP_MARKER: &str = "damaged";
const MAX_BACKUP_ATTEMPTS: u32 = 1000;

static TEMPORARY_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{reason}")]
pub struct SaveError {
    pub reason: String,
}

impl SaveError {
    fn writing(error: &io::Error) -> Self {
        Self {
            reason: reason::writing(error),
        }
    }

    fn encoding(error: &impl std::fmt::Display) -> Self {
        log::error!("could not encode the model: {error}");
        Self {
            reason: "the model could not be converted for saving".to_owned(),
        }
    }
}

pub fn encode(document: &Document) -> Result<Vec<u8>, SaveError> {
    binary::encode(document).map_err(|error| SaveError::encoding(&error))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SaveOptions<'a> {
    pub keep_original: bool,
    pub history_from: Option<&'a Path>,
    pub label: Option<&'a str>,
}

pub fn save(
    document: &Document,
    path: &Path,
    keep_original: bool,
) -> Result<Option<PathBuf>, SaveError> {
    save_with(
        document,
        path,
        &SaveOptions {
            keep_original,
            history_from: Some(path),
            label: None,
        },
    )
}

pub fn save_with(
    document: &Document,
    path: &Path,
    options: &SaveOptions<'_>,
) -> Result<Option<PathBuf>, SaveError> {
    let previous = options.history_from.and_then(read_previous);
    let contents = binary::save_bytes(
        document,
        previous.as_deref(),
        SystemTime::now(),
        options.label,
    )
    .map_err(|error| SaveError::encoding(&error))?;
    let backup = if options.keep_original && path.exists() {
        Some(keep_backup(path).map_err(|error| SaveError::writing(&error))?)
    } else {
        None
    };
    write_atomically(path, &contents).map_err(|error| SaveError::writing(&error))?;
    Ok(backup)
}

fn read_previous(path: &Path) -> Option<Vec<u8>> {
    match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => {
            log::warn!(
                "could not read the earlier versions in {}: {error}",
                path.display()
            );
            None
        }
    }
}

pub fn write_atomically(path: &Path, contents: &[u8]) -> io::Result<()> {
    let temporary = temporary_sibling(path)?;
    let written = write_and_sync(&temporary, path, contents)
        .and_then(|()| fs::rename(&temporary, path))
        .and_then(|()| sync_parent(path));
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

fn write_and_sync(temporary: &Path, target: &Path, contents: &[u8]) -> io::Result<()> {
    let mut file = File::create_new(temporary)?;
    if let Ok(existing) = fs::metadata(target) {
        file.set_permissions(existing.permissions())?;
    }
    file.write_all(contents)?;
    file.sync_all()
}

pub(crate) fn temporary_sibling(path: &Path) -> io::Result<PathBuf> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the path has no file name"))?;
    let mut temporary = OsString::from(".");
    temporary.push(name);
    temporary.push(format!(
        ".{}-{}.tmp",
        std::process::id(),
        TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    Ok(path.with_file_name(temporary))
}

pub(crate) fn sync_parent(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    File::open(parent)?.sync_all()
}

fn keep_backup(path: &Path) -> io::Result<PathBuf> {
    let stem = path.file_stem().unwrap_or_default().to_os_string();
    let extension = path.extension().map(|extension| extension.to_os_string());
    for attempt in 1..=MAX_BACKUP_ATTEMPTS {
        let mut name = stem.clone();
        name.push(match attempt {
            1 => format!(".{BACKUP_MARKER}"),
            _ => format!(".{BACKUP_MARKER}-{attempt}"),
        });
        if let Some(extension) = &extension {
            name.push(".");
            name.push(extension);
        }
        let backup = path.with_file_name(name);
        match fs::hard_link(path, &backup) {
            Ok(()) => {
                sync_parent(&backup)?;
                return Ok(backup);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(_) => {
                let Ok(mut copy) = File::create_new(&backup) else {
                    continue;
                };
                io::copy(&mut File::open(path)?, &mut copy)?;
                copy.sync_all()?;
                sync_parent(&backup)?;
                return Ok(backup);
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "too many backups of this file already exist",
    ))
}
