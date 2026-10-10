use std::sync::Arc;

#[cfg(unix)]
use caditor_wayland::ToplevelExport;
use winit::window::Window;

#[cfg(unix)]
pub struct WindowExport {
    export: Option<ToplevelExport>,
    named: bool,
}

#[cfg(unix)]
impl WindowExport {
    pub fn attach(window: &Arc<Window>) -> Self {
        let export = match ToplevelExport::attach(Arc::clone(window)) {
            Ok(export) => export,
            Err(error) => {
                log::warn!("file dialogs will not be tied to the window: {error}");
                None
            }
        };
        Self {
            export,
            named: false,
        }
    }

    pub fn poll(&mut self) {
        if self.named {
            return;
        }
        let Some(export) = self.export.as_mut() else {
            return;
        };
        match export.handle() {
            Ok(Some(handle)) => {
                crate::portal::own_dialogs(crate::portal::ParentWindow::Wayland(handle));
                self.named = true;
            }
            Ok(None) => {}
            Err(error) => {
                log::warn!("file dialogs will not be tied to the window: {error}");
                self.export = None;
            }
        }
    }
}

#[cfg(windows)]
pub struct WindowExport;

#[cfg(windows)]
impl WindowExport {
    pub fn attach(_window: &Arc<Window>) -> Self {
        Self
    }

    pub fn poll(&mut self) {}
}
