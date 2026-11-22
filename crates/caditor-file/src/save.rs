use std::{
    ffi::{OsStr, OsString},
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, SystemTime},
};

use caditor_document::Document;

use crate::{
    binary::{
        self, EncodeError, EncodeExtras, Encoded, FileDigest, FreshVersion, KeepError, PackError,
        Retention, Shared, value::ValueError,
    },
    load::{VERSIONS_CHANGED, VersionDigest},
    os::{self, ChangeStamp, FileKey},
    paths::{MAX_NAME_BYTES, fitting},
    preview::PreviewSlot,
    read::{ensure_regular, open_file, read_open},
    reason::{ReadFailure, WriteFailure},
};

const BACKUP_MARKER: &str = "damaged";
const KEPT_MARKER: &str = "kept";
const MAX_BACKUP_ATTEMPTS: u32 = 1000;
const MAX_LINK_DEPTH: usize = 40;
const TEMPORARY_SUFFIX: &str = ".tmp";
const TAG_LENGTH: usize = 16;
const UNKNOWN_TAG: &str = "unknown";
const TEMPORARY_STEM_LIMIT: usize = 170;
const BACKUP_ROOM: usize = 16;
const RENAME_ATTEMPTS: u32 = 5;
const RENAME_PAUSE: Duration = Duration::from_millis(100);

static TEMPORARY_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SaveError {
    #[error("{0}")]
    Writing(WriteFailure),
    #[error(
        "a number in the model is infinite or undefined, so it cannot be stored; undo the last \
         change and save again"
    )]
    NonFinite,
    #[error("the model could not be converted for saving")]
    Unconvertible,
    #[error(
        "there was not enough memory to rewrite the model's earlier versions; close other \
         programs and save again"
    )]
    HistoryTooLarge,
    #[error(
        "there was not enough memory to save the model, so the file was left as it was; close \
         other programs and save again"
    )]
    OutOfMemory,
    #[error(
        "the model is larger than the {} GiB a model file can hold, so caditor could not open it \
         again; remove imported bodies or split the model and save again",
        largest >> 30
    )]
    ModelTooLarge { largest: usize },
    #[error(
        "the earlier versions kept in “{name}” could not be read ({failure}), and saving now \
         would lose them"
    )]
    EarlierVersionsUnreadable { name: String, failure: ReadFailure },
    #[error("it is open in another caditor window")]
    OpenInAnotherWindow,
    #[error("it was changed by another program since it was opened")]
    ChangedOnDisk,
    #[error(
        "the save stopped unexpectedly and the file was left as it was; your changes are still \
         kept for recovery, so save again"
    )]
    StoppedUnexpectedly,
    #[error(
        "the saved copy could not take the file's place, so the model and its earlier versions \
         are kept in “{}” in the same folder; rename it to the model's name to carry on from it",
        kept.file_name().unwrap_or(kept.as_os_str()).display()
    )]
    KeptAside { kept: PathBuf },
}

impl SaveError {
    fn writing(error: &io::Error) -> Self {
        match kept_aside(error) {
            Some(kept) => Self::KeptAside {
                kept: kept.to_path_buf(),
            },
            None if error.kind() == io::ErrorKind::OutOfMemory => Self::OutOfMemory,
            None => Self::Writing(WriteFailure::of(error)),
        }
    }

    fn encoding(error: &EncodeError) -> Self {
        log::error!("could not encode the model: {error}");
        match error {
            EncodeError::Value(ValueError::NonFinite(_)) => Self::NonFinite,
            EncodeError::Pack(PackError::OutOfMemory) => Self::OutOfMemory,
            EncodeError::Value(_) | EncodeError::Pack(_) => Self::Unconvertible,
            EncodeError::HistoryTooLarge => Self::HistoryTooLarge,
            EncodeError::ModelTooLarge { largest, .. } => Self::ModelTooLarge { largest: *largest },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum KeepVersionError {
    #[error("{0}")]
    Writing(WriteFailure),
    #[error("“{name}” could not be read ({failure})")]
    Unreadable { name: String, failure: ReadFailure },
    #[error("that version is no longer in the file")]
    NoSuchVersion,
    #[error("{}", VERSIONS_CHANGED)]
    VersionsChanged,
    #[error(
        "the file has damaged parts, so its versions cannot be changed until it is saved again"
    )]
    FileDamaged,
    #[error("the file was written by a newer version of caditor")]
    FromNewerVersion,
    #[error("the file was changed by another program since it was opened")]
    ChangedOnDisk,
    #[error("the file is not the model open in this window")]
    NotTheOpenModel,
    #[error("the file's versions could not be converted")]
    Unconvertible,
    #[error("the change stopped unexpectedly; try it again")]
    StoppedUnexpectedly,
    #[error(
        "the rewritten copy could not take the file's place, so it is kept in “{}” in the same \
         folder; rename it to the model's name to carry on from it",
        kept.file_name().unwrap_or(kept.as_os_str()).display()
    )]
    KeptAside { kept: PathBuf },
}

