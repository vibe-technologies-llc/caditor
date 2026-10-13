use super::{Preferences, PreferencesTab};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Units,
    Files,
    NewModels,
    FeatureTree,
    Keyboard,
    Tips,
    SavingOnWindows,
    Theme,
    Colours,
    Interface,
    InTheView,
    NavigationView,
    Input,
    Movement,
    SketchEditing,
    SketchStart,
    GraphicsDisplay,
    GraphicsQuality,
    GraphicsAdapter,
}

impl Section {
    pub const ALL: [Self; 19] = [
        Self::Units,
        Self::Files,
        Self::NewModels,
        Self::FeatureTree,
        Self::Keyboard,
        Self::Tips,
        Self::SavingOnWindows,
        Self::Theme,
        Self::Colours,
        Self::Interface,
        Self::InTheView,
        Self::NavigationView,
        Self::Input,
        Self::Movement,
        Self::SketchEditing,
        Self::SketchStart,
        Self::GraphicsDisplay,
        Self::GraphicsQuality,
        Self::GraphicsAdapter,
    ];

    pub fn tab(self) -> PreferencesTab {
        match self {
            Self::Units
            | Self::Files
            | Self::NewModels
            | Self::FeatureTree
            | Self::Keyboard
            | Self::Tips
            | Self::SavingOnWindows => PreferencesTab::General,
            Self::Theme | Self::Colours | Self::Interface | Self::InTheView => {
                PreferencesTab::Appearance
            }
            Self::NavigationView | Self::Input | Self::Movement => PreferencesTab::Navigation,
            Self::SketchEditing | Self::SketchStart => PreferencesTab::Sketching,
            Self::GraphicsDisplay | Self::GraphicsQuality | Self::GraphicsAdapter => {
                PreferencesTab::Graphics
            }
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Units => "units",
            Self::Files => "files",
            Self::NewModels => "new-models",
            Self::FeatureTree => "feature-tree",
            Self::Keyboard => "keyboard",
            Self::Tips => "tips",
            Self::SavingOnWindows => "saving-on-windows",
            Self::Theme => "theme",
            Self::Colours => "colours",
            Self::Interface => "interface",
            Self::InTheView => "in-the-view",
            Self::NavigationView => "navigation-view",
            Self::Input => "navigation-input",
            Self::Movement => "navigation",
            Self::SketchEditing => "sketch-editing",
            Self::SketchStart => "sketch-start",
            Self::GraphicsDisplay => "graphics-display",
            Self::GraphicsQuality => "graphics-quality",
            Self::GraphicsAdapter => "graphics-adapter",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Units => "Units",
            Self::Files => "Starting and recent files",
            Self::NewModels => "New models",
            Self::FeatureTree => "Feature tree",
            Self::Keyboard => "Keyboard",
            Self::Tips => "Tips",
            Self::SavingOnWindows => "Saving on Windows",
            Self::Theme => "Theme",
            Self::Colours => "Colours",
            Self::Interface => "Interface",
            Self::InTheView => "In the 3D view",
            Self::NavigationView => "View",
            Self::Input => "Input",
            Self::Movement => "Movement",
            Self::SketchEditing => "Editing a sketch",
            Self::SketchStart => "On when caditor starts",
            Self::GraphicsDisplay => "Display",
            Self::GraphicsQuality => "Quality",
            Self::GraphicsAdapter => "Graphics adapter",
        }
    }

