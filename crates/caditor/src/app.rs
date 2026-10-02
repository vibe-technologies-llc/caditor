use std::{
    panic::{self, AssertUnwindSafe},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use caditor_render::{
    FrameStart, ImagePoll, ImageRequest, PickPoll, Renderer, SurfaceSize, ViewportFrame, Wake,
    WindowTarget,
};
use egui_winit::accesskit_winit;
use parking_lot::Mutex;
use winit::{
    application::ApplicationHandler,
    dpi::{self, PhysicalPosition, PhysicalSize},
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
    feature_tree,
    files::{self, FileCommand, Files},
    fonts,
    graphics::{FramePacer, Hardware},
    layout::{
        LogicalSize, MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH, MonitorArea, PanelLayout, Position,
        WindowPlacement,
    },
    logo,
    measure::MeasureTool,
    measure_panel::{self, MeasureContext},
    menu_bar::{self, MenuContext},
    model::{Action, Model, Notice, WakerFactory},
    offers::SelectionOffers,
    onboarding::{self, HintChoice, WelcomeChoice},
    overlay::Overlay,
    palette::Palette,
    panels::{self, PanelState},
    preferences::{
        self, Appearance, PreferenceChange, Preferences, PreferencesCommand, PreferencesTab,
        PreferencesView, Restored, TitleBar,
    },
    reference_picking,
    shortcut_editor::{self, ShortcutEditor},
    sketch_toolbar,
    status_bar::{self, StatusContext},
    toolbar::{self, ToolbarContext},
    viewport::ViewportState,
    window_frame::{self, Chrome},
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

const FIRST_RETRY: Duration = Duration::from_millis(16);
const MAX_RETRY: Duration = Duration::from_secs(1);
const MAX_RETRY_DOUBLINGS: u32 = 6;
const HIDDEN_PROBE: Duration = Duration::from_secs(5);
const LAYOUT_SAVE_DELAY: Duration = Duration::from_secs(1);
const SETTINGS_FLUSH: Duration = Duration::from_secs(2);
const NO_TIP: &str = "No tip is shown";
const GIVE_UP_AFTER_FAILED_FRAMES: u32 = 5;
const FRAME_FAILED: &str = "Something went wrong while drawing the window, so caditor closed its \
                            dialogs and tools and cleared the selection. Your model and unsaved \
                            changes are kept.";
const FEATURES_SUPPRESSED: &str = "The window still could not be drawn, so every feature was \
                                   suppressed. Unsuppress them one at a time in the feature tree \
                                   to find the one at fault, or Undo to bring them all back.";

pub struct Workspace {
    pub viewport: ViewportState,
    pub panels: PanelState,
    pub editing: SketchEditing,
    pub preferences: Preferences,
    pub preferences_open: bool,
    pub preferences_tab: PreferencesTab,
    pub hardware: Hardware,
    pub palette: Palette,
    pub shortcut_editor: Option<ShortcutEditor>,
    pub restored: Option<Restored>,
    pub welcome_open: bool,
    pub about_open: bool,
    pub last_offers: Vec<Offer>,
    pub selection_offers: SelectionOffers,
    pub measure: MeasureTool,
    pub(crate) frame_failures: FrameFailures,
    applied_appearance: Option<Appearance>,
    applied_title_bar: Option<TitleBar>,
    keyboard_was_taken: bool,
    deferred_commands: Vec<Command>,
    session: u64,
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
            panels: PanelState::with_layout(preferences.panels),
            editing: SketchEditing::default(),
            preferences,
            preferences_open: false,
            preferences_tab: PreferencesTab::default(),
            hardware: Hardware::default(),
            palette: Palette::default(),
            shortcut_editor: None,
            restored: None,
            welcome_open,
            about_open: false,
            last_offers: Vec::new(),
            selection_offers: SelectionOffers::default(),
            measure: MeasureTool::default(),
            frame_failures: FrameFailures::default(),
            applied_appearance: None,
            applied_title_bar: None,
            keyboard_was_taken: false,
            deferred_commands: Vec::new(),
            session: 0,
        }
    }

    fn after_failed_frame(&mut self) {
        self.viewport.forget_document();
        self.panels = PanelState::with_layout(self.preferences.panels);
        self.editing = SketchEditing::default();
        self.preferences_open = false;
        self.shortcut_editor = None;
        self.palette = Palette::default();
        self.restored = None;
        self.welcome_open = false;
        self.about_open = false;
        self.last_offers.clear();
        self.selection_offers = SelectionOffers::default();
        self.measure = MeasureTool::default();
        self.applied_appearance = None;
        self.applied_title_bar = None;
        self.keyboard_was_taken = false;
        self.deferred_commands.clear();
    }

    fn sync(&mut self, model: &Model) {
        if self.session != model.session() {
            self.session = model.session();
            self.viewport.forget_document();
            self.panels.forget_document();
        }
    }

    fn preview_preference(&mut self, change: PreferenceChange, model: &mut Model) {
        self.preferences.apply(change);
        apply_preferences(model, &self.preferences);
        self.viewport.set_navigation(self.preferences.navigation);
    }

    fn preferences_command(
        &mut self,
        command: PreferencesCommand,
        model: &mut Model,
        files: &mut Files,
    ) {
        match command {
            PreferencesCommand::Show => self.preferences_open = true,
            PreferencesCommand::Hide => {
                self.preferences_open = false;
                self.restored = None;
            }
            PreferencesCommand::ShowShortcuts => {
                self.shortcut_editor
                    .get_or_insert_with(ShortcutEditor::default);
            }
            PreferencesCommand::HideShortcuts => {
                self.shortcut_editor = None;
                self.restored = None;
            }
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
            PreferencesCommand::Tab(tab) => {
                self.preferences_tab = tab;
                self.restored = None;
            }
            PreferencesCommand::Change(change) => {
                self.restored = self.preferences.restoring(change);
                self.preview_preference(change, model);
                files.store_settings(self.preferences.settings());
            }
            PreferencesCommand::Undo => {
                if let Some(restored) = self.restored.take() {
                    self.preferences.undo(restored);
                    apply_preferences(model, &self.preferences);
                    self.viewport.set_navigation(self.preferences.navigation);
                    files.store_settings(self.preferences.settings());
                }
            }
            PreferencesCommand::Preview(change) => self.preview_preference(change, model),
        }
    }
}

