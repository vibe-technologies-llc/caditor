use std::{cell::OnceCell, sync::Arc};

use caditor_expression::Dimension;
use caditor_render::ShadedMesh;
use egui::{Id, Label, Sense, Slider, TextWrapMode, Ui, vec2};

use crate::{
    analysis::{
        self, Analyses, Analysis, AnalysisTool, FaceAnalysis, Kind, MAX_STRIPES, MIN_STRIPES,
        Measure, Problem, Pull, Tally,
    },
    appearance::{SPACE_M, SPACE_S},
    bodies::BodyMeshes,
    commands::{Command, CommandFrame},
    display_style::DisplayStyle,
    feature_fields,
    field::{self, ExpressionError},
    guide::Page,
    icons,
    layout::RightPanel,
    model::Model,
    scene_palette::{self, Canvas, Contrast},
    selection::{Axis, Selection},
    units::Units,
    widgets::{self, FIELD_WIDTH, PanelTitle, Tone},
};

pub const TITLE: &str = "Analyse faces";
pub const CLOSE: &str = "Close the analysis panel";
pub const NO_BODIES: &str = "There are no bodies to analyse.";
pub const WORKING: &str = "Working out the faces of a large body…";
pub const FAILED: &str = "The faces of a body could not be analysed.";
pub const STOPPED: &str = "Stopped before every body was analysed. The bodies left are shown in \
                           their own colours and the areas leave them out.";
pub const ANALYSE_AGAIN: &str = "Analyse again";
const NOT_ANALYSING: &str = "No faces are being analysed";
const NOT_STOPPED: &str = "The face analysis was not stopped";
const STOP_HOVER: &str = "Stop working out the faces and show the bodies left uncoloured";
const ANALYSE_AGAIN_HOVER: &str = "Work out the faces of the bodies left";
pub const HIDDEN_STYLE: &str = "This display style does not colour faces. Choose a shaded style \
                                to see the analysis.";
pub const DRAFT_ABOUT: &str = "Faces are coloured by the angle between their surface and the \
                               pull direction, as a mould or a die is pulled away. It never \
                               changes the model.";
pub const RADIUS_ABOUT: &str = "Concave faces curving tighter than the radius are coloured, \
                                where a cutter or a nozzle that size cannot reach. Sharp inside \
                                corners have no radius and are not coloured.";
pub const REACH_ABOUT: &str = "Faces are coloured by whether a tool coming from the direction, as \
                               on a three-axis machine, gets to them: faces turned from it are \
                               undercuts, and faces with another part of the bodies over them are \
                               hidden. It never changes the model.";
pub const CURVATURE_ABOUT: &str = "Faces are coloured by how they curve, read from the display \
                                   mesh, against a reference radius. It never changes the model.";
pub const ZEBRA_ABOUT: &str = "The bodies reflect a ring of light and dark stripes. Stripes that \
                               run on unbroken and without a kink across an edge show faces \
                               meeting with continuous curvature (G2); stripes that meet but \
                               turn sharply show faces only tangent (G1); stripes that jump show \
                               a corner (G0). It never changes the model.";
pub const CHROME_ABOUT: &str = "The bodies reflect a sky, a horizon and a light as polished \
                                metal; turn the view to slide the reflection over them. A \
                                reflection that breaks or bends sharply at an edge shows the \
                                same jumps zebra stripes do. It never changes the model.";
pub const STRIPES: &str = "Stripes";
pub const STRIPES_ALONG: &str = "Stripes along";
const PANEL: RightPanel = RightPanel { id: "analysis" };
const HEADER: PanelTitle<'static> = PanelTitle {
    glyph: icons::ANALYSIS,
    title: TITLE,
    close: CLOSE,
    help: Some(Page::FaceAnalysis),
};
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
    pub canvas: Canvas,
    pub shown: OnceCell<Vec<Arc<ShadedMesh>>>,
}

