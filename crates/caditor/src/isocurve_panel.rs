use caditor_kernel::Along;
use egui::{Label, ScrollArea, Slider, TextWrapMode, Ui};

use crate::{
    appearance::{SPACE_M, SPACE_S},
    comb,
    guide::Page,
    guide_panel, icons,
    isocurves::{self, Directions, FaceLines, IsocurveTool, Isocurves},
    layout,
    units::Units,
    widgets::{self, Tone},
};

pub const TITLE: &str = "Isocurves";
pub const CLOSE: &str = "Close the isocurves";
pub const ABOUT: &str = "The u and v parameter lines of each chosen face are drawn on it, each \
                         with a curvature comb: teeth square to the line grow with how tightly \
                         the face bends along it, so the flow of a spline face shows as even \
                         lines and smooth combs, and a ripple as a wave in the line through the \
                         tips. It never changes the model.";
pub const NOTHING_CHOSEN: &str = "Select faces of bodies to draw their parameter lines. The lines \
                                  follow the selection and stay when you select something \
                                  else.";
pub const WORKING: &str = "Working out the lines…";
pub const FAILED: &str = "The lines of the chosen faces could not be worked out. Select them \
                          again or other faces.";
pub const LINES: &str = "Lines each way";
pub const DIRECTIONS: &str = "Lines along";
pub const TEETH: &str = "Teeth per line";
pub const SCALE: &str = "Scale";
const PANEL_WIDTH: f32 = 300.0;
const MIN_PANEL_WIDTH: f32 = 220.0;
const MAX_LISTED: usize = 12;
const APPROXIMATELY: &str = "≈ ";

fn direction_meaning(directions: Directions) -> &'static str {
    match directions {
        Directions::Both => "Draw the lines of both parameters",
        Directions::U => "Draw only the lines along which the first parameter, u, runs",
        Directions::V => "Draw only the lines along which the second parameter, v, runs",
    }
}

fn wrapped(ui: &mut Ui, text: impl Into<String>) {
    ui.add(
        Label::new(text.into())
            .wrap_mode(TextWrapMode::Wrap)
            .selectable(true),
    );
}

fn settings(ui: &mut Ui, tool: &mut IsocurveTool) {
    widgets::properties(ui, "isocurve-properties", |ui| {
        widgets::caption(ui, DIRECTIONS);
        let choices: Vec<(&str, &str)> = Directions::ALL
            .iter()
            .map(|directions| (directions.title(), direction_meaning(*directions)))
            .collect();
        let chosen = Directions::ALL
            .iter()
            .position(|directions| *directions == tool.directions)
            .unwrap_or(0);
        if let Some(index) = widgets::segmented(ui, &choices, chosen)
            && let Some(directions) = Directions::ALL.get(index)
        {
            tool.directions = *directions;
        }
        ui.end_row();

        widgets::caption(ui, LINES);
        let lines = ui.add(Slider::new(
            &mut tool.lines,
            isocurves::MIN_LINES..=isocurves::MAX_LINES,
        ));
        widgets::tie_to_caption(ui, &lines);
        ui.end_row();

        widgets::caption(ui, TEETH);
        let teeth = ui.add(Slider::new(
            &mut tool.teeth,
            comb::MIN_TEETH..=comb::MAX_TEETH,
        ));
        widgets::tie_to_caption(ui, &teeth);
        ui.end_row();

        widgets::caption(ui, SCALE);
        let scale = ui
            .add(Slider::new(&mut tool.scale, comb::MIN_SCALE..=comb::MAX_SCALE).logarithmic(true));
        widgets::tie_to_caption(ui, &scale);
        ui.end_row();
    });
}

pub fn bend_text(units: Units, face: &FaceLines, along: Along) -> Option<String> {
    let lines = isocurves::along_title(along);
    Some(match face.smallest_radius(along)? {
        Some(radius) => format!(
            "{lines} bend to a smallest radius of {APPROXIMATELY}{}",
            units.length.measured_length(radius)
        ),
        None => format!("{lines} straight"),
    })
}

fn faces(ui: &mut Ui, units: Units, isocurves: &Isocurves) {
    widgets::card(ui, |ui| {
        for face in isocurves.faces.iter().take(MAX_LISTED) {
            let bends: Vec<String> = [Along::U, Along::V]
                .into_iter()
                .filter_map(|along| bend_text(units, face, along))
                .collect();
            let text = if bends.is_empty() {
                format!("{}: no line crosses it", face.name)
            } else {
                format!("{}: {}", face.name, bends.join(", "))
            };
            wrapped(ui, text);
        }
        if isocurves.faces.len() > MAX_LISTED {
            ui.label(widgets::muted(
                format!("and {} more", isocurves.faces.len() - MAX_LISTED),
                ui,
            ));
        }
    });
}

fn findings(ui: &mut Ui, units: Units, tool: &IsocurveTool, shown: Option<&Isocurves>) {
    if tool.is_working() {
        ui.label(widgets::muted(WORKING, ui));
        ui.add_space(SPACE_S);
    }
    let Some(shown) = shown else {
        return;
    };
    if shown.failed {
        widgets::callout(ui, Tone::Warning, |ui| ui.label(FAILED));
        ui.add_space(SPACE_S);
    }
    if shown.gone > 0 {
        widgets::callout(ui, Tone::Warning, |ui| ui.label(gone_text(shown.gone)));
        ui.add_space(SPACE_S);
    }
    if tool.left_out() > 0 {
        widgets::callout(ui, Tone::Info, |ui| {
            ui.label(left_out_text(tool.left_out()))
        });
        ui.add_space(SPACE_S);
    }
    if !shown.faces.is_empty() {
        faces(ui, units, shown);
    }
}

pub fn show(
    ui: &mut Ui,
    units: Units,
    tool: &mut IsocurveTool,
    shown: Option<&Isocurves>,
    room: f32,
) {
    let mut close = false;
    egui::Panel::right("isocurves")
        .resizable(true)
        .default_size(PANEL_WIDTH)
        .size_range(layout::panel_widths(room, MIN_PANEL_WIDTH))
        .show(ui, |ui| {
            ui.add_space(SPACE_S);
            widgets::panel_header(ui, icons::ISOCURVES, TITLE, |ui| {
                close = widgets::icon_button(ui, icons::CLOSE, CLOSE).clicked();
                guide_panel::help_button(ui, Page::Isocurves);
            });
            ui.add_space(SPACE_S);
            ScrollArea::vertical().show(ui, |ui| {
                ui.label(widgets::muted(ABOUT, ui));
                ui.add_space(SPACE_S);
                settings(ui, tool);
                ui.add_space(SPACE_M);
                if tool.faces().is_empty() {
                    widgets::callout(ui, Tone::Info, |ui| ui.label(NOTHING_CHOSEN));
                    return;
                }
                findings(ui, units, tool, shown);
            });
        });
    if close {
        tool.toggle();
    }
}

pub fn gone_text(gone: usize) -> String {
    if gone == 1 {
        "A chosen face is gone. Select others to draw their lines.".to_owned()
    } else {
        format!("{gone} chosen faces are gone. Select others to draw their lines.")
    }
}

pub fn left_out_text(left_out: usize) -> String {
    format!(
        "Only the first {} selected faces are drawn; {left_out} more were left out.",
        isocurves::MOST_FACES
    )
}