pub fn apply_preferences(model: &mut Model, preferences: &Preferences) {
    model.set_length_unit(preferences.unit);
    model.set_mesh_quality(preferences.graphics.curves.mesh_quality());
}

fn tip_commands(
    hint: Option<onboarding::Hint>,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let shown = hint.ok_or(NO_TIP);
    let mut change = None;
    if commands.invoke(Command::DismissTip, &shown)
        && let Ok(hint) = shown
    {
        change = Some(PreferenceChange::DismissHint(hint));
    }
    if commands.invoke(Command::HideTips, &shown) {
        change = Some(PreferenceChange::ShowHints(false));
    }
    if let Some(change) = change {
        actions.push(Action::Preferences(PreferencesCommand::Change(change)));
    }
}

pub fn show(
    ui: &mut egui::Ui,
    model: &Model,
    files: &Files,
    workspace: &mut Workspace,
    actions: &mut Vec<Action>,
) {
    workspace.sync(model);
    match apply_appearance(ui.ctx(), workspace) {
        Applied::FontsPending => return,
        Applied::Changed => ui.set_style(ui.ctx().global_style()),
        Applied::Unchanged => {}
    }
    window_frame::apply_title_bar(
        ui.ctx(),
        &mut workspace.applied_title_bar,
        workspace.preferences.title_bar,
    );
    let chrome = Chrome::of(ui.ctx(), workspace.preferences.title_bar);
    let text_focused = ui.ctx().text_edit_focused();
    let keyboard_taken = ui.ctx().egui_wants_keyboard_input() || workspace.keyboard_was_taken;
    let palette_open = workspace.palette.is_open();
    let modal_open = workspace.preferences_open
        || workspace.shortcut_editor.is_some()
        || workspace.welcome_open
        || workspace.about_open
        || workspace.panels.deleting.is_some();
    let dialog_open = modal_open || palette_open;
    let blocked = files.is_blocking() || dialog_open;
    let keys_free = !keyboard_taken && !blocked;
    let Workspace {
        viewport,
        panels,
        editing,
        preferences,
        preferences_open,
        preferences_tab,
        hardware,
        palette,
        shortcut_editor,
        restored,
        welcome_open,
        about_open,
        last_offers,
        selection_offers,
        measure,
        keyboard_was_taken,
        deferred_commands,
        ..
    } = workspace;
    let offers = selection_offers.refresh(model, viewport.selection());
    let situation = Situation {
        editing_sketch: editing.active().is_some(),
        drawing: viewport.is_drawing(),
        text_focused,
        keys_free,
    };
    let deferred = std::mem::take(deferred_commands);
    let mut triggered = if blocked {
        Vec::new()
    } else {
        commands::dispatch(ui.ctx(), &preferences.keymap, &situation)
    };
    if text_focused && !triggered.is_empty() {
        leave_text_field(ui.ctx());
        *deferred_commands = std::mem::take(&mut triggered);
    }
    if !blocked {
        triggered.extend(deferred);
    }
    triggered.extend(palette.take_chosen());
    if let Some(focus) = palette.take_focus() {
        panels.request_focus(focus);
    }
    let mut commands = CommandFrame::new(&preferences.keymap, triggered);
    let menu = MenuContext {
        files,
        editing,
        offers: last_offers,
        chrome,
    };
    menu_bar::show(ui, model, &menu, &mut commands, actions);
    let toolbar = ToolbarContext {
        selection: viewport.selection(),
        editing,
        offers,
        measuring: measure.open,
    };
    toolbar::show(ui, model, &toolbar, &mut commands, actions);
    if commands.available(Command::Measure) {
        measure.toggle();
    }
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
        offers,
        appearance: &preferences.appearance,
    };
    status_bar::show(ui, model, &status, panels, &mut commands, actions);
    let context = feature_tree::CommandContext {
        model,
        selection: viewport.selection(),
        editing,
    };
    panels::commands(&context, panels, &mut commands, actions);
    route_dimension_focus(panels, editing, viewport);
    reference_picking::publish(ui.ctx(), editing.picking());
    panels::show(ui, model, viewport.selection(), editing, panels, actions);
    preferences.panels = panels.layout();
    route_dimension_focus(panels, editing, viewport);
    if let Some(chosen) = panels.chosen_in_tree.take() {
        viewport.select_only(chosen);
    }
    viewport.hover_from_tree(panels.hovered_in_tree.take());
    if measure.open {
        let context = MeasureContext {
            model,
            selection: viewport.selection(),
            bodies: viewport.bodies(),
            tree_selected: panels.selected,
        };
        measure_panel::show(ui, &context, measure);
    }
    let measured = measure
        .open
        .then(|| measure.measurements.readout())
        .flatten()
        .and_then(|readout| readout.line)
        .map(|line| {
            let label = measure_panel::line_label(&line, model.length_unit());
            (line, label)
        });
    viewport.set_measured(measured);
    let selected_before = viewport.selection().clone();
    viewport.show(ui, model, editing, keys_free, &mut commands, actions);
    if viewport.selection() != &selected_before && !viewport.selection().is_empty() {
        panels.selected = None;
    }
    interface_size(&preferences.appearance, &mut commands, actions);
    window_frame::commands(ui.ctx(), chrome.state, &mut commands);
    if blocked {
        window_frame::over_dialogs(ui.ctx(), chrome, &commands, actions);
    }
    let open_palette = commands.available(Command::Palette);
    let open_shortcuts = commands.available(Command::KeyboardShortcuts);
    if commands.available(Command::Welcome) {
        actions.push(Action::Preferences(PreferencesCommand::ShowWelcome));
    }
    if commands.available(Command::About) {
        actions.push(Action::Preferences(PreferencesCommand::ShowAbout));
    }
    let hint = {
        let situation = onboarding::Situation {
            model,
            editing,
            offers: commands.offers(),
        };
        onboarding::current(&preferences.onboarding, &situation)
            .filter(|_| !modal_open && !files.is_blocking())
            .zip(viewport.rect())
    };
    tip_commands(hint.map(|(hint, _)| hint), &mut commands, actions);
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
    files::show(ui, model, files, viewport.view_pixels(), actions);
    if !files.is_blocking() {
        let view = PreferencesView {
            tab: *preferences_tab,
            hardware,
            switch_keys: shortcut_editor.is_none(),
            restored: restored.as_ref(),
        };
        if *preferences_open
            && let Some(command) = preferences::dialog(ui.ctx(), preferences, &view)
        {
            actions.push(Action::Preferences(command));
        }
        if let Some(editor) = shortcut_editor
            && let Some(command) = shortcut_editor::dialog(
                ui.ctx(),
                editor,
                &preferences.keymap,
                restored.as_ref().is_some_and(Restored::is_shortcuts),
            )
        {
            actions.push(Action::Preferences(command));
        }
        if open_palette && !dialog_open {
            palette.open();
        }
        palette.show(ui.ctx(), &offers, &preferences.keymap, model.document());
        if *welcome_open
            && let Some(choice) = onboarding::welcome(ui.ctx(), &preferences.keymap, files.recent())
        {
            welcome_chosen(choice, model, actions);
        }
        if *about_open && about::dialog(ui.ctx()) {
            actions.push(Action::Preferences(PreferencesCommand::CloseAbout));
        }
        feature_tree::delete_dialog(ui.ctx(), model.document(), panels, actions);
        if let Some((hint, rect)) = hint.filter(|_| !palette_open)
            && let Some(choice) = onboarding::show_hint(ui.ctx(), rect, hint, &preferences.keymap)
        {
            let change = match choice {
                HintChoice::Dismiss(hint) => PreferenceChange::DismissHint(hint),
                HintChoice::HideAll => PreferenceChange::ShowHints(false),
            };
            actions.push(Action::Preferences(PreferencesCommand::Change(change)));
        }
    }
    window_frame::frame(ui.ctx(), chrome);
    *last_offers = offers;
    *keyboard_was_taken = ui.ctx().egui_wants_keyboard_input();
}