impl KeepVersionError {
    fn writing(error: &io::Error) -> Self {
        match kept_aside(error) {
            Some(kept) => Self::KeptAside {
                kept: kept.to_path_buf(),
            },
            None => Self::Writing(WriteFailure::of(error)),
        }
    }

    fn rewriting(error: KeepError) -> Self {
        match error {
            KeepError::NoSuchVersion { .. } => Self::NoSuchVersion,
            KeepError::VersionsChanged { .. } => Self::VersionsChanged,
            KeepError::FileDamaged => Self::FileDamaged,
            KeepError::FromNewerVersion => Self::FromNewerVersion,
            KeepError::Value(_) | KeepError::Pack(_) => Self::Unconvertible,
        }
    }
}

pub fn set_version_kept(
    path: &Path,
    index: usize,
    digest: &VersionDigest,
    kept: bool,
    unless_changed_from: Option<&FileDigest>,
) -> Result<(), KeepVersionError> {
    keep_version_interleaved(path, index, digest, kept, unless_changed_from, || {})
}

pub(crate) fn keep_version_interleaved(
    path: &Path,
    index: usize,
    digest: &VersionDigest,
    kept: bool,
    unless_changed_from: Option<&FileDigest>,
    while_rewriting: impl FnOnce(),
) -> Result<(), KeepVersionError> {
    let target = resolve_links(path).map_err(|error| KeepVersionError::writing(&error))?;

    ensure_replaceable(&target).map_err(|error| KeepVersionError::writing(&error))?;

    let previous = Previous::open(&target).map_err(|error| {
        log::warn!(
            "could not read the versions in {}: {error}",
            target.display()
        );
        KeepVersionError::Unreadable {
            name: target
                .file_name()
                .unwrap_or(target.as_os_str())
                .display()
                .to_string(),
            failure: ReadFailure::of(&error),
        }
    })?;

    if let Some(expected) = unless_changed_from
        && binary::head_digest(&previous.bytes).as_ref() != Some(expected)
    {
        return Err(KeepVersionError::ChangedOnDisk);
    }

    let rewritten = binary::with_version_kept(&previous.bytes, index, digest, kept)
        .map_err(KeepVersionError::rewriting)?;

    while_rewriting();
    replace_checked(
        &target,
        |file| file.write_all(&rewritten.bytes),
        |file| {
            check_reads_back(file, &rewritten.digest, &[])?;
            previous.watched.ensure_still_at(&target)
        },
    )
    .map_err(|error| {
        if changed_meanwhile(&error) {
            KeepVersionError::ChangedOnDisk
        } else {
            KeepVersionError::writing(&error)
        }
    })
}

#[derive(Debug, thiserror::Error)]
#[error("the file was changed by another program while it was being saved")]
struct ChangedMeanwhile;

fn changed_meanwhile(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.is::<ChangedMeanwhile>())
}

fn outside_change() -> io::Error {
    io::Error::other(ChangedMeanwhile)
}

pub fn encode(document: &Document) -> Result<Vec<u8>, SaveError> {
    binary::encode(document).map_err(|error| SaveError::encoding(&error))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SaveOptions<'a> {
    pub keep_original: bool,
    pub history_from: Option<&'a Path>,
    pub label: Option<&'a str>,
    pub unless_changed_from: Option<&'a FileDigest>,
    pub retention: Retention,
    pub preview: Option<&'a PreviewSlot>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saved {
    pub backup: Option<PathBuf>,
    pub dropped_for_size: usize,
    pub digest: FileDigest,
}

pub fn save(document: &Document, path: &Path, keep_original: bool) -> Result<Saved, SaveError> {
    save_with(
        document,
        path,
        &SaveOptions {
            keep_original,
            history_from: Some(path),
            label: None,
            unless_changed_from: None,
            retention: Retention::Standard,
            preview: None,
        },
    )
}

