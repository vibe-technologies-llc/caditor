use std::{sync::Arc, time::Instant};

use anyhow::{Context, Result};
use caditor_document::Document;
use caditor_render::{Renderer, SurfaceSize, ViewportFrame, WindowTarget};
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::WindowEvent,
    event_loop::ActiveEventLoop,
    window::{Window, WindowId},
};

use crate::{overlay::Overlay, panels, viewport::ViewportState};

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
    viewport: ViewportState,
    last_redraw: Option<Instant>,
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
            viewport: ViewportState::new(),
            last_redraw: None,
        })
    }

    fn redraw(&mut self, document: &Document) {
        let now = Instant::now();
        let elapsed = self
            .last_redraw
            .replace(now)
            .map(|previous| now.saturating_duration_since(previous))
            .unwrap_or_default();
        if let Some(result) = self.renderer.poll_pick() {
            self.viewport.apply_pick(&result);
        }
        self.viewport.advance(elapsed);

        let viewport = &mut self.viewport;
        let ui = self.overlay.run(&self.window, |ui| {
            panels::show(ui, document, viewport.selection());
            viewport.show(ui, document);
        });

        let built = self.viewport.build_scene(document);
        let request = self
            .viewport
            .request(&built, !self.renderer.is_pick_pending());
        let pick_requested = request
            .as_ref()
            .is_some_and(|request| request.pick_at.is_some());
        let viewport_frame = request.as_ref().map(|request| ViewportFrame {
            rect: request.rect,
            view: &request.view,
            scene: &built.scene,
            pick_at: request.pick_at,
        });

        let repaint_now = ui.repaint_now;
        match self.renderer.begin_frame(viewport_frame.as_ref()) {
            Ok(Some(mut frame)) => {
                let command_buffers = self.overlay.paint(&self.renderer, Some(&mut frame), ui);
                self.renderer.submit(frame, command_buffers);
            }
            Ok(None) => {
                self.overlay.paint(&self.renderer, None, ui);
                self.window.request_redraw();
            }
            Err(error) => {
                log::error!("skipping frame: {error}");
                self.overlay.paint(&self.renderer, None, ui);
                self.window.request_redraw();
            }
        }
        if pick_requested && !self.renderer.is_pick_pending() {
            self.viewport.pick_was_not_issued();
        }

        if repaint_now || self.viewport.is_animating() || self.renderer.is_pick_pending() {
            self.window.request_redraw();
        } else {
            self.last_redraw = None;
        }
    }
}

fn surface_size(size: PhysicalSize<u32>) -> SurfaceSize {
    SurfaceSize {
        width: size.width,
        height: size.height,
    }
}
