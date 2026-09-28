use caditor_file::Settings;
use egui::{Id, KeyboardShortcut, Modal, Modifiers, RichText, ThemePreference, Ui};

use crate::units::LengthUnit;

pub const PREFERENCES: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND, egui::Key::Comma);
pub const MIN_SPEED: f64 = 0.25;
pub const MAX_SPEED: f64 = 4.0;
const UNIT_KEY: &str = "units.length";
const THEME_KEY: &str = "appearance.theme";
const ORBIT_KEY: &str = "navigation.orbit_speed";
const ZOOM_KEY: &str = "navigation.zoom_speed";
const INVERT_ZOOM_KEY: &str = "navigation.invert_zoom";
const DIALOG_WIDTH: f32 = 440.0;

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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Navigation {
    pub orbit_speed: f64,
    pub zoom_speed: f64,
    pub invert_zoom: bool,
}

impl Default for Navigation {
    fn default() -> Self {
        Self {
            orbit_speed: 1.0,
            zoom_speed: 1.0,
            invert_zoom: false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Preferences {
    pub unit: LengthUnit,
    pub theme: Theme,
    pub navigation: Navigation,
    raw: Settings,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PreferenceChange {
    Unit(LengthUnit),
    Theme(Theme),
    OrbitSpeed(f64),
    ZoomSpeed(f64),
    InvertZoom(bool),
    Defaults,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PreferencesCommand {
    Show,
    Hide,
    Change(PreferenceChange),
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
            theme: raw
                .text(THEME_KEY)
                .and_then(Theme::from_key)
                .unwrap_or_default(),
            navigation: Navigation {
                orbit_speed: speed(raw.number(ORBIT_KEY)),
                zoom_speed: speed(raw.number(ZOOM_KEY)),
                invert_zoom: raw.flag(INVERT_ZOOM_KEY).unwrap_or(false),
            },
            raw,
        }
    }

    pub fn settings(&self) -> Settings {
        let mut settings = self.raw.clone();
        settings.set_text(UNIT_KEY, self.unit.symbol());
        settings.set_text(THEME_KEY, self.theme.key());
        settings.set_number(ORBIT_KEY, self.navigation.orbit_speed);
        settings.set_number(ZOOM_KEY, self.navigation.zoom_speed);
        settings.set_flag(INVERT_ZOOM_KEY, self.navigation.invert_zoom);
        settings
    }

    pub fn apply(&mut self, change: PreferenceChange) {
        match change {
            PreferenceChange::Unit(unit) => self.unit = unit,
            PreferenceChange::Theme(theme) => self.theme = theme,
            PreferenceChange::OrbitSpeed(value) => {
                self.navigation.orbit_speed = value.clamp(MIN_SPEED, MAX_SPEED);
            }
            PreferenceChange::ZoomSpeed(value) => {
                self.navigation.zoom_speed = value.clamp(MIN_SPEED, MAX_SPEED);
            }
            PreferenceChange::InvertZoom(invert) => self.navigation.invert_zoom = invert,
            PreferenceChange::Defaults => {
                self.unit = LengthUnit::default();
                self.theme = Theme::default();
                self.navigation = Navigation::default();
            }
        }
    }
}

pub fn dialog(ctx: &egui::Context, preferences: &Preferences) -> Option<PreferencesCommand> {
    let response = Modal::new(Id::new("preferences")).show(ctx, |ui| {
        ui.set_max_width(DIALOG_WIDTH);
        ui.heading("Preferences");
        let mut command = None;
        units(ui, preferences, &mut command);
        appearance(ui, preferences, &mut command);
        navigation(ui, preferences, &mut command);
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Close").clicked() {
                command = Some(PreferencesCommand::Hide);
            }
            if ui
                .button("Restore defaults")
                .on_hover_text("Go back to millimetres, the system theme and normal speeds")
                .clicked()
            {
                command = Some(PreferencesCommand::Change(PreferenceChange::Defaults));
            }
        });
        command
    });
    let closed = response.should_close().then_some(PreferencesCommand::Hide);
    response.inner.or(closed)
}

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(title).strong());
}

fn units(ui: &mut Ui, preferences: &Preferences, command: &mut Option<PreferencesCommand>) {
    section(ui, "Units");
    ui.horizontal_wrapped(|ui| {
        for unit in LengthUnit::ALL {
            if ui.radio(preferences.unit == unit, unit.label()).clicked() {
                *command = Some(PreferencesCommand::Change(PreferenceChange::Unit(unit)));
            }
        }
    });
    ui.weak(format!(
        "Lengths are shown in {} and plain numbers typed for a length mean {}. Values already in \
         the model keep the units they were entered in.",
        preferences.unit.label().to_lowercase(),
        preferences.unit.label().to_lowercase()
    ));
}

fn appearance(ui: &mut Ui, preferences: &Preferences, command: &mut Option<PreferencesCommand>) {
    section(ui, "Appearance");
    ui.horizontal(|ui| {
        for theme in Theme::ALL {
            if ui
                .radio(preferences.theme == theme, theme.label())
                .clicked()
            {
                *command = Some(PreferencesCommand::Change(PreferenceChange::Theme(theme)));
            }
        }
    });
    ui.weak("Panels and menus follow the theme; the 3D view keeps its dark background.");
}

fn navigation(ui: &mut Ui, preferences: &Preferences, command: &mut Option<PreferencesCommand>) {
    section(ui, "Navigation");
    let navigation = preferences.navigation;
    let mut orbit = navigation.orbit_speed;
    let orbit_slider = egui::Slider::new(&mut orbit, MIN_SPEED..=MAX_SPEED)
        .logarithmic(true)
        .text("Orbit speed");
    if ui.add(orbit_slider).changed() {
        *command = Some(PreferencesCommand::Change(PreferenceChange::OrbitSpeed(
            orbit,
        )));
    }
    let mut zoom = navigation.zoom_speed;
    let zoom_slider = egui::Slider::new(&mut zoom, MIN_SPEED..=MAX_SPEED)
        .logarithmic(true)
        .text("Zoom speed");
    if ui.add(zoom_slider).changed() {
        *command = Some(PreferencesCommand::Change(PreferenceChange::ZoomSpeed(
            zoom,
        )));
    }
    let mut invert = navigation.invert_zoom;
    if ui.checkbox(&mut invert, "Scroll up to zoom out").changed() {
        *command = Some(PreferencesCommand::Change(PreferenceChange::InvertZoom(
            invert,
        )));
    }
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
        let mut preferences = Preferences::from_settings(raw);
        assert_eq!(preferences.unit, LengthUnit::Centimetre);
        assert_eq!(preferences.theme, Theme::Light);
        assert_eq!(preferences.navigation.orbit_speed, MAX_SPEED);
        assert_eq!(preferences.navigation.zoom_speed, 1.0);
        preferences.apply(PreferenceChange::ZoomSpeed(0.5));
        preferences.apply(PreferenceChange::InvertZoom(true));
        let settings = preferences.settings();
        assert_eq!(settings.text("future.option"), Some("kept"));
        assert_eq!(settings.number(ZOOM_KEY), Some(0.5));
        assert_eq!(
            Preferences::from_settings(settings.clone()).settings(),
            settings
        );
        preferences.apply(PreferenceChange::Defaults);
        assert_eq!(preferences.unit, LengthUnit::Millimetre);
        assert_eq!(preferences.navigation, Navigation::default());
    }
}
