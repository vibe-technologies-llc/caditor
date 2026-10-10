use std::{fs::File, io, os::windows::io::AsRawHandle, ptr};

use windows_sys::Win32::System::{
    IO::DeviceIoControl,
    Ioctl::{
        DUPLICATE_EXTENTS_DATA, FSCTL_DUPLICATE_EXTENTS_TO_FILE, FSCTL_GET_INTEGRITY_INFORMATION,
        FSCTL_GET_INTEGRITY_INFORMATION_BUFFER,
    },
};

pub fn cluster_size(file: &File) -> io::Result<u64> {
    let mut information = FSCTL_GET_INTEGRITY_INFORMATION_BUFFER::default();
    let mut returned = 0u32;
    let size = u32::try_from(size_of::<FSCTL_GET_INTEGRITY_INFORMATION_BUFFER>())
        .map_err(io::Error::other)?;
    #[allow(unsafe_code)]
    let done = unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            FSCTL_GET_INTEGRITY_INFORMATION,
            ptr::null(),
            0,
            (&raw mut information).cast(),
            size,
            &mut returned,
            ptr::null_mut(),
        )
    };
    if done == 0 {
        return Err(io::Error::last_os_error());
    }
    match information.ClusterSizeInBytes {
        0 => Err(io::Error::from(io::ErrorKind::Unsupported)),
        cluster => Ok(u64::from(cluster)),
    }
}

pub fn duplicate_extents(
    source: &File,
    target: &File,
    from: u64,
    at: u64,
    length: u64,
) -> io::Result<()> {
    let request = DUPLICATE_EXTENTS_DATA {
        FileHandle: source.as_raw_handle(),
        SourceFileOffset: i64::try_from(from).map_err(io::Error::other)?,
        TargetFileOffset: i64::try_from(at).map_err(io::Error::other)?,
        ByteCount: i64::try_from(length).map_err(io::Error::other)?,
    };
    let size = u32::try_from(size_of::<DUPLICATE_EXTENTS_DATA>()).map_err(io::Error::other)?;
    let mut returned = 0u32;
    #[allow(unsafe_code)]
    let done = unsafe {
        DeviceIoControl(
            target.as_raw_handle(),
            FSCTL_DUPLICATE_EXTENTS_TO_FILE,
            (&raw const request).cast(),
            size,
            ptr::null_mut(),
            0,
            &mut returned,
            ptr::null_mut(),
        )
    };
    if done == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
