use caditor_document::FeatureId;
use caditor_sketch::{EntityId, Sketch};
use egui::{Id, Label, TextEdit, TextWrapMode, Ui};

use crate::{
    appearance::{SPACE_M, SPACE_S},
    editing::{EditingCommand, Tool},
    feature_tree::count,
    guide::Page,
    icons,
    layout::RightPanel,
    model::{Action, Model, Notice},
    patterning::{PatternEvaluation, PatternField, PatternKind, PatternSettings, Patterning},
    trimming::capitalized,
    widgets::{self, PanelTitle, Tone},
};

pub const SHAPE: &str = "Shape";
pub const CENTRE: &str = "Centre";
pub const REPEATS: &str = "Repeats";
pub const NO_CENTRE: &str = "None chosen";
pub const CENTRE_HINT: &str = "Click a point in the sketch to repeat about.";
pub const USE_ORIGIN: &str = "Use the origin";
pub const NOTHING_REPEATED: &str =
    "Nothing yet: click items in the sketch or drag a box around them.";
pub const MAKE: &str = "Make the copies";
pub const RECTANGULAR_ABOUT: &str = "Repeat the selection in a row, or in a grid with more than \
                                     one row. Every value takes units and expressions using your \
                                     parameters; the copies keep following them.";
pub const CIRCULAR_ABOUT: &str = "Repeat the selection about a point. Every value takes units \
                                  and expressions using your parameters; the copies keep \
                                  following them.";
pub const FULL_TURN_NOTE: &str = "A total angle of 360° spaces the copies evenly; a smaller angle \
                                  runs from the original to the last copy.";
const PANEL: RightPanel = RightPanel {
    id: "sketch-pattern",
};

pub fn field_id(field: PatternField) -> Id {
    Id::unique(("sketch-pattern-field", field.key()))
}

pub fn close_label(kind: PatternKind) -> String {
    format!("Close {}", kind.tool().label())
}

pub struct Context<'a> {
    pub model: &'a Model,
    pub feature: FeatureId,
    pub sketch: Option<&'a Sketch>,
}

pub fn show(
    ui: &mut Ui,
    context: &Context<'_>,
    (settings, patterning): (&mut PatternSettings, &mut Patterning),
    room: f32,
) -> Vec<Action> {
    let mut actions = Vec::new();
    let kind = patterning.kind();
    let evaluation = settings.evaluate(context.model, kind);
    let rows = settings.shows_rows(context.model);
    let centre = patterning.chosen_centre();
    let ready = evaluation.spec.is_ok()
        && patterning.repeated() > 0
        && (kind == PatternKind::Rectangular || centre.is_some())
        && patterning.problem().is_none();
    let close = close_label(kind);
    let header = PanelTitle {
        glyph: icons::tool(kind.tool()),
        title: kind.tool().label(),
        close: &close,
        help: Some(Page::SketchPatterns),
    };
    let mut chosen = Chosen::default();
    let panel = widgets::side_panel_parts(
        ui,
        room,
        PANEL,
        &header,
        |_| {},
        |ui| {
            let pressed = widgets::panel_footer(ui, PANEL.id, |ui| {
                widgets::footer(ui, |ui| {
                    ui.add_enabled(ready, widgets::primary_button(ui, MAKE))
                        .clicked()
                })
            });
            widgets::panel_scroll(ui, PANEL.id, |ui| {
                let about = match kind {
                    PatternKind::Rectangular => RECTANGULAR_ABOUT,
                    PatternKind::Circular => CIRCULAR_ABOUT,
                };
                wrapped(ui, widgets::muted(about, ui));
                ui.add_space(SPACE_S);
                widgets::properties(ui, "sketch-pattern-values", |ui| {
                    chosen.kind = shapes(ui, kind);
                    if kind == PatternKind::Circular {
                        chosen.origin = centre_row(ui, context.sketch, centre);
                    }
                    let fields = match kind {
                        PatternKind::Rectangular if rows => {
                            [PatternField::RECTANGULAR.as_slice(), &PatternField::ROWS].concat()
                        }
                        PatternKind::Rectangular => PatternField::RECTANGULAR.to_vec(),
                        PatternKind::Circular => PatternField::CIRCULAR.to_vec(),
                    };
                    for field in fields {
                        field_row(ui, settings, field, &evaluation);
                    }
                    widgets::property(ui, REPEATS, |ui| match patterning.repeated() {
                        0 => wrapped(ui, widgets::muted(NOTHING_REPEATED, ui)),
                        items => wrapped(ui, count(items, "item", "items")),
                    });
                });
                ui.add_space(SPACE_M);
                outcome(ui, kind, &evaluation, patterning);
            });
            pressed
        },
    );
    if panel.closed {
        actions.push(Action::Editing(EditingCommand::SetTool(Tool::Select)));
    }
    if let Some(other) = chosen.kind.filter(|other| *other != kind) {
        actions.push(Action::Editing(EditingCommand::SetTool(other.tool())));
    }
    if chosen.origin {
        patterning.choose_centre(EntityId::ORIGIN);
    }
    if panel.inner {
        actions.push(match patterning.make(context.model, context.feature) {
            Ok(transaction) => Action::Apply(transaction),
            Err(reason) => Action::Inform(Notice::warning(reason.to_string())),
        });
    }
    actions
}

