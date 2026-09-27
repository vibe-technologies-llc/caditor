use std::{path::PathBuf, sync::Arc, time::Instant};

use anyhow::{Context, Result};
use caditor_render::{Renderer, SurfaceSize, ViewportFrame, WindowTarget};
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::{StartCause, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy},
    window::{Window, WindowId},
};

use crate::{
    files::{self, FileCommand, Files},
    model::{Action, Model, WakerFactory},
    overlay::Overlay,
    panels::{self, PanelState},
    toolbar,
    viewport::ViewportState,
};

const APPLICATION_NAME: &str = "caditor";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppEvent {
    Wake,
}

pub fn waker_factory(proxy: EventLoopProxy<AppEvent>) -> WakerFactory {
    Box::new(move || {
        let proxy = proxy.clone();
        Box::new(move || {
            if proxy.send_event(AppEvent::Wake).is_err() {
                log::debug!("the event loop closed before background work finished");
            }
        })
    })
}

pub fn window_title(model: &Model) -> String {
    let marker = if model.is_dirty() { "*" } else { "" };
    format!("{marker}{} — {APPLICATION_NAME}", model.display_name())
}

pub fn perform(actions: Vec<Action>, model: &mut Model, files: &mut Files) {
    for action in actions {
        match action {
            Action::File(command) => files.perform(command, model),
            other => model.perform(other),
        }
    }
}

pub struct App {
    model: Model,
    files: Files,
    session: Option<Session>,
    startup_error: Option<anyhow::Error>,
}

impl App {
    pub fn new(mut model: Model, mut files: Files, open: Option<PathBuf>) -> Self {
        files.start(open, &mut model);
        Self {
            model,
            files,
            session: None,
            startup_error: None,
        }
    }

    pub fn finish(self) -> Result<()> {
        self.startup_error.map_or(Ok(()), Err)
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: StartCause) {
        if let (StartCause::ResumeTimeReached { .. }, Some(session)) = (cause, &self.session) {
            session.window.request_redraw();
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.session.is_some() {
            return;
        }
        match Session::open(event_loop, &window_title(&self.model)) {
            Ok(session) => self.session = Some(session),
            Err(error) => {
                self.startup_error = Some(error);
                event_loop.exit();
            }
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::Wake => {
                if let Some(session) = &self.session {
                    session.window.request_redraw();
                }
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
            WindowEvent::CloseRequested => {
                self.files.perform(FileCommand::Quit, &mut self.model);
                session.window.request_redraw();
            }
            WindowEvent::Resized(size) => {
                session.renderer.resize(surface_size(size));
                session.window.request_redraw();
            }
            WindowEvent::RedrawRequested => session.redraw(&mut self.model, &mut self.files),
            _ => {}
        }
        if self.files.should_quit() {
            event_loop.exit();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let flow = self
            .session
            .as_ref()
            .and_then(|session| session.next_repaint)
            .map_or(ControlFlow::Wait, ControlFlow::WaitUntil);
        event_loop.set_control_flow(flow);
    }
}

struct Session {
    window: Arc<Window>,
    renderer: Renderer,
    overlay: Overlay,
    viewport: ViewportState,
    panels: PanelState,
    last_redraw: Option<Instant>,
    next_repaint: Option<Instant>,
    title: String,
}

impl Session {
    fn open(event_loop: &ActiveEventLoop, title: &str) -> Result<Self> {
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_title(title))
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
            panels: PanelState::default(),
            last_redraw: None,
            next_repaint: None,
            title: title.to_owned(),
        })
    }

    fn redraw(&mut self, model: &mut Model, files: &mut Files) {
        let now = Instant::now();
        let elapsed = self
            .last_redraw
            .replace(now)
            .map(|previous| now.saturating_duration_since(previous))
            .unwrap_or_default();
        model.poll();
        files.poll(model);
        if let Some(result) = self.renderer.poll_pick() {
            self.viewport.apply_pick(&result);
        }
        self.viewport.advance(elapsed);

        let mut actions = Vec::new();
        let viewport = &mut self.viewport;
        let panel_state = &mut self.panels;
        let view_model: &Model = model;
        let view_files: &Files = files;
        let ui = self.overlay.run(&self.window, |ui| {
            toolbar::show(ui, view_model, view_files, &mut actions);
            panels::show(
                ui,
                view_model,
                viewport.selection(),
                panel_state,
                &mut actions,
            );
            viewport.show(ui, view_model.document());
            files::show(ui, view_model, view_files, &mut actions);
        });
        let changed = !actions.is_empty();
        perform(actions, model, files);
        let title = window_title(model);
        if title != self.title {
            self.window.set_title(&title);
            self.title = title;
        }

        let built = self
            .viewport
            .build_scene(model.document(), model.evaluation());
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

        let repaint_after = ui.repaint_after;
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

        let repaint_now = changed || repaint_after.is_some_and(|delay| delay.is_zero());
        self.next_repaint = None;
        if repaint_now || self.viewport.is_animating() || self.renderer.is_pick_pending() {
            self.window.request_redraw();
        } else {
            self.last_redraw = None;
            self.next_repaint = repaint_after.and_then(|delay| now.checked_add(delay));
        }
    }
}

fn surface_size(size: PhysicalSize<u32>) -> SurfaceSize {
    SurfaceSize {
        width: size.width,
        height: size.height,
    }
}
