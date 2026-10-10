use std::sync::Arc;

#[cfg(unix)]
use caditor_wayland::{DropEvent, DropTarget};
use winit::{event::WindowEvent, window::Window};

use crate::model::Waker;

#[cfg(unix)]
pub struct FileDrops {
    target: Option<DropTarget>,
}

#[cfg(unix)]
impl FileDrops {
    pub fn attach(window: &Arc<Window>, wake: Waker) -> Self {
        let target = match DropTarget::attach(Arc::clone(window), wake) {
            Ok(target) => target,
            Err(error) => {
                log::warn!("files dragged onto the window will not be seen: {error}");
                None
            }
        };
        Self { target }
    }

    pub fn events(&mut self) -> Vec<WindowEvent> {
        let Some(target) = self.target.as_mut() else {
            return Vec::new();
        };
        match target.events() {
            Ok(events) => events.into_iter().flat_map(window_events).collect(),
            Err(error) => {
                log::warn!("files dragged onto the window will no longer be seen: {error}");
                self.target = None;
                vec![WindowEvent::HoveredFileCancelled]
            }
        }
    }
}

#[cfg(unix)]
fn window_events(event: DropEvent) -> Vec<WindowEvent> {
    match event {
        DropEvent::Hovered(paths) => paths.into_iter().map(WindowEvent::HoveredFile).collect(),
        DropEvent::Left => vec![WindowEvent::HoveredFileCancelled],
        DropEvent::Dropped(paths) => paths.into_iter().map(WindowEvent::DroppedFile).collect(),
    }
}

#[cfg(windows)]
pub struct FileDrops;

#[cfg(windows)]
impl FileDrops {
    pub fn attach(_window: &Arc<Window>, _wake: Waker) -> Self {
        Self
    }

    pub fn events(&mut self) -> Vec<WindowEvent> {
        Vec::new()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn wayland_drops_reach_the_app_as_the_events_x11_gives() {
        let paths = vec![PathBuf::from("/tmp/part.step"), PathBuf::from("/tmp/a.dxf")];

        let hovered = window_events(DropEvent::Hovered(paths.clone()));
        let left = window_events(DropEvent::Left);
        let dropped = window_events(DropEvent::Dropped(paths.clone()));

        assert_eq!(
            hovered,
            paths
                .iter()
                .cloned()
                .map(WindowEvent::HoveredFile)
                .collect::<Vec<_>>()
        );
        assert_eq!(left, [WindowEvent::HoveredFileCancelled]);
        assert_eq!(
            dropped,
            paths
                .into_iter()
                .map(WindowEvent::DroppedFile)
                .collect::<Vec<_>>()
        );
    }
}
