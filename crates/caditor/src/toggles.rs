use caditor_render::ProjectionMode;

use crate::{
    commands::Command, display_style::DisplayStyle, section::SectionCommand,
    selection::SelectionFilter, view_aids::ViewAids, viewport::ViewportState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shown {
    On,
    Off,
    Current,
    Other,
}

impl Shown {
    pub fn is_on(self) -> bool {
        matches!(self, Self::On | Self::Current)
    }

    pub fn pill(self) -> Option<&'static str> {
        match self {
            Self::On => Some("On"),
            Self::Off => Some("Off"),
            Self::Current => Some("Current"),
            Self::Other => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToggleStates {
    pub filter: SelectionFilter,
    pub style: DisplayStyle,
    pub snapping: bool,
    pub grid_snapping: bool,
    pub lasso: bool,
    pub paint: bool,
    pub select_through: bool,
    pub automatic_projection: bool,
    pub typed_dimensions: bool,
    pub first_dimension_scales: bool,
    pub glyphs: bool,
    pub aids: ViewAids,
    pub sectioning: bool,
    pub sketch_slice: bool,
}

impl ToggleStates {
    pub fn of(viewport: &ViewportState, sectioning: bool) -> Self {
        Self {
            filter: viewport.filter(),
            style: viewport.style(),
            snapping: viewport.snapping(),
            grid_snapping: viewport.grid_snapping(),
            lasso: viewport.lasso(),
            paint: viewport.paint(),
            select_through: viewport.select_through(),
            automatic_projection: viewport.projection() == ProjectionMode::Automatic,
            typed_dimensions: viewport.typed_dimensions(),
            first_dimension_scales: viewport.first_dimension_scales(),
            glyphs: viewport.glyphs_shown(),
            aids: viewport.aids(),
            sectioning,
            sketch_slice: viewport.sketch_slice(),
        }
    }

    pub fn shown(&self, command: Command) -> Option<Shown> {
        let choice = |chosen: bool| {
            if chosen { Shown::Current } else { Shown::Other }
        };
        let toggle = |on: bool| if on { Shown::On } else { Shown::Off };
        Some(match command {
            Command::Filter(filter) => choice(filter == self.filter),
            Command::Style(style) => choice(style == self.style),
            Command::AutomaticProjection => toggle(self.automatic_projection),
            Command::ToggleSnapping => toggle(self.snapping),
            Command::ToggleGridSnapping => toggle(self.grid_snapping),
            Command::ToggleLasso => toggle(self.lasso),
            Command::TogglePaintSelection => toggle(self.paint),
            Command::ToggleSelectThrough => toggle(self.select_through),
            Command::ToggleTypedDimensions => toggle(self.typed_dimensions),
            Command::ToggleFirstDimensionScales => toggle(self.first_dimension_scales),
            Command::ToggleGlyphs => toggle(self.glyphs),
            Command::ToggleCentresOfMass => toggle(self.aids.centres_of_mass),
            Command::ToggleControlPolygons => toggle(!self.aids.control_polygons_hidden),
            Command::Section(SectionCommand::Toggle) => toggle(self.sectioning),
            Command::Section(SectionCommand::SliceSketch) => toggle(self.sketch_slice),
            _ => return None,
        })
    }

    pub fn is_on(&self, command: Command) -> bool {
        self.shown(command).is_some_and(Shown::is_on)
    }
}

pub fn quiet_toggle_notice(command: Command, on: bool) -> Option<&'static str> {
    Some(match (command, on) {
        (Command::ToggleSnapping, true) => {
            "Snapping on. Hold Ctrl to place a point exactly under the pointer."
        }
        (Command::ToggleSnapping, false) => "Snapping off. Hold Alt to snap a point.",
        (Command::ToggleGridSnapping, true) => {
            "Grid snapping on: free points land on the grid's crossings."
        }
        (Command::ToggleGridSnapping, false) => {
            "Grid snapping off: free points land under the pointer."
        }
        (Command::ToggleLasso, true) => {
            "Lasso selection on: dragging in the view draws a freehand outline."
        }
        (Command::ToggleLasso, false) => "Lasso selection off: dragging in the view draws a box.",
        (Command::TogglePaintSelection, true) => {
            "Paint selection on: dragging over faces selects them."
        }
        (Command::TogglePaintSelection, false) => {
            "Paint selection off: dragging in the view draws a box."
        }
        (Command::ToggleSelectThrough, true) => {
            "Select through on: boxes, lassos and painting also take what lies hidden behind."
        }
        (Command::ToggleSelectThrough, false) => {
            "Select through off: boxes, lassos and painting take only what can be seen."
        }
        (Command::ToggleTypedDimensions, true) => {
            "Typed values are kept as dimensions on the points they place."
        }
        (Command::ToggleTypedDimensions, false) => {
            "Typed values are no longer kept as dimensions; points are placed without them."
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles_read_on_or_off_and_choices_mark_only_the_current_one() {
        let states = ToggleStates {
            filter: SelectionFilter::Faces,
            style: DisplayStyle::ShadedWithEdges,
            snapping: true,
            grid_snapping: false,
            lasso: false,
            paint: false,
            select_through: false,
            automatic_projection: false,
            typed_dimensions: true,
            first_dimension_scales: false,
            glyphs: true,
            aids: ViewAids::default(),
            sectioning: false,
            sketch_slice: false,
        };

        assert_eq!(states.shown(Command::ToggleSnapping), Some(Shown::On));
        assert_eq!(states.shown(Command::ToggleGridSnapping), Some(Shown::Off));
        assert_eq!(
            states.shown(Command::Filter(SelectionFilter::Faces)),
            Some(Shown::Current)
        );
        assert_eq!(
            states.shown(Command::Filter(SelectionFilter::Edges)),
            Some(Shown::Other)
        );
        assert_eq!(states.shown(Command::Save), None);
        assert!(states.is_on(Command::ToggleControlPolygons));
        assert!(!states.is_on(Command::Save));
    }

    #[test]
    fn every_quiet_toggle_says_its_new_state_both_ways() {
        for command in [
            Command::ToggleSnapping,
            Command::ToggleGridSnapping,
            Command::ToggleLasso,
            Command::TogglePaintSelection,
            Command::ToggleSelectThrough,
            Command::ToggleTypedDimensions,
        ] {
            let on = quiet_toggle_notice(command, true);
            let off = quiet_toggle_notice(command, false);
            assert!(
                on.is_some() && off.is_some() && on != off,
                "{}",
                command.id()
            );
        }
        assert_eq!(quiet_toggle_notice(Command::ToggleGlyphs, true), None);
    }
}
