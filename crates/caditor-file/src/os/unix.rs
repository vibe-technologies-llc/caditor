use std::{
    ffi::{OsStr, OsString},
    fs::{self, File, Metadata, OpenOptions, Permissions},
    io,
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::{FileExt, MetadataExt, OpenOptionsExt, fchown},
    },
    path::{Path, PathBuf},
};

use rustix::io::Errno;
use xattr::FileExt as _;

const PRIVATE_MODE: u32 = 0o600;
const PROCESSES: &str = "/proc";
const BOOT_ID: &str = "/proc/sys/kernel/random/boot_id";
const MACHINE_IDS: [&str; 2] = ["/etc/machine-id", "/var/lib/dbus/machine-id"];

pub(crate) fn state_base() -> Option<PathBuf> {
    base_dir("XDG_STATE_HOME", &[".local", "state"])
}

pub(crate) fn config_base() -> Option<PathBuf> {
    base_dir("XDG_CONFIG_HOME", &[".config"])
}

fn base_dir(variable: &str, under_home: &[&str]) -> Option<PathBuf> {
    let from_xdg = std::env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute());
    let from_home = || {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .map(|home| under_home.iter().fold(home, |path, part| path.join(part)))
    };
    from_xdg.or_else(from_home)
}

pub(crate) fn private(options: &mut OpenOptions) -> &mut OpenOptions {
    options.mode(PRIVATE_MODE)
}

pub(crate) fn hidden(options: &mut OpenOptions) -> &mut OpenOptions {
    options
}

pub(crate) fn read_exact_at(file: &File, buffer: &mut [u8], offset: u64) -> io::Result<()> {
    file.read_exact_at(buffer, offset)
}

pub(crate) fn write_all_at(file: &File, buffer: &[u8], offset: u64) -> io::Result<()> {
    file.write_all_at(buffer, offset)
}

pub(crate) fn clone_range(
    source: &File,
    target: &File,
    from: u64,
    at: u64,
    length: usize,
) -> usize {
    let (mut from, mut at) = (from, at);
    let mut copied = 0;
    while copied < length {
        match rustix::fs::copy_file_range(
            source,
            Some(&mut from),
            target,
            Some(&mut at),
            length - copied,
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
    copied.min(length)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChangeStamp {
    length: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

impl ChangeStamp {
    pub(crate) fn of(metadata: &Metadata) -> Self {
        Self {
            length: metadata.size(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FileKey {
    device: u64,
    inode: u64,
}

impl FileKey {
    pub(crate) fn of(file: &File) -> io::Result<Self> {
        Ok(Self::from_metadata(&file.metadata()?))
    }

    pub(crate) fn of_link(path: &Path) -> io::Result<Self> {
        Ok(Self::from_metadata(&fs::symlink_metadata(path)?))
    }

    fn from_metadata(metadata: &Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
}

pub(crate) fn writable(target: &Path, metadata: &Metadata) -> bool {
    !metadata.permissions().readonly()
        && rustix::fs::access(target, rustix::fs::Access::WRITE_OK).is_ok()
}

pub(crate) fn replace(temporary: &Path, target: &Path) -> io::Result<()> {
    fs::rename(temporary, target)
}

pub(crate) fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

pub(crate) fn inherited_permissions(metadata: &Metadata) -> Option<Permissions> {
    Some(metadata.permissions())
}

pub(crate) fn take_ownership_and_attributes(file: &File, target: &Path, existing: &Metadata) {
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

pub(crate) fn processes_known() -> bool {
    Path::new(PROCESSES).join("self").exists()
}

pub(crate) fn process_running(process: u32) -> bool {
    Path::new(PROCESSES).join(process.to_string()).exists()
}

pub(crate) fn machine_ids() -> Vec<String> {
    MACHINE_IDS
        .iter()
        .filter_map(|source| fs::read_to_string(source).ok())
        .collect()
}

pub(crate) fn boot_ids() -> Vec<String> {
    fs::read_to_string(BOOT_ID).into_iter().collect()
}

pub(crate) fn same_file_path(first: &Path, second: &Path) -> bool {
    first == second
}

pub(crate) fn path_bytes(path: &OsStr) -> Vec<u8> {
    path.as_bytes().to_vec()
}

pub(crate) fn path_from_bytes(bytes: Vec<u8>) -> OsString {
    OsString::from_vec(bytes)
}

pub(crate) fn name_prefix(name: &OsStr, room: usize) -> OsString {
    let bytes = name.as_bytes();
    let cut = match name.to_str() {
        Some(text) => text.floor_char_boundary(room),
        None => room,
    };
    OsString::from_vec(bytes.get(..cut).unwrap_or_default().to_vec())
}
