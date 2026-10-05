use std::{
    ffi::{OsStr, OsString},
    fs::{self, File, Metadata, OpenOptions, Permissions},
    io,
    os::windows::{
        ffi::OsStringExt,
        fs::{FileExt, MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
};

use caditor_windows::{FileId, HIDDEN_ATTRIBUTE};

const REPLACEMENT_CHARACTER: u16 = 0xfffd;

pub(crate) fn state_base() -> Option<PathBuf> {
    known_folder("LOCALAPPDATA")
}

pub(crate) fn config_base() -> Option<PathBuf> {
    known_folder("APPDATA")
}

fn known_folder(variable: &str) -> Option<PathBuf> {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

pub(crate) fn private(options: &mut OpenOptions) -> &mut OpenOptions {
    options
}

pub(crate) fn hidden(options: &mut OpenOptions) -> &mut OpenOptions {
    options.attributes(HIDDEN_ATTRIBUTE)
}

pub(crate) fn read_exact_at(file: &File, buffer: &mut [u8], offset: u64) -> io::Result<()> {
    let mut done = 0;
    while let Some(rest) = buffer.get_mut(done..).filter(|rest| !rest.is_empty()) {
        match file.seek_read(rest, offset + done as u64) {
            Ok(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
            Ok(count) => done += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub(crate) fn write_all_at(file: &File, buffer: &[u8], offset: u64) -> io::Result<()> {
    let mut done = 0;
    while let Some(rest) = buffer.get(done..).filter(|rest| !rest.is_empty()) {
        match file.seek_write(rest, offset + done as u64) {
            Ok(0) => return Err(io::Error::from(io::ErrorKind::WriteZero)),
            Ok(count) => done += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub(crate) fn clone_range(
    _source: &File,
    _target: &File,
    _from: u64,
    _at: u64,
    _length: usize,
) -> usize {
    0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChangeStamp {
    length: u64,
    written: u64,
    created: u64,
}

impl ChangeStamp {
    pub(crate) fn of(metadata: &Metadata) -> Self {
        Self {
            length: metadata.file_size(),
            written: metadata.last_write_time(),
            created: metadata.creation_time(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FileKey(FileId);

impl FileKey {
    pub(crate) fn of(file: &File) -> io::Result<Self> {
        FileId::of(file).map(Self)
    }

    pub(crate) fn of_link(path: &Path) -> io::Result<Self> {
        FileId::of_path(path).map(Self)
    }
}

pub(crate) fn writable(_target: &Path, metadata: &Metadata) -> bool {
    !metadata.permissions().readonly()
}

pub(crate) fn replace(temporary: &Path, target: &Path) -> io::Result<()> {
    if fs::symlink_metadata(target).is_err() {
        return caditor_windows::move_file_durably(temporary, target);
    }
    match caditor_windows::replace_file(temporary, target) {
        Ok(()) => Ok(()),
        Err(error) if fs::symlink_metadata(temporary).is_ok() => {
            log::debug!(
                "could not replace {} keeping its attributes, so it is renamed over: {error}",
                target.display()
            );
            fs::rename(temporary, target)
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn sync_dir(_dir: &Path) -> io::Result<()> {
    Ok(())
}

pub(crate) fn inherited_permissions(_metadata: &Metadata) -> Option<Permissions> {
    None
}

pub(crate) fn take_ownership_and_attributes(_file: &File, _target: &Path, _existing: &Metadata) {}

pub(crate) fn processes_known() -> bool {
    true
}

pub(crate) fn process_running(process: u32) -> bool {
    caditor_windows::process_running(process)
}

pub(crate) fn machine_ids() -> Vec<String> {
    caditor_windows::machine_guid().into_iter().collect()
}

pub(crate) fn boot_ids() -> Vec<String> {
    caditor_windows::boot_id()
        .map(|boot| format!("{boot:016x}"))
        .into_iter()
        .collect()
}

pub(crate) fn path_bytes(path: &OsStr) -> Vec<u8> {
    path.as_encoded_bytes().to_vec()
}

pub(crate) fn path_from_bytes(bytes: Vec<u8>) -> OsString {
    match String::from_utf8(bytes) {
        Ok(text) => OsString::from(text),
        Err(error) => OsString::from_wide(&wide_from_wtf8(error.as_bytes())),
    }
}

pub(crate) fn name_prefix(name: &OsStr, room: usize) -> OsString {
    let text = name.to_string_lossy();
    let cut = text.floor_char_boundary(room);
    OsString::from(text.get(..cut).unwrap_or_default())
}

fn wide_from_wtf8(bytes: &[u8]) -> Vec<u16> {
    let mut units = Vec::with_capacity(bytes.len());
    let mut rest = bytes;
    while let Some((&lead, tail)) = rest.split_first() {
        let (length, initial) = match lead {
            0x00..=0x7f => (0, u32::from(lead)),
            0xc2..=0xdf => (1, u32::from(lead & 0x1f)),
            0xe0..=0xef => (2, u32::from(lead & 0x0f)),
            0xf0..=0xf4 => (3, u32::from(lead & 0x07)),
            _ => {
                units.push(REPLACEMENT_CHARACTER);
                rest = tail;
                continue;
            }
        };
        let continuation = tail
            .get(..length)
            .filter(|bytes| bytes.iter().all(|byte| byte & 0xc0 == 0x80));
        let Some(continuation) = continuation else {
            units.push(REPLACEMENT_CHARACTER);
            rest = tail;
            continue;
        };
        let point = continuation
            .iter()
            .fold(initial, |point, byte| (point << 6) | u32::from(byte & 0x3f));
        let shortest = match length {
            0 => true,
            1 => point >= 0x80,
            2 => point >= 0x800,
            _ => (0x1_0000..=0x10_ffff).contains(&point),
        };
        if shortest {
            push_point(&mut units, point);
        } else {
            units.push(REPLACEMENT_CHARACTER);
        }
        rest = tail.get(length..).unwrap_or_default();
    }
    units
}

fn push_point(units: &mut Vec<u16>, point: u32) {
    match u16::try_from(point) {
        Ok(unit) => units.push(unit),
        Err(_) => {
            let offset = point - 0x1_0000;
            units.extend([
                0xd800 | (offset >> 10) as u16,
                0xdc00 | (offset & 0x3ff) as u16,
            ]);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::windows::ffi::OsStrExt;

    use super::*;

    #[test]
    fn a_name_with_an_unpaired_surrogate_survives_a_round_trip_through_bytes() {
        let units = [0x0063, 0x0061, 0xd800, 0x0066, 0xd83d, 0xde00, 0x00e9];
        let name = OsString::from_wide(&units);

        let back = path_from_bytes(path_bytes(&name));

        assert_eq!(back.encode_wide().collect::<Vec<_>>(), units);
        assert_eq!(path_from_bytes(b"plain.caditor".to_vec()), "plain.caditor");
    }

    #[test]
    fn bytes_that_are_not_wtf8_become_replacement_characters() {
        let back = path_from_bytes(vec![b'a', 0xff, 0xc3]);

        assert_eq!(
            back.encode_wide().collect::<Vec<_>>(),
            [0x61, REPLACEMENT_CHARACTER, REPLACEMENT_CHARACTER]
        );
    }
}