fn welcome_chosen(choice: WelcomeChoice, model: &Model, actions: &mut Vec<Action>) {
    if choice == WelcomeChoice::ClearRecent {
        actions.push(Action::File(FileCommand::ClearRecent));
        return;
    }
    actions.push(Action::Preferences(PreferencesCommand::CloseWelcome));
    match choice {
        WelcomeChoice::Close | WelcomeChoice::ClearRecent => {}
        WelcomeChoice::Empty if model.is_empty_and_untitled() => {}
        WelcomeChoice::Empty => actions.push(Action::File(FileCommand::New)),
        WelcomeChoice::Sample(sample) => {
            actions.push(Action::File(FileCommand::OpenSample(sample)));
        }
        WelcomeChoice::Open => actions.push(Action::File(FileCommand::Open)),
        WelcomeChoice::OpenRecent(path) => actions.push(Action::File(FileCommand::OpenPath(path))),
    }
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

fn leave_text_field(ctx: &egui::Context) {
    ctx.request_repaint();
    ctx.memory_mut(|memory| {
        if let Some(focused) = memory.focused() {
            memory.surrender_focus(focused);
        }
    });
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AfterFailedFrame {
    ResetInterface,
    SuppressFeatures,
    GiveUp,
}

#[derive(Debug, Default)]
pub(crate) struct FrameFailures {
    in_a_row: u32,
}

impl FrameFailures {
    pub(crate) fn drawn(&mut self) {
        self.in_a_row = 0;
    }

    fn failed(&mut self) -> AfterFailedFrame {
        self.in_a_row = self.in_a_row.saturating_add(1);
        match self.in_a_row {
            2 => AfterFailedFrame::SuppressFeatures,
            count if count >= GIVE_UP_AFTER_FAILED_FRAMES => AfterFailedFrame::GiveUp,
            _ => AfterFailedFrame::ResetInterface,
        }
    }
}

pub(crate) fn after_failed_frame(
    workspace: &mut Workspace,
    model: &mut Model,
    files: &mut Files,
) -> AfterFailedFrame {
    let step = workspace.frame_failures.failed();
    workspace.after_failed_frame();
    files.close_dialogs(model);
    match step {
        AfterFailedFrame::SuppressFeatures if model.suppress_every_feature() => {
            model.set_notice(Notice::failure(FEATURES_SUPPRESSED));
        }
        AfterFailedFrame::ResetInterface | AfterFailedFrame::SuppressFeatures => {
            model.set_notice(Notice::failure(FRAME_FAILED));
        }
        AfterFailedFrame::GiveUp => {}
    }
    step
}

pub struct App {
    model: Model,
    files: Files,
    preferences: Preferences,
    session: Option<Session>,
    fatal_error: Option<anyhow::Error>,
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
        apply_preferences(&mut model, &preferences);
        files.settings_loaded(preferences.settings());
        files.start(open, &mut model);
        Self {
            model,
            files,
            preferences,
            session: None,
            fatal_error: None,
            proxy,
        }
    }

    pub fn finish(mut self) -> Result<()> {
        if !self.files.wait_for_jobs(SETTINGS_FLUSH) {
            log::warn!("quitting before the preferences were saved");
        }
        self.fatal_error.map_or(Ok(()), Err)
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
                self.fatal_error = Some(error);
                event_loop.exit();
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        self.contained(event_loop, |app| app.handle_user_event(event));
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window: WindowId,
        event: WindowEvent,
    ) {
        self.contained(event_loop, |app| app.handle_window_event(event_loop, event));
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(session) = &mut self.session {
            session.redraw_when_due(Instant::now());
        }
        let flow = self
            .session
            .as_ref()
            .and_then(|session| {
                [session.next_repaint, session.layout_deadline()]
                    .into_iter()
                    .flatten()
                    .min()
            })
            .map_or(ControlFlow::Wait, ControlFlow::WaitUntil);
        event_loop.set_control_flow(flow);
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(mut session) = self.session.take() {
            session.remember_layout(&mut self.files, Instant::now());
            session.store_layout(&mut self.files);
        }
    }
}

impl App {
    fn contained(&mut self, event_loop: &ActiveEventLoop, handle: impl FnOnce(&mut Self)) {
        if panic::catch_unwind(AssertUnwindSafe(|| handle(self))).is_ok() {
            return;
        }
        let Some(session) = self.session.as_mut() else {
            return;
        };
        let step = session.recover(event_loop, &mut self.model, &mut self.files, &self.proxy);
        if step == AfterFailedFrame::GiveUp {
            self.fatal_error = Some(anyhow!(
                "caditor could not draw its window; unsaved changes are kept and offered when it \
                 starts again"
            ));
            event_loop.exit();
        }
    }

    fn handle_user_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Wake => {
                if let Some(session) = &mut self.session {
                    session.request_redraw();
                }
            }
            AppEvent::Accessibility(event) => {
                if let Some(session) = &mut self.session
                    && event.window_id == session.window.id()
                {
                    session.overlay.on_accessibility_event(event.window_event);
                    session.request_redraw();
                }
            }
        }
    }

    fn handle_window_event(&mut self, event_loop: &ActiveEventLoop, event: WindowEvent) {
        let Some(session) = self.session.as_mut() else {
            return;
        };

        if session.overlay.on_window_event(&session.window, &event) {
            session.request_redraw();
        }

        if shows_the_window(&event) {
            session.hidden_until = None;
            session.request_redraw();
        }
        match event {
            WindowEvent::Occluded(true) => {
                session.hidden_until = Instant::now().checked_add(HIDDEN_PROBE);
            }
            WindowEvent::CloseRequested => {
                self.files.perform(FileCommand::Quit, &mut self.model);
                session.request_redraw();
            }
            WindowEvent::Resized(size) => {
                session.renderer.resize(surface_size(size));
                session.note_placement();
                session.note_display();
                session.request_redraw();
            }
            WindowEvent::Moved(_) => {
                session.note_placement();
                session.note_display();
            }
            WindowEvent::ScaleFactorChanged { .. } => session.note_display(),
            WindowEvent::RedrawRequested => {
                session.redraw(&mut self.model, &mut self.files);
                session.workspace.frame_failures.drawn();
            }
            WindowEvent::DroppedFile(path) => {
                session.dropped.push(path);
                session.request_redraw();
            }
            _ => {}
        }
        if let Some(session) = self.session.as_mut() {
            session.remember_layout(&mut self.files, Instant::now());
        }
        if self.files.should_quit() {
            event_loop.exit();
        }
    }
}

