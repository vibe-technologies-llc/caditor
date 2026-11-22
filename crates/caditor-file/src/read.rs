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

pub(crate) fn read_file_up_to(path: &Path, limit: usize) -> io::Result<Option<Vec<u8>>> {
    let file = open_file(path)?;
    let metadata = file.metadata()?;

    ensure_regular(&metadata)?;

    let limit = u64::try_from(limit).unwrap_or(u64::MAX);

    if metadata.len() > limit {
        return Ok(None);
    }

    let mut bytes = Vec::new();
    let expected = usize::try_from(metadata.len()).map_err(|_| too_large())?;

    bytes
        .try_reserve_exact(expected)
        .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
    Ok((u64::try_from(bytes.len()).is_ok_and(|length| length <= limit)).then_some(bytes))
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

#[cfg(test)]
mod tests {
    use std::{fs, io};

    use super::*;

    #[test]
    fn a_file_over_the_limit_is_refused() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("big");

        fs::write(&path, vec![7_u8; 8192]).unwrap();

        let read = read_file_up_to(&path, 4096).unwrap();

        assert_eq!(read, None);
    }

    #[test]
    fn a_file_within_the_limit_is_read_whole() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("small");

        fs::write(&path, b"abcd").unwrap();

        assert_eq!(read_file_up_to(&path, 4).unwrap(), Some(b"abcd".to_vec()));
        assert_eq!(read_file_up_to(&path, 3).unwrap(), None);
        assert_eq!(read_file_up_to(&path, 0).unwrap(), None);
    }

    #[test]
    fn a_missing_file_or_a_folder_is_still_an_error() {
        let folder = tempfile::tempdir().unwrap();

        let missing = read_file_up_to(&folder.path().join("none"), 10).unwrap_err();
        let directory = read_file_up_to(folder.path(), 10).unwrap_err();

        assert_eq!(missing.kind(), io::ErrorKind::NotFound);
        assert_eq!(directory.kind(), io::ErrorKind::IsADirectory);
    }
}
