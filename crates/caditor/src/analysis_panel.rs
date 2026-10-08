use caditor_expression::Dimension;
use caditor_render::Color;
use egui::{Color32, Id, Label, ScrollArea, Sense, TextWrapMode, Ui, vec2};

use crate::{
    analysis::{self, Analyses, AnalysisTool, FaceAnalysis, Problem, Pull, Tally},
    appearance::{SPACE_M, SPACE_S},
    bodies::BodyMeshes,
    display_style::DisplayStyle,
    feature_fields, field, icons, layout,
    model::Model,
    scene_palette::Contrast,
    selection::{Axis, Selection},
    units::Units,
    visibility,
    widgets::{self, FIELD_WIDTH, Tone},
};

pub const TITLE: &str = "Analyse faces";
pub const CLOSE: &str = "Close the analysis panel";
pub const NO_BODIES: &str = "There are no bodies to analyse.";
pub const WORKING: &str = "Working out the faces of a large body…";
pub const FAILED: &str = "The faces of a body could not be analysed.";
pub const HIDDEN_STYLE: &str = "This display style does not colour faces. Choose a shaded style \
                                to see the analysis.";
pub const DRAFT_ABOUT: &str = "Faces are coloured by the angle between their surface and the \
                               pull direction, as a mould or a die is pulled away. It never \
                               changes the model.";
const PANEL_WIDTH: f32 = 300.0;
const MIN_PANEL_WIDTH: f32 = 220.0;
const SWATCH_SIDE: f32 = 14.0;
const SWATCH_RADIUS: f32 = 3.0;
const APPROXIMATELY: &str = "≈ ";
const AXES: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];

pub struct AnalysisContext<'a> {
    pub model: &'a Model,
    pub selection: &'a Selection,
    pub bodies: &'a BodyMeshes,
    pub analyses: &'a Analyses,
    pub style: DisplayStyle,
    pub contrast: Contrast,
}

fn swatch_colour(colour: Color) -> Color32 {
    let channel = |value: f32| (value * 255.0).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(
        channel(colour.red),
        channel(colour.green),
        channel(colour.blue),
    )
}

fn limit_row(
    ui: &mut Ui,
    model: &Model,
    (caption, id, dimension): (&str, &str, Dimension),
    expression: &mut caditor_expression::Expression,
    check: fn(f64) -> Result<(), Problem>,
) {
    widgets::caption(ui, caption);
    let document = model.document();
    let parameters = model.parameters();
    let units = model.units();
    let field = field::commit_field(
        ui,
        Id::new(("analysis-limit", id)),
        &document.expression_text(expression),
        FIELD_WIDTH,
        false,
        |text| {
            let parsed = field::parse_expression(
                document,
                parameters,
                text,
                field::Expected {
                    dimension: Some(dimension),
                    non_negative: true,
                },
                units,
            )?;
            let value = parameters
                .evaluate_expression(&parsed)
                .map_err(|error| field::sentence(&error.to_string()))?
                .value;
            check(value).map_err(|problem| problem.to_string())?;
            Ok(parsed)
        },
    );
    if let Some(parsed) = field.committed {
        *expression = parsed;
    }
    ui.end_row();
    if let Some(error) = field.error {
        widgets::error_row(ui, &error);
    }
}

