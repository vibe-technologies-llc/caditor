use std::{
    fs::{self, File, Metadata},
    io::{self, Read},
    path::Path,
};

pub const MAX_FILE_SIZE: u64 = 2 << 30;

#[derive(Debug, thiserror::Error)]
#[error("it is a device, pipe or socket, not a file")]
pub(crate) struct NotAFile;

#[derive(Debug, thiserror::Error)]
#[error("it is larger than the {} GiB caditor reads", MAX_FILE_SIZE >> 30)]
pub(crate) struct TooLarge;

pub(crate) fn read_file(path: &Path) -> io::Result<Vec<u8>> {
    read_open(&open_file(path)?)
}

pub(crate) fn open_file(path: &Path) -> io::Result<File> {
    ensure_regular(&fs::metadata(path)?)?;
    File::open(path)
}

pub(crate) fn read_open(file: &File) -> io::Result<Vec<u8>> {
    let metadata = file.metadata()?;
    ensure_regular(&metadata)?;
    if metadata.len() > MAX_FILE_SIZE {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    let expected = usize::try_from(metadata.len()).map_err(|_| too_large())?;
    bytes
        .try_reserve_exact(expected)
        .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
    file.take(MAX_FILE_SIZE.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).map_or(true, |length| length > MAX_FILE_SIZE) {
        return Err(too_large());
    }
    Ok(bytes)
}

pub(crate) fn ensure_regular(metadata: &Metadata) -> io::Result<()> {
    if metadata.is_file() {
        Ok(())
    } else if metadata.is_dir() {
        Err(io::Error::from(io::ErrorKind::IsADirectory))
    } else {
        Err(io::Error::new(io::ErrorKind::InvalidInput, NotAFile))
    }
}

fn too_large() -> io::Error {
    io::Error::new(io::ErrorKind::FileTooLarge, TooLarge)
}
