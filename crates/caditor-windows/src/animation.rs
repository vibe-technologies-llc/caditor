use std::{ffi::c_void, io};

use windows_sys::Win32::UI::WindowsAndMessaging::{
    SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
};

pub fn client_area_animations() -> io::Result<bool> {
    let mut enabled: i32 = 0;
    #[expect(unsafe_code, reason = "the Win32 call is unsafe to make")]
    let read = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&raw mut enabled).cast::<c_void>(),
            0,
        )
    };
    if read == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(enabled != 0)
    }
}
