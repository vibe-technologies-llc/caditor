use std::{path::PathBuf, sync::Arc, time::Instant};

use anyhow::{Context, Result};
use caditor_render::{Renderer, SurfaceSize, ViewportFrame, WindowTarget};
use egui_winit::accesskit_winit;
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::{StartCause, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy},
    platform::{wayland::WindowAttributesExtWayland, x11::WindowAttributesExtX11},
    window::{Window, WindowAttributes, WindowId},
};

use crate::{
    about,
    appearance::{self, MAX_SCALE, MIN_SCALE, SCALE_STEP},
    commands::{self, Command, CommandFrame, Offer, Situation},
    editing::SketchEditing,
    files::{self, FileCommand, Files},
    fonts,
    menu_bar::{self, MenuContext},
    model::{Action, Model, Notice, WakerFactory},
    onboarding::{self, HintChoice, WelcomeChoice},
    overlay::Overlay,
    palette::Palette,
    panels::{self, PanelState},
    preferences::{self, Appearance, PreferenceChange, Preferences, PreferencesCommand},
    shortcut_editor::{self, ShortcutEditor},
    sketch_toolbar,
    status_bar::{self, StatusContext},
    toolbar::{self, ToolbarContext},
    viewport::ViewportState,
};

#[derive(Debug)]
pub enum AppEvent {
    Wake,
    Accessibility(accesskit_winit::Event),
}

impl From<accesskit_winit::Event> for AppEvent {
    fn from(event: accesskit_winit::Event) -> Self {
        Self::Accessibility(event)
    }
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
    format!("{marker}{} — {}", model.display_name(), about::NAME)
}

pub struct Workspace {
    pub viewport: ViewportState,
    pub panels: PanelState,
    pub editing: SketchEditing,
    pub preferences: Preferences,
    pub preferences_open: bool,
    pub palette: Palette,
    pub shortcut_editor: Option<ShortcutEditor>,
    pub welcome_open: bool,
    pub about_open: bool,
    pub last_offers: Vec<Offer>,
    applied_appearance: Option<Appearance>,
    keyboard_was_taken: bool,
}

impl Workspace {
    #[cfg(test)]
    pub fn new() -> Self {
        let mut preferences = Preferences::default();
        preferences.onboarding = crate::onboarding::Onboarding::finished();
        Self::with_preferences(preferences)
    }

    pub fn with_preferences(preferences: Preferences) -> Self {
        let welcome_open = !preferences.onboarding.welcomed;
        let mut viewport = ViewportState::new();
        viewport.set_navigation(preferences.navigation);
        Self {
            viewport,
            panels: PanelState::default(),
            editing: SketchEditing::default(),
            preferences,
            preferences_open: false,
            palette: Palette::default(),
            shortcut_editor: None,
            welcome_open,
            about_open: false,
            last_offers: Vec::new(),
            applied_appearance: None,
            keyboard_was_taken: false,
        }
    }

    fn preferences_command(
        &mut self,
        command: PreferencesCommand,
        model: &mut Model,
        files: &mut Files,
    ) {
        match command {
            PreferencesCommand::Show => self.preferences_open = true,
            PreferencesCommand::Hide => self.preferences_open = false,
            PreferencesCommand::ShowShortcuts => {
                self.shortcut_editor
                    .get_or_insert_with(ShortcutEditor::default);
            }
            PreferencesCommand::HideShortcuts => self.shortcut_editor = None,
            PreferencesCommand::ShowWelcome => self.welcome_open = true,
            PreferencesCommand::CloseWelcome => {
                self.welcome_open = false;
                if !self.preferences.onboarding.welcomed {
                    self.preferences.apply(PreferenceChange::Welcomed);
                    files.store_settings(self.preferences.settings());
                }
            }
            PreferencesCommand::ShowAbout => self.about_open = true,
            PreferencesCommand::CloseAbout => self.about_open = false,
            PreferencesCommand::Change(change) => {
                self.preferences.apply(change);
                model.set_length_unit(self.preferences.unit);
                self.viewport.set_navigation(self.preferences.navigation);
                files.store_settings(self.preferences.settings());
            }
        }
    }
}