    fn words(self) -> &'static [&'static str] {
        match self {
            Self::Units => &[
                "length",
                "angle",
                "millimetres",
                "centimetres",
                "metres",
                "micrometres",
                "degrees",
                "radians",
            ],
            Self::Files => &[
                "at startup",
                "open the last model",
                "recent files",
                "start",
                "start card",
                "empty model",
            ],
            Self::NewModels => &["start from", "template", "new model"],
            Self::FeatureTree => &["double-click", "rename", "edit"],
            Self::Keyboard => &["shortcuts", "keys", "keymap"],
            Self::Tips => &["getting started", "show tips", "hints"],
            Self::SavingOnWindows => &["microsoft defender", "saving"],
            Self::Theme => &["theme", "dark", "light", "themes folder"],
            Self::Colours => &["accent", "3d view", "background", "grid", "contrast"],
            Self::Interface => &[
                "size",
                "scale",
                "title bar",
                "hover help",
                "tooltips",
                "delay",
            ],
            Self::InTheView => &[
                "edges",
                "edge width",
                "outlines",
                "silhouettes",
                "view cube",
                "principal planes",
                "axes",
                "origin",
            ],
            Self::NavigationView => &[
                "projection",
                "perspective",
                "orthographic",
                "field of view",
                "view changes",
                "animation",
                "motion",
            ],
            Self::Input => &[
                "input mode",
                "mouse",
                "touchpad",
                "laptop",
                "fusion",
                "freecad",
                "blender",
                "solidworks",
            ],
            Self::Movement => &[
                "orbit speed",
                "zoom speed",
                "scrolling",
                "zoom toward",
                "orbit around",
                "invert",
                "rotate",
            ],
            Self::SketchEditing => &["look at the sketch", "escape", "finish"],
            Self::SketchStart => &[
                "constraint glyphs",
                "snapping",
                "snap to the grid",
                "typed values",
                "dimensions",
            ],
            Self::GraphicsDisplay => &["vsync", "frame rate"],
            Self::GraphicsQuality => &[
                "anti-aliasing",
                "multisampling",
                "shading",
                "curve smoothness",
            ],
            Self::GraphicsAdapter => &["prefer", "adapter", "power", "performance", "details"],
        }
    }

    pub fn matches(self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        let title = self.title().to_lowercase();
        query
            .split_whitespace()
            .all(|term| title.contains(term) || self.words().iter().any(|word| word.contains(term)))
    }

    pub fn of_tab(tab: PreferencesTab) -> Vec<Self> {
        Self::ALL
            .into_iter()
            .filter(|section| section.tab() == tab)
            .collect()
    }

    pub fn restored_with(tab: PreferencesTab) -> Vec<Self> {
        Self::of_tab(tab)
            .into_iter()
            .filter(|section| section.is_resettable())
            .collect()
    }

    pub fn is_resettable(self) -> bool {
        !matches!(self, Self::Keyboard | Self::Tips | Self::SavingOnWindows)
    }

    pub fn is_default(self, preferences: &Preferences) -> bool {
        let mut reset = preferences.clone();
        self.copy(&mut reset, &Preferences::default());
        reset == *preferences
    }

    pub fn copy(self, to: &mut Preferences, from: &Preferences) {
        match self {
            Self::Units => {
                to.unit = from.unit;
                to.angle = from.angle;
            }
            Self::Files => {
                to.startup = from.startup;
                to.recent_listed = from.recent_listed;
                to.onboarding.start_card = from.onboarding.start_card;
            }
            Self::NewModels => to.default_template.clone_from(&from.default_template),
            Self::FeatureTree => to.double_click = from.double_click,
            Self::Keyboard | Self::Tips | Self::SavingOnWindows => {}
            Self::Theme => to.appearance.theme = from.appearance.theme.clone(),
            Self::Colours => {
                to.appearance.accent = from.appearance.accent;
                to.appearance.view = from.appearance.view;
                to.appearance.grid = from.appearance.grid;
                to.appearance.high_contrast = from.appearance.high_contrast;
            }
            Self::Interface => {
                to.appearance.scale = from.appearance.scale;
                to.title_bar = from.title_bar;
                to.tooltip_delay = from.tooltip_delay;
            }
            Self::InTheView => to.view_details = from.view_details,
            Self::NavigationView => {
                to.navigation.projection = from.navigation.projection;
                to.navigation.field_of_view = from.navigation.field_of_view;
                to.navigation.transitions = from.navigation.transitions;
            }
            Self::Input => to.navigation.input_mode = from.navigation.input_mode,
            Self::Movement => {
                to.navigation.orbit_speed = from.navigation.orbit_speed;
                to.navigation.zoom_speed = from.navigation.zoom_speed;
                to.navigation.invert_zoom = from.navigation.invert_zoom;
                to.navigation.zoom_toward = from.navigation.zoom_toward;
                to.navigation.orbit_around = from.navigation.orbit_around;
                to.navigation.invert_orbit_x = from.navigation.invert_orbit_x;
                to.navigation.invert_orbit_y = from.navigation.invert_orbit_y;
            }
            Self::SketchEditing => {
                to.sketching.look_at_on_entering = from.sketching.look_at_on_entering;
                to.sketching.escape_finishes = from.sketching.escape_finishes;
            }
            Self::SketchStart => to.sketching.at_start = from.sketching.at_start,
            Self::GraphicsDisplay => {
                to.graphics.vsync = from.graphics.vsync;
                to.graphics.frame_limit = from.graphics.frame_limit;
            }
            Self::GraphicsQuality => {
                to.graphics.msaa = from.graphics.msaa;
                to.graphics.shading = from.graphics.shading;
                to.graphics.curves = from.graphics.curves;
            }
            Self::GraphicsAdapter => to.graphics.adapter = from.graphics.adapter,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        preferences::{FieldOfView, PreferenceChange, TooltipDelay},
        units::LengthUnit,
    };

    #[test]
    fn every_tab_restores_exactly_its_resettable_sections() {
        for section in Section::ALL {
            assert!(
                PreferencesTab::ALL.contains(&section.tab()),
                "{section:?} sits on no tab"
            );
            assert!(!section.title().is_empty() && !section.words().is_empty());
        }
        let ids: Vec<&str> = Section::ALL.iter().map(|section| section.id()).collect();
        for (index, id) in ids.iter().enumerate() {
            assert!(!ids[index + 1..].contains(id), "{id} is used twice");
        }

        let mut changed = Preferences::default();
        changed.apply(PreferenceChange::Unit(LengthUnit::Metre));
        changed.apply(PreferenceChange::TooltipDelay(TooltipDelay::Later));
        changed.apply(PreferenceChange::FieldOfView(FieldOfView::Wide));

        assert!(!Section::Units.is_default(&changed));
        assert!(Section::NewModels.is_default(&changed));
        assert!(!Section::Interface.is_default(&changed));

        changed.apply(PreferenceChange::SectionDefaults(Section::Interface));
        assert_eq!(changed.tooltip_delay, TooltipDelay::Standard);
        assert_eq!(changed.unit, LengthUnit::Metre);
        assert_eq!(changed.navigation.field_of_view, FieldOfView::Wide);

        changed.apply(PreferenceChange::Defaults(PreferencesTab::Navigation));
        assert_eq!(changed.navigation.field_of_view, FieldOfView::Standard);
        assert_eq!(changed.unit, LengthUnit::Metre);
    }

    #[test]
    fn a_search_finds_sections_by_their_title_or_their_settings_names() {
        let found = |query: &str| -> Vec<Section> {
            Section::ALL
                .into_iter()
                .filter(|section| section.matches(query))
                .collect()
        };

        assert_eq!(found("field of view"), [Section::NavigationView]);
        assert_eq!(found("ZOOM toward"), [Section::Movement]);
        assert_eq!(found("view cube"), [Section::InTheView]);
        assert!(found("grid").contains(&Section::Colours));
        assert!(found("grid").contains(&Section::SketchStart));
        assert!(found("no such setting").is_empty());
        assert_eq!(found("").len(), Section::ALL.len());
    }
}
