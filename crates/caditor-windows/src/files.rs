use std::{
    fs::{File, OpenOptions},
    io,
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle},
    path::{self, Path},
    ptr,
};

use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_HIDDEN, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO,
    FileIdInfo, GetFileInformationByHandleEx, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    MoveFileExW, REPLACEFILE_IGNORE_MERGE_ERRORS, ReplaceFileW,
};

pub const HIDDEN_ATTRIBUTE: u32 = FILE_ATTRIBUTE_HIDDEN;

const VERBATIM: &str = r"\\?\";
const VERBATIM_UNC: &str = r"\\?\UNC\";
const UNC: &str = r"\\";
const DEVICE: &str = r"\\.\";
const NO_ACCESS: u32 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileId {
    volume: u64,
    file: [u8; 16],
}

impl FileId {
    pub fn of(file: &File) -> io::Result<Self> {
        let mut info = FILE_ID_INFO::default();
        let size = u32::try_from(size_of::<FILE_ID_INFO>()).map_err(io::Error::other)?;
        #[allow(unsafe_code)]
        let done = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileIdInfo,
                (&raw mut info).cast(),
                size,
            )
        };
        if done == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            volume: info.VolumeSerialNumber,
            file: info.FileId.Identifier,
        })
    }

    pub fn of_path(path: &Path) -> io::Result<Self> {
        Self::of(&open_for_identity(path)?)
    }
}

pub fn open_for_identity(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .access_mode(NO_ACCESS)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

pub fn replace_file(replacement: &Path, replaced: &Path) -> io::Result<()> {
    let replacement = verbatim(replacement)?;
    let replaced = verbatim(replaced)?;
    #[allow(unsafe_code)]
    let done = unsafe {
        ReplaceFileW(
            replaced.as_ptr(),
            replacement.as_ptr(),
            ptr::null(),
            REPLACEFILE_IGNORE_MERGE_ERRORS,
            ptr::null(),
            ptr::null(),
        )
    };
    if done == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn move_file_durably(from: &Path, to: &Path) -> io::Result<()> {
    let from = verbatim(from)?;
    let to = verbatim(to)?;
    #[allow(unsafe_code)]
    let done = unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if done == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn verbatim(path: &Path) -> io::Result<Vec<u16>> {
    let absolute = path::absolute(path)?;
    let units: Vec<u16> = absolute.as_os_str().encode_wide().collect();
    let verbatim = units_of(VERBATIM);
    let device = units_of(DEVICE);
    let unc = units_of(UNC);
    let mut prefixed = if units.starts_with(&verbatim) || units.starts_with(&device) {
        units
    } else if let Some(share) = units.strip_prefix(unc.as_slice()) {
        units_of(VERBATIM_UNC)
            .into_iter()
            .chain(share.iter().copied())
            .collect()
    } else {
        verbatim.into_iter().chain(units).collect()
    };
    prefixed.push(0);
    Ok(prefixed)
}

fn units_of(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

#[cfg(test)]
pub(crate) fn verbatim_for_tests(path: &Path) -> io::Result<String> {
    let mut units = verbatim(path)?;
    units.pop();
    String::from_utf16(&units).map_err(io::Error::other)
}

pub(crate) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}