pub fn show(
    ui: &mut egui::Ui,
    model: &Model,
    files: &Files,
    workspace: &mut Workspace,
    actions: &mut Vec<Action>,
) {
    match apply_appearance(ui.ctx(), workspace) {
        Applied::FontsPending => return,
        Applied::Changed => ui.set_style(ui.ctx().global_style()),
        Applied::Unchanged => {}
    }
    let text_focused = ui.ctx().text_edit_focused();
    let keyboard_taken = ui.ctx().egui_wants_keyboard_input() || workspace.keyboard_was_taken;
    let dialog_open = workspace.preferences_open
        || workspace.palette.is_open()
        || workspace.shortcut_editor.is_some()
        || workspace.welcome_open
        || workspace.about_open;
    let blocked = files.is_blocking() || dialog_open;
    let keys_free = !keyboard_taken && !blocked;
    let Workspace {
        viewport,
        panels,
        editing,
        preferences,
        preferences_open,
        palette,
        shortcut_editor,
        welcome_open,
        about_open,
        last_offers,
        keyboard_was_taken,
        ..
    } = workspace;
    let situation = Situation {
        editing_sketch: editing.active().is_some(),
        drawing: viewport.is_drawing(),
        text_focused,
        keys_free,
    };
    let mut triggered = if blocked {
        Vec::new()
    } else {
        commands::dispatch(ui.ctx(), &preferences.keymap, &situation)
    };
    triggered.extend(palette.take_chosen());
    let mut commands = CommandFrame::new(&preferences.keymap, triggered);
    let menu = MenuContext {
        files,
        editing,
        offers: last_offers,
    };
    menu_bar::show(ui, model, &menu, &mut commands, actions);
    let toolbar = ToolbarContext {
        selection: viewport.selection(),
        editing,
    };
    toolbar::show(ui, model, &toolbar, &mut commands, actions);
    sketch_toolbar::show(
        ui,
        model,
        editing,
        viewport.selection(),
        &mut commands,
        panels,
        actions,
    );
    let status = StatusContext {
        files,
        selection: viewport.selection(),
        appearance: &preferences.appearance,
    };
    status_bar::show(ui, model, &status, panels, &mut commands, actions);
    panels::commands(model, editing, panels, &mut commands, actions);
    route_dimension_focus(panels, editing, viewport);
    panels::show(ui, model, viewport.selection(), editing, panels, actions);
    route_dimension_focus(panels, editing, viewport);
    let selected_before = viewport.selection().clone();
    viewport.show(ui, model, editing, keys_free, &mut commands, actions);
    if viewport.selection() != &selected_before && !viewport.selection().is_empty() {
        panels.selected = None;
    }
    interface_size(&preferences.appearance, &mut commands, actions);
    let open_palette = commands.available(Command::Palette);
    let open_shortcuts = commands.available(Command::KeyboardShortcuts);
    if commands.available(Command::Welcome) {
        actions.push(Action::Preferences(PreferencesCommand::ShowWelcome));
    }
    if commands.available(Command::About) {
        actions.push(Action::Preferences(PreferencesCommand::ShowAbout));
    }
    let (offers, refused) = commands.finish();
    for (command, reason) in refused {
        actions.push(Action::Inform(Notice::info(format!(
            "{}: {reason}",
            command.title()
        ))));
    }
    if open_shortcuts {
        actions.push(Action::Preferences(PreferencesCommand::ShowShortcuts));
    }
    files::show(ui, model, files, actions);
    if !files.is_blocking() {
        if *preferences_open && let Some(command) = preferences::dialog(ui.ctx(), preferences) {
            actions.push(Action::Preferences(command));
        }
        if let Some(editor) = shortcut_editor
            && let Some(command) = shortcut_editor::dialog(ui.ctx(), editor, &preferences.keymap)
        {
            actions.push(Action::Preferences(command));
        }
        if open_palette && !dialog_open {
            palette.open();
        }
        palette.show(ui.ctx(), &offers, &preferences.keymap);
        if *welcome_open && let Some(choice) = onboarding::welcome(ui.ctx(), &preferences.keymap) {
            actions.push(Action::Preferences(PreferencesCommand::CloseWelcome));
            match choice {
                WelcomeChoice::Close => {}
                WelcomeChoice::Empty if model.is_empty_and_untitled() => {}
                WelcomeChoice::Empty => actions.push(Action::File(FileCommand::New)),
                WelcomeChoice::Sample(sample) => {
                    actions.push(Action::File(FileCommand::OpenSample(sample)));
                }
                WelcomeChoice::Open => actions.push(Action::File(FileCommand::Open)),
            }
        }
        if *about_open && about::dialog(ui.ctx()) {
            actions.push(Action::Preferences(PreferencesCommand::CloseAbout));
        }
        let situation = onboarding::Situation {
            model,
            editing,
            offers: &offers,
        };
        let hint = onboarding::current(&preferences.onboarding, &situation)
            .filter(|_| !dialog_open)
            .zip(viewport.rect());
        if let Some((hint, rect)) = hint
            && let Some(choice) = onboarding::show_hint(ui.ctx(), rect, hint, &preferences.keymap)
        {
            let change = match choice {
                HintChoice::Dismiss(hint) => PreferenceChange::DismissHint(hint),
                HintChoice::HideAll => PreferenceChange::ShowHints(false),
            };
            actions.push(Action::Preferences(PreferencesCommand::Change(change)));
        }
    }
    *last_offers = offers;
    *keyboard_was_taken = ui.ctx().egui_wants_keyboard_input();
}

