use std::{io, sync::Arc};

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use wayland_client::{
    Connection, Dispatch, DispatchError, EventQueue, QueueHandle,
    backend::WaylandError,
    delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::wl_registry::{self, WlRegistry},
};
use wayland_protocols::xdg::foreign::zv2::client::{
    zxdg_exported_v2::{self, ZxdgExportedV2},
    zxdg_exporter_v2::ZxdgExporterV2,
};

use crate::connection::{AttachError, WindowConnection};

const EXPORTER_VERSION: u32 = 1;
const WAYLAND_SCHEME: &str = "wayland:";

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("the Wayland events for the window's handle could not be handled: {0}")]
    Dispatch(#[from] DispatchError),
    #[error("the Wayland connection failed: {0}")]
    Connection(#[from] WaylandError),
}

#[derive(Default)]
struct State {
    handle: Option<String>,
}

pub struct ToplevelExport {
    queue: EventQueue<State>,
    state: State,
    exporter: ZxdgExporterV2,
    exported: ZxdgExportedV2,
    window: WindowConnection,
}

impl ToplevelExport {
    pub fn attach<W>(window: Arc<W>) -> Result<Option<Self>, AttachError>
    where
        W: HasWindowHandle + HasDisplayHandle + Send + Sync + 'static,
    {
        let Some(window) = WindowConnection::of_window(window)? else {
            return Ok(None);
        };
        let (globals, queue) = registry_queue_init::<State>(&window.connection)?;
        let handle = queue.handle();
        let exporter: ZxdgExporterV2 = globals
            .bind(&handle, 1..=EXPORTER_VERSION, ())
            .map_err(AttachError::NoExporter)?;
        let exported = exporter.export_toplevel(&window.surface, &handle, ());
        let export = Self {
            queue,
            state: State::default(),
            exporter,
            exported,
            window,
        };
        export.flush()?;
        Ok(Some(export))
    }

    pub fn handle(&mut self) -> Result<Option<String>, ExportError> {
        self.queue.dispatch_pending(&mut self.state)?;
        self.flush()?;
        Ok(self
            .state
            .handle
            .as_deref()
            .map(|handle| format!("{WAYLAND_SCHEME}{handle}")))
    }

    fn flush(&self) -> Result<(), WaylandError> {
        match self.window.connection.flush() {
            Err(WaylandError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => Ok(()),
            flushed => flushed,
        }
    }
}

impl Drop for ToplevelExport {
    fn drop(&mut self) {
        self.exported.destroy();
        self.exporter.destroy();
        if let Err(error) = self.flush() {
            log::debug!("the window's export could not say goodbye: {error}");
        }
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZxdgExportedV2, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZxdgExportedV2,
        event: zxdg_exported_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zxdg_exported_v2::Event::Handle { handle } = event {
            state.handle = Some(handle);
        }
    }
}

delegate_noop!(State: ZxdgExporterV2);