fn window_attributes(
    title: &str,
    placement: WindowPlacement,
    title_bar: TitleBar,
) -> WindowAttributes {
    let mut attributes = Window::default_attributes()
        .with_title(title)
        .with_visible(false)
        .with_decorations(title_bar.decorated())
        .with_min_inner_size(dpi::LogicalSize::new(MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT))
        .with_maximized(placement.maximized);
    if let Some(size) = placement.size {
        attributes = attributes.with_inner_size(dpi::LogicalSize::new(size.width, size.height));
    }
    if let Some(position) = placement.position {
        attributes = attributes.with_position(PhysicalPosition::new(position.x, position.y));
    }
    match logo::window_icon() {
        Ok(icon) => attributes = attributes.with_window_icon(Some(icon)),
        Err(error) => log::warn!("opening the window without its icon: {error}"),
    }
    let attributes =
        WindowAttributesExtWayland::with_name(attributes, about::APP_ID, about::APP_ID);
    WindowAttributesExtX11::with_name(attributes, about::APP_ID, about::APP_ID)
}

fn monitor_areas(event_loop: &ActiveEventLoop) -> Vec<MonitorArea> {
    event_loop
        .available_monitors()
        .map(|monitor| {
            let position = monitor.position();
            let size = monitor.size();
            MonitorArea {
                x: position.x,
                y: position.y,
                width: size.width,
                height: size.height,
                scale: monitor.scale_factor(),
            }
        })
        .collect()
}

