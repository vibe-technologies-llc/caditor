use caditor_document::Rgb;
use caditor_file::{Settings, SettingsError};
use caditor_render::{AdapterPreference, Msaa, ProjectionMode, Shading};
use egui::KeyboardShortcut;

#[cfg(test)]
pub use self::dialog::{
    AUTOMATIC_GRID, CUSTOM_GRID, GRID_SELECTOR, accent_name, custom_grid_name, theme_card_id,
    view_name,
};
pub use self::dialog::{PreferencesView, change, choice, dialog, section};
pub use crate::themes::{Accent, Appearance, GridColour, Theme, ViewChoice};
use crate::{
    appearance::{self},
    body_appearance::SWATCHES,
    commands::{Command, Keymap},
    files::templates::{self},
    graphics::{CurveQuality, FrameLimit, Graphics},
    icons,
    last_values::{LastValues, Remembered},
    layout::{PanelLayout, WindowPlacement},
    model::Notice,
    onboarding::{Hint, Onboarding},
    palette,
    units::{AngleUnit, LengthUnit},
    visibility::PrincipalPreference,
};

pub const MIN_SPEED: f64 = 0.25;
pub const MAX_SPEED: f64 = 4.0;
const UNIT_KEY: &str = "units.length";
const ANGLE_UNIT_KEY: &str = "units.angle";
const THEME_KEY: &str = "appearance.theme";
const ACCENT_KEY: &str = "appearance.accent";
const VIEW_KEY: &str = "appearance.view";
const GRID_KEY: &str = "appearance.grid";
const SCALE_KEY: &str = "appearance.scale";
const HIGH_CONTRAST_KEY: &str = "appearance.high_contrast";
const ORBIT_KEY: &str = "navigation.orbit_speed";
const ZOOM_KEY: &str = "navigation.zoom_speed";
const INVERT_ZOOM_KEY: &str = "navigation.invert_zoom";
const INPUT_MODE_KEY: &str = "navigation.input_mode";
const PROJECTION_KEY: &str = "navigation.projection";
const PRINCIPAL_WHEN_NEEDED_KEY: &str = "navigation.principal_when_needed";
const TITLE_BAR_KEY: &str = "appearance.title_bar";
const PALETTE_RECENT_KEY: &str = "palette.recent";
const RECENT_COLOURS_KEY: &str = "body.recent_colours";
pub const RECENT_COLOURS_LIMIT: usize = 6;

mod dialog;

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
                "Go back to millimetres and to new models starting empty. Only this tab \
                 changes; shortcuts are reset in the shortcut editor"
            }
            Self::Appearance => {
                "Go back to the system theme with its own accent and 3D view, at normal size and \
                 contrast with caditor's title bar. Only this tab changes"
            }
            Self::Navigation => {
                "Go back to perspective, principal planes shown only when needed, normal orbit \
                 and zoom speeds, and scrolling up to zoom in. Only this tab changes"
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

pub fn projection_label(projection: ProjectionMode) -> &'static str {
    match projection {
        ProjectionMode::Perspective => "Perspective",
        ProjectionMode::Orthographic => "Orthographic",
        ProjectionMode::Automatic => "Automatic",
    }
}

fn projection_description(projection: ProjectionMode) -> &'static str {
    match projection {
        ProjectionMode::Perspective => "Farther parts look smaller, as they do to the eye",
        ProjectionMode::Orthographic => {
            "Parallel edges stay parallel and sizes compare across the view, as in a drawing"
        }
        ProjectionMode::Automatic => {
            "Perspective, turning orthographic while the view looks square at a front, back, top, bottom, left or right view"
        }
    }
}

fn projection_key(projection: ProjectionMode) -> &'static str {
    match projection {
        ProjectionMode::Perspective => "perspective",
        ProjectionMode::Orthographic => "orthographic",
        ProjectionMode::Automatic => "automatic",
    }
}