fn pull_rows(ui: &mut Ui, context: &AnalysisContext<'_>, tool: &mut AnalysisTool) {
    widgets::caption(ui, "Pull along");
    let chosen = match tool.pull {
        Pull::Axis(axis) => AXES.iter().position(|candidate| *candidate == axis),
        Pull::Picked(_) => None,
    };
    let choices: Vec<(&str, &str)> = AXES
        .iter()
        .map(|axis| (axis.letter(), axis.name()))
        .collect();
    if let Some(index) = widgets::segmented(ui, &choices, chosen.unwrap_or(usize::MAX))
        && let Some(axis) = AXES.get(index)
    {
        tool.pull = Pull::Axis(*axis);
    }
    ui.end_row();

    widgets::caption(ui, "Direction");
    ui.horizontal_wrapped(|ui| {
        let name = tool.pull_name(context.model);
        let reversed = if tool.reversed { ", reversed" } else { "" };
        ui.add(Label::new(format!("{name}{reversed}")).wrap_mode(TextWrapMode::Wrap));
        let selected = AnalysisTool::pull_from(context.model, context.selection);
        let button = widgets::small_button(ui, icons::USE_SELECTED, feature_fields::USE_SELECTED);
        let hover = match &selected {
            Ok(_) => "Pull along the selected axis, edge or face".to_owned(),
            Err(refusal) => refusal.to_string(),
        };
        if ui
            .add_enabled(selected.is_ok(), button)
            .on_hover_text(hover)
            .clicked()
            && let Ok(pull) = selected
        {
            tool.pull = pull;
        }
    });
    ui.end_row();

    if let Some(reversed) =
        feature_fields::reverse_row(ui, "Reverse the pull direction", tool.reversed)
    {
        tool.reversed = reversed;
    }
}

fn legend(ui: &mut Ui, context: &AnalysisContext<'_>, analysis: FaceAnalysis) {
    let palette = context.contrast.palette();
    let units: Units = context.model.units();
    let meshes: Vec<_> = context
        .bodies
        .iter()
        .filter(|(body, _)| visibility::is_shown(context.model.document(), *body))
        .map(|(_, mesh)| &mesh.mesh)
        .collect();
    if meshes.is_empty() {
        ui.label(widgets::muted(NO_BODIES, ui));
        return;
    }
    let tally = Tally::of(context.analyses, meshes, analysis);
    widgets::card(ui, |ui| {
        for band in analysis.bands() {
            let area = tally.areas.get(band).copied().unwrap_or(0.0);
            let area = format!("{APPROXIMATELY}{}", units.length.measured_area(area));
            ui.horizontal(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(vec2(SWATCH_SIDE, SWATCH_SIDE), Sense::hover());
                ui.painter()
                    .rect_filled(rect, SWATCH_RADIUS, swatch_colour(band.colour(palette)));
                ui.add(
                    Label::new(format!("{}: {area}", band.title()))
                        .wrap_mode(TextWrapMode::Wrap)
                        .selectable(true),
                )
                .on_hover_text(band.meaning());
            });
        }
    });
    if tally.working {
        ui.label(widgets::muted(WORKING, ui));
    }
    if tally.failed {
        widgets::callout(ui, Tone::Warning, |ui| ui.label(FAILED));
    }
}

pub fn show(ui: &mut Ui, context: &AnalysisContext<'_>, tool: &mut AnalysisTool, room: f32) {
    let mut close = false;
    egui::Panel::right("analysis")
        .resizable(true)
        .default_size(PANEL_WIDTH)
        .size_range(layout::panel_widths(room, MIN_PANEL_WIDTH))
        .show(ui, |ui| {
            ui.add_space(SPACE_S);
            widgets::panel_header(ui, icons::ANALYSIS, TITLE, |ui| {
                close = widgets::icon_button(ui, icons::CLOSE, CLOSE).clicked();
            });
            ui.add_space(SPACE_S);
            ScrollArea::vertical().show(ui, |ui| {
                ui.label(widgets::muted(DRAFT_ABOUT, ui));
                ui.add_space(SPACE_S);
                widgets::properties(ui, "analysis-properties", |ui| {
                    pull_rows(ui, context, tool);
                    limit_row(
                        ui,
                        context.model,
                        ("Draft limit", "draft", Dimension::ANGLE),
                        &mut tool.draft_limit,
                        analysis::check_draft_limit,
                    );
                });
                ui.add_space(SPACE_M);
                if !context.style.shows_faces() || context.style.is_translucent() {
                    widgets::callout(ui, Tone::Info, |ui| ui.label(HIDDEN_STYLE));
                    ui.add_space(SPACE_S);
                }
                match tool.analysis(context.model) {
                    Ok(analysis) => legend(ui, context, analysis),
                    Err(problem) => {
                        widgets::callout(ui, Tone::Warning, |ui| ui.label(problem.to_string()));
                    }
                }
            });
        });
    if close {
        tool.open = false;
    }
}
