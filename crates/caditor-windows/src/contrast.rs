use std::{ffi::c_void, io, mem, ptr};

use windows_sys::Win32::UI::{
    Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW},
    WindowsAndMessaging::{SPI_GETHIGHCONTRAST, SystemParametersInfoW},
};

pub fn high_contrast() -> io::Result<bool> {
    let size = mem::size_of::<HIGHCONTRASTW>();
    let mut contrast = HIGHCONTRASTW {
        cbSize: u32::try_from(size).unwrap_or(u32::MAX),
        dwFlags: 0,
        lpszDefaultScheme: ptr::null_mut(),
    };
    #[expect(unsafe_code, reason = "the Win32 call is unsafe to make")]
    let read = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            contrast.cbSize,
            (&raw mut contrast).cast::<c_void>(),
            0,
        )
    };
    if read == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(contrast.dwFlags & HCF_HIGHCONTRASTON != 0)
    }
}