#[derive(Default)]
struct Chosen {
    kind: Option<PatternKind>,
    origin: bool,
}

fn shapes(ui: &mut Ui, kind: PatternKind) -> Option<PatternKind> {
    widgets::property(ui, SHAPE, |ui| {
        let choices = PatternKind::ALL.map(|kind| (kind.label(), kind.about()));
        let current = PatternKind::ALL
            .iter()
            .position(|shown| *shown == kind)
            .unwrap_or(usize::MAX);
        widgets::segmented(ui, &choices, current)
            .and_then(|index| PatternKind::ALL.get(index).copied())
    })
}

fn centre_row(ui: &mut Ui, sketch: Option<&Sketch>, centre: Option<EntityId>) -> bool {
    widgets::property(ui, CENTRE, |ui| {
        ui.vertical(|ui| {
            let named = centre
                .zip(sketch)
                .map(|(point, sketch)| sketch.entity_label(point));
            match named {
                Some(name) => wrapped(ui, name),
                None => wrapped(ui, widgets::muted(NO_CENTRE, ui)),
            }
            wrapped(ui, widgets::muted(CENTRE_HINT, ui));
            centre != Some(EntityId::ORIGIN) && ui.add(widgets::button(USE_ORIGIN)).clicked()
        })
        .inner
    })
}

fn field_row(
    ui: &mut Ui,
    settings: &mut PatternSettings,
    field: PatternField,
    evaluation: &PatternEvaluation,
) {
    widgets::property(ui, field.label(), |ui| {
        let Some(text) = settings.text_mut(field) else {
            return;
        };
        let response = widgets::text_field(ui, |ui| {
            ui.add(
                TextEdit::singleline(text)
                    .id(field_id(field))
                    .hint_text(field.placeholder())
                    .desired_width(f32::INFINITY),
            )
        });
        widgets::tie_to_caption(ui, &response);
        response.on_hover_text(field.about());
    });
    if let Some((_, problem)) = evaluation
        .problems
        .iter()
        .find(|(wrong, _)| *wrong == field)
    {
        widgets::error_row(ui, &problem.to_string());
    }
}

fn outcome(
    ui: &mut Ui,
    kind: PatternKind,
    evaluation: &PatternEvaluation,
    patterning: &Patterning,
) {
    if let Err(problem) = &evaluation.spec {
        widgets::callout(ui, Tone::Error, |ui| wrapped(ui, problem.to_string()));
        return;
    }
    if let Some(problem) = patterning.problem() {
        widgets::callout(ui, Tone::Error, |ui| {
            wrapped(ui, capitalized(&format!("{problem}.")))
        });
        return;
    }
    if kind == PatternKind::Circular {
        widgets::callout(ui, Tone::Info, |ui| wrapped(ui, FULL_TURN_NOTE));
    }
}

fn wrapped(ui: &mut Ui, text: impl Into<egui::WidgetText>) {
    ui.add(Label::new(text).wrap_mode(TextWrapMode::Wrap));
}
