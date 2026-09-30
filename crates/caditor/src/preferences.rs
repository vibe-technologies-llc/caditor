use caditor_file::Settings;
use caditor_render::{Msaa, Projection, Shading};
use egui::{KeyboardShortcut, ThemePreference, Ui};

use crate::{
    appearance::{self, MAX_SCALE, MIN_SCALE, SCALE_STEP},
    commands::{Command, Keymap},
    graphics::{self, CurveQuality, FrameLimit, Graphics, Hardware},
    icons,
    layout::{PanelLayout, WindowPlacement},
    onboarding::{Hint, Onboarding},
    units::LengthUnit,
    widgets::{self, DialogWidth, Tab},
};

pub const MIN_SPEED: f64 = 0.25;
pub const MAX_SPEED: f64 = 4.0;
const UNIT_KEY: &str = "units.length";
const THEME_KEY: &str = "appearance.theme";
const SCALE_KEY: &str = "appearance.scale";
const HIGH_CONTRAST_KEY: &str = "appearance.high_contrast";
const ORBIT_KEY: &str = "navigation.orbit_speed";
const ZOOM_KEY: &str = "navigation.zoom_speed";
const INVERT_ZOOM_KEY: &str = "navigation.invert_zoom";
const PROJECTION_KEY: &str = "navigation.projection";
const TITLE_BAR_KEY: &str = "appearance.title_bar";
const SECTION_GAP: f32 = 12.0;
const BODY_HEIGHT_SHARE: f32 = 0.75;

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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Navigation {
    pub orbit_speed: f64,
    pub zoom_speed: f64,
    pub invert_zoom: bool,
    pub projection: Projection,
}

impl Default for Navigation {
    fn default() -> Self {
        Self {
            orbit_speed: 1.0,
            zoom_speed: 1.0,
            invert_zoom: false,
            projection: Projection::default(),
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
    Theme(Theme),
    Scale(f32),
    HighContrast(bool),
    OrbitSpeed(f64),
    ZoomSpeed(f64),
    InvertZoom(bool),
    Projection(Projection),
    TitleBar(TitleBar),
    Vsync(bool),
    FrameLimit(FrameLimit),
    Msaa(Msaa),
    Shading(Shading),
    CurveQuality(CurveQuality),
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
    Tab(PreferencesTab),
    Change(PreferenceChange),
    Preview(PreferenceChange),
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
        settings.set_text(THEME_KEY, self.appearance.theme.key());
        settings.set_number(SCALE_KEY, f64::from(self.appearance.scale));
        settings.set_flag(HIGH_CONTRAST_KEY, self.appearance.high_contrast);
        settings.set_number(ORBIT_KEY, self.navigation.orbit_speed);
        settings.set_number(ZOOM_KEY, self.navigation.zoom_speed);
        settings.set_flag(INVERT_ZOOM_KEY, self.navigation.invert_zoom);
        settings.set_text(PROJECTION_KEY, projection_key(self.navigation.projection));
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
            PreferenceChange::TitleBar(bar) => self.title_bar = bar,
            PreferenceChange::Vsync(vsync) => self.graphics.vsync = vsync,
            PreferenceChange::FrameLimit(limit) => self.graphics.frame_limit = limit,
            PreferenceChange::Msaa(msaa) => self.graphics.msaa = msaa,
            PreferenceChange::Shading(shading) => self.graphics.shading = shading,
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

    fn restore_defaults(&mut self, tab: PreferencesTab) {
        match tab {
            PreferencesTab::General => self.unit = LengthUnit::default(),
            PreferencesTab::Appearance => {
                self.appearance = Appearance::default();
                self.title_bar = TitleBar::default();
            }
            PreferencesTab::Navigation => self.navigation = Navigation::default(),
            PreferencesTab::Graphics => self.graphics = Graphics::default(),
        }
    }
}

pub struct PreferencesView<'a> {
    pub tab: PreferencesTab,
    pub hardware: &'a Hardware,
    pub switch_keys: bool,
}

pub fn dialog(
    ctx: &egui::Context,
    preferences: &Preferences,
    view: &PreferencesView<'_>,
) -> Option<PreferencesCommand> {
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
        }
        let height = ui.ctx().content_rect().height() * BODY_HEIGHT_SHARE;
        egui::ScrollArea::vertical()
            .id_salt(("preferences", tab.label()))
            .max_height(height)
            .min_scrolled_height(height)
            .show(ui, |ui| match tab {
                PreferencesTab::General => {
                    units(ui, preferences, &mut command);
                    help(ui, preferences, &mut command);
                }
                PreferencesTab::Appearance => appearance(ui, preferences, &mut command),
                PreferencesTab::Navigation => navigation(ui, preferences, &mut command),
                PreferencesTab::Graphics => {
                    graphics::tab(ui, &preferences.graphics, view.hardware, &mut command);
                }
            });
        widgets::footer(ui, |ui| {
            if ui.add(widgets::primary_button(ui, "Close")).clicked() {
                command = Some(PreferencesCommand::Hide);
            }
            if ui
                .button("Restore defaults")
                .on_hover_text(tab.defaults())
                .clicked()
            {
                command = Some(PreferencesCommand::Change(PreferenceChange::Defaults(tab)));
            }
        });
        command
    });
    let closed = response.should_close().then_some(PreferencesCommand::Hide);
    response.inner.or(closed)
}

pub fn section(
    ui: &mut Ui,
    title: &str,
    id: &str,
    note: Option<String>,
    rows: impl FnOnce(&mut Ui),
) {
    ui.add_space(SECTION_GAP);
    ui.label(widgets::section_title(title));
    widgets::card(ui, |ui| {
        widgets::properties(ui, id, rows);
        if let Some(note) = note {
            ui.label(widgets::muted(note, ui));
        }
    });
}

