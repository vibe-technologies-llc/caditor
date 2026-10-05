use std::{
    io,
    num::NonZeroIsize,
    ptr,
    sync::{
        OnceLock,
        atomic::{AtomicIsize, Ordering},
    },
};

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawWindowHandle,
    Win32WindowHandle, WindowHandle,
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::WindowsAndMessaging::{
        CallWindowProcW, DefWindowProcW, GWLP_WNDPROC, MB_ICONERROR, MB_OK, MessageBoxW,
        SetWindowLongPtrW, WM_ENDSESSION, WNDPROC,
    },
};

use crate::files::wide;

type Callback = Box<dyn Fn() + Send + Sync>;

static ON_SESSION_END: OnceLock<Callback> = OnceLock::new();
static PREVIOUS_PROCEDURE: AtomicIsize = AtomicIsize::new(0);

const SESSION_ENDING: WPARAM = 1;

pub fn on_session_end(
    window: NonZeroIsize,
    callback: impl Fn() + Send + Sync + 'static,
) -> io::Result<()> {
    ON_SESSION_END.set(Box::new(callback)).map_err(|_| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "a session end handler is already installed",
        )
    })?;
    let procedure: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT =
        session_procedure;
    #[allow(unsafe_code)]
    let previous = unsafe {
        SetWindowLongPtrW(
            window.get() as HWND,
            GWLP_WNDPROC,
            procedure as usize as isize,
        )
    };
    if previous == 0 {
        return Err(io::Error::last_os_error());
    }
    PREVIOUS_PROCEDURE.store(previous, Ordering::Release);
    Ok(())
}

#[allow(unsafe_code)]
unsafe extern "system" fn session_procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_ENDSESSION
        && wparam == SESSION_ENDING
        && let Some(callback) = ON_SESSION_END.get()
    {
        callback();
    }
    let previous = PREVIOUS_PROCEDURE.load(Ordering::Acquire);
    if previous == 0 {
        return unsafe { DefWindowProcW(window, message, wparam, lparam) };
    }
    let previous: WNDPROC = unsafe { std::mem::transmute::<isize, WNDPROC>(previous) };
    unsafe { CallWindowProcW(previous, window, message, wparam, lparam) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DialogParent(NonZeroIsize);

impl DialogParent {
    pub fn new(window: NonZeroIsize) -> Self {
        Self(window)
    }
}

impl HasWindowHandle for DialogParent {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let raw = RawWindowHandle::Win32(Win32WindowHandle::new(self.0));
        #[allow(unsafe_code)]
        let handle = unsafe { WindowHandle::borrow_raw(raw) };
        Ok(handle)
    }
}

impl HasDisplayHandle for DialogParent {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Ok(DisplayHandle::windows())
    }
}

pub fn show_error(title: &str, text: &str) -> bool {
    let title = wide(title);
    let text = wide(text);
    #[allow(unsafe_code)]
    let shown = unsafe {
        MessageBoxW(
            ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        )
    };
    shown != 0
}