pub fn save_with(
    document: &Document,
    path: &Path,
    options: &SaveOptions<'_>,
) -> Result<Saved, SaveError> {
    save_interleaved(document, path, options, || {})
}

pub(crate) fn save_interleaved(
    document: &Document,
    path: &Path,
    options: &SaveOptions<'_>,
    while_encoding: impl FnOnce(),
) -> Result<Saved, SaveError> {
    let target = resolve_links(path).map_err(|error| SaveError::writing(&error))?;

    ensure_replaceable(&target).map_err(|error| SaveError::writing(&error))?;

    let previous = options
        .history_from
        .map(read_previous)
        .transpose()?
        .flatten();
    let guarded = options
        .unless_changed_from
        .filter(|_| reads_target(options, &target));

    if let (Some(expected), Some(previous)) = (guarded, &previous)
        && binary::head_digest(&previous.bytes).as_ref() != Some(expected)
    {
        return Err(SaveError::ChangedOnDisk);
    }

    let encoded = binary::encode_keeping(
        document,
        previous.as_ref().map(|previous| previous.bytes.as_slice()),
        (SystemTime::now(), options.retention),
        EncodeExtras {
            label: options.label,
            preview: options.preview,
        },
    )
    .map_err(|error| SaveError::encoding(&error))?;
    let mut previous = previous.map(Previous::into_watched);
    let Encoded {
        bytes,
        shared,
        digest,
        previous_damaged,
        dropped_for_size,
        fresh_versions,
    } = encoded;

    while_encoding();

    let ensure_unchanged = |previous: Option<&Watched>| match guarded {
        Some(_) => ensure_unchanged_since_read(previous, &target),
        None => Ok(()),
    };
    let damaged_in_place = previous_damaged && reads_target(options, &target);

    let backup = if (options.keep_original || damaged_in_place) && target.exists() {
        ensure_unchanged(previous.as_ref()).map_err(|_| SaveError::ChangedOnDisk)?;

        let untouched = previous
            .as_ref()
            .is_some_and(|previous| !previous.changed_since_read());

        let backup = keep_backup(&target).map_err(|error| SaveError::writing(&error))?;

        if untouched && let Some(previous) = &mut previous {
            previous.restamp();
        }
        Some(backup)
    } else {
        None
    };
    let reads_back = |file: &File| {
        check_reads_back(file, &digest, &fresh_versions)?;
        ensure_unchanged(previous.as_ref())
    };
    let written = match &previous {
        Some(source) if !shared.is_empty() => replace_checked(
            &target,
            |file| write_sharing(file, bytes, &shared, source),
            reads_back,
        ),
        _ => replace_checked(&target, |file| write_whole(file, bytes), reads_back),
    };

    written.map_err(|error| {
        if changed_meanwhile(&error) {
            SaveError::ChangedOnDisk
        } else {
            SaveError::writing(&error)
        }
    })?;
    Ok(Saved {
        backup,
        dropped_for_size,
        digest,
    })
}

fn reads_target(options: &SaveOptions<'_>, target: &Path) -> bool {
    options
        .history_from
        .is_some_and(|from| same_file(from, target))
}

fn check_reads_back(file: &File, digest: &FileDigest, fresh: &[FreshVersion]) -> io::Result<()> {
    let length = usize::try_from(file.metadata()?.len()).map_err(io::Error::other)?;
    let mut written = zeroed(length)?;

    os::read_exact_at(file, &mut written, 0)?;
    binary::reads_back(&written, digest, fresh).map_err(|error| {
        log::error!("a saved model did not decode back to what was written: {error}");
        io::Error::new(io::ErrorKind::InvalidData, error)
    })
}

fn zeroed(length: usize) -> io::Result<Vec<u8>> {
    let mut buffer = Vec::new();

    buffer
        .try_reserve_exact(length)
        .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
    buffer.resize(length, 0);
    Ok(buffer)
}

struct Previous {
    bytes: Vec<u8>,
    watched: Watched,
}

struct Watched {
    file: File,
    stamp: ChangeStamp,
    key: FileKey,
}

impl Previous {
    fn open(path: &Path) -> io::Result<Self> {
        let file = open_file(path)?;
        let stamp = ChangeStamp::of(&file.metadata()?);
        let key = FileKey::of(&file)?;
        let bytes = read_open(&file)?;
        Ok(Self {
            bytes,
            watched: Watched { file, stamp, key },
        })
    }

    fn into_watched(self) -> Watched {
        self.watched
    }
}

