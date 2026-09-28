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
const MAX_LINK_DEPTH: usize = 40;
const TEMPORARY_SUFFIX: &str = ".tmp";
const PROCESSES: &str = "/proc";

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
    let previous = options
        .history_from
        .map(read_previous)
        .transpose()?
        .flatten();
    let contents = binary::save_bytes(
        document,
        previous.as_deref(),
        SystemTime::now(),
        options.label,
    )
    .map_err(|error| SaveError::encoding(&error))?;
    let target = resolve_links(path).map_err(|error| SaveError::writing(&error))?;
    let backup = if options.keep_original && target.exists() {
        Some(keep_backup(&target).map_err(|error| SaveError::writing(&error))?)
    } else {
        None
    };
    write_atomically(&target, &contents).map_err(|error| SaveError::writing(&error))?;
    Ok(backup)
}

fn read_previous(path: &Path) -> Result<Option<Vec<u8>>, SaveError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => {
            log::warn!(
                "could not read the earlier versions in {}: {error}",
                path.display()
            );
            let name = path.file_name().unwrap_or(path.as_os_str()).display();
            Err(SaveError {
                reason: format!(
                    "the earlier versions kept in “{name}” could not be read ({}), and saving now \
                     would lose them",
                    reason::reading(&error)
                ),
            })
        }
    }
}

pub fn write_atomically(path: &Path, contents: &[u8]) -> io::Result<()> {
    let target = resolve_links(path)?;
    let temporary = temporary_sibling(&target)?;
    let written = write_and_sync(&temporary, &target, contents)
        .and_then(|()| fs::rename(&temporary, &target));
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written?;
    if let Err(error) = sync_parent(&target) {
        log::warn!(
            "saved {}, but its folder could not be flushed to disk: {error}",
            target.display()
        );
    }
    remove_orphaned_temporaries(&target);
    Ok(())
}

pub(crate) fn resolve_links(path: &Path) -> io::Result<PathBuf> {
    let mut resolved = path.to_path_buf();
    for _ in 0..MAX_LINK_DEPTH {
        match fs::read_link(&resolved) {
            Ok(target) => {
                resolved = match resolved.parent() {
                    Some(parent) => parent.join(target),
                    None => target,
                };
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::InvalidInput | io::ErrorKind::NotFound
                ) =>
            {
                return Ok(resolved);
            }
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::other(
        "the path goes through too many symbolic links",
    ))
}

pub(crate) fn remove_orphaned_temporaries(path: &Path) {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return;
    };
    let processes = Path::new(PROCESSES);
    if !processes.join("self").exists() {
        return;
    }
    let parent = if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    };
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    let mut prefix = OsString::from(".");
    prefix.push(name);
    prefix.push(".");
    let prefix = prefix.as_encoded_bytes();
    let ours = std::process::id();
    for entry in entries.filter_map(Result::ok) {
        let file_name = entry.file_name();
        let Some(owner) = file_name
            .as_encoded_bytes()
            .strip_prefix(prefix)
            .and_then(temporary_owner)
        else {
            continue;
        };
        if owner != ours && !processes.join(owner.to_string()).exists() {
            let orphan = entry.path();
            match fs::remove_file(&orphan) {
                Ok(()) => log::info!("removed {}, left by an interrupted save", orphan.display()),
                Err(error) => log::warn!("could not remove {}: {error}", orphan.display()),
            }
        }
    }
}

fn temporary_owner(suffix: &[u8]) -> Option<u32> {
    let suffix = std::str::from_utf8(suffix).ok()?;
    let (owner, counter) = suffix.strip_suffix(TEMPORARY_SUFFIX)?.split_once('-')?;
    let digits = |text: &str| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    (digits(owner) && digits(counter))
        .then(|| owner.parse().ok())
        .flatten()
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
        ".{}-{}{TEMPORARY_SUFFIX}",
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
            Err(_) => match File::create_new(&backup) {
                Ok(mut copy) => {
                    io::copy(&mut File::open(path)?, &mut copy)?;
                    copy.sync_all()?;
                    sync_parent(&backup)?;
                    return Ok(backup);
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            },
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "too many backups of this file already exist",
    ))
}
