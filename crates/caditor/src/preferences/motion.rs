use caditor_render::ProjectionMode;
use egui::{Label, Ui};

use super::{
    FieldOfView, InputMode, MAX_SPEED, MIN_SPEED, OrbitAround, PreferenceChange,
    PreferencesCommand, Section, Transitions, ZoomToward,
    dialog::{Shown, change, choice, pick, section, toggle},
    projection_description, projection_label,
};
use crate::{commands::Command, widgets};

fn speed_slider(
    ui: &mut Ui,
    current: f64,
    make: fn(f64) -> PreferenceChange,
    command: &mut Option<PreferencesCommand>,
) {
    let mut speed = current;
    let response = widgets::slider(
        ui,
        egui::Slider::new(&mut speed, MIN_SPEED..=MAX_SPEED).logarithmic(true),
    );
    widgets::tie_to_caption(ui, &response);
    let settled = response.drag_stopped() || !response.dragged();
    if settled && (response.changed() || response.drag_stopped()) {
        change(command, make(speed));
    } else if response.changed() {
        *command = Some(PreferencesCommand::Preview(make(speed)));
    }
}

pub fn navigation(ui: &mut Ui, shown: &Shown<'_>, command: &mut Option<PreferencesCommand>) {
    let preferences = shown.preferences;
    let navigation = preferences.navigation;
    let toggle_keys = preferences
        .keymap
        .hint(Command::ToggleProjection)
        .map_or_else(|| Command::ToggleProjection.title(), str::to_owned);
    let note = format!("{toggle_keys} switches between perspective and orthographic in the view.");
    section(ui, shown, Section::NavigationView, Some(note), |ui| {
        widgets::property(ui, "Projection", |ui| {
            let options = ProjectionMode::ALL.map(|projection| {
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
        widgets::property(ui, "Field of view", |ui| {
            if let Some(field) = pick::<FieldOfView>(ui, navigation.field_of_view) {
                change(command, PreferenceChange::FieldOfView(field));
            }
        });
        widgets::property(ui, "View changes", |ui| {
            if let Some(transitions) = pick::<Transitions>(ui, navigation.transitions) {
                change(command, PreferenceChange::Transitions(transitions));
            }
        });
    });
    section(ui, shown, Section::Input, None, |ui| {
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
    section(ui, shown, Section::Movement, None, |ui| {
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
            toggle(
                ui,
                navigation.invert_zoom,
                "Scroll up to zoom out",
                "Turn the wheel the other way round for zooming",
                PreferenceChange::InvertZoom,
                command,
            );
        });
        widgets::property(ui, "Zoom toward", |ui| {
            if let Some(toward) = pick::<ZoomToward>(ui, navigation.zoom_toward) {
                change(command, PreferenceChange::ZoomToward(toward));
            }
        });
        widgets::property(ui, "Orbit around", |ui| {
            if let Some(around) = pick::<OrbitAround>(ui, navigation.orbit_around) {
                change(command, PreferenceChange::OrbitAround(around));
            }
        });
        widgets::property(ui, "Orbit direction", |ui| {
            ui.vertical(|ui| {
                toggle(
                    ui,
                    navigation.invert_orbit_x,
                    "Invert left and right",
                    "Dragging to the right turns the model the other way",
                    PreferenceChange::InvertOrbitX,
                    command,
                );
                toggle(
                    ui,
                    navigation.invert_orbit_y,
                    "Invert up and down",
                    "Dragging up tilts the model the other way",
                    PreferenceChange::InvertOrbitY,
                    command,
                );
            });
        });
    });
}

pub fn sketching(ui: &mut Ui, shown: &Shown<'_>, command: &mut Option<PreferencesCommand>) {
    let sketching = shown.preferences.sketching;
    let finish = Command::FinishSketch.title();
    let note = format!(
        "{} returns to the facing view whenever you want it, and {finish} always finishes.",
        Command::LookAtSketch.title()
    );
    section(ui, shown, Section::SketchEditing, Some(note), |ui| {
        widgets::property(ui, "Opening a sketch", |ui| {
            toggle(
                ui,
                sketching.look_at_on_entering,
                "Turn the view to face it",
                "Entering a sketch turns and fits the view to look straight at its plane; off, \
                 the view stays where it is",
                PreferenceChange::LookAtSketch,
                command,
            );
        });
        widgets::property(ui, "Escape", |ui| {
            toggle(
                ui,
                sketching.escape_finishes,
                "Finishes the sketch when nothing is left to cancel",
                "Off, Escape still cancels a shape, a tool and the selection, but never finishes \
                 the sketch",
                PreferenceChange::EscapeFinishesSketch,
                command,
            );
        });
    });
    let at_start = sketching.at_start;
    let note = "Each of these can still be turned on or off while you work, from the View or \
                Sketch menu and the palette; changing one here also changes it now.";
    section(
        ui,
        shown,
        Section::SketchStart,
        Some(note.to_owned()),
        |ui| {
            widgets::property(ui, "Constraint glyphs", |ui| {
                toggle(
                    ui,
                    at_start.glyphs,
                    "Show constraint glyphs",
                    "The small marks beside sketch geometry naming each constraint",
                    PreferenceChange::StartWithGlyphs,
                    command,
                );
            });
            widgets::property(ui, "Snapping", |ui| {
                toggle(
                    ui,
                    at_start.snapping,
                    "Snap to geometry",
                    "Points snap to sketch geometry, guides and body edges while drawing",
                    PreferenceChange::StartWithSnapping,
                    command,
                );
            });
            widgets::property(ui, "Grid", |ui| {
                toggle(
                    ui,
                    at_start.grid_snapping,
                    "Snap to the grid",
                    "A point no geometry took lands on the nearest grid crossing",
                    PreferenceChange::StartWithGridSnapping,
                    command,
                );
            });
            widgets::property(ui, "Typed values", |ui| {
                toggle(
                    ui,
                    at_start.typed_dimensions,
                    "Keep typed values as dimensions",
                    "Lengths and angles typed while drawing become dimensions of the sketch",
                    PreferenceChange::StartWithTypedDimensions,
                    command,
                );
            });
            widgets::property(ui, "Model", |ui| {
                toggle(
                    ui,
                    at_start.model_dimensions,
                    "Show dimensions on the model",
                    "Sketch dimensions and feature values drawn on the model outside sketch editing",
                    PreferenceChange::StartWithModelDimensions,
                    command,
                );
            });
        },
    );
}