enum Applied {
    FontsPending,
    Changed,
    Unchanged,
}

fn apply_appearance(ctx: &egui::Context, workspace: &mut Workspace) -> Applied {
    let wanted = workspace.preferences.appearance;
    if workspace.applied_appearance == Some(wanted) {
        return Applied::Unchanged;
    }
    if !fonts::installed(ctx) {
        ctx.options_mut(|options| {
            options.zoom_with_keyboard = false;
            options.quit_shortcuts.clear();
        });
        ctx.set_fonts(fonts::definitions());
        ctx.request_repaint();
        return Applied::FontsPending;
    }
    for (theme, dark) in [(egui::Theme::Dark, true), (egui::Theme::Light, false)] {
        ctx.set_style_of(theme, appearance::style(dark, wanted.high_contrast));
    }
    ctx.set_theme(wanted.theme.egui());
    ctx.set_zoom_factor(wanted.scale);
    workspace.applied_appearance = Some(wanted);
    Applied::Changed
}

fn interface_size(
    current: &Appearance,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let largest = format!("The interface is at its largest, {:.0}%", MAX_SCALE * 100.0);
    let smallest = format!(
        "The interface is at its smallest, {:.0}%",
        MIN_SCALE * 100.0
    );
    let steps = [
        (
            Command::LargerInterface,
            current.scale + SCALE_STEP,
            if current.scale < MAX_SCALE {
                Ok(())
            } else {
                Err(largest)
            },
        ),
        (
            Command::SmallerInterface,
            current.scale - SCALE_STEP,
            if current.scale > MIN_SCALE {
                Ok(())
            } else {
                Err(smallest)
            },
        ),
        (Command::NormalInterface, 1.0, Ok(())),
    ];
    for (command, scale, availability) in steps {
        if commands.invoke(command, &availability) {
            actions.push(Action::Preferences(PreferencesCommand::Change(
                PreferenceChange::Scale(scale),
            )));
        }
    }
}

fn route_dimension_focus(
    panels: &mut PanelState,
    editing: &SketchEditing,
    viewport: &mut ViewportState,
) {
    if let Some(feature) = editing.feature()
        && let Some(constraint) = panels.take_dimension_focus(feature)
    {
        viewport.edit_dimension(feature, constraint);
    }
}

