use caditor_document::{FeatureId, Transaction};
use caditor_geometry::Point2;
use caditor_sketch::{EntityId, Sketch};
use egui::{Id, Label, TextEdit, TextWrapMode, Ui};

use crate::{
    appearance::{SPACE_M, SPACE_S},
    editing::{EditingCommand, Tool},
    guide::Page,
    icons,
    layout::RightPanel,
    model::{Action, Model, Notice},
    texting::{
        self, FieldProblem, Lettering, LetteringProblem, TextAnchor, TextField, TextSettings,
        TextingError,
    },
    widgets::{self, PanelTitle, Tone},
};

pub const TITLE: &str = "Text";
pub const CLOSE: &str = "Close Text";
pub const DESCRIPTION: &str = "Type the text and set its height and angle, then click in the \
                               sketch where its baseline starts, or write it at the selected \
                               point or the origin. Height and angle take units and expressions \
                               using your parameters.";
pub const EDITING: &str = "Change the selected text's letters, height, angle or style, then \
                           update it. Its point stays where it is and keeps its constraints.";
pub const BOLD: &str = "Bold";
pub const ITALIC: &str = "Italic";
pub const NEW_TEXT: &str = "Write new text instead";
const PANEL: RightPanel = RightPanel { id: "sketch-text" };

pub fn field_id(field: TextField) -> Id {
    Id::unique(("sketch-text-field", field.label()))
}

pub fn write_label(sketch: Option<&Sketch>, point: Option<EntityId>) -> String {
    let words = match (sketch, point) {
        (Some(sketch), Some(point)) => sketch.entity_label(point),
        _ => "the origin".to_owned(),
    };
    format!("Write at {words}")
}

pub fn update_label(sketch: Option<&Sketch>, text: EntityId) -> String {
    let name = sketch.map_or_else(
        || format!("Text {text}"),
        |sketch| sketch.entity_label(text),
    );
    format!("Update {name}")
}

pub struct Context<'a> {
    pub model: &'a Model,
    pub feature: FeatureId,
    pub sketch: Option<&'a Sketch>,
    pub selected: &'a [EntityId],
}

enum Pressed {
    Primary,
    NewText,
}

pub fn show(
    ui: &mut Ui,
    context: &Context<'_>,
    settings: &mut TextSettings,
    room: f32,
) -> Vec<Action> {
    let mut actions = Vec::new();
    settings.load(context.model, context.sketch);
    let evaluation = settings.evaluate(context.model);
    let editing = settings.editing();
    let point = context
        .sketch
        .and_then(|sketch| texting::lone_point(sketch, context.selected));
    let label = match editing {
        Some(text) => update_label(context.sketch, text),
        None => write_label(context.sketch, point),
    };
    let header = PanelTitle {
        glyph: icons::tool(Tool::Text),
        title: TITLE,
        close: CLOSE,
        help: Some(Page::Text),
    };
    let panel = widgets::side_panel_parts(
        ui,
        room,
        PANEL,
        &header,
        |_| {},
        |ui| {
            let pressed = widgets::panel_footer(ui, PANEL.id, |ui| {
                widgets::footer(ui, |ui| {
                    let primary = ui
                        .add_enabled(
                            evaluation.lettering.is_ok(),
                            widgets::primary_button(ui, &label),
                        )
                        .clicked();
                    let new_text = editing.is_some() && ui.add(widgets::button(NEW_TEXT)).clicked();
                    match (primary, new_text) {
                        (true, _) => Some(Pressed::Primary),
                        (false, true) => Some(Pressed::NewText),
                        (false, false) => None,
                    }
                })
            });
            widgets::panel_scroll(ui, PANEL.id, |ui| {
                let about = if editing.is_some() {
                    EDITING
                } else {
                    DESCRIPTION
                };
                wrapped(ui, widgets::muted(about, ui));
                ui.add_space(SPACE_S);
                fields(ui, settings, &evaluation.problems);
                ui.add_space(SPACE_S);
                styles(ui, settings);
                ui.add_space(SPACE_M);
                outcome(ui, &evaluation.lettering);
            });
            pressed
        },
    );
    if panel.closed {
        actions.push(Action::Editing(EditingCommand::SetTool(Tool::Select)));
    }
    match (panel.inner, &evaluation.lettering, editing) {
        (Some(Pressed::NewText), ..) => settings.stop_editing(),
        (Some(Pressed::Primary), Ok(lettering), Some(text)) => {
            actions.push(applied(texting::update(
                context.model,
                context.feature,
                text,
                lettering,
            )));
        }
        (Some(Pressed::Primary), Ok(lettering), None) => {
            let anchor = TextAnchor::Point(point.unwrap_or(EntityId::ORIGIN));
            actions.push(applied(texting::write(
                context.model,
                context.feature,
                lettering,
                anchor,
            )));
        }
        (Some(Pressed::Primary), Err(_), _) | (None, ..) => {}
    }
    actions
}

fn applied(outcome: Result<Transaction, TextingError>) -> Action {
    match outcome {
        Ok(transaction) => Action::Apply(transaction),
        Err(reason) => Action::Inform(Notice::warning(reason.to_string())),
    }
}

fn fields(ui: &mut Ui, settings: &mut TextSettings, problems: &[(TextField, FieldProblem)]) {
    widgets::properties(ui, "sketch-text-values", |ui| {
        for field in TextField::ALL {
            widgets::property(ui, field.label(), |ui| {
                let Some(text) = settings.text_mut(field) else {
                    return;
                };
                let response = widgets::text_field(ui, |ui| {
                    ui.add(
                        TextEdit::singleline(text)
                            .id(field_id(field))
                            .desired_width(f32::INFINITY),
                    )
                });
                widgets::tie_to_caption(ui, &response);
                response.on_hover_text(field.about());
            });
            if let Some((_, problem)) = problems.iter().find(|(wrong, _)| *wrong == field) {
                widgets::error_row(ui, &problem.to_string());
            }
        }
    });
}

fn styles(ui: &mut Ui, settings: &mut TextSettings) {
    ui.horizontal_wrapped(|ui| {
        widgets::checkbox(ui, settings.bold_mut(), BOLD);
        widgets::checkbox(ui, settings.italic_mut(), ITALIC);
    });
}

fn outcome(ui: &mut Ui, lettering: &Result<Lettering, LetteringProblem>) {
    match lettering {
        Ok(lettering) => {
            let missing = lettering
                .placed(Point2::ZERO)
                .map(|placed| placed.missing)
                .unwrap_or_default();
            if !missing.is_empty() {
                let letters: String = missing.iter().collect();
                widgets::callout(ui, Tone::Warning, |ui| {
                    wrapped(
                        ui,
                        format!(
                            "Inter has no letters for {letters}, so they are left out. Type \
                             them another way or leave them off."
                        ),
                    );
                });
            }
        }
        Err(problem) => {
            widgets::callout(ui, Tone::Error, |ui| wrapped(ui, problem.to_string()));
        }
    }
}

fn wrapped(ui: &mut Ui, text: impl Into<egui::WidgetText>) {
    ui.add(Label::new(text).wrap_mode(TextWrapMode::Wrap));
}