fn refresh_rate(window: &Window) -> Option<f64> {
    window
        .current_monitor()
        .and_then(|monitor| monitor.refresh_rate_millihertz())
        .map(|millihertz| f64::from(millihertz) / 1000.0)
}

fn redraw_wake(proxy: EventLoopProxy<AppEvent>) -> Wake {
    let waker = Mutex::new(waker_factory(proxy)());
    Arc::new(move || (waker.lock())())
}

type Layout = (WindowPlacement, PanelLayout);

struct Session {
    window: Arc<Window>,
    renderer: Renderer,
    overlay: Overlay,
    workspace: Workspace,
    last_redraw: Option<Instant>,
    next_repaint: Option<Instant>,
    pacer: FramePacer,
    dropped: Vec<PathBuf>,
    hidden_until: Option<Instant>,
    failed_frames: u32,
    title: String,
    layout_seen: Layout,
    layout_stored: Layout,
    layout_changed_at: Option<Instant>,
}

impl Session {
    fn open(
        event_loop: &ActiveEventLoop,
        title: &str,
        preferences: Preferences,
        proxy: EventLoopProxy<AppEvent>,
    ) -> Result<Self> {
        let placement = preferences.window.fitted(&monitor_areas(event_loop));
        let window = Arc::new(
            event_loop
                .create_window(window_attributes(title, placement, preferences.title_bar))
                .context("could not open the main window")?,
        );
        let renderer = pollster::block_on(Renderer::new(
            Arc::clone(&window) as Arc<dyn WindowTarget>,
            surface_size(window.inner_size()),
            redraw_wake(proxy.clone()),
            preferences.graphics.render(),
        ))
        .context("could not start the renderer")?;
        let layout = (preferences.window, preferences.panels);
        let mut overlay = Overlay::new(&window, &renderer);
        overlay.enable_accessibility(event_loop, &window, proxy);
        window.set_visible(true);
        let mut workspace = Workspace::with_preferences(preferences);
        workspace.hardware = Hardware {
            adapter: Some(renderer.graphics_info().clone()),
            refresh_rate: refresh_rate(&window),
        };
        Ok(Self {
            window,
            renderer,
            overlay,
            workspace,
            last_redraw: None,
            next_repaint: None,
            pacer: FramePacer::default(),
            dropped: Vec::new(),
            hidden_until: None,
            failed_frames: 0,
            title: title.to_owned(),
            layout_seen: layout,
            layout_stored: layout,
            layout_changed_at: None,
        })
    }