impl AnalysisContext<'_> {
    fn shown(&self) -> &[Arc<ShadedMesh>] {
        self.shown
            .get_or_init(|| analysis::shown_meshes(self.model, self.bodies))
    }

    pub fn into_shown(self) -> Vec<Arc<ShadedMesh>> {
        match self.shown.into_inner() {
            Some(shown) => shown,
            None => analysis::shown_meshes(self.model, self.bodies),
        }
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
enum LimitError {
    #[error(transparent)]
    Expression(#[from] ExpressionError),
    #[error(transparent)]
    Limit(#[from] Problem),
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
    let field = field::value_field(
        ui,
        Id::unique(("analysis-limit", id)),
        &document.expression_text(expression),
        FIELD_WIDTH,
        false,
        |text| -> Result<_, LimitError> {
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
                .map_err(ExpressionError::Unevaluable)?
                .value;
            check(value)?;
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
    let (caption, use_selected, reverse) = match tool.kind {
        Kind::Reach => (
            "Reach from",
            "Reach from along the selected axis, edge or face",
            "Reverse the reach direction",
        ),
        Kind::Draft | Kind::Radius | Kind::Curvature | Kind::Zebra | Kind::Chrome => (
            "Pull along",
            "Pull along the selected axis, edge or face",
            "Reverse the pull direction",
        ),
    };
    widgets::caption(ui, caption);
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
    ui.vertical(|ui| {
        let name = tool.pull_name(context.model);
        let reversed = if tool.reversed { ", reversed" } else { "" };
        ui.add(Label::new(format!("{name}{reversed}")).wrap_mode(TextWrapMode::Wrap));
        let selected = AnalysisTool::pull_from(context.model, context.selection);
        let button = widgets::small_button(ui, icons::USE_SELECTED, feature_fields::USE_SELECTED);
        let hover = match &selected {
            Ok(_) => use_selected.to_owned(),
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

    if let Some(reversed) = feature_fields::reverse_row(ui, reverse, tool.reversed) {
        tool.reversed = reversed;
    }
}

fn legend(
    ui: &mut Ui,
    context: &AnalysisContext<'_>,
    analysis: FaceAnalysis,
    commands: &mut CommandFrame<'_>,
) {
    let palette = context.contrast.palette(context.canvas);
    let units: Units = context.model.units();
    let meshes = context.shown();
    if meshes.is_empty() {
        ui.label(widgets::muted(NO_BODIES, ui));
        return;
    }
    let Some(analysis) = context.analyses.prepare(Some(analysis), meshes) else {
        return;
    };
    let tally = Tally::of(context.analyses, meshes, analysis);
    widgets::card(ui, |ui| {
        for band in analysis.bands() {
            let area = tally.areas.get(band).copied().unwrap_or(0.0);
            let area = format!("{APPROXIMATELY}{}", units.measured_area(area));
            ui.horizontal(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(vec2(SWATCH_SIDE, SWATCH_SIDE), Sense::hover());
                ui.painter().rect_filled(
                    rect,
                    SWATCH_RADIUS,
                    scene_palette::color32(band.colour(palette)),
                );
                ui.add(
                    Label::new(format!("{}: {area}", band.title()))
                        .wrap_mode(TextWrapMode::Wrap)
                        .selectable(true),
                )
                .on_hover_text(band.meaning());
            });
        }
    });
    let working = tally.working.then_some(()).ok_or(NOT_ANALYSING);
    let mut stop = commands.invoke(Command::CancelAnalysis, &working);
    if tally.working {
        stop |= widgets::progress(WORKING)
            .wrapping()
            .cancel(STOP_HOVER)
            .show(ui);
    }
    if stop {
        context.analyses.stop();
    }
    let stopped = tally.stopped.then_some(()).ok_or(NOT_STOPPED);
    let mut again = commands.invoke(Command::AnalyseAgain, &stopped);
    if tally.stopped {
        widgets::callout(ui, Tone::Info, |ui| {
            ui.add(Label::new(STOPPED).wrap_mode(TextWrapMode::Wrap));
            again |= ui
                .add(widgets::button(ANALYSE_AGAIN))
                .on_hover_text(ANALYSE_AGAIN_HOVER)
                .clicked();
        });
    }
    if again {
        context.analyses.resume();
    }
    if tally.failed {
        widgets::callout(ui, Tone::Warning, |ui| ui.label(FAILED));
    }
}

fn kind_row(ui: &mut Ui, tool: &mut AnalysisTool) {
    let choices: Vec<(&str, &str)> = Kind::ALL
        .iter()
        .map(|kind| (kind.title(), kind.meaning()))
        .collect();
    let chosen = Kind::ALL
        .iter()
        .position(|kind| *kind == tool.kind)
        .unwrap_or(0);
    if let Some(index) = widgets::segmented(ui, &choices, chosen)
        && let Some(kind) = Kind::ALL.get(index)
    {
        tool.kind = *kind;
    }
}

fn measure_row(ui: &mut Ui, tool: &mut AnalysisTool) {
    widgets::caption(ui, "Curvature");
    let choices: Vec<(&str, &str)> = Measure::ALL
        .iter()
        .map(|measure| (measure.title(), measure.meaning()))
        .collect();
    let chosen = Measure::ALL
        .iter()
        .position(|measure| *measure == tool.measure)
        .unwrap_or(0);
    if let Some(index) = widgets::segmented(ui, &choices, chosen)
        && let Some(measure) = Measure::ALL.get(index)
    {
        tool.measure = *measure;
    }
    ui.end_row();
}

fn stripe_rows(ui: &mut Ui, tool: &mut AnalysisTool) {
    widgets::caption(ui, STRIPES_ALONG);
    let choices: Vec<(&str, &str)> = AXES
        .iter()
        .map(|axis| (axis.letter(), axis.name()))
        .collect();
    let chosen = AXES
        .iter()
        .position(|axis| *axis == tool.stripes_along)
        .unwrap_or(0);
    if let Some(index) = widgets::segmented(ui, &choices, chosen)
        && let Some(axis) = AXES.get(index)
    {
        tool.stripes_along = *axis;
    }
    ui.end_row();

    widgets::caption(ui, STRIPES);
    let stripes = widgets::slider(
        ui,
        Slider::new(&mut tool.stripes, MIN_STRIPES..=MAX_STRIPES),
    );
    widgets::tie_to_caption(ui, &stripes);
    ui.end_row();
}

fn about(kind: Kind) -> &'static str {
    match kind {
        Kind::Draft => DRAFT_ABOUT,
        Kind::Radius => RADIUS_ABOUT,
        Kind::Reach => REACH_ABOUT,
        Kind::Curvature => CURVATURE_ABOUT,
        Kind::Zebra => ZEBRA_ABOUT,
        Kind::Chrome => CHROME_ABOUT,
    }
}

fn body(
    ui: &mut Ui,
    context: &AnalysisContext<'_>,
    tool: &mut AnalysisTool,
    commands: &mut CommandFrame<'_>,
) {
    kind_row(ui, tool);
    ui.add_space(SPACE_S);
    ui.label(widgets::muted(about(tool.kind), ui));
    ui.add_space(SPACE_S);
    widgets::properties(ui, "analysis-properties", |ui| match tool.kind {
        Kind::Draft => {
            pull_rows(ui, context, tool);
            limit_row(
                ui,
                context.model,
                ("Draft limit", "draft", Dimension::ANGLE),
                &mut tool.draft_limit,
                analysis::check_draft_limit,
            );
        }
        Kind::Radius => limit_row(
            ui,
            context.model,
            ("Smallest radius", "radius", Dimension::LENGTH),
            &mut tool.radius_limit,
            analysis::check_radius_limit,
        ),
        Kind::Reach => pull_rows(ui, context, tool),
        Kind::Curvature => {
            measure_row(ui, tool);
            limit_row(
                ui,
                context.model,
                ("Reference radius", "reference", Dimension::LENGTH),
                &mut tool.reference_radius,
                analysis::check_radius_limit,
            );
        }
        Kind::Zebra => stripe_rows(ui, tool),
        Kind::Chrome => {}
    });
    ui.add_space(SPACE_M);
    if !context.style.shows_faces() || context.style.is_translucent() {
        widgets::callout(ui, Tone::Info, |ui| ui.label(HIDDEN_STYLE));
        ui.add_space(SPACE_S);
    }
    match tool.analysis(context.model) {
        Ok(Analysis::Faces(analysis)) => legend(ui, context, analysis, commands),
        Ok(Analysis::Reflection(_)) => {}
        Err(problem) => {
            widgets::callout(ui, Tone::Warning, |ui| ui.label(problem.to_string()));
        }
    }
}

pub fn show(
    ui: &mut Ui,
    context: &AnalysisContext<'_>,
    tool: &mut AnalysisTool,
    room: f32,
    commands: &mut CommandFrame<'_>,
) {
    let shown = widgets::side_panel(
        ui,
        room,
        PANEL,
        &HEADER,
        |_| {},
        |ui| body(ui, context, tool, commands),
    );
    if shown.closed {
        tool.open = false;
    }
}
