use std::{
    ffi::{OsStr, OsString},
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Write},
    os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt, fchown},
    path::{Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::SystemTime,
};

use caditor_document::Document;
use rustix::io::Errno;
use xattr::FileExt as _;

use crate::{
    binary::{self, EncodeError, Encoded, Shared, value::ValueError},
    paths::{MAX_NAME_BYTES, fitting},
    read::{ensure_regular, open_file, read_open},
    reason,
};

const BACKUP_MARKER: &str = "damaged";
const MAX_BACKUP_ATTEMPTS: u32 = 1000;
const MAX_LINK_DEPTH: usize = 40;
const TEMPORARY_SUFFIX: &str = ".tmp";
const PROCESSES: &str = "/proc";
const BOOT_ID: &str = "/proc/sys/kernel/random/boot_id";
const MACHINE_IDS: [&str; 2] = ["/etc/machine-id", "/var/lib/dbus/machine-id"];
const TAG_LENGTH: usize = 16;
const UNKNOWN_TAG: &str = "unknown";
const TEMPORARY_STEM_LIMIT: usize = 170;
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

    fn encoding(error: &EncodeError) -> Self {
        log::error!("could not encode the model: {error}");
        let reason = match error {
            EncodeError::Value(ValueError::NonFinite(_)) => {
                "a number in the model is infinite or undefined, so it cannot be stored; undo \
                 the last change and save again"
                    .to_owned()
            }
            EncodeError::Value(ValueError::Malformed(_)) | EncodeError::Pack(_) => {
                "the model could not be converted for saving".to_owned()
            }
            EncodeError::HistoryTooLarge => {
                "there was not enough memory to rewrite the model's earlier versions; close other \
                 programs and save again"
                    .to_owned()
            }
            EncodeError::ModelTooLarge { largest, .. } => format!(
                "the model is larger than the {} GiB a model file can hold, so caditor could not \
                 open it again; remove imported bodies or split the model and save again",
                largest >> 30
            ),
        };
        Self { reason }
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Saved {
    pub backup: Option<PathBuf>,
    pub dropped_for_size: usize,
}

pub fn save(document: &Document, path: &Path, keep_original: bool) -> Result<Saved, SaveError> {
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
) -> Result<Saved, SaveError> {
    let target = resolve_links(path).map_err(|error| SaveError::writing(&error))?;
    ensure_replaceable(&target).map_err(|error| SaveError::writing(&error))?;
    let previous = options
        .history_from
        .map(read_previous)
        .transpose()?
        .flatten();
    let encoded = binary::encode_over(
        document,
        previous.as_ref().map(|previous| previous.bytes.as_slice()),
        SystemTime::now(),
        options.label,
    )
    .map_err(|error| SaveError::encoding(&error))?;
    let damaged_in_place = encoded.previous_damaged && reads_target(options, &target);
    let backup = if (options.keep_original || damaged_in_place) && target.exists() {
        Some(keep_backup(&target).map_err(|error| SaveError::writing(&error))?)
    } else {
        None
    };
    let reads_back = |file: &File| check_reads_back(file, &encoded.digest);
    let written = match &previous {
        Some(previous) if !encoded.shared.is_empty() => replace_checked(
            &target,
            |file| write_sharing(file, &encoded, previous),
            reads_back,
        ),
        _ => replace_checked(&target, |file| file.write_all(&encoded.bytes), reads_back),
    };
    written.map_err(|error| SaveError::writing(&error))?;
    Ok(Saved {
        backup,
        dropped_for_size: encoded.dropped_for_size,
    })
}

fn reads_target(options: &SaveOptions<'_>, target: &Path) -> bool {
    options
        .history_from
        .and_then(|from| resolve_links(from).ok())
        .is_some_and(|from| from == target)
}

#[derive(Debug, thiserror::Error)]
#[error("the saved copy did not read back intact, so the file was left as it was")]
pub(crate) struct NotReadBack;

fn check_reads_back(file: &File, digest: &str) -> io::Result<()> {
    let length = usize::try_from(file.metadata()?.len()).map_err(io::Error::other)?;
    let mut written = vec![0; length];
    file.read_exact_at(&mut written, 0)?;
    if binary::reads_back(&written, digest) {
        Ok(())
    } else {
        log::error!("a saved model did not decode back to what was written");
        Err(io::Error::new(io::ErrorKind::InvalidData, NotReadBack))
    }
}

struct Previous {
    file: File,
    bytes: Vec<u8>,
    stamp: Stamp,
}

impl Previous {
    fn open(path: &Path) -> io::Result<Self> {
        let file = open_file(path)?;
        let stamp = Stamp::of(&file.metadata()?);
        let bytes = read_open(&file)?;
        Ok(Self { file, bytes, stamp })
    }

    fn changed_since_read(&self) -> bool {
        self.file
            .metadata()
            .map_or(true, |metadata| Stamp::of(&metadata) != self.stamp)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    length: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

impl Stamp {
    fn of(metadata: &Metadata) -> Self {
        Self {
            length: metadata.size(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        }
    }
}

fn read_previous(path: &Path) -> Result<Option<Previous>, SaveError> {
    match Previous::open(path) {
        Ok(previous) => Ok(Some(previous)),
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

fn write_sharing(file: &mut File, encoded: &Encoded, source: &Previous) -> io::Result<()> {
    let bytes = encoded.bytes.as_slice();
    let mut cloned = Vec::new();
    for range in &encoded.shared {
        let copied = copy_range(&source.file, file, range);
        if copied > 0 {
            cloned.push(Shared {
                length: copied,
                ..*range
            });
        }
        let rest = range.at + copied;
        write_range(file, bytes, rest, range.at + range.length)?;
    }
    let mut position = 0;
    for range in &encoded.shared {
        write_range(file, bytes, position, range.at)?;
        position = range.at + range.length;
    }
    write_range(file, bytes, position, bytes.len())?;
    file.set_len(bytes.len() as u64)?;
    if source.changed_since_read() {
        log::warn!("the previous file changed while saving, so its versions are written again");
        for range in cloned {
            write_range(file, bytes, range.at, range.at + range.length)?;
        }
    }
    Ok(())
}

fn copy_range(source: &File, target: &File, range: &Shared) -> usize {
    let mut from = range.from as u64;
    let mut at = range.at as u64;
    let mut copied = 0;
    while copied < range.length {
        match rustix::fs::copy_file_range(
            source,
            Some(&mut from),
            target,
            Some(&mut at),
            range.length - copied,
        ) {
            Ok(0) => break,
            Ok(count) => copied += count,
            Err(Errno::INTR) => {}
            Err(error) => {
                log::debug!("could not share earlier versions with the previous file: {error}");
                break;
            }
        }
    }
    copied.min(range.length)
}

fn write_range(file: &File, bytes: &[u8], start: usize, end: usize) -> io::Result<()> {
    let range = bytes.get(start..end).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "a shared range lies outside the saved content",
        )
    })?;
    file.write_all_at(range, start as u64)
}

pub fn write_atomically(path: &Path, contents: &[u8]) -> io::Result<()> {
    replace_atomically(path, |file| file.write_all(contents))
}

fn replace_atomically(
    path: &Path,
    fill: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<()> {
    replace_checked(path, fill, |_| Ok(()))
}

fn replace_checked(
    path: &Path,
    fill: impl FnOnce(&mut File) -> io::Result<()>,
    check: impl FnOnce(&File) -> io::Result<()>,
) -> io::Result<()> {
    let target = resolve_links(path)?;
    ensure_replaceable(&target)?;
    let temporary = temporary_sibling(&target)?;
    let written = write_and_sync(&temporary, &target, fill, check)
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

#[derive(Debug, thiserror::Error)]
#[error("the file is read-only")]
pub(crate) struct ReadOnly;

fn ensure_replaceable(target: &Path) -> io::Result<()> {
    match fs::metadata(target) {
        Ok(metadata) => {
            ensure_regular(&metadata)?;
            let writable = !metadata.permissions().readonly()
                && rustix::fs::access(target, rustix::fs::Access::WRITE_OK).is_ok();
            if writable {
                Ok(())
            } else {
                Err(io::Error::new(io::ErrorKind::PermissionDenied, ReadOnly))
            }
        }
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
    let prefix = temporary_prefix(name);
    remove_orphans_in(parent, Some(prefix.as_encoded_bytes()));
}

pub(crate) fn sweep_orphaned_temporaries(dir: &Path) {
    remove_orphans_in(dir, None);
}

fn remove_orphans_in(dir: &Path, prefix: Option<&[u8]>) {
    let processes = Path::new(PROCESSES);
    let host = Host::current();
    if host.boot == UNKNOWN_TAG || !processes.join("self").exists() {
        return;
    }
    let dir = if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let ours = std::process::id();
    let running = |process: u32| processes.join(process.to_string()).exists();
    for entry in entries.filter_map(Result::ok) {
        let file_name = entry.file_name();
        let name = file_name.as_encoded_bytes();
        let named_as_wanted = prefix.is_none_or(|prefix| name.starts_with(prefix));
        let orphaned = named_as_wanted
            && temporary_tag(name).is_some_and(|tag| host.orphaned(tag, ours, running));
        if orphaned {
            let orphan = entry.path();
            match fs::remove_file(&orphan) {
                Ok(()) => log::info!("removed {}, left by an interrupted save", orphan.display()),
                Err(error) => log::warn!("could not remove {}: {error}", orphan.display()),
            }
        }
    }
}

fn temporary_tag(name: &[u8]) -> Option<&str> {
    let name = std::str::from_utf8(name).ok()?;
    let without_suffix = name.strip_prefix('.')?.strip_suffix(TEMPORARY_SUFFIX)?;
    let (_, tag) = without_suffix.rsplit_once('.')?;
    Some(tag)
}

#[derive(Debug, Clone, Copy)]
struct Host<'a> {
    machine: &'a str,
    boot: &'a str,
}

impl Host<'static> {
    fn current() -> Self {
        Self {
            machine: machine_tag(),
            boot: boot_tag(),
        }
    }
}

impl Host<'_> {
    fn orphaned(&self, tag: &str, ours: u32, running: impl Fn(u32) -> bool) -> bool {
        let parts: Vec<&str> = tag.split('-').collect();
        let (machine, boot, owner, counter) = match parts.as_slice() {
            [machine, boot, owner, counter] => (Some(*machine), *boot, *owner, *counter),
            [boot, owner, counter] => (None, *boot, *owner, *counter),
            _ => return false,
        };
        let digits =
            |text: &str| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
        if !digits(counter) || !digits(owner) {
            return false;
        }
        let Ok(owner) = owner.parse::<u32>() else {
            return false;
        };
        match machine {
            Some(machine) if machine != self.machine => return false,
            Some(_) if boot != self.boot => return self.machine != UNKNOWN_TAG,
            None if boot != self.boot => return false,
            Some(_) | None => {}
        }
        owner != ours && !running(owner)
    }
}

fn write_and_sync(
    temporary: &Path,
    target: &Path,
    fill: impl FnOnce(&mut File) -> io::Result<()>,
    check: impl FnOnce(&File) -> io::Result<()>,
) -> io::Result<()> {
    let mut file = match fs::metadata(target) {
        Ok(existing) => {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .mode(PRIVATE_MODE)
                .open(temporary)?;
            take_ownership_and_attributes(&file, target, &existing);
            file.set_permissions(existing.permissions())?;
            file
        }
        Err(_) => OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(temporary)?,
    };
    fill(&mut file)?;
    file.sync_all()?;
    check(&file)
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
        "{}-{}-{}-{}{TEMPORARY_SUFFIX}",
        machine_tag(),
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
    TAG.get_or_init(|| tag_from(&[BOOT_ID]))
}

pub(crate) fn machine_tag() -> &'static str {
    static TAG: OnceLock<String> = OnceLock::new();
    TAG.get_or_init(|| tag_from(&MACHINE_IDS))
}

fn tag_from(sources: &[&str]) -> String {
    sources
        .iter()
        .filter_map(|source| fs::read_to_string(source).ok())
        .map(|id| {
            id.chars()
                .filter(char::is_ascii_hexdigit)
                .take(TAG_LENGTH)
                .collect::<String>()
        })
        .find(|tag| tag.len() == TAG_LENGTH)
        .unwrap_or_else(|| UNKNOWN_TAG.to_owned())
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

pub(crate) fn keep_unreadable(path: &Path, stem: &str) -> io::Result<PathBuf> {
    let candidates = (1..=MAX_BACKUP_ATTEMPTS).map(|attempt| {
        path.with_file_name(match attempt {
            1 => format!("{stem}.json"),
            _ => format!("{stem}-{attempt}.json"),
        })
    });
    keep_copy(path, candidates.collect())?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "too many unreadable copies of this file already exist",
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

    const BLOCK: usize = 4096;

    fn pattern(length: usize, seed: u8) -> Vec<u8> {
        (0..length)
            .map(|index| (index as u8).wrapping_mul(31).wrapping_add(seed) ^ (index >> 9) as u8)
            .collect()
    }

    fn sharing_layout(previous: &[u8]) -> Encoded {
        let mut bytes = pattern(BLOCK + 100, 7);
        let middle = previous.len() / BLOCK / 2 * BLOCK;
        let first_at = 2 * BLOCK;
        bytes.resize(first_at, 1);
        bytes.extend_from_slice(&previous[BLOCK..middle]);
        bytes.extend_from_slice(&pattern(3 * BLOCK + 17, 9));
        let second_at = bytes.len().next_multiple_of(BLOCK);
        bytes.resize(second_at, 2);
        bytes.extend_from_slice(&previous[middle..]);
        bytes.extend_from_slice(&pattern(BLOCK / 2, 11));
        Encoded {
            shared: vec![
                Shared {
                    at: first_at,
                    from: BLOCK,
                    length: middle - BLOCK,
                },
                Shared {
                    at: second_at,
                    from: middle,
                    length: previous.len() - middle,
                },
            ],
            bytes,
            digest: String::new(),
            previous_damaged: false,
            dropped_for_size: 0,
        }
    }

    #[test]
    fn a_shared_save_writes_exactly_what_was_encoded() {
        let dir = TempDir::new().unwrap();
        let source = dir.path().join("before.caditor");
        let target = dir.path().join("after.caditor");
        fs::write(&source, pattern(9 * BLOCK + 1234, 3)).unwrap();
        fs::write(&target, "old").unwrap();

        let previous = Previous::open(&source).unwrap();
        let encoded = sharing_layout(&previous.bytes);
        replace_atomically(&target, |file| write_sharing(file, &encoded, &previous)).unwrap();

        assert_eq!(fs::read(&target).unwrap(), encoded.bytes);
        assert_eq!(fs::read(&source).unwrap(), previous.bytes);
    }

    #[test]
    fn a_previous_file_changed_after_it_was_read_is_not_shared() {
        let dir = TempDir::new().unwrap();
        let source = dir.path().join("before.caditor");
        let target = dir.path().join("after.caditor");
        fs::write(&source, pattern(9 * BLOCK + 1234, 3)).unwrap();

        let previous = Previous::open(&source).unwrap();
        let encoded = sharing_layout(&previous.bytes);
        fs::write(&source, pattern(10 * BLOCK, 5)).unwrap();
        assert!(previous.changed_since_read());
        replace_atomically(&target, |file| write_sharing(file, &encoded, &previous)).unwrap();

        assert_eq!(fs::read(&target).unwrap(), encoded.bytes);
    }

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