pub fn perform(
    actions: Vec<Action>,
    model: &mut Model,
    files: &mut Files,
    workspace: &mut Workspace,
) {
    for action in actions {
        match action {
            Action::File(command) => files.perform(command, model),
            Action::Editing(command) => workspace.editing.perform(command, model),
            Action::Preferences(command) => workspace.preferences_command(command, model, files),
            other => model.perform(other),
        }
    }
    workspace.editing.sync(model);
}

pub struct App {
    model: Model,
    files: Files,
    preferences: Preferences,
    session: Option<Session>,
    startup_error: Option<anyhow::Error>,
    proxy: EventLoopProxy<AppEvent>,
}

impl App {
    pub fn new(
        mut model: Model,
        mut files: Files,
        preferences: Preferences,
        open: Option<PathBuf>,
        proxy: EventLoopProxy<AppEvent>,
    ) -> Self {
        model.set_length_unit(preferences.unit);
        files.settings_loaded(preferences.settings());
        files.start(open, &mut model);
        Self {
            model,
            files,
            preferences,
            session: None,
            startup_error: None,
            proxy,
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
        let preferences = self.preferences.clone();
        let title = window_title(&self.model);
        match Session::open(event_loop, &title, preferences, self.proxy.clone()) {
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
            AppEvent::Accessibility(event) => {
                if let Some(session) = &mut self.session
                    && event.window_id == session.window.id()
                {
                    session.overlay.on_accessibility_event(event.window_event);
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

fn window_attributes(title: &str) -> WindowAttributes {
    let attributes = Window::default_attributes()
        .with_title(title)
        .with_visible(false);
    let attributes =
        WindowAttributesExtWayland::with_name(attributes, about::APP_ID, about::APP_ID);
    WindowAttributesExtX11::with_name(attributes, about::APP_ID, about::APP_ID)
}

struct Session {
    window: Arc<Window>,
    renderer: Renderer,
    overlay: Overlay,
    workspace: Workspace,
    last_redraw: Option<Instant>,
    next_repaint: Option<Instant>,
    title: String,
}

impl Session {
    fn open(
        event_loop: &ActiveEventLoop,
        title: &str,
        preferences: Preferences,
        proxy: EventLoopProxy<AppEvent>,
    ) -> Result<Self> {
        let window = Arc::new(
            event_loop
                .create_window(window_attributes(title))
                .context("could not open the main window")?,
        );
        let renderer = pollster::block_on(Renderer::new(
            Arc::clone(&window) as Arc<dyn WindowTarget>,
            surface_size(window.inner_size()),
        ))
        .context("could not start the renderer")?;
        let mut overlay = Overlay::new(&window, &renderer);
        overlay.enable_accessibility(event_loop, &window, proxy);
        window.set_visible(true);
        Ok(Self {
            window,
            renderer,
            overlay,
            workspace: Workspace::with_preferences(preferences),
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
        files.poll(model, &mut self.workspace.editing);
        self.workspace.editing.sync(model);
        if let Some(result) = self.renderer.poll_pick() {
            self.workspace.viewport.apply_pick(&result);
        }
        self.workspace.viewport.advance(elapsed);

        let mut actions = Vec::new();
        let workspace = &mut self.workspace;
        let view_model: &Model = model;
        let view_files: &Files = files;
        let ui = self.overlay.run(&self.window, |ui| {
            show(ui, view_model, view_files, workspace, &mut actions);
        });
        let changed = !actions.is_empty();
        perform(actions, model, files, &mut self.workspace);
        let title = window_title(model);
        if title != self.title {
            self.window.set_title(&title);
            self.title = title;
        }

        model.mesh_before(self.workspace.editing.context().solid);
        let workspace = &mut self.workspace;
        let built = workspace.viewport.build_scene(
            model.document(),
            model.evaluation(),
            &workspace.editing,
        );
        let request = workspace
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
            self.workspace.viewport.pick_was_not_issued();
        }

        let repaint_now = changed || repaint_after.is_some_and(|delay| delay.is_zero());
        self.next_repaint = None;
        if repaint_now || self.workspace.viewport.is_animating() || self.renderer.is_pick_pending()
        {
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
