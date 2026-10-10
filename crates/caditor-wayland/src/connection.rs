use std::sync::Arc;

use raw_window_handle::{
    HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
};
use wayland_client::{
    Connection, Proxy,
    backend::{Backend, InvalidId, ObjectId},
    protocol::wl_surface::WlSurface,
};

#[derive(Debug, thiserror::Error)]
pub enum AttachError {
    #[error("the window has no handle: {0}")]
    Handle(#[from] HandleError),
    #[error("the window's Wayland surface is not a wl_surface")]
    Surface(#[from] InvalidId),
    #[error("the compositor's globals could not be listed: {0}")]
    Globals(#[from] wayland_client::globals::GlobalError),
    #[error("the compositor offers no drag and drop: {0}")]
    NoDataDeviceManager(#[from] wayland_client::globals::BindError),
    #[error("the Wayland connection failed: {0}")]
    Connection(#[from] wayland_client::backend::WaylandError),
}

pub(crate) struct WindowConnection {
    pub(crate) connection: Connection,
    pub(crate) surface: WlSurface,
    _window: Arc<dyn Send + Sync>,
}

impl WindowConnection {
    pub(crate) fn of_window<W>(window: Arc<W>) -> Result<Option<Self>, AttachError>
    where
        W: HasWindowHandle + HasDisplayHandle + Send + Sync + 'static,
    {
        let display = window.display_handle()?.as_raw();
        let surface = window.window_handle()?.as_raw();
        let (RawDisplayHandle::Wayland(display), RawWindowHandle::Wayland(surface)) =
            (display, surface)
        else {
            return Ok(None);
        };
        #[allow(unsafe_code)]
        let backend = unsafe { Backend::from_foreign_display(display.display.as_ptr().cast()) };
        let connection = Connection::from_backend(backend);
        #[allow(unsafe_code)]
        let surface =
            unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.surface.as_ptr().cast()) }?;
        let surface = WlSurface::from_id(&connection, surface)?;
        Ok(Some(Self {
            connection,
            surface,
            _window: window,
        }))
    }
}