    fn note_display(&mut self) {
        self.workspace.hardware.refresh_rate = refresh_rate(&self.window);
    }

    fn recover(
        &mut self,
        event_loop: &ActiveEventLoop,
        model: &mut Model,
        files: &mut Files,
        proxy: &EventLoopProxy<AppEvent>,
    ) -> AfterFailedFrame {
        log::error!("a frame failed; resetting the interface");
        self.overlay = Overlay::new(&self.window, &self.renderer);
        self.overlay
            .enable_accessibility(event_loop, &self.window, proxy.clone());
        let step = after_failed_frame(&mut self.workspace, model, files);
        self.next_repaint = Instant::now().checked_add(self.retry_delay());
        step
    }

    fn note_adapter(&mut self) {
        let info = self.renderer.graphics_info();
        if self.workspace.hardware.adapter.as_ref() != Some(info) {
            self.workspace.hardware.adapter = Some(info.clone());
        }
    }

    fn frame_interval(&self) -> Option<Duration> {
        self.workspace
            .preferences
            .graphics
            .frame_interval(&self.workspace.hardware)
    }

    fn request_redraw(&mut self) {
        let now = Instant::now();
        match self.pacer.next_frame_at(now, self.frame_interval()) {
            Some(at) => self.schedule_redraw(at),
            None => self.window.request_redraw(),
        }
    }

