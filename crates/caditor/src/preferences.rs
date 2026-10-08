use caditor_file::{Settings, SettingsError};
use caditor_render::{AdapterPreference, Msaa, Projection, Shading};
use egui::{Id, KeyboardShortcut, Label, ThemePreference, Ui};

use crate::{
    appearance::{self, MAX_SCALE, MIN_SCALE, SCALE_STEP, SPACE_M, SPACE_S},
    commands::{self, Command, Keymap},
    dialog_parts::{self, BodyRoom},
    graphics::{self, CurveQuality, FrameLimit, Graphics, Hardware},
    icons,
    layout::{PanelLayout, WindowPlacement},
    model::Notice,
    onboarding::{Hint, Onboarding},
    units::{AngleUnit, LengthUnit},
    widgets::{self, DialogWidth, Tab},
};

pub const MIN_SPEED: f64 = 0.25;
pub const MAX_SPEED: f64 = 4.0;
const UNIT_KEY: &str = "units.length";
const ANGLE_UNIT_KEY: &str = "units.angle";
const THEME_KEY: &str = "appearance.theme";
const SCALE_KEY: &str = "appearance.scale";
const HIGH_CONTRAST_KEY: &str = "appearance.high_contrast";
const ORBIT_KEY: &str = "navigation.orbit_speed";
const ZOOM_KEY: &str = "navigation.zoom_speed";
const INVERT_ZOOM_KEY: &str = "navigation.invert_zoom";
const INPUT_MODE_KEY: &str = "navigation.input_mode";
const PROJECTION_KEY: &str = "navigation.projection";
const TITLE_BAR_KEY: &str = "appearance.title_bar";
const DIALOG_HEIGHT_SHARE: f32 = 0.75;
const HEIGHT_CHANGE: f32 = 0.5;
const CONFIRM_KEY: &str = "preferences-confirm-defaults";
const TALLEST_KEY: &str = "preferences-tallest-body";
const RESTORE_DEFAULTS: &str = "Restore defaults";

pub fn unreadable_notice(error: &SettingsError) -> Notice {
    let cause = match error {
        SettingsError::Unreadable { .. } => "could not be read",
        SettingsError::Malformed { .. } => "is damaged",
    };
    Notice::failure(format!(
        "Your preferences file {cause}, so caditor started with its default settings and \
         shortcuts. A copy of it is kept beside it when you next change a preference."
    ))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Theme {
    #[default]
    System,
    Dark,
    Light,
}

impl Theme {
    pub const ALL: [Self; 3] = [Self::System, Self::Dark, Self::Light];

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "Follow the system",
            Self::Dark => "Dark",
            Self::Light => "Light",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::System => "Dark or light as the desktop is, changing when it changes",
            Self::Dark => "Light text on dark panels",
            Self::Light => "Dark text on light panels",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|theme| theme.key() == key)
    }

    pub fn egui(self) -> ThemePreference {
        match self {
            Self::System => ThemePreference::System,
            Self::Dark => ThemePreference::Dark,
            Self::Light => ThemePreference::Light,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TitleBar {
    #[default]
    BuiltIn,
    System,
}

impl TitleBar {
    pub const ALL: [Self; 2] = [Self::BuiltIn, Self::System];

    pub fn label(self) -> &'static str {
        match self {
            Self::BuiltIn => "caditor's",
            Self::System => "The system's",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::BuiltIn => {
                "The menu bar is the title bar: drag it to move the window, double-click it to \
                 maximize, and use its buttons to minimize, maximize and close"
            }
            Self::System => {
                "The window manager draws its own title bar above the menu bar; suits tiling \
                 window managers"
            }
        }
    }

    pub fn decorated(self) -> bool {
        self == Self::System
    }

    fn key(self) -> &'static str {
        match self {
            Self::BuiltIn => "built_in",
            Self::System => "system",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|bar| bar.key() == key)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PreferencesTab {
    #[default]
    General,
    Appearance,
    Navigation,
    Graphics,
}

impl PreferencesTab {
    pub const ALL: [Self; 4] = [
        Self::General,
        Self::Appearance,
        Self::Navigation,
        Self::Graphics,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Navigation => "Navigation",
            Self::Graphics => "Graphics",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::General => icons::GENERAL,
            Self::Appearance => icons::APPEARANCE,
            Self::Navigation => icons::NAVIGATION,
            Self::Graphics => icons::GRAPHICS,
        }
    }

    fn defaults(self) -> &'static str {
        match self {
            Self::General => {
                "Go back to millimetres. Only this tab changes; shortcuts are reset in the \
                 shortcut editor"
            }
            Self::Appearance => {
                "Go back to the system theme at normal size and contrast with caditor's title \
                 bar. Only this tab changes"
            }
            Self::Navigation => {
                "Go back to perspective, normal orbit and zoom speeds, and scrolling up to zoom \
                 in. Only this tab changes"
            }
            Self::Graphics => {
                "Go back to vsync on, matching the display's frame rate, 4× anti-aliasing, \
                 standard shading and smooth curves. Only this tab changes"
            }
        }
    }

    fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|tab| *tab == self)
            .unwrap_or_default()
    }
}