impl Watched {
    fn changed_since_read(&self) -> bool {
        self.file
            .metadata()
            .map_or(true, |metadata| ChangeStamp::of(&metadata) != self.stamp)
    }

    fn restamp(&mut self) {
        if let Ok(metadata) = self.file.metadata() {
            self.stamp = ChangeStamp::of(&metadata);
        }
    }

    fn ensure_still_at(&self, path: &Path) -> io::Result<()> {
        let same_file = FileKey::of_link(path).is_ok_and(|key| key == self.key);
        if same_file && !self.changed_since_read() {
            Ok(())
        } else {
            Err(outside_change())
        }
    }
}

fn ensure_unchanged_since_read(previous: Option<&Watched>, target: &Path) -> io::Result<()> {
    match previous {
        Some(previous) => previous.ensure_still_at(target),
        None if fs::symlink_metadata(target).is_ok() => Err(outside_change()),
        None => Ok(()),
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
            Err(SaveError::EarlierVersionsUnreadable {
                name: name.to_string(),
                failure: ReadFailure::of(&error),
            })
        }
    }
}

fn write_whole(file: &mut File, bytes: Vec<u8>) -> io::Result<()> {
    file.write_all(&bytes)
}

fn write_sharing(
    file: &mut File,
    bytes: Vec<u8>,
    shared: &[Shared],
    source: &Watched,
) -> io::Result<()> {
    let bytes = bytes.as_slice();
    let mut cloned = Vec::new();

    for range in shared {
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

    for range in shared {
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
    os::clone_range(
        source,
        target,
        range.from as u64,
        range.at as u64,
        range.length,
    )
}

fn write_range(file: &File, bytes: &[u8], start: usize, end: usize) -> io::Result<()> {
    let range = bytes.get(start..end).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "a shared range lies outside the saved content",
        )
    })?;
    os::write_all_at(file, range, start as u64)
}

pub fn write_atomically(path: &Path, contents: &[u8]) -> io::Result<()> {
    replace_atomically(path, |file| file.write_all(contents))
}