    fn schedule_redraw(&mut self, at: Instant) {
        self.next_repaint = Some(self.next_repaint.map_or(at, |scheduled| scheduled.min(at)));
    }

    fn redraw_when_due(&mut self, now: Instant) {
        if self.next_repaint.is_some_and(|at| at <= now) {
            self.next_repaint = None;
            self.window.request_redraw();
        }
    }

    fn note_placement(&mut self) {
        let window = &self.window;
        let placement = &mut self.workspace.preferences.window;
        placement.maximized = window.is_maximized();
        if placement.maximized {
            return;
        }
        let size = window.inner_size().to_logical::<f64>(window.scale_factor());
        if let Some(size) = LogicalSize::clamped(size.width, size.height) {
            placement.size = Some(size);
        }
        if let Ok(position) = window.outer_position() {
            placement.position = Some(Position {
                x: position.x,
                y: position.y,
            });
        }
    }

    fn remember_layout(&mut self, files: &mut Files, now: Instant) {
        let preferences = &self.workspace.preferences;
        let current = (preferences.window, preferences.panels);
        if current != self.layout_seen {
            self.layout_seen = current;
            self.layout_changed_at = Some(now);
        }
        if self
            .layout_deadline()
            .is_some_and(|deadline| now >= deadline)
        {
            self.layout_changed_at = None;
            self.store_layout(files);
        }
    }

    fn store_layout(&mut self, files: &mut Files) {
        if self.layout_seen != self.layout_stored {
            files.store_settings(self.workspace.preferences.settings());
            self.layout_stored = self.layout_seen;
        }
    }

    fn layout_deadline(&self) -> Option<Instant> {
        self.layout_changed_at
            .and_then(|changed| changed.checked_add(LAYOUT_SAVE_DELAY))
    }

    fn retry_delay(&mut self) -> Duration {
        let delay = retry_delay(self.failed_frames);
        self.failed_frames = self.failed_frames.saturating_add(1);
        delay
    }

    fn export_image(&mut self, model: &mut Model, files: &mut Files) {
        if let Some(job) = files.image_job() {
            let workspace = &self.workspace;
            let image = workspace
                .viewport
                .image(model, &workspace.editing, job.size);
            let started = self.renderer.render_image(&ImageRequest {
                size: job.size,
                view: &image.view,
                scene: &image.scene,
                pixels_per_point: image.pixels_per_point,
                background: job.background,
            });
            if let Err(error) = started {
                files.image_rendered(Err(error), model);
            }
        }
        match self.renderer.poll_image() {
            ImagePoll::Idle | ImagePoll::Pending => {}
            ImagePoll::Ready(readback) => {
                files.image_rendered(Ok(Box::new(move || readback.into_image())), model);
            }
            ImagePoll::Failed(error) => files.image_rendered(Err(error), model),
        }
    }

