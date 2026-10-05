use std::{
    fs,
    fs::{File, OpenOptions, TryLockError},
    io,
    path::Path,
    thread,
    time::{Duration, Instant},
};

use crate::{
    os::{self, FileKey},
    read::ensure_regular,
};

const UPDATE_LOCK_WAIT: Duration = Duration::from_secs(3);
const UPDATE_LOCK_POLL: Duration = Duration::from_millis(5);

pub(crate) enum Location {
    Gone,
    Replaced,
    Here,
}

pub(crate) fn location(file: &File, path: &Path) -> io::Result<Location> {
    let locked = FileKey::of(file)?;
    let current = match FileKey::of_link(path) {
        Ok(current) => current,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Location::Gone),
        Err(error) => return Err(error),
    };
    if locked == current {
        Ok(Location::Here)
    } else {
        Ok(Location::Replaced)
    }
}

pub(crate) fn holds(file: &File, path: &Path) -> bool {
    matches!(location(file, path), Ok(Location::Here))
}

pub(crate) fn lock_existing(path: &Path) -> io::Result<Option<File>> {
    ensure_regular(&fs::metadata(path)?)?;
    let writable = OpenOptions::new().read(true).write(true).open(path);
    let (file, exclusive) = match writable {
        Ok(file) => (file, true),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem
            ) =>
        {
            (File::open(path)?, false)
        }
        Err(error) => return Err(error),
    };
    let locked = if exclusive {
        file.try_lock()
    } else {
        file.try_lock_shared()
    };
    match locked {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(error)) => Err(error),
    }
}

pub(crate) fn locked_elsewhere(path: &Path) -> bool {
    matches!(lock_existing(path), Ok(None))
}

pub(crate) fn in_use() -> io::Error {
    io::Error::new(
        io::ErrorKind::ResourceBusy,
        "the recovery file is in use by another caditor window",
    )
}

pub(crate) fn install(temporary: &Path, path: &Path, own: Option<&File>) -> io::Result<()> {
    if own.is_some_and(|own| holds(own, path)) {
        return fs::rename(temporary, path);
    }
    match fs::hard_link(temporary, path) {
        Ok(()) => {
            if let Err(error) = fs::remove_file(temporary) {
                log::warn!("could not remove {}: {error}", temporary.display());
            }
            return Ok(());
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            log::debug!("could not link {}: {error}", path.display());
            if fs::symlink_metadata(path).is_err() {
                return fs::rename(temporary, path);
            }
        }
    }
    let existing = lock_existing(path)?.ok_or_else(in_use)?;
    if !holds(&existing, path) {
        return Err(in_use());
    }
    fs::rename(temporary, path)
}

pub(crate) fn remove_held(file: &File, path: &Path) {
    if !holds(file, path) {
        return;
    }
    remove(path);
}

pub(crate) fn remove_unheld(path: &Path) {
    match lock_existing(path) {
        Ok(Some(file)) if holds(&file, path) => remove(path),
        Ok(_) => log::warn!("{} is in use and was left in place", path.display()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => log::warn!("could not lock {}: {error}", path.display()),
    }
}

fn remove(path: &Path) {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => log::warn!("could not remove {}: {error}", path.display()),
    }
}

pub(crate) fn locked_update<T>(
    dir: &Path,
    lock_name: &str,
    update: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    fs::create_dir_all(dir)?;
    let lock_path = dir.join(lock_name);
    let _held = hold_update_lock(&lock_path);
    update()
}

fn hold_update_lock(path: &Path) -> Option<File> {
    let file = match os::private(
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false),
    )
    .open(path)
    {
        Ok(file) => file,
        Err(error) => {
            log::warn!("could not open {}: {error}", path.display());
            return None;
        }
    };
    let giving_up = Instant::now() + UPDATE_LOCK_WAIT;
    loop {
        match file.try_lock() {
            Ok(()) => return Some(file),
            Err(TryLockError::WouldBlock) if Instant::now() < giving_up => {
                thread::sleep(UPDATE_LOCK_POLL);
            }
            Err(TryLockError::WouldBlock) => {
                log::warn!("{} stayed locked, so it is updated anyway", path.display());
                return None;
            }
            Err(TryLockError::Error(error)) => {
                log::warn!("could not lock {}: {error}", path.display());
                return None;
            }
        }
    }
}