pub fn change(command: &mut Option<PreferencesCommand>, change: PreferenceChange) {
    *command = Some(PreferencesCommand::Change(change));
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

fn units(ui: &mut Ui, preferences: &Preferences, command: &mut Option<PreferencesCommand>) {
    let unit = preferences.unit.label().to_lowercase();
    let note = format!(
        "Lengths are shown in {unit} and plain numbers typed for a length mean {unit}. Values \
         already in the model keep the units they were entered in."
    );
    section(ui, "Units", "units", Some(note), |ui| {
        widgets::property(ui, "Length", |ui| {
            ui.horizontal_wrapped(|ui| {
                for unit in LengthUnit::ALL {
                    if ui
                        .selectable_label(preferences.unit == unit, unit.label())
                        .clicked()
                    {
                        change(command, PreferenceChange::Unit(unit));
                    }
                }
            });
        });
    });
}

fn appearance(ui: &mut Ui, preferences: &Preferences, command: &mut Option<PreferencesCommand>) {
    let current = preferences.appearance;
    let note = "Panels and menus follow the theme; the 3D view keeps its dark background.";
    section(ui, "Theme", "appearance", Some(note.to_owned()), |ui| {
        widgets::property(ui, "Theme", |ui| {
            ui.horizontal_wrapped(|ui| {
                for theme in Theme::ALL {
                    if ui
                        .selectable_label(current.theme == theme, theme.label())
                        .clicked()
                    {
                        change(command, PreferenceChange::Theme(theme));
                    }
                }
            });
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
    section(ui, "Window", "window", None, |ui| {
        widgets::property(ui, "Title bar", |ui| {
            ui.horizontal_wrapped(|ui| {
                for bar in TitleBar::ALL {
                    if ui
                        .selectable_label(preferences.title_bar == bar, bar.label())
                        .on_hover_text(bar.description())
                        .clicked()
                    {
                        change(command, PreferenceChange::TitleBar(bar));
                    }
                }
            });
        });
        widgets::property(ui, "Interface size", |ui| {
            ui.horizontal(|ui| {
                let smaller = ui
                    .add_enabled(
                        current.scale > MIN_SCALE,
                        widgets::Named::new(
                            egui::Button::new(widgets::icon(icons::SUBTRACT)),
                            Command::SmallerInterface.title(),
                        ),
                    )
                    .on_hover_text("Smaller");
                if smaller.clicked() {
                    change(command, PreferenceChange::Scale(current.scale - SCALE_STEP));
                }
                ui.label(format!("{:.0}%", current.scale * 100.0));
                let larger = ui
                    .add_enabled(
                        current.scale < MAX_SCALE,
                        widgets::Named::new(
                            egui::Button::new(widgets::icon(icons::ADD)),
                            Command::LargerInterface.title(),
                        ),
                    )
                    .on_hover_text("Larger");
                if larger.clicked() {
                    change(command, PreferenceChange::Scale(current.scale + SCALE_STEP));
                }
            });
        });
    });
}

fn navigation(ui: &mut Ui, preferences: &Preferences, command: &mut Option<PreferencesCommand>) {
    let navigation = preferences.navigation;
    section(ui, "View", "navigation-view", None, |ui| {
        widgets::property(ui, "Projection", |ui| {
            ui.horizontal_wrapped(|ui| {
                for projection in Projection::ALL {
                    if ui
                        .selectable_label(
                            navigation.projection == projection,
                            projection_label(projection),
                        )
                        .on_hover_text(Command::ToggleProjection.title())
                        .clicked()
                    {
                        change(command, PreferenceChange::Projection(projection));
                    }
                }
            });
        });
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

fn help(ui: &mut Ui, preferences: &Preferences, command: &mut Option<PreferencesCommand>) {
    let onboarding = &preferences.onboarding;
    section(ui, "Help", "help", None, |ui| {
        widgets::property(ui, "Shortcuts", |ui| {
            let button = widgets::small_button(
                ui,
                icons::command(Command::KeyboardShortcuts),
                "Keyboard shortcuts…",
            );
            if ui
                .add(button)
                .on_hover_text("See every shortcut and change any of them")
                .clicked()
            {
                *command = Some(PreferencesCommand::ShowShortcuts);
            }
        });
        widgets::property(ui, "Tips", |ui| {
            ui.horizontal_wrapped(|ui| {
                let mut shown = onboarding.hints;
                if ui
                    .checkbox(&mut shown, "Show tips for getting started")
                    .changed()
                {
                    change(command, PreferenceChange::ShowHints(shown));
                }
                let restore = ui
                    .add_enabled(
                        !onboarding.dismissed.is_empty(),
                        egui::Button::new("Show dismissed tips again"),
                    )
                    .clicked();
                if restore {
                    change(command, PreferenceChange::RestoreHints);
                }
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

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
        preferences.apply(PreferenceChange::Unit(LengthUnit::Metre));

        let settings = preferences.settings();
        let read = Preferences::from_settings(settings.clone());

        assert_eq!(settings.flag("graphics.vsync"), Some(false));
        assert_eq!(settings.text("graphics.frame_limit"), Some("120"));
        assert_eq!(settings.number("graphics.msaa"), Some(8.0));
        assert_eq!(settings.text("graphics.shading"), Some("enhanced"));
        assert_eq!(settings.text("graphics.curve_quality"), Some("coarse"));
        assert_eq!(read.graphics, preferences.graphics);

        preferences.apply(PreferenceChange::Defaults(PreferencesTab::Graphics));
        assert_eq!(preferences.graphics, Graphics::default());
        assert_eq!(preferences.unit, LengthUnit::Metre);
    }
}