pub fn projection_label(projection: Projection) -> &'static str {
    match projection {
        Projection::Perspective => "Perspective",
        Projection::Orthographic => "Orthographic",
    }
}

fn projection_description(projection: Projection) -> &'static str {
    match projection {
        Projection::Perspective => "Farther parts look smaller, as they do to the eye",
        Projection::Orthographic => {
            "Parallel edges stay parallel and sizes compare across the view, as in a drawing"
        }
    }
}

fn projection_key(projection: Projection) -> &'static str {
    match projection {
        Projection::Perspective => "perspective",
        Projection::Orthographic => "orthographic",
    }
}

fn projection_from_key(key: &str) -> Option<Projection> {
    Projection::ALL
        .into_iter()
        .find(|projection| projection_key(*projection) == key)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InputMode {
    #[default]
    Caditor,
    Laptop,
    Fusion360,
    FreeCad,
    Blender,
}

impl InputMode {
    pub const ALL: [Self; 5] = [
        Self::Caditor,
        Self::Laptop,
        Self::Fusion360,
        Self::FreeCad,
        Self::Blender,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Caditor => "caditor",
            Self::Laptop => "Laptop",
            Self::Fusion360 => "Fusion 360",
            Self::FreeCad => "FreeCAD",
            Self::Blender => "Blender",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Caditor => {
                "For a mouse with three buttons: right-drag orbits, middle-drag or Shift+right-drag \
                 pans, and the wheel zooms"
            }
            Self::Laptop => {
                "For a touchpad: two fingers orbit, Alt and two fingers pan, and pinching or \
                 Ctrl and two fingers zoom; Alt-drag orbits and Shift+Alt-drag pans"
            }
            Self::Fusion360 => {
                "As in Fusion 360: middle-drag pans, Shift+middle-drag orbits and the wheel \
                 zooms; right-drag still orbits"
            }
            Self::FreeCad => {
                "As FreeCAD's CAD style: middle-drag pans, holding the middle button with the left \
                 or right one orbits and the wheel zooms; right-drag still orbits"
            }
            Self::Blender => {
                "As in Blender: middle-drag orbits, Shift+middle-drag pans, Ctrl+middle-drag and \
                 the wheel zoom; right-drag still orbits"
            }
        }
    }

    pub fn navigation_tip(self) -> &'static str {
        match self {
            Self::Caditor => "Right-drag to orbit, middle-drag to pan and scroll to zoom.",
            Self::Laptop => "Slide two fingers to orbit, hold Alt to pan and pinch to zoom.",
            Self::Fusion360 => "Shift+middle-drag to orbit, middle-drag to pan and scroll to zoom.",
            Self::FreeCad => {
                "Hold the middle button with the left or right one to orbit, middle-drag to pan \
                 and scroll to zoom."
            }
            Self::Blender => "Middle-drag to orbit, Shift+middle-drag to pan and scroll to zoom.",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Caditor => "caditor",
            Self::Laptop => "laptop",
            Self::Fusion360 => "fusion360",
            Self::FreeCad => "freecad",
            Self::Blender => "blender",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.key() == key)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Navigation {
    pub orbit_speed: f64,
    pub zoom_speed: f64,
    pub invert_zoom: bool,
    pub projection: Projection,
    pub input_mode: InputMode,
}

impl Default for Navigation {
    fn default() -> Self {
        Self {
            orbit_speed: 1.0,
            zoom_speed: 1.0,
            invert_zoom: false,
            projection: Projection::default(),
            input_mode: InputMode::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Appearance {
    pub theme: Theme,
    pub scale: f32,
    pub high_contrast: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            scale: 1.0,
            high_contrast: false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Preferences {
    pub unit: LengthUnit,
    pub angle: AngleUnit,
    pub appearance: Appearance,
    pub navigation: Navigation,
    pub title_bar: TitleBar,
    pub graphics: Graphics,
    pub onboarding: Onboarding,
    pub keymap: Keymap,
    pub window: WindowPlacement,
    pub panels: PanelLayout,
    loaded_keymap: Keymap,
    raw: Settings,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PreferenceChange {
    Unit(LengthUnit),
    Angle(AngleUnit),
    Theme(Theme),
    Scale(f32),
    HighContrast(bool),
    OrbitSpeed(f64),
    ZoomSpeed(f64),
    InvertZoom(bool),
    Projection(Projection),
    InputMode(InputMode),
    TitleBar(TitleBar),
    Vsync(bool),
    FrameLimit(FrameLimit),
    Msaa(Msaa),
    Shading(Shading),
    CurveQuality(CurveQuality),
    Adapter(AdapterPreference),
    Bind(Command, KeyboardShortcut),
    Unbind(Command, KeyboardShortcut),
    ResetShortcut(Command),
    ResetShortcuts,
    Welcomed,
    DismissHint(Hint),
    ShowHints(bool),
    RestoreHints,
    Defaults(PreferencesTab),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PreferencesCommand {
    Show,
    Hide,
    ShowShortcuts,
    HideShortcuts,
    ShowWelcome,
    CloseWelcome,
    ShowAbout,
    CloseAbout,
    ShowMessages,
    CloseMessages,
    ShowUndoHistory,
    CloseUndoHistory,
    ShowModelProperties,
    CloseModelProperties,
    ShowSavedViews,
    CloseSavedViews,
    GoToView(usize),
    Tab(PreferencesTab),
    Change(PreferenceChange),
    Preview(PreferenceChange),
    Undo,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Restored {
    Defaults {
        tab: PreferencesTab,
        before: Box<Preferences>,
    },
    Shortcuts(Box<Keymap>),
}

impl Restored {
    pub fn tab(&self) -> Option<PreferencesTab> {
        match self {
            Self::Defaults { tab, .. } => Some(*tab),
            Self::Shortcuts(_) => None,
        }
    }

    pub fn is_shortcuts(&self) -> bool {
        matches!(self, Self::Shortcuts(_))
    }
}

fn speed(value: Option<f64>) -> f64 {
    value.map_or(1.0, |value| value.clamp(MIN_SPEED, MAX_SPEED))
}

impl Preferences {
    pub fn from_settings(raw: Settings) -> Self {
        Self {
            unit: raw
                .text(UNIT_KEY)
                .and_then(LengthUnit::from_symbol)
                .unwrap_or_default(),
            angle: raw
                .text(ANGLE_UNIT_KEY)
                .and_then(AngleUnit::from_symbol)
                .unwrap_or_default(),
            appearance: Appearance {
                theme: raw
                    .text(THEME_KEY)
                    .and_then(Theme::from_key)
                    .unwrap_or_default(),
                scale: raw
                    .number(SCALE_KEY)
                    .map_or(1.0, |scale| appearance::clamp_scale(scale as f32)),
                high_contrast: raw.flag(HIGH_CONTRAST_KEY).unwrap_or(false),
            },
            navigation: Navigation {
                orbit_speed: speed(raw.number(ORBIT_KEY)),
                zoom_speed: speed(raw.number(ZOOM_KEY)),
                invert_zoom: raw.flag(INVERT_ZOOM_KEY).unwrap_or(false),
                projection: raw
                    .text(PROJECTION_KEY)
                    .and_then(projection_from_key)
                    .unwrap_or_default(),
                input_mode: raw
                    .text(INPUT_MODE_KEY)
                    .and_then(InputMode::from_key)
                    .unwrap_or_default(),
            },
            title_bar: raw
                .text(TITLE_BAR_KEY)
                .and_then(TitleBar::from_key)
                .unwrap_or_default(),
            graphics: Graphics::from_settings(&raw),
            onboarding: Onboarding::from_settings(&raw),
            keymap: Keymap::from_settings(&raw),
            window: WindowPlacement::from_settings(&raw),
            panels: PanelLayout::from_settings(&raw),
            loaded_keymap: Keymap::from_settings(&raw),
            raw,
        }
    }

    pub fn settings(&self) -> Settings {
        let mut settings = self.raw.clone();
        settings.set_text(UNIT_KEY, self.unit.symbol());
        settings.set_text(ANGLE_UNIT_KEY, self.angle.symbol());
        settings.set_text(THEME_KEY, self.appearance.theme.key());
        settings.set_number(SCALE_KEY, f64::from(self.appearance.scale));
        settings.set_flag(HIGH_CONTRAST_KEY, self.appearance.high_contrast);
        settings.set_number(ORBIT_KEY, self.navigation.orbit_speed);
        settings.set_number(ZOOM_KEY, self.navigation.zoom_speed);
        settings.set_flag(INVERT_ZOOM_KEY, self.navigation.invert_zoom);
        settings.set_text(PROJECTION_KEY, projection_key(self.navigation.projection));
        settings.set_text(INPUT_MODE_KEY, self.navigation.input_mode.key());
        settings.set_text(TITLE_BAR_KEY, self.title_bar.key());
        self.graphics.write(&mut settings);
        self.keymap.write(&self.loaded_keymap, &mut settings);
        self.onboarding.write(&mut settings);
        self.window.write(&mut settings);
        self.panels.write(&mut settings);
        settings
    }

    pub fn apply(&mut self, change: PreferenceChange) {
        match change {
            PreferenceChange::Unit(unit) => self.unit = unit,
            PreferenceChange::Angle(angle) => self.angle = angle,
            PreferenceChange::Theme(theme) => self.appearance.theme = theme,
            PreferenceChange::Scale(scale) => {
                self.appearance.scale = appearance::clamp_scale(scale);
            }
            PreferenceChange::HighContrast(on) => self.appearance.high_contrast = on,
            PreferenceChange::OrbitSpeed(value) => {
                self.navigation.orbit_speed = value.clamp(MIN_SPEED, MAX_SPEED);
            }
            PreferenceChange::ZoomSpeed(value) => {
                self.navigation.zoom_speed = value.clamp(MIN_SPEED, MAX_SPEED);
            }
            PreferenceChange::InvertZoom(invert) => self.navigation.invert_zoom = invert,
            PreferenceChange::Projection(projection) => self.navigation.projection = projection,
            PreferenceChange::InputMode(mode) => self.navigation.input_mode = mode,
            PreferenceChange::TitleBar(bar) => self.title_bar = bar,
            PreferenceChange::Vsync(vsync) => self.graphics.vsync = vsync,
            PreferenceChange::FrameLimit(limit) => self.graphics.frame_limit = limit,
            PreferenceChange::Msaa(msaa) => self.graphics.msaa = msaa,
            PreferenceChange::Shading(shading) => self.graphics.shading = shading,
            PreferenceChange::Adapter(adapter) => self.graphics.adapter = adapter,
            PreferenceChange::CurveQuality(curves) => self.graphics.curves = curves,
            PreferenceChange::Bind(command, shortcut) => self.keymap.bind(command, shortcut),
            PreferenceChange::Unbind(command, shortcut) => self.keymap.unbind(command, shortcut),
            PreferenceChange::ResetShortcut(command) => self.keymap.reset(command),
            PreferenceChange::ResetShortcuts => self.keymap.reset_all(),
            PreferenceChange::Welcomed => self.onboarding.welcomed = true,
            PreferenceChange::DismissHint(hint) => {
                self.onboarding.dismissed.insert(hint);
            }
            PreferenceChange::ShowHints(shown) => self.onboarding.hints = shown,
            PreferenceChange::RestoreHints => {
                self.onboarding.hints = true;
                self.onboarding.dismissed.clear();
            }
            PreferenceChange::Defaults(tab) => self.restore_defaults(tab),
        }
    }

    pub fn restoring(&self, change: PreferenceChange) -> Option<Restored> {
        match change {
            PreferenceChange::Defaults(tab) => Some(Restored::Defaults {
                tab,
                before: Box::new(self.clone()),
            }),
            PreferenceChange::ResetShortcuts => {
                Some(Restored::Shortcuts(Box::new(self.keymap.clone())))
            }
            _ => None,
        }
    }

    pub fn undo(&mut self, restored: Restored) {
        match restored {
            Restored::Defaults { tab, before } => self.copy_tab(tab, &before),
            Restored::Shortcuts(keymap) => self.keymap = *keymap,
        }
    }

    fn restore_defaults(&mut self, tab: PreferencesTab) {
        self.copy_tab(tab, &Self::default());
    }

    fn copy_tab(&mut self, tab: PreferencesTab, from: &Self) {
        match tab {
            PreferencesTab::General => {
                self.unit = from.unit;
                self.angle = from.angle;
            }
            PreferencesTab::Appearance => {
                self.appearance = from.appearance;
                self.title_bar = from.title_bar;
            }
            PreferencesTab::Navigation => self.navigation = from.navigation,
            PreferencesTab::Graphics => self.graphics = from.graphics,
        }
    }
}

pub struct PreferencesView<'a> {
    pub tab: PreferencesTab,
    pub hardware: &'a Hardware,
    pub switch_keys: bool,
    pub restored: Option<&'a Restored>,
}

pub fn dialog(
    ctx: &egui::Context,
    preferences: &Preferences,
    view: &PreferencesView<'_>,
) -> Option<PreferencesCommand> {
    let confirm = Id::new(CONFIRM_KEY);
    let response = widgets::dialog(ctx, "preferences", "Preferences", DialogWidth::Wide, |ui| {
        let mut command = None;
        let tabs = PreferencesTab::ALL.map(|tab| Tab {
            glyph: tab.icon(),
            label: tab.label(),
        });
        let mut tab = view.tab;
        if let Some(chosen) = widgets::tabs(ui, "preferences", &tabs, tab.index(), view.switch_keys)
            .and_then(|index| PreferencesTab::ALL.get(index).copied())
        {
            tab = chosen;
            command = Some(PreferencesCommand::Tab(chosen));
            ui.data_mut(|data| data.remove::<PreferencesTab>(confirm));
        }
        let mut room = BodyRoom::measure(ui, "preferences", DIALOG_HEIGHT_SHARE);
        body(ui, preferences, view, tab, room.height, &mut command);
        room.body_ended(ui);
        if view.restored.and_then(Restored::tab) == Some(tab)
            && dialog_parts::undo_note(
                ui,
                "This tab is back to its defaults.",
                "Put back the settings this tab had before",
            )
        {
            command = Some(PreferencesCommand::Undo);
        }
        let confirming = ui.data(|data| data.get_temp::<PreferencesTab>(confirm)) == Some(tab);
        if confirming {
            ui.add_space(SPACE_M);
            dialog_parts::confirmation(
                ui,
                &format!("Restore the defaults of the {} tab?", tab.label()),
                tab.defaults(),
            );
            if let Some(confirmed) = dialog_parts::confirm_footer(ui, RESTORE_DEFAULTS, "Cancel") {
                ui.data_mut(|data| data.remove::<PreferencesTab>(confirm));
                if confirmed {
                    command = Some(PreferencesCommand::Change(PreferenceChange::Defaults(tab)));
                }
            }
        } else {
            let chosen = widgets::footer_split(
                ui,
                |ui| {
                    ui.add(widgets::danger_button(RESTORE_DEFAULTS))
                        .on_hover_text(tab.defaults())
                        .clicked()
                        .then_some(false)
                },
                |ui| {
                    ui.add(widgets::primary_button(ui, "Close"))
                        .clicked()
                        .then_some(true)
                },
            );
            match chosen {
                Some(true) => command = Some(PreferencesCommand::Hide),
                Some(false) => ui.data_mut(|data| {
                    data.insert_temp(confirm, tab);
                }),
                None => {}
            }
        }
        room.dialog_ended(ui);
        command
    });
    let closed = response.should_close().then_some(PreferencesCommand::Hide);
    let command = response.inner.or(closed);
    if command == Some(PreferencesCommand::Hide) {
        ctx.data_mut(|data| data.remove::<PreferencesTab>(confirm));
    }
    command
}

fn body(
    ui: &mut Ui,
    preferences: &Preferences,
    view: &PreferencesView<'_>,
    tab: PreferencesTab,
    room: f32,
    command: &mut Option<PreferencesCommand>,
) {
    let cap = room;
    let tallest_id = Id::new(TALLEST_KEY);
    let tallest = ui
        .data(|data| data.get_temp::<f32>(tallest_id))
        .unwrap_or_default()
        .min(cap);
    let shown = egui::ScrollArea::vertical()
        .id_salt(("preferences", tab.label()))
        .max_height(cap)
        .min_scrolled_height(tallest)
        .show(ui, |ui| {
            ui.set_min_height(tallest);
            ui.scope(|ui| match tab {
                PreferencesTab::General => general(ui, preferences, command),
                PreferencesTab::Appearance => appearance(ui, preferences, command),
                PreferencesTab::Navigation => navigation(ui, preferences, command),
                PreferencesTab::Graphics => {
                    graphics::tab(ui, &preferences.graphics, view.hardware, command);
                }
            })
            .response
            .rect
            .height()
        });
    let height = shown.inner.min(cap);
    if height > tallest + HEIGHT_CHANGE {
        ui.data_mut(|data| data.insert_temp(tallest_id, height));
        ui.ctx().request_repaint();
    }
}

pub fn section(
    ui: &mut Ui,
    title: &str,
    id: &str,
    note: Option<String>,
    rows: impl FnOnce(&mut Ui),
) {
    ui.add_space(SPACE_M);
    widgets::section(ui, &format!("preferences-{id}"), title, None, None, |ui| {
        widgets::card(ui, |ui| {
            widgets::properties(ui, id, rows);
            if let Some(note) = note {
                ui.add_space(SPACE_S);
                ui.add(Label::new(widgets::muted(note, ui)).wrap());
            }
        });
    });
}

pub fn change(command: &mut Option<PreferencesCommand>, change: PreferenceChange) {
    *command = Some(PreferencesCommand::Change(change));
}

pub fn choice<T: Copy + PartialEq>(
    ui: &mut Ui,
    options: &[(T, &str, &str)],
    current: T,
) -> Option<T> {
    let selected = options
        .iter()
        .position(|(value, _, _)| *value == current)
        .unwrap_or(usize::MAX);
    let labels: Vec<(&str, &str)> = options
        .iter()
        .map(|(_, label, hover)| (*label, *hover))
        .collect();
    widgets::segmented(ui, &labels, selected)
        .and_then(|index| options.get(index))
        .map(|(value, _, _)| *value)
}

fn speed_slider(
    ui: &mut Ui,
    current: f64,
    make: fn(f64) -> PreferenceChange,
    command: &mut Option<PreferencesCommand>,
) {
    let mut speed = current;
    let response = ui.add(egui::Slider::new(&mut speed, MIN_SPEED..=MAX_SPEED).logarithmic(true));
    widgets::tie_to_caption(ui, &response);
    let settled = response.drag_stopped() || !response.dragged();
    if settled && (response.changed() || response.drag_stopped()) {
        change(command, make(speed));
    } else if response.changed() {
        *command = Some(PreferencesCommand::Preview(make(speed)));
    }
}

fn general(ui: &mut Ui, preferences: &Preferences, command: &mut Option<PreferencesCommand>) {
    units(ui, preferences, command);
    keyboard(ui, command);
    tips(ui, preferences, command);
}

fn units(ui: &mut Ui, preferences: &Preferences, command: &mut Option<PreferencesCommand>) {
    let unit = preferences.unit.label().to_lowercase();
    let angle = preferences.angle.label().to_lowercase();
    let note = format!(
        "Lengths are shown in {unit} and plain numbers typed for a length mean {unit}; angles are \
         shown in {angle} and plain numbers typed for an angle mean {angle}. Values already in the \
         model keep the units they were entered in."
    );
    let hovers = LengthUnit::ALL.map(|unit| {
        format!(
            "Show lengths in {} ({})",
            unit.label().to_lowercase(),
            unit.symbol()
        )
    });
    let angle_hovers = AngleUnit::ALL.map(|unit| {
        format!(
            "Show angles in {} ({})",
            unit.label().to_lowercase(),
            unit.symbol()
        )
    });
    section(ui, "Units", "units", Some(note), |ui| {
        widgets::property(ui, "Length", |ui| {
            let options: Vec<(LengthUnit, &str, &str)> = LengthUnit::ALL
                .iter()
                .zip(&hovers)
                .map(|(unit, hover)| (*unit, unit.label(), hover.as_str()))
                .collect();
            if let Some(unit) = choice(ui, &options, preferences.unit) {
                change(command, PreferenceChange::Unit(unit));
            }
        });
        widgets::property(ui, "Angle", |ui| {
            let options: Vec<(AngleUnit, &str, &str)> = AngleUnit::ALL
                .iter()
                .zip(&angle_hovers)
                .map(|(unit, hover)| (*unit, unit.label(), hover.as_str()))
                .collect();
            if let Some(unit) = choice(ui, &options, preferences.angle) {
                change(command, PreferenceChange::Angle(unit));
            }
        });
    });
}

fn keyboard(ui: &mut Ui, command: &mut Option<PreferencesCommand>) {
    let note = "Search commands finds every command by name, whether or not it has a shortcut.";
    section(ui, "Keyboard", "keyboard", Some(note.to_owned()), |ui| {
        widgets::property(ui, "Shortcuts", |ui| {
            let button = widgets::small_button(
                ui,
                icons::command(Command::KeyboardShortcuts),
                &Command::KeyboardShortcuts.title(),
            );
            if ui
                .add(button)
                .on_hover_text("See every shortcut and change any of them")
                .clicked()
            {
                *command = Some(PreferencesCommand::ShowShortcuts);
            }
        });
    });
}

fn tips(ui: &mut Ui, preferences: &Preferences, command: &mut Option<PreferencesCommand>) {
    let onboarding = &preferences.onboarding;
    section(ui, "Tips", "tips", None, |ui| {
        widgets::property(ui, "Getting started", |ui| {
            let mut shown = onboarding.hints;
            if ui.checkbox(&mut shown, "Show tips in the view").changed() {
                change(command, PreferenceChange::ShowHints(shown));
            }
        });
        ui.label("");
        let restore = ui
            .add_enabled(
                !onboarding.dismissed.is_empty(),
                widgets::button("Show dismissed tips again"),
            )
            .on_hover_text("Bring back the tips dismissed with Got it")
            .on_disabled_hover_text("No tip has been dismissed")
            .clicked();
        if restore {
            change(command, PreferenceChange::RestoreHints);
        }
        ui.end_row();
    });
}

fn appearance(ui: &mut Ui, preferences: &Preferences, command: &mut Option<PreferencesCommand>) {
    let current = preferences.appearance;
    let note = "Panels and menus follow the theme; the 3D view keeps its dark background.";
    section(ui, "Colours", "colours", Some(note.to_owned()), |ui| {
        widgets::property(ui, "Theme", |ui| {
            let options = Theme::ALL.map(|theme| (theme, theme.label(), theme.description()));
            if let Some(theme) = choice(ui, &options, current.theme) {
                change(command, PreferenceChange::Theme(theme));
            }
        });
        widgets::property(ui, "Contrast", |ui| {
            let mut high_contrast = current.high_contrast;
            if ui
                .checkbox(&mut high_contrast, "High contrast")
                .on_hover_text("Stronger text, outlined buttons and a bright focus outline")
                .changed()
            {
                change(command, PreferenceChange::HighContrast(high_contrast));
            }
        });
    });
    section(ui, "Interface", "interface", None, |ui| {
        widgets::property(ui, "Size", |ui| {
            let smaller = Command::SmallerInterface.title();
            let larger = Command::LargerInterface.title();
            let step = widgets::stepper(
                ui,
                &format!("{:.0}%", current.scale * 100.0),
                (&smaller, current.scale > MIN_SCALE),
                (&larger, current.scale < MAX_SCALE),
            );
            if let Some(step) = step {
                let scale = current.scale + f32::from(step) * SCALE_STEP;
                change(command, PreferenceChange::Scale(scale));
            }
        });
        widgets::property(ui, "Title bar", |ui| {
            let options = TitleBar::ALL.map(|bar| (bar, bar.label(), bar.description()));
            if let Some(bar) = choice(ui, &options, preferences.title_bar) {
                change(command, PreferenceChange::TitleBar(bar));
            }
        });
    });
}

fn navigation(ui: &mut Ui, preferences: &Preferences, command: &mut Option<PreferencesCommand>) {
    let navigation = preferences.navigation;
    let toggle = preferences
        .keymap
        .first(Command::ToggleProjection)
        .map_or_else(
            || Command::ToggleProjection.title(),
            |shortcut| commands::display(&shortcut),
        );
    let note = format!("{toggle} switches between them in the view.");
    section(ui, "View", "navigation-view", Some(note), |ui| {
        widgets::property(ui, "Projection", |ui| {
            let options = Projection::ALL.map(|projection| {
                (
                    projection,
                    projection_label(projection),
                    projection_description(projection),
                )
            });
            if let Some(projection) = choice(ui, &options, navigation.projection) {
                change(command, PreferenceChange::Projection(projection));
            }
        });
    });
    section(ui, "Input", "navigation-input", None, |ui| {
        widgets::property(ui, "Input mode", |ui| {
            let options = InputMode::ALL.map(|mode| (mode, mode.label(), mode.description()));
            if let Some(mode) = choice(ui, &options, navigation.input_mode) {
                change(command, PreferenceChange::InputMode(mode));
            }
        });
        ui.label("");
        ui.add(Label::new(widgets::muted(navigation.input_mode.description(), ui)).wrap());
        ui.end_row();
    });
    section(ui, "Movement", "navigation", None, |ui| {
        widgets::property(ui, "Orbit speed", |ui| {
            speed_slider(
                ui,
                navigation.orbit_speed,
                PreferenceChange::OrbitSpeed,
                command,
            );
        });
        widgets::property(ui, "Zoom speed", |ui| {
            speed_slider(
                ui,
                navigation.zoom_speed,
                PreferenceChange::ZoomSpeed,
                command,
            );
        });
        widgets::property(ui, "Scrolling", |ui| {
            let mut invert = navigation.invert_zoom;
            if ui.checkbox(&mut invert, "Scroll up to zoom out").changed() {
                change(command, PreferenceChange::InvertZoom(invert));
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn a_damaged_preferences_file_is_reported_with_what_to_expect() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("preferences.json"), "{ not json").unwrap();

        let (settings, problem) = Settings::load_reporting(dir.path());
        let notice = unreadable_notice(&problem.unwrap());

        assert_eq!(settings, Settings::default());
        assert!(notice.outlasts_edits);
        assert_eq!(
            notice.text,
            "Your preferences file is damaged, so caditor started with its default settings and \
             shortcuts. A copy of it is kept beside it when you next change a preference."
        );
    }

    #[test]
    fn preferences_read_their_settings_and_keep_the_rest() {
        let mut raw = Settings::default();
        raw.set_text(UNIT_KEY, "cm");
        raw.set_text(THEME_KEY, "light");
        raw.set_number(ORBIT_KEY, 100.0);
        raw.set_text("future.option", "kept");
        raw.set_number(SCALE_KEY, 1.3);
        raw.set_flag(HIGH_CONTRAST_KEY, true);
        let mut preferences = Preferences::from_settings(raw);
        assert_eq!(preferences.unit, LengthUnit::Centimetre);
        assert_eq!(preferences.appearance.theme, Theme::Light);
        assert_eq!(preferences.appearance.scale, 1.25);
        assert!(preferences.appearance.high_contrast);
        assert_eq!(preferences.navigation.orbit_speed, MAX_SPEED);
        assert_eq!(preferences.navigation.zoom_speed, 1.0);
        preferences.apply(PreferenceChange::ZoomSpeed(0.5));
        preferences.apply(PreferenceChange::InvertZoom(true));
        preferences.apply(PreferenceChange::Projection(Projection::Orthographic));
        preferences.apply(PreferenceChange::TitleBar(TitleBar::System));
        let settings = preferences.settings();
        assert_eq!(settings.text("future.option"), Some("kept"));
        assert_eq!(settings.number(ZOOM_KEY), Some(0.5));
        assert_eq!(settings.text(PROJECTION_KEY), Some("orthographic"));
        assert_eq!(settings.text(TITLE_BAR_KEY), Some("system"));
        assert_eq!(
            Preferences::from_settings(settings.clone()).title_bar,
            TitleBar::System
        );
        assert_eq!(
            Preferences::from_settings(settings.clone())
                .navigation
                .projection,
            Projection::Orthographic
        );
        assert_eq!(
            Preferences::from_settings(settings.clone()).settings(),
            settings
        );
        preferences.apply(PreferenceChange::Defaults(PreferencesTab::Navigation));
        assert_eq!(preferences.navigation, Navigation::default());
        assert_eq!(preferences.unit, LengthUnit::Centimetre);
        assert_eq!(preferences.title_bar, TitleBar::System);
        for tab in PreferencesTab::ALL {
            preferences.apply(PreferenceChange::Defaults(tab));
        }
        assert_eq!(preferences.unit, LengthUnit::Millimetre);
        assert_eq!(preferences.appearance, Appearance::default());
        assert_eq!(preferences.title_bar, TitleBar::BuiltIn);
        assert_eq!(preferences.graphics, Graphics::default());
    }

    #[test]
    fn graphics_preferences_are_stored_beside_the_others_and_restored_by_their_tab() {
        let mut preferences = Preferences::from_settings(Settings::default());
        preferences.apply(PreferenceChange::Vsync(false));
        preferences.apply(PreferenceChange::FrameLimit(FrameLimit::Fps120));
        preferences.apply(PreferenceChange::Msaa(Msaa::X8));
        preferences.apply(PreferenceChange::Shading(Shading::Enhanced));
        preferences.apply(PreferenceChange::CurveQuality(CurveQuality::Coarse));
        preferences.apply(PreferenceChange::Adapter(AdapterPreference::Performance));
        preferences.apply(PreferenceChange::Unit(LengthUnit::Metre));
        preferences.apply(PreferenceChange::Angle(AngleUnit::Radian));

        let settings = preferences.settings();
        let read = Preferences::from_settings(settings.clone());

        assert_eq!(settings.flag("graphics.vsync"), Some(false));
        assert_eq!(settings.text("graphics.frame_limit"), Some("120"));
        assert_eq!(settings.number("graphics.msaa"), Some(8.0));
        assert_eq!(settings.text("graphics.shading"), Some("enhanced"));
        assert_eq!(settings.text("graphics.curve_quality"), Some("coarse"));
        assert_eq!(settings.text("graphics.adapter"), Some("performance"));
        assert_eq!(settings.text("units.angle"), Some("rad"));
        assert_eq!(read.graphics, preferences.graphics);
        assert_eq!(read.angle, AngleUnit::Radian);

        preferences.apply(PreferenceChange::Defaults(PreferencesTab::Graphics));
        assert_eq!(preferences.graphics, Graphics::default());
        assert_eq!(preferences.unit, LengthUnit::Metre);
        assert_eq!(preferences.angle, AngleUnit::Radian);
        preferences.apply(PreferenceChange::Defaults(PreferencesTab::General));
        assert_eq!(preferences.angle, AngleUnit::Degree);
    }
}
