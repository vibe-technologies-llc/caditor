use std::{io, sync::OnceLock};

use windows_sys::{
    Win32::{
        Foundation::INVALID_HANDLE_VALUE,
        System::Console::{
            ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE,
            SetConsoleCtrlHandler,
        },
    },
    core::BOOL,
};

type Callback = Box<dyn Fn() + Send + Sync>;

static ON_CLOSE: OnceLock<Callback> = OnceLock::new();

const HANDLED: BOOL = 1;
const ADD: BOOL = 1;

pub fn on_console_close(callback: impl Fn() + Send + Sync + 'static) -> io::Result<()> {
    ON_CLOSE.set(Box::new(callback)).map_err(|_| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "a console close handler is already installed",
        )
    })?;
    #[allow(unsafe_code)]
    let done = unsafe { SetConsoleCtrlHandler(Some(console_event), ADD) };
    if done == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[allow(unsafe_code)]
unsafe extern "system" fn console_event(_event: u32) -> BOOL {
    if let Some(callback) = ON_CLOSE.get() {
        callback();
    }
    HANDLED
}

pub fn attach_parent_console() -> bool {
    #[allow(unsafe_code)]
    let output = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    if !output.is_null() && output != INVALID_HANDLE_VALUE {
        return false;
    }
    #[allow(unsafe_code)]
    let attached = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
    attached != 0
}