fn projection_from_key(key: &str) -> Option<ProjectionMode> {
    ProjectionMode::ALL
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

    pub fn navigation_hint(self) -> &'static str {
        match self {
            Self::Caditor => {
                "Right-drag: orbit   Middle-drag or Shift+right-drag: pan   Scroll: zoom"
            }
            Self::Laptop => {
                "Two-finger-scroll or Alt-drag: orbit   Alt+two-finger-scroll or Shift+Alt-drag: \
                 pan   Pinch or Ctrl+two-finger-scroll: zoom"
            }
            Self::Fusion360 => {
                "Shift+middle-drag or right-drag: orbit   Middle-drag or Shift+right-drag: pan   \
                 Scroll: zoom"
            }
            Self::FreeCad => {
                "Middle+left-drag or right-drag: orbit   Middle-drag or Shift+right-drag: pan   \
                 Scroll: zoom"
            }
            Self::Blender => {
                "Middle-drag or right-drag: orbit   Shift+middle-drag or Shift+right-drag: pan   \
                 Scroll or Ctrl+middle-drag: zoom"
            }
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
    pub projection: ProjectionMode,
    pub input_mode: InputMode,
}

impl Default for Navigation {
    fn default() -> Self {
        Self {
            orbit_speed: 1.0,
            zoom_speed: 1.0,
            invert_zoom: false,
            projection: ProjectionMode::default(),
            input_mode: InputMode::default(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Preferences {
    pub unit: LengthUnit,
    pub angle: AngleUnit,
    pub appearance: Appearance,
    pub navigation: Navigation,
    pub principal: PrincipalPreference,
    pub title_bar: TitleBar,
    pub graphics: Graphics,
    pub onboarding: Onboarding,
    pub keymap: Keymap,
    pub window: WindowPlacement,
    pub panels: PanelLayout,
    pub default_template: Option<String>,
    pub palette_recent: Vec<Command>,
    pub recent_colours: Vec<Rgb>,
    pub last_values: LastValues,
    loaded_keymap: Keymap,
    raw: Settings,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PreferenceChange {
    Unit(LengthUnit),
    Angle(AngleUnit),
    Theme(Theme),
    Accent(Accent),
    View(ViewChoice),
    Grid(GridColour),
    Scale(f32),
    HighContrast(bool),
    OrbitSpeed(f64),
    ZoomSpeed(f64),
    InvertZoom(bool),
    Projection(ProjectionMode),
    PrincipalWhenNeeded(bool),
    InputMode(InputMode),
    TitleBar(TitleBar),
    DefaultTemplate(Option<String>),
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
    DefenderReminded,
    RecentColour(Rgb),
    Defaults(PreferencesTab),
}

#[derive(Debug, Clone, PartialEq)]
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
    ShowSelectionSets,
    CloseSelectionSets,
    ShowScaleModel,
    CloseScaleModel,
    ShowConfigurations,
    CloseConfigurations,
    ShowDefenderReminder,
    ReloadThemes,
    SelectSet(usize),
    Tab(PreferencesTab),
    Change(PreferenceChange),
    Preview(PreferenceChange),
    RememberRecent(Vec<Command>),
    RememberLast(Vec<(Remembered, String)>),
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

fn palette_recent(raw: &Settings) -> Vec<Command> {
    let mut recent = Vec::new();
    let stored = raw.texts(PALETTE_RECENT_KEY).unwrap_or_default();
    for command in stored.iter().filter_map(|id| Command::from_id(id)) {
        if recent.len() < palette::RECENT_LIMIT && !recent.contains(&command) {
            recent.push(command);
        }
    }
    recent
}

fn recent_colours(raw: &Settings) -> Vec<Rgb> {
    let mut recent = Vec::new();
    let stored = raw.texts(RECENT_COLOURS_KEY).unwrap_or_default();
    for colour in stored.iter().filter_map(|hex| Rgb::from_hex(hex)) {
        if recent.len() < RECENT_COLOURS_LIMIT && !recent.contains(&colour) {
            recent.push(colour);
        }
    }
    recent
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
                accent: raw
                    .text(ACCENT_KEY)
                    .and_then(Accent::from_key)
                    .unwrap_or_default(),
                view: raw
                    .text(VIEW_KEY)
                    .and_then(ViewChoice::from_key)
                    .unwrap_or_default(),
                grid: raw
                    .text(GRID_KEY)
                    .and_then(GridColour::from_key)
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
            principal: raw
                .flag(PRINCIPAL_WHEN_NEEDED_KEY)
                .map(PrincipalPreference::from_flag)
                .unwrap_or_default(),
            title_bar: raw
                .text(TITLE_BAR_KEY)
                .and_then(TitleBar::from_key)
                .unwrap_or_default(),
            graphics: Graphics::from_settings(&raw),
            onboarding: Onboarding::from_settings(&raw),
            keymap: Keymap::from_settings(&raw),
            window: WindowPlacement::from_settings(&raw),
            panels: PanelLayout::from_settings(&raw),
            default_template: templates::default_template(&raw),
            palette_recent: palette_recent(&raw),
            recent_colours: recent_colours(&raw),
            last_values: LastValues::from_settings(&raw),
            loaded_keymap: Keymap::from_settings(&raw),
            raw,
        }
    }

    pub fn settings(&self) -> Settings {
        let mut settings = self.raw.clone();
        settings.set_text(UNIT_KEY, self.unit.symbol());
        settings.set_text(ANGLE_UNIT_KEY, self.angle.symbol());
        settings.set_text(THEME_KEY, &self.appearance.theme.key());
        settings.set_text(ACCENT_KEY, self.appearance.accent.key());
        settings.set_text(VIEW_KEY, self.appearance.view.key());
        settings.set_text(GRID_KEY, &self.appearance.grid.key());
        settings.set_number(SCALE_KEY, f64::from(self.appearance.scale));
        settings.set_flag(HIGH_CONTRAST_KEY, self.appearance.high_contrast);
        settings.set_number(ORBIT_KEY, self.navigation.orbit_speed);
        settings.set_number(ZOOM_KEY, self.navigation.zoom_speed);
        settings.set_flag(INVERT_ZOOM_KEY, self.navigation.invert_zoom);
        settings.set_text(PROJECTION_KEY, projection_key(self.navigation.projection));
        settings.set_text(INPUT_MODE_KEY, self.navigation.input_mode.key());
        settings.set_flag(PRINCIPAL_WHEN_NEEDED_KEY, self.principal.when_needed());
        settings.set_text(TITLE_BAR_KEY, self.title_bar.key());
        self.graphics.write(&mut settings);
        self.keymap.write(&self.loaded_keymap, &mut settings);
        self.onboarding.write(&mut settings);
        self.window.write(&mut settings);
        self.panels.write(&mut settings);
        templates::write_default_template(self.default_template.as_deref(), &mut settings);
        if !self.palette_recent.is_empty() {
            let ids: Vec<String> = self
                .palette_recent
                .iter()
                .map(|command| command.id().to_owned())
                .collect();
            settings.set_texts(PALETTE_RECENT_KEY, &ids);
        }
        if !self.recent_colours.is_empty() {
            let hexes: Vec<String> = self
                .recent_colours
                .iter()
                .map(|colour| colour.hex())
                .collect();
            settings.set_texts(RECENT_COLOURS_KEY, &hexes);
        }
        self.last_values.write(&mut settings);
        settings
    }

    pub fn apply(&mut self, change: PreferenceChange) {
        match change {
            PreferenceChange::Unit(unit) => self.unit = unit,
            PreferenceChange::Angle(angle) => self.angle = angle,
            PreferenceChange::Theme(theme) => self.appearance.theme = theme,
            PreferenceChange::Accent(accent) => self.appearance.accent = accent,
            PreferenceChange::View(view) => self.appearance.view = view,
            PreferenceChange::Grid(grid) => self.appearance.grid = grid,
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
            PreferenceChange::PrincipalWhenNeeded(when_needed) => {
                self.principal = PrincipalPreference::from_flag(when_needed);
            }
            PreferenceChange::TitleBar(bar) => self.title_bar = bar,
            PreferenceChange::DefaultTemplate(name) => self.default_template = name,
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
            PreferenceChange::DefenderReminded => self.onboarding.defender_reminded = true,
            PreferenceChange::RecentColour(colour) => self.remember_colour(colour),
            PreferenceChange::RestoreHints => {
                self.onboarding.hints = true;
                self.onboarding.dismissed.clear();
            }
            PreferenceChange::Defaults(tab) => self.restore_defaults(tab),
        }
    }

    fn remember_colour(&mut self, colour: Rgb) {
        if SWATCHES.iter().any(|swatch| swatch.colour == colour) {
            return;
        }
        self.recent_colours.retain(|kept| *kept != colour);
        self.recent_colours.insert(0, colour);
        self.recent_colours.truncate(RECENT_COLOURS_LIMIT);
    }

    pub fn restoring(&self, change: &PreferenceChange) -> Option<Restored> {
        match change {
            PreferenceChange::Defaults(tab) => Some(Restored::Defaults {
                tab: *tab,
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
                self.default_template.clone_from(&from.default_template);
            }
            PreferencesTab::Appearance => {
                self.appearance = from.appearance.clone();
                self.title_bar = from.title_bar;
            }
            PreferencesTab::Navigation => {
                self.navigation = from.navigation;
                self.principal = from.principal;
            }
            PreferencesTab::Graphics => self.graphics = from.graphics,
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::scene_palette::Canvas;

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
    fn the_palette_remembers_its_recent_commands_skipping_unknown_ones() {
        let mut raw = Settings::default();
        raw.set_texts(
            PALETTE_RECENT_KEY,
            &[
                Command::FitView.id().to_owned(),
                "future.command".to_owned(),
                Command::Save.id().to_owned(),
                Command::FitView.id().to_owned(),
            ],
        );

        let preferences = Preferences::from_settings(raw);

        assert_eq!(
            preferences.palette_recent,
            [Command::FitView, Command::Save]
        );
        let again = Preferences::from_settings(preferences.settings());
        assert_eq!(again.palette_recent, preferences.palette_recent);

        let many: Vec<String> = Command::all()
            .take(palette::RECENT_LIMIT + 3)
            .map(|command| command.id().to_owned())
            .collect();
        let mut raw = Settings::default();
        raw.set_texts(PALETTE_RECENT_KEY, &many);
        assert_eq!(
            Preferences::from_settings(raw).palette_recent.len(),
            palette::RECENT_LIMIT
        );
    }

    #[test]
    fn recent_custom_colours_are_kept_newest_first_without_the_fixed_swatches() {
        let mut preferences = Preferences::default();
        let fixed = SWATCHES[0].colour;

        for step in 1..=RECENT_COLOURS_LIMIT as u8 + 2 {
            preferences.apply(PreferenceChange::RecentColour(Rgb::new(step, 0, 0)));
        }
        preferences.apply(PreferenceChange::RecentColour(Rgb::new(5, 0, 0)));
        preferences.apply(PreferenceChange::RecentColour(fixed));

        let expected: Vec<Rgb> = [5, 8, 7, 6, 4, 3]
            .into_iter()
            .map(|red| Rgb::new(red, 0, 0))
            .collect();

        assert_eq!(preferences.recent_colours, expected);
        assert_eq!(
            Preferences::from_settings(preferences.settings()).recent_colours,
            expected
        );
    }

    #[test]
    fn stored_recent_colours_skip_what_cannot_be_read_and_older_files_have_none() {
        let mut raw = Settings::default();
        raw.set_texts(
            RECENT_COLOURS_KEY,
            &[
                "#336699".to_owned(),
                "not a colour".to_owned(),
                "#336699".to_owned(),
                "#f80".to_owned(),
            ],
        );

        let preferences = Preferences::from_settings(raw);

        assert_eq!(
            preferences.recent_colours,
            [Rgb::new(51, 102, 153), Rgb::new(255, 136, 0)]
        );
        assert!(
            Preferences::from_settings(Settings::default())
                .recent_colours
                .is_empty()
        );
        assert_eq!(
            Preferences::default().settings().texts(RECENT_COLOURS_KEY),
            None
        );
    }

    #[test]
    fn themes_accents_and_the_view_are_stored_and_older_files_read_as_before() {
        let mut older = Settings::default();
        older.set_text(THEME_KEY, "light");
        let read = Preferences::from_settings(older);

        assert_eq!(read.appearance.theme, Theme::Light);
        assert_eq!(read.appearance.accent, Accent::Theme);
        assert_eq!(read.appearance.view, ViewChoice::Theme);

        let mut preferences = Preferences::default();
        preferences.apply(PreferenceChange::Theme(Theme::User("ocean".to_owned())));
        preferences.apply(PreferenceChange::Accent(Accent::Rose));
        preferences.apply(PreferenceChange::View(ViewChoice::Canvas(Canvas::Studio)));
        preferences.apply(PreferenceChange::Grid(GridColour::Custom(Rgb::new(
            0x8a, 0x90, 0x9c,
        ))));
        let settings = preferences.settings();
        let again = Preferences::from_settings(settings.clone());

        assert_eq!(settings.text(THEME_KEY), Some("user:ocean"));
        assert_eq!(settings.text(ACCENT_KEY), Some("rose"));
        assert_eq!(settings.text(VIEW_KEY), Some("studio"));
        assert_eq!(settings.text(GRID_KEY), Some("#8a909c"));
        assert_eq!(again.appearance, preferences.appearance);

        let mut unknown = Settings::default();
        unknown.set_text(THEME_KEY, "from a newer caditor");
        unknown.set_text(ACCENT_KEY, "chartreuse");
        unknown.set_text(VIEW_KEY, "sepia");
        unknown.set_text(GRID_KEY, "8a909c");
        let read = Preferences::from_settings(unknown);
        assert_eq!(read.appearance, Appearance::default());

        let mut older_views = Settings::default();
        older_views.set_text(VIEW_KEY, "light");
        older_views.set_text(GRID_KEY, "automatic");
        let read = Preferences::from_settings(older_views);
        assert_eq!(read.appearance.view, ViewChoice::Canvas(Canvas::Light));
        assert_eq!(read.appearance.grid, GridColour::Automatic);

        preferences.apply(PreferenceChange::Defaults(PreferencesTab::Appearance));
        assert_eq!(preferences.appearance, Appearance::default());
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
        preferences.apply(PreferenceChange::Projection(ProjectionMode::Orthographic));
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
            ProjectionMode::Orthographic
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

    #[test]
    fn each_input_mode_hints_its_own_mouse_buttons_as_keys() {
        let hints: Vec<&str> = InputMode::ALL
            .into_iter()
            .map(InputMode::navigation_hint)
            .collect();

        for hint in &hints {
            let parts = crate::canvas::hint_parts(hint);
            assert_eq!(parts.len(), 3, "{hint}");
            assert!(
                parts
                    .iter()
                    .all(|part| matches!(part, crate::canvas::HintPart::Keys { .. })),
                "{hint}"
            );
        }
        for (index, hint) in hints.iter().enumerate() {
            assert!(!hints[index + 1..].contains(hint), "{hint}");
        }
        assert!(
            InputMode::Blender
                .navigation_hint()
                .contains("Ctrl+middle-drag: zoom")
        );
    }
}
