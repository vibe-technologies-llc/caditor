use std::{
    io,
    mem::size_of,
    num::NonZeroIsize,
    sync::{
        OnceLock,
        atomic::{AtomicIsize, AtomicU8, Ordering},
    },
};

use parking_lot::Mutex;
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
    Graphics::Gdi::ScreenToClient,
    UI::{
        Input::KeyboardAndMouse::{TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent},
        WindowsAndMessaging::{
            CallWindowProcW, DefWindowProcW, GWLP_WNDPROC, HTMAXBUTTON, IsZoomed, SW_MAXIMIZE,
            SW_RESTORE, SetWindowLongPtrW, ShowWindow, WM_NCHITTEST, WM_NCLBUTTONDBLCLK,
            WM_NCLBUTTONDOWN, WM_NCLBUTTONUP, WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WNDPROC,
        },
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ButtonRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl ButtonRect {
    pub fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub fn contains(self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonState {
    #[default]
    Idle,
    Hovered,
    Pressed,
}

impl ButtonState {
    fn code(self) -> u8 {
        match self {
            Self::Idle => 0,
            Self::Hovered => 1,
            Self::Pressed => 2,
        }
    }

    fn from_code(code: u8) -> Self {
        match code {
            1 => Self::Hovered,
            2 => Self::Pressed,
            _ => Self::Idle,
        }
    }
}

type Callback = Box<dyn Fn(ButtonState) + Send + Sync>;

static BUTTON: Mutex<Option<ButtonRect>> = Mutex::new(None);
static ON_STATE: OnceLock<Callback> = OnceLock::new();
static STATE: AtomicU8 = AtomicU8::new(0);
static PREVIOUS_PROCEDURE: AtomicIsize = AtomicIsize::new(0);

const HIT_MAXIMIZE_BUTTON: WPARAM = HTMAXBUTTON as WPARAM;
const HANDLED: LRESULT = 0;

pub fn on_maximize_button(
    window: NonZeroIsize,
    callback: impl Fn(ButtonState) + Send + Sync + 'static,
) -> io::Result<()> {
    ON_STATE.set(Box::new(callback)).map_err(|_| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "a maximize button handler is already installed",
        )
    })?;
    let procedure: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT =
        maximize_procedure;
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

pub fn set_maximize_button(rect: Option<ButtonRect>) {
    let changed = {
        let mut button = BUTTON.lock();
        let changed = *button != rect;
        *button = rect;
        changed
    };
    if changed && rect.is_none() {
        announce(ButtonState::Idle);
    }
}

fn announce(state: ButtonState) {
    let previous = STATE.swap(state.code(), Ordering::AcqRel);
    if previous != state.code()
        && let Some(callback) = ON_STATE.get()
    {
        callback(state);
    }
}

fn current_state() -> ButtonState {
    ButtonState::from_code(STATE.load(Ordering::Acquire))
}

pub(crate) fn client_point(lparam: LPARAM) -> POINT {
    POINT {
        x: i32::from(lparam as u16 as i16),
        y: i32::from((lparam >> 16) as u16 as i16),
    }
}

#[allow(unsafe_code)]
fn over_button(window: HWND, lparam: LPARAM) -> bool {
    let Some(rect) = *BUTTON.lock() else {
        return false;
    };
    let mut point = client_point(lparam);
    let converted = unsafe { ScreenToClient(window, &mut point) };
    converted != 0 && rect.contains(point.x, point.y)
}

#[allow(unsafe_code)]
fn track_leaving(window: HWND) {
    let mut request = TRACKMOUSEEVENT {
        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE | TME_NONCLIENT,
        hwndTrack: window,
        dwHoverTime: 0,
    };
    unsafe { TrackMouseEvent(&mut request) };
}

#[allow(unsafe_code)]
fn toggle_maximized(window: HWND) {
    let zoomed = unsafe { IsZoomed(window) } != 0;
    let command = if zoomed { SW_RESTORE } else { SW_MAXIMIZE };
    unsafe { ShowWindow(window, command) };
}

#[allow(unsafe_code)]
unsafe extern "system" fn maximize_procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let on_button = wparam == HIT_MAXIMIZE_BUTTON;
    match message {
        WM_NCHITTEST if over_button(window, lparam) => return HTMAXBUTTON as LRESULT,
        WM_NCMOUSEMOVE if on_button => {
            track_leaving(window);
            if current_state() == ButtonState::Idle {
                announce(ButtonState::Hovered);
            }
            return HANDLED;
        }
        WM_NCMOUSEMOVE | WM_NCMOUSELEAVE => announce(ButtonState::Idle),
        WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK if on_button => {
            announce(ButtonState::Pressed);
            return HANDLED;
        }
        WM_NCLBUTTONUP if on_button => {
            let pressed = current_state() == ButtonState::Pressed;
            announce(ButtonState::Hovered);
            if pressed {
                toggle_maximized(window);
            }
            return HANDLED;
        }
        _ => {}
    }
    let previous = PREVIOUS_PROCEDURE.load(Ordering::Acquire);
    if previous == 0 {
        return unsafe { DefWindowProcW(window, message, wparam, lparam) };
    }
    let previous: WNDPROC = unsafe { std::mem::transmute::<isize, WNDPROC>(previous) };
    unsafe { CallWindowProcW(previous, window, message, wparam, lparam) }
}
