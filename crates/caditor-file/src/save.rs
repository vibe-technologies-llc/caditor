use std::{
    ffi::{OsStr, OsString},
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, fchown},
    path::{Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::SystemTime,
};

use caditor_document::Document;
use xattr::FileExt;

use crate::{
    binary,
    paths::{MAX_NAME_BYTES, fitting},
    read::{ensure_regular, read_file},
    reason,
};

const BACKUP_MARKER: &str = "damaged";
const MAX_BACKUP_ATTEMPTS: u32 = 1000;
const MAX_LINK_DEPTH: usize = 40;
const TEMPORARY_SUFFIX: &str = ".tmp";
const PROCESSES: &str = "/proc";
const BOOT_ID: &str = "/proc/sys/kernel/random/boot_id";
const BOOT_TAG_LENGTH: usize = 16;
const UNKNOWN_BOOT: &str = "unknown";
const TEMPORARY_STEM_LIMIT: usize = 190;
const BACKUP_ROOM: usize = 16;
const PRIVATE_MODE: u32 = 0o600;

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
    let target = resolve_links(path).map_err(|error| SaveError::writing(&error))?;
    ensure_replaceable(&target).map_err(|error| SaveError::writing(&error))?;
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
    let backup = if options.keep_original && target.exists() {
        Some(keep_backup(&target).map_err(|error| SaveError::writing(&error))?)
    } else {
        None
    };
    write_atomically(&target, &contents).map_err(|error| SaveError::writing(&error))?;
    Ok(backup)
}