    fn redraw(&mut self, model: &mut Model, files: &mut Files) {
        let now = Instant::now();
        self.pacer.frame_started(now, self.frame_interval());
        let elapsed = self
            .last_redraw
            .replace(now)
            .map(|previous| now.saturating_duration_since(previous))
            .unwrap_or_default();
        model.poll();
        files.poll(model, &mut self.workspace.editing);
        self.workspace.editing.sync(model);
        match self.renderer.poll_pick() {
            PickPoll::Pending => {}
            PickPoll::Ready(result) => self.workspace.viewport.apply_pick(&result),
            PickPoll::Failed => self.workspace.viewport.pick_was_not_issued(),
        }
        self.workspace.viewport.advance(elapsed);

        let mut actions = Vec::new();
        let workspace = &mut self.workspace;
        let view_model: &Model = model;
        let view_files: &Files = files;
        let ui = self.overlay.run(&self.window, |ui| {
            show(ui, view_model, view_files, workspace, &mut actions);
        });
        if !self.dropped.is_empty() {
            actions.push(Action::File(FileCommand::Drop {
                paths: std::mem::take(&mut self.dropped),
                into: self.workspace.editing.feature(),
            }));
        }
        let changed = !actions.is_empty();
        perform(actions, model, files, &mut self.workspace);
        self.renderer
            .set_graphics(self.workspace.preferences.graphics.render());
        let title = window_title(model);
        if title != self.title {
            self.window.set_title(&title);
            self.title = title;
        }

        model.mesh_before(self.workspace.editing.context().solid);
        self.export_image(model, files);
        let workspace = &mut self.workspace;
        workspace.viewport.build_scene(model, &workspace.editing);
        let request = workspace.viewport.request(!self.renderer.is_pick_pending());
        let pick_requested = request
            .as_ref()
            .is_some_and(|request| request.pick_at.is_some());
        let scene = workspace.viewport.scene();
        let viewport_frame = request
            .as_ref()
            .zip(scene)
            .map(|(request, scene)| ViewportFrame {
                rect: request.rect,
                view: &request.view,
                scene,
                pick_at: request.pick_at,
                pixels_per_point: request.pixels_per_point,
            });

        let repaint_after = ui.repaint_after;
        let hidden = self.hidden_until.filter(|until| now < *until);
        let started = match hidden {
            Some(_) => Ok(FrameStart::Hidden),
            None => self.renderer.begin_frame(
                surface_size(self.window.inner_size()),
                viewport_frame.as_ref(),
            ),
        };
        let wait = match started {
            Ok(FrameStart::Ready(mut frame)) => {
                let command_buffers = self.overlay.paint(&self.renderer, Some(&mut *frame), ui);
                self.renderer.submit(*frame, command_buffers);
                self.failed_frames = 0;
                self.hidden_until = None;
                None
            }
            Ok(FrameStart::Hidden) => {
                self.overlay.paint(&self.renderer, None, ui);
                let until = hidden.or_else(|| now.checked_add(HIDDEN_PROBE));
                self.hidden_until = until;
                Some(until.map_or(HIDDEN_PROBE, |until| until.saturating_duration_since(now)))
            }
            Ok(FrameStart::Skipped) => {
                self.overlay.paint(&self.renderer, None, ui);
                Some(self.retry_delay())
            }
            Err(error) => {
                log::error!("skipping frame: {error}");
                self.overlay.paint(&self.renderer, None, ui);
                Some(self.retry_delay())
            }
        };
        if pick_requested && !self.renderer.is_pick_pending() {
            self.workspace.viewport.pick_was_not_issued();
        }
        self.note_adapter();
        for fault in self.renderer.take_faults() {
            model.set_notice(Notice::error(fault.to_string()));
        }

        let repaint_now = changed || repaint_after.is_some_and(|delay| delay.is_zero());
        self.next_repaint = None;
        if let Some(wait) = wait {
            self.next_repaint = now.checked_add(wait);
        } else if repaint_now
            || self.workspace.viewport.is_animating()
            || self.renderer.is_pick_pending()
            || self.renderer.is_image_pending()
        {
            self.request_redraw();
        } else {
            self.last_redraw = None;
            self.next_repaint = repaint_after.and_then(|delay| now.checked_add(delay));
        }
    }
}

fn retry_delay(failures: u32) -> Duration {
    FIRST_RETRY
        .saturating_mul(1 << failures.min(MAX_RETRY_DOUBLINGS))
        .min(MAX_RETRY)
}

fn shows_the_window(event: &WindowEvent) -> bool {
    matches!(
        event,
        WindowEvent::Occluded(false)
            | WindowEvent::Resized(_)
            | WindowEvent::Focused(true)
            | WindowEvent::CursorEntered { .. }
            | WindowEvent::ScaleFactorChanged { .. }
    )
}

fn surface_size(size: PhysicalSize<u32>) -> SurfaceSize {
    SurfaceSize {
        width: size.width,
        height: size.height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_frames_are_retried_later_and_later_up_to_a_second() {
        let delays: Vec<Duration> = (0..10).map(retry_delay).collect();

        assert_eq!(delays[0], FIRST_RETRY);
        assert!(delays.windows(2).all(|pair| pair[1] >= pair[0]));
        assert_eq!(delays[9], MAX_RETRY);
        assert_eq!(retry_delay(u32::MAX), MAX_RETRY);
    }
}