pub fn write_privately(path: &Path, contents: &[u8]) -> io::Result<()> {
    replace_checked_as(
        path,
        Privacy::OwnerOnly,
        |file| file.write_all(contents),
        |_| Ok(()),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Privacy {
    AsTheTargetWas,
    OwnerOnly,
}

pub(crate) fn replace_atomically(
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
    replace_checked_as(path, Privacy::AsTheTargetWas, fill, check)
}

type Move = fn(&Path, &Path) -> io::Result<()>;

#[derive(Clone, Copy)]
struct Moves {
    replace: Move,
    rescue: Move,
}

const SYSTEM_MOVES: Moves = Moves {
    replace: os::replace,
    rescue: put_in_place,
};

#[derive(Debug, thiserror::Error)]
#[error("the saved copy could not take the file's place and is kept as {}", path.display())]
pub(crate) struct KeptAside {
    path: PathBuf,
}

pub(crate) fn kept_aside(error: &io::Error) -> Option<&Path> {
    error
        .get_ref()?
        .downcast_ref::<KeptAside>()
        .map(|kept| kept.path.as_path())
}

fn replace_checked_as(
    path: &Path,
    privacy: Privacy,
    fill: impl FnOnce(&mut File) -> io::Result<()>,
    check: impl FnOnce(&File) -> io::Result<()>,
) -> io::Result<()> {
    replace_checked_with(path, privacy, SYSTEM_MOVES, fill, check)
}

fn replace_checked_with(
    path: &Path,
    privacy: Privacy,
    moves: Moves,
    fill: impl FnOnce(&mut File) -> io::Result<()>,
    check: impl FnOnce(&File) -> io::Result<()>,
) -> io::Result<()> {
    let target = resolve_links(path)?;

    ensure_replaceable(&target)?;

    let temporary = temporary_sibling(&target)?;

    if let Err(error) = write_and_sync(&temporary, &target, privacy, fill, check) {
        let _ = fs::remove_file(&temporary);

        return Err(error);
    }
    (moves.replace)(&temporary, &target)
        .or_else(|error| after_failed_replace(&temporary, &target, error, moves.rescue))?;

    if let Err(error) = sync_parent(&target) {
        log::warn!(
            "saved {}, but its folder could not be flushed to disk: {error}",
            target.display()
        );
    }
    remove_orphaned_temporaries(&target);
    Ok(())
}

fn after_failed_replace(
    temporary: &Path,
    target: &Path,
    error: io::Error,
    rescue: Move,
) -> io::Result<()> {
    match fs::symlink_metadata(target) {
        Ok(_) => {
            let _ = fs::remove_file(temporary);
            Err(error)
        }
        Err(_) if fs::symlink_metadata(temporary).is_err() => Err(error),
        Err(state) => {
            log::warn!(
                "{} is not there after a failed replace ({error}), so the saved copy is put in \
                 its place",
                target.display()
            );

            let rescued = match state.kind() {
                io::ErrorKind::NotFound => rescue(temporary, target),
                _ => Err(state),
            };
            rescued.map_err(|rescue_error| {
                let kept = keep_under_kept_name(temporary, target);

                log::error!(
                    "the saved copy could not take the place of {}, so it is kept as {}: \
                     {rescue_error}",
                    target.display(),
                    kept.display()
                );
                io::Error::new(error.kind(), KeptAside { path: kept })
            })
        }
    }
}

fn keep_under_kept_name(temporary: &Path, target: &Path) -> PathBuf {
    match move_aside(temporary, &marked_siblings(target, KEPT_MARKER)) {
        Ok(Some(kept)) => kept,
        Ok(None) => {
            log::warn!(
                "every name for keeping {} is taken, so it keeps its own",
                temporary.display()
            );
            temporary.to_path_buf()
        }
        Err(error) => {
            log::warn!(
                "could not give {} a name of its own: {error}",
                temporary.display()
            );
            temporary.to_path_buf()
        }
    }
}

fn move_aside(temporary: &Path, candidates: &[PathBuf]) -> io::Result<Option<PathBuf>> {
    match rename_to_first_free(temporary, candidates) {
        Ok(Some(kept)) => {
            if let Err(error) = sync_parent(&kept) {
                log::warn!(
                    "kept {}, but its folder could not be flushed to disk: {error}",
                    kept.display()
                );
            }
            Ok(Some(kept))
        }
        Ok(None) => Ok(None),
        Err(error) => {
            log::warn!("could not rename {}: {error}", temporary.display());

            let kept = keep_copy(temporary, candidates.to_vec())?;

            if kept.is_some() {
                remove_placed(temporary);
            }
            Ok(kept)
        }
    }
}

fn put_in_place(temporary: &Path, target: &Path) -> io::Result<()> {
    let mut attempt = 1;

    loop {
        match fs::rename(temporary, target) {
            Ok(()) => return Ok(()),
            Err(_) if attempt < RENAME_ATTEMPTS => {
                attempt += 1;
                thread::sleep(RENAME_PAUSE);
            }
            Err(error) => {
                log::warn!("could not rename {}: {error}", temporary.display());
                break;
            }
        }
    }
    match fs::hard_link(temporary, target) {
        Ok(()) => {
            remove_placed(temporary);
            return Ok(());
        }
        Err(error) => log::warn!("could not link {}: {error}", temporary.display()),
    }
    copy_into_place(temporary, target)
}

fn copy_into_place(temporary: &Path, target: &Path) -> io::Result<()> {
    let mut source = File::open(temporary)?;
    let mut placed = File::create_new(target)?;

    let copied = io::copy(&mut source, &mut placed).and_then(|_| placed.sync_all());

    drop(placed);

    if copied.is_err() {
        let _ = fs::remove_file(target);
    }
    copied?;
    remove_placed(temporary);
    Ok(())
}

fn remove_placed(temporary: &Path) {
    if let Err(error) = fs::remove_file(temporary) {
        log::debug!("could not remove {}: {error}", temporary.display());
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the file is read-only")]
pub(crate) struct ReadOnly;

fn ensure_replaceable(target: &Path) -> io::Result<()> {
    match fs::metadata(target) {
        Ok(metadata) => {
            ensure_regular(&metadata)?;
            if os::writable(target, &metadata) {
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
        match fs::symlink_metadata(&resolved) {
            Ok(metadata) if metadata.file_type().is_symlink() => {}
            Ok(_) => return Ok(resolved),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(resolved),
            Err(error) => return Err(error),
        }

        let target = fs::read_link(&resolved)?;

        resolved = match resolved.parent() {
            Some(parent) => parent.join(target),
            None => target,
        };
    }
    Err(io::Error::other(
        "the path goes through too many symbolic links",
    ))
}

pub fn same_file(first: &Path, second: &Path) -> bool {
    match (identity(first), identity(second)) {
        (Ok(first), Ok(second)) => first == second,
        _ => same_place(first, second),
    }
}

fn identity(path: &Path) -> io::Result<FileKey> {
    FileKey::of_link(&resolve_links(path)?)
}

fn same_place(first: &Path, second: &Path) -> bool {
    if os::same_file_path(first, second) {
        return true;
    }

    let (Some(first_name), Some(second_name)) = (first.file_name(), second.file_name()) else {
        return false;
    };

    let names_match = os::same_file_path(Path::new(first_name), Path::new(second_name));

    names_match && same_file(containing_folder(first), containing_folder(second))
}

fn containing_folder(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
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
    let host = Host::current();

    if host.boot == UNKNOWN_TAG || !os::processes_known() {
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
    for entry in entries.filter_map(Result::ok) {
        let file_name = entry.file_name();
        let name = file_name.as_encoded_bytes();
        let named_as_wanted = prefix.is_none_or(|prefix| name.starts_with(prefix));

        let orphaned = named_as_wanted
            && temporary_tag(name).is_some_and(|tag| host.orphaned(tag, ours, os::process_running));
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

#[derive(Clone, Copy, Debug)]
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
    privacy: Privacy,
    fill: impl FnOnce(&mut File) -> io::Result<()>,
    check: impl FnOnce(&File) -> io::Result<()>,
) -> io::Result<()> {
    let mut file = match fs::metadata(target) {
        Ok(existing) => {
            let file = os::private(OpenOptions::new().read(true).write(true).create_new(true))
                .open(temporary)?;

            os::take_ownership_and_attributes(&file, target, &existing);
            if privacy == Privacy::AsTheTargetWas {
                file.set_permissions(existing.permissions())?;
            }
            file
        }
        Err(_) => {
            let mut options = OpenOptions::new();

            options.read(true).write(true).create_new(true);
            if privacy == Privacy::OwnerOnly {
                os::private(&mut options);
            }
            options.open(temporary)?
        }
    };

    fill(&mut file)?;
    file.sync_all()?;
    check(&file)
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
    TAG.get_or_init(|| tag_from(os::boot_ids()))
}

pub(crate) fn machine_tag() -> &'static str {
    static TAG: OnceLock<String> = OnceLock::new();
    TAG.get_or_init(|| tag_from(os::machine_ids()))
}

fn tag_from(ids: Vec<String>) -> String {
    ids.iter()
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
    os::sync_dir(parent)
}

fn keep_backup(path: &Path) -> io::Result<PathBuf> {
    keep_copy(path, marked_siblings(path, BACKUP_MARKER))?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "too many backups of this file already exist",
        )
    })
}

fn marked_siblings(path: &Path, marker: &str) -> Vec<PathBuf> {
    let extension = path.extension().map(|extension| extension.to_os_string());
    let extension_room = extension
        .as_ref()
        .map_or(0, |extension| extension.len() + 1);
    let stem = fitting(
        path.file_stem().unwrap_or_default(),
        MAX_NAME_BYTES.saturating_sub(extension_room + BACKUP_ROOM),
    );
    (1..=MAX_BACKUP_ATTEMPTS)
        .map(|attempt| {
            let mut name = stem.clone();

            name.push(match attempt {
                1 => format!(".{marker}"),
                _ => format!(".{marker}-{attempt}"),
            });
            if let Some(extension) = &extension {
                name.push(".");
                name.push(extension);
            }
            path.with_file_name(name)
        })
        .collect()
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
    OpenOptions::new().write(true).open(temporary)?.sync_all()?;

    let placed = rename_to_first_free(temporary, candidates)?;

    if let Some(placed) = &placed {
        sync_parent(placed)?;
    }
    Ok(placed)
}

fn rename_to_first_free(source: &Path, candidates: &[PathBuf]) -> io::Result<Option<PathBuf>> {
    for candidate in candidates {
        match File::create_new(candidate) {
            Ok(placeholder) => {
                drop(placeholder);
                if let Err(error) = fs::rename(source, candidate) {
                    let _ = fs::remove_file(candidate);

                    return Err(error);
                }
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
            digest: FileDigest(String::new()),
            previous_damaged: false,
            dropped_for_size: 0,
            fresh_versions: Vec::new(),
        }
    }

    fn folder_contents(dir: &TempDir) -> Vec<OsString> {
        let mut names: Vec<OsString> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();

        names.sort();
        names
    }

    #[cfg(unix)]
    #[test]
    fn a_private_write_makes_a_new_file_and_replaces_an_old_one_readable_by_its_owner_alone() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::TempDir::new().unwrap();
        let target = dir.path().join("secret.json");
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;

        write_privately(&target, b"first").unwrap();

        assert_eq!(mode(&target), 0o600);

        fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
        write_privately(&target, b"second").unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"second");
        assert_eq!(mode(&target), 0o600);

        write_atomically(&dir.path().join("plain.txt"), b"x").unwrap();

        assert_ne!(mode(&dir.path().join("plain.txt")) & 0o044, 0);
    }

    #[test]
    fn a_save_failing_halfway_through_writing_leaves_the_file_as_it_was() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("model.caditor");
        let original = pattern(3 * BLOCK + 5, 1);

        fs::write(&target, &original).unwrap();

        let replacement = pattern(5 * BLOCK, 2);

        let error = replace_checked(
            &target,
            |file| {
                file.write_all(&replacement[..replacement.len() / 2])?;
                Err(io::Error::new(io::ErrorKind::StorageFull, "no space left"))
            },
            |_| Ok(()),
        )
        .unwrap_err();

        assert_eq!(WriteFailure::of(&error), WriteFailure::DiskFull);
        assert_eq!(fs::read(&target).unwrap(), original);
        assert_eq!(folder_contents(&dir), [OsString::from("model.caditor")]);
    }

    #[test]
    fn a_save_that_does_not_read_back_leaves_the_file_as_it_was() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("model.caditor");
        let original = pattern(2 * BLOCK + 9, 4);

        fs::write(&target, &original).unwrap();

        let replacement = pattern(4 * BLOCK, 6);

        let error = replace_checked(
            &target,
            |file| file.write_all(&replacement),
            |_| {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    binary::NotReadBack::Model,
                ))
            },
        )
        .unwrap_err();

        assert_eq!(WriteFailure::of(&error), WriteFailure::NotReadBack);
        assert_eq!(fs::read(&target).unwrap(), original);
        assert_eq!(folder_contents(&dir), [OsString::from("model.caditor")]);
    }

    const UNABLE_TO_MOVE_REPLACEMENT: i32 = 1176;

    fn replace_losing_the_target(_temporary: &Path, target: &Path) -> io::Result<()> {
        fs::remove_file(target)?;
        Err(io::Error::from_raw_os_error(UNABLE_TO_MOVE_REPLACEMENT))
    }

    fn refused(_temporary: &Path, _target: &Path) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::ResourceBusy))
    }

    fn replace_with(target: &Path, moves: Moves, contents: &[u8]) -> io::Result<()> {
        replace_checked_with(
            target,
            Privacy::AsTheTargetWas,
            moves,
            |file| file.write_all(contents),
            |_| Ok(()),
        )
    }

    #[test]
    fn a_replace_that_loses_the_target_puts_the_saved_copy_in_its_place() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("model.caditor");

        fs::write(&target, "old").unwrap();

        let moves = Moves {
            replace: replace_losing_the_target,
            ..SYSTEM_MOVES
        };

        replace_with(&target, moves, b"new").unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert_eq!(folder_contents(&dir), [OsString::from("model.caditor")]);
    }

    #[test]
    fn a_saved_copy_that_cannot_take_the_lost_targets_place_is_kept_and_named() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("model.caditor");

        fs::write(&target, "old").unwrap();

        let moves = Moves {
            replace: replace_losing_the_target,
            rescue: refused,
        };

        let error = replace_with(&target, moves, b"new").unwrap_err();
        let kept = kept_aside(&error).unwrap().to_path_buf();

        assert_eq!(fs::read(&kept).unwrap(), b"new");
        assert_eq!(kept, dir.path().join("model.kept.caditor"));
        assert!(!target.exists());
        assert_eq!(folder_contents(&dir), [kept.file_name().unwrap()]);
        assert_eq!(WriteFailure::of(&error), WriteFailure::KeptAside);
        assert_eq!(
            SaveError::writing(&error),
            SaveError::KeptAside { kept: kept.clone() }
        );
        assert_eq!(
            KeepVersionError::writing(&error),
            KeepVersionError::KeptAside { kept: kept.clone() }
        );
        assert!(
            SaveError::writing(&error)
                .to_string()
                .contains(&*kept.file_name().unwrap().to_string_lossy())
        );
    }

    fn with_length(text: &str) -> Document {
        let mut document = Document::default();

        let mut transaction = document.transaction("Length");

        transaction.add_parameter("length", transaction.parse(text).unwrap());
        document.apply(transaction.finish()).unwrap();
        document
    }

    #[test]
    fn a_copy_kept_aside_survives_later_saves_and_opens_with_its_versions() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("model.caditor");
        let earlier = with_length("10 mm");
        let later = with_length("20 mm");

        save(&earlier, &target, false).unwrap();
        save(&later, &target, false).unwrap();

        let with_versions = fs::read(&target).unwrap();
        let moves = Moves {
            replace: replace_losing_the_target,
            rescue: refused,
        };
        let orphan = dir.path().join(format!(
            ".model.caditor.{}-{}-4294967295-3.tmp",
            machine_tag(),
            boot_tag()
        ));

        let first = replace_with(&target, moves, &with_versions).unwrap_err();

        save(&later, &target, false).unwrap();

        let second = replace_with(&target, moves, b"newer").unwrap_err();

        fs::write(&orphan, "partial").unwrap();
        save(&Document::default(), &target, false).unwrap();

        let kept = kept_aside(&first).unwrap().to_path_buf();
        let kept_again = kept_aside(&second).unwrap().to_path_buf();
        let reopened = crate::load(&kept).unwrap();
        let history = crate::history(&kept).unwrap();

        let versions: Vec<Document> = (0..history.versions.len())
            .map(|index| crate::load_version(&kept, index).unwrap().document)
            .collect();

        assert_eq!(kept, dir.path().join("model.kept.caditor"));
        assert_eq!(kept_again, dir.path().join("model.kept-2.caditor"));
        assert!(temporary_tag(kept.file_name().unwrap().as_encoded_bytes()).is_none());
        assert_eq!(reopened.document, later);
        assert!(versions.contains(&earlier));
        assert_eq!(fs::read(&kept_again).unwrap(), b"newer");
        assert_eq!(!orphan.exists(), boot_tag() != UNKNOWN_TAG);
        assert_eq!(
            folder_contents(&dir)
                .into_iter()
                .filter(|name| name != orphan.file_name().unwrap())
                .collect::<Vec<_>>(),
            [
                OsString::from("model.caditor"),
                OsString::from("model.kept-2.caditor"),
                OsString::from("model.kept.caditor"),
            ]
        );
    }

    #[test]
    fn a_replace_refused_with_the_target_still_there_leaves_the_file_as_it_was() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("model.caditor");

        fs::write(&target, "old").unwrap();

        let moves = Moves {
            replace: refused,
            rescue: refused,
        };

        let error = replace_with(&target, moves, b"new").unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::ResourceBusy);
        assert!(kept_aside(&error).is_none());
        assert_eq!(fs::read(&target).unwrap(), b"old");
        assert_eq!(folder_contents(&dir), [OsString::from("model.caditor")]);
    }

    #[test]
    fn a_copy_into_place_moves_the_contents_and_never_overwrites() {
        let dir = TempDir::new().unwrap();
        let temporary = dir.path().join(".model.caditor.tag.tmp");
        let target = dir.path().join("model.caditor");

        fs::write(&temporary, "new").unwrap();

        copy_into_place(&temporary, &target).unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert!(!temporary.exists());

        fs::write(&temporary, "newer").unwrap();

        assert!(copy_into_place(&temporary, &target).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert_eq!(fs::read(&temporary).unwrap(), b"newer");
    }

    #[test]
    fn running_out_of_memory_fails_the_save_in_words_saying_what_to_do() {
        let read_back = zeroed(usize::MAX).unwrap_err();
        let packing = EncodeError::Pack(PackError::OutOfMemory);

        assert_eq!(read_back.kind(), io::ErrorKind::OutOfMemory);
        assert_eq!(SaveError::writing(&read_back), SaveError::OutOfMemory);
        assert_eq!(SaveError::encoding(&packing), SaveError::OutOfMemory);
        assert!(
            SaveError::OutOfMemory
                .to_string()
                .contains("close other programs and save again")
        );
        assert_eq!(zeroed(2).unwrap(), [0, 0]);
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

        replace_atomically(&target, |file| {
            write_sharing(
                file,
                encoded.bytes.clone(),
                &encoded.shared,
                &previous.watched,
            )
        })
        .unwrap();

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

        assert!(previous.watched.changed_since_read());

        replace_atomically(&target, |file| {
            write_sharing(
                file,
                encoded.bytes.clone(),
                &encoded.shared,
                &previous.watched,
            )
        })
        .unwrap();

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