fn read_previous(path: &Path) -> Result<Option<Vec<u8>>, SaveError> {
    match read_file(path) {
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
    ensure_replaceable(&target)?;
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

fn ensure_replaceable(target: &Path) -> io::Result<()> {
    match fs::metadata(target) {
        Ok(metadata) => ensure_regular(&metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
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
    let tag = boot_tag();
    if tag == UNKNOWN_BOOT || !processes.join("self").exists() {
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
    let prefix = temporary_prefix(name);
    let prefix = prefix.as_encoded_bytes();
    let ours = std::process::id();
    for entry in entries.filter_map(Result::ok) {
        let file_name = entry.file_name();
        let Some(owner) = file_name
            .as_encoded_bytes()
            .strip_prefix(prefix)
            .and_then(|suffix| temporary_owner(suffix, tag))
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

fn temporary_owner(suffix: &[u8], tag: &str) -> Option<u32> {
    let suffix = std::str::from_utf8(suffix).ok()?;
    let rest = suffix
        .strip_suffix(TEMPORARY_SUFFIX)?
        .strip_prefix(tag)?
        .strip_prefix('-')?;
    let (owner, counter) = rest.split_once('-')?;
    let digits = |text: &str| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    (digits(owner) && digits(counter))
        .then(|| owner.parse().ok())
        .flatten()
}

fn write_and_sync(temporary: &Path, target: &Path, contents: &[u8]) -> io::Result<()> {
    let mut file = match fs::metadata(target) {
        Ok(existing) => {
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(PRIVATE_MODE)
                .open(temporary)?;
            take_ownership_and_attributes(&file, target, &existing);
            file.set_permissions(existing.permissions())?;
            file
        }
        Err(_) => File::create_new(temporary)?,
    };
    file.write_all(contents)?;
    file.sync_all()
}

fn take_ownership_and_attributes(file: &File, target: &Path, existing: &Metadata) {
    let group_differs = file
        .metadata()
        .is_ok_and(|created| created.gid() != existing.gid());
    if group_differs && let Err(error) = fchown(file, None, Some(existing.gid())) {
        log::debug!(
            "could not give the saved file the group of {}: {error}",
            target.display()
        );
    }
    let names = match xattr::list(target) {
        Ok(names) => names,
        Err(error) => {
            log::debug!(
                "could not list the attributes of {}: {error}",
                target.display()
            );
            return;
        }
    };
    for name in names {
        let copied = xattr::get(target, &name)
            .and_then(|value| value.map_or(Ok(()), |value| file.set_xattr(&name, &value)));
        if let Err(error) = copied {
            log::debug!(
                "could not copy the attribute {} of {}: {error}",
                name.display(),
                target.display()
            );
        }
    }
}

pub(crate) fn temporary_sibling(path: &Path) -> io::Result<PathBuf> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the path has no file name"))?;
    let mut temporary = temporary_prefix(name);
    temporary.push(format!(
        "{}-{}-{}{TEMPORARY_SUFFIX}",
        boot_tag(),
        std::process::id(),
        TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    Ok(path.with_file_name(temporary))
}

fn temporary_prefix(name: &OsStr) -> OsString {
    let mut prefix = OsString::from(".");
    prefix.push(fitting(name, TEMPORARY_STEM_LIMIT));
    prefix.push(".");
    prefix
}

pub(crate) fn boot_tag() -> &'static str {
    static TAG: OnceLock<String> = OnceLock::new();
    TAG.get_or_init(|| {
        fs::read_to_string(BOOT_ID)
            .ok()
            .map(|id| {
                id.chars()
                    .filter(char::is_ascii_hexdigit)
                    .take(BOOT_TAG_LENGTH)
                    .collect::<String>()
            })
            .filter(|tag| tag.len() == BOOT_TAG_LENGTH)
            .unwrap_or_else(|| UNKNOWN_BOOT.to_owned())
    })
}

pub(crate) fn sync_parent(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    File::open(parent)?.sync_all()
}

fn keep_backup(path: &Path) -> io::Result<PathBuf> {
    let extension = path.extension().map(|extension| extension.to_os_string());
    let extension_room = extension
        .as_ref()
        .map_or(0, |extension| extension.len() + 1);
    let stem = fitting(
        path.file_stem().unwrap_or_default(),
        MAX_NAME_BYTES.saturating_sub(extension_room + BACKUP_ROOM),
    );
    let candidates = (1..=MAX_BACKUP_ATTEMPTS).map(|attempt| {
        let mut name = stem.clone();
        name.push(match attempt {
            1 => format!(".{BACKUP_MARKER}"),
            _ => format!(".{BACKUP_MARKER}-{attempt}"),
        });
        if let Some(extension) = &extension {
            name.push(".");
            name.push(extension);
        }
        path.with_file_name(name)
    });
    keep_copy(path, candidates.collect())?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "too many backups of this file already exist",
        )
    })
}

pub(crate) fn keep_copy(source: &Path, candidates: Vec<PathBuf>) -> io::Result<Option<PathBuf>> {
    for (index, candidate) in candidates.iter().enumerate() {
        match fs::hard_link(source, candidate) {
            Ok(()) => {
                sync_parent(candidate)?;
                return Ok(Some(candidate.clone()));
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                log::debug!("could not link {}: {error}", candidate.display());
                return keep_by_copying(source, candidates.get(index..).unwrap_or_default());
            }
        }
    }
    Ok(None)
}

fn keep_by_copying(source: &Path, candidates: &[PathBuf]) -> io::Result<Option<PathBuf>> {
    let Some(first) = candidates.first() else {
        return Ok(None);
    };
    let temporary = temporary_sibling(first)?;
    let kept = copy_then_place(source, &temporary, candidates);
    if !matches!(kept, Ok(Some(_))) {
        let _ = fs::remove_file(&temporary);
    }
    kept
}

fn copy_then_place(
    source: &Path,
    temporary: &Path,
    candidates: &[PathBuf],
) -> io::Result<Option<PathBuf>> {
    fs::copy(source, temporary)?;
    File::open(temporary)?.sync_all()?;
    for candidate in candidates {
        match File::create_new(candidate) {
            Ok(_) => {
                if let Err(error) = fs::rename(temporary, candidate) {
                    let _ = fs::remove_file(candidate);
                    return Err(error);
                }
                sync_parent(candidate)?;
                return Ok(Some(candidate.clone()));
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn a_copy_is_placed_whole_under_the_first_free_name_or_not_at_all() {
        let dir = TempDir::new().unwrap();
        let source = dir.path().join("model.caditor");
        fs::write(&source, "original").unwrap();
        let taken = dir.path().join("model.damaged.caditor");
        fs::write(&taken, "earlier").unwrap();
        let free = dir.path().join("model.damaged-2.caditor");

        let kept = keep_by_copying(&source, &[taken.clone(), free.clone()]).unwrap();

        assert_eq!(kept, Some(free.clone()));
        assert_eq!(fs::read_to_string(&free).unwrap(), "original");
        assert_eq!(fs::read_to_string(&taken).unwrap(), "earlier");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 3);

        let folder = dir.path().join("folder");
        fs::create_dir(&folder).unwrap();
        let wanted = dir.path().join("folder.damaged");
        assert!(keep_by_copying(&folder, std::slice::from_ref(&wanted)).is_err());
        assert!(!wanted.exists());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 4);

        assert_eq!(keep_by_copying(&source, &[taken, free]).unwrap(), None);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 4);
    }
}
