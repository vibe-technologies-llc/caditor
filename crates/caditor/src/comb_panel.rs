use egui::{Label, ScrollArea, Slider, TextWrapMode, Ui};

use crate::{
    appearance::{SPACE_M, SPACE_S},
    comb::{self, Comb, CombTool, Continuity},
    icons, layout,
    model::Model,
    units::Units,
    widgets::{self, Tone},
};

pub const TITLE: &str = "Curvature comb";
pub const CLOSE: &str = "Close the curvature comb";
pub const ABOUT: &str = "Lines square to each chosen curve grow with how tightly it bends, so a \
                         bend that tightens shows as longer teeth, and a jump in curvature where \
                         two curves meet shows as a step in the line through their tips. It \
                         never changes the model.";
pub const NOTHING_CHOSEN: &str = "Select edges of bodies or sketch curves to comb them. The comb \
                                  follows the selection and stays when you select something \
                                  else.";
pub const TEETH: &str = "Teeth per curve";
pub const SCALE: &str = "Scale";
pub const JOINTS: &str = "Where curves meet";
const PANEL_WIDTH: f32 = 300.0;
const MIN_PANEL_WIDTH: f32 = 220.0;
const MAX_LISTED: usize = 12;
const APPROXIMATELY: &str = "≈ ";
const FLAT: f64 = 1e-9;

fn bend_text(units: Units, curvature: f64) -> String {
    if curvature > FLAT {
        format!(
            "radius {APPROXIMATELY}{}",
            units.length.measured_length(1.0 / curvature)
        )
    } else {
        "straight".to_owned()
    }
}

fn wrapped(ui: &mut Ui, text: impl Into<String>) {
    ui.add(
        Label::new(text.into())
            .wrap_mode(TextWrapMode::Wrap)
            .selectable(true),
    );
}

fn more(ui: &mut Ui, total: usize) {
    if total > MAX_LISTED {
        ui.label(widgets::muted(
            format!("and {} more", total - MAX_LISTED),
            ui,
        ));
    }
}

fn settings(ui: &mut Ui, tool: &mut CombTool) {
    widgets::properties(ui, "comb-properties", |ui| {
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

fn curves(ui: &mut Ui, units: Units, comb: &Comb) {
    widgets::card(ui, |ui| {
        for curve in comb.curves.iter().take(MAX_LISTED) {
            let bend = match curve.smallest_radius() {
                Some(radius) => format!(
                    "smallest radius {APPROXIMATELY}{}",
                    units.length.measured_length(radius)
                ),
                None => "straight".to_owned(),
            };
            wrapped(ui, format!("{}: {bend}", curve.name));
        }
        more(ui, comb.curves.len());
    });
}

fn joints(ui: &mut Ui, units: Units, comb: &Comb) {
    if comb.joints.is_empty() {
        return;
    }
    ui.add_space(SPACE_M);
    ui.label(widgets::section_title(JOINTS));
    ui.add_space(SPACE_S);
    widgets::card(ui, |ui| {
        for joint in comb.joints.iter().take(MAX_LISTED) {
            let names = joint.curves.map(|index| {
                comb.curves
                    .get(index)
                    .map_or("", |curve| curve.name.as_str())
            });
            let [first, second] = names;
            let tone = match joint.continuity {
                Continuity::Curvature => Tone::Success,
                Continuity::Tangent => Tone::Info,
                Continuity::Corner => Tone::Neutral,
            };
            widgets::status_pill(
                ui,
                tone,
                format!(
                    "{} ({})",
                    joint.continuity.title(),
                    joint.continuity.grade()
                ),
            );
            let [arriving, leaving] = joint
                .curvatures
                .map(|curvature| bend_text(units, curvature));
            let detail = match joint.continuity {
                Continuity::Tangent => format!(" go from {arriving} to {leaving}"),
                Continuity::Curvature => format!(" share a {arriving}"),
                Continuity::Corner => " meet at an angle".to_owned(),
            };
            wrapped(ui, format!("{first} and {second}{detail}"));
            ui.add_space(SPACE_S);
        }
        more(ui, comb.joints.len());
    });
}

pub fn show(ui: &mut Ui, model: &Model, tool: &mut CombTool, comb: &Comb, room: f32) {
    let mut close = false;
    let units = model.units();
    egui::Panel::right("curvature-comb")
        .resizable(true)
        .default_size(PANEL_WIDTH)
        .size_range(layout::panel_widths(room, MIN_PANEL_WIDTH))
        .show(ui, |ui| {
            ui.add_space(SPACE_S);
            widgets::panel_header(ui, icons::CURVATURE_COMB, TITLE, |ui| {
                close = widgets::icon_button(ui, icons::CLOSE, CLOSE).clicked();
            });
            ui.add_space(SPACE_S);
            ScrollArea::vertical().show(ui, |ui| {
                ui.label(widgets::muted(ABOUT, ui));
                ui.add_space(SPACE_S);
                settings(ui, tool);
                ui.add_space(SPACE_M);
                if tool.curves().is_empty() {
                    widgets::callout(ui, Tone::Info, |ui| ui.label(NOTHING_CHOSEN));
                    return;
                }
                if comb.gone > 0 {
                    widgets::callout(ui, Tone::Warning, |ui| {
                        ui.label(gone_text(comb.gone));
                    });
                    ui.add_space(SPACE_S);
                }
                if tool.left_out() > 0 {
                    widgets::callout(ui, Tone::Info, |ui| {
                        ui.label(left_out_text(tool.left_out()));
                    });
                    ui.add_space(SPACE_S);
                }
                if !comb.curves.is_empty() {
                    curves(ui, units, comb);
                }
                joints(ui, units, comb);
            });
        });
    if close {
        tool.toggle();
    }
}

pub fn gone_text(gone: usize) -> String {
    if gone == 1 {
        "A combed curve is gone or is not a curve. Select others to comb.".to_owned()
    } else {
        format!("{gone} combed curves are gone or are not curves. Select others to comb.")
    }
}

pub fn left_out_text(left_out: usize) -> String {
    format!(
        "Only the first {} selected curves are combed; {left_out} more were left out.",
        comb::MOST_COMBED
    )
}
