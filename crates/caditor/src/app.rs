use std::sync::Arc;

use anyhow::{Context, Result};
use caditor_document::Document;
use caditor_render::{Renderer, SurfaceSize, WindowTarget};
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::WindowEvent,
    event_loop::ActiveEventLoop,
    window::{Window, WindowId},
};

use crate::overlay::Overlay;

pub struct App {
    document: Document,
    session: Option<Session>,
    startup_error: Option<anyhow::Error>,
}

impl App {
    pub fn new(document: Document) -> Self {
        Self {
            document,
            session: None,
            startup_error: None,
        }
    }

    pub fn finish(self) -> Result<()> {
        self.startup_error.map_or(Ok(()), Err)
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.session.is_some() {
            return;
        }
        match Session::open(event_loop) {
            Ok(session) => self.session = Some(session),
            Err(error) => {
                self.startup_error = Some(error);
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window: WindowId,
        event: WindowEvent,
    ) {
        let Some(session) = self.session.as_mut() else {
            return;
        };

        if session.overlay.on_window_event(&session.window, &event) {
            session.window.request_redraw();
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                session.renderer.resize(surface_size(size));
                session.window.request_redraw();
            }
            WindowEvent::RedrawRequested => session.redraw(&self.document),
            _ => {}
        }
    }
}

struct Session {
    window: Arc<Window>,
    renderer: Renderer,
    overlay: Overlay,
}

impl Session {
    fn open(event_loop: &ActiveEventLoop) -> Result<Self> {
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_title("caditor"))
                .context("could not open the main window")?,
        );
        let renderer = pollster::block_on(Renderer::new(
            Arc::clone(&window) as Arc<dyn WindowTarget>,
            surface_size(window.inner_size()),
        ))
        .context("could not start the renderer")?;
        let overlay = Overlay::new(&window, &renderer);
        Ok(Self {
            window,
            renderer,
            overlay,
        })
    }

    fn redraw(&mut self, document: &Document) {
        let mut frame = match self.renderer.begin_frame() {
            Ok(Some(frame)) => frame,
            Ok(None) => {
                self.window.request_redraw();
                return;
            }
            Err(error) => {
                log::error!("skipping frame: {error}");
                self.window.request_redraw();
                return;
            }
        };

        let paint = self
            .overlay
            .paint(&self.window, &self.renderer, &mut frame, document);
        self.renderer.submit(frame, paint.command_buffers);
        if paint.repaint_now {
            self.window.request_redraw();
        }
    }
}

fn surface_size(size: PhysicalSize<u32>) -> SurfaceSize {
    SurfaceSize {
        width: size.width,
        height: size.height,
    }
}
