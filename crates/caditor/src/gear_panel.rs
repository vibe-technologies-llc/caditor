use caditor_document::FeatureId;
use caditor_sketch::{EntityId, GearCentre, Sketch, SpurGear};
use egui::{Id, Label, ScrollArea, TextEdit, TextWrapMode, Ui};

use crate::{
    appearance::{SPACE_M, SPACE_S},
    editing::{EditingCommand, Tool},
    gearing::{self, GearField, GearSettings},
    guide::Page,
    guide_panel, icons,
    layout::RightPanel,
    model::{Action, Model, Notice},
    modifying::length_text,
    units::LengthUnit,
    widgets::{self, Tone},
};

pub const TITLE: &str = "Spur gear";
pub const CLOSE: &str = "Close Spur gear";
pub const DESCRIPTION: &str = "Set the gear's values, then click in the sketch where its centre \
                               goes, or draw it at the selected point or the origin. Every value \
                               takes units and expressions using your parameters.";
pub const CIRCLES: &str = "Its circles, drawn as construction geometry";
const PANEL: RightPanel = RightPanel {
    id: "spur-gear",
    width: 320.0,
    least: 240.0,
};

pub fn field_id(field: GearField) -> Id {
    Id::new(("spur-gear-field", field.label()))
}

pub fn draw_label(sketch: Option<&Sketch>, centre: Option<EntityId>) -> String {
    let words = sketch.map_or_else(
        || "the origin".to_owned(),
        |sketch| gearing::centre_words(sketch, centre),
    );
    format!("Draw at {words}")
}

pub struct Context<'a> {
    pub model: &'a Model,
    pub feature: FeatureId,
    pub sketch: Option<&'a Sketch>,
    pub selected: &'a [EntityId],
}

pub fn show(
    ui: &mut Ui,
    context: &Context<'_>,
    settings: &mut GearSettings,
    room: f32,
) -> Vec<Action> {
    let mut actions = Vec::new();
    let evaluation = settings.evaluate(context.model);
    let centre = context
        .sketch
        .and_then(|sketch| gearing::lone_point(sketch, context.selected));
    let label = draw_label(context.sketch, centre);
    let mut close = false;
    let mut draw = false;
    PANEL.show(ui, room, |ui| {
        ui.add_space(SPACE_S);
        widgets::panel_header(ui, icons::tool(Tool::Gear), TITLE, |ui| {
            close = widgets::icon_button(ui, icons::CLOSE, CLOSE).clicked();
            guide_panel::help_button(ui, Page::Gear);
        });
        ui.add_space(SPACE_S);
        egui::Panel::bottom("spur-gear-footer")
            .show_separator_line(false)
            .show(ui, |ui| {
                draw = widgets::footer(ui, |ui| {
                    ui.add_enabled(evaluation.gear.is_ok(), widgets::primary_button(ui, &label))
                        .clicked()
                });
            });
        ScrollArea::vertical().show(ui, |ui| {
            wrapped(ui, widgets::muted(DESCRIPTION, ui));
            ui.add_space(SPACE_S);
            fields(ui, settings, &evaluation.problems);
            ui.add_space(SPACE_M);
            match &evaluation.gear {
                Ok(gear) => circles(ui, gear, context.model.length_unit()),
                Err(problem) => {
                    widgets::callout(ui, Tone::Error, |ui| wrapped(ui, problem.as_str()));
                }
            }
        });
    });
    if close {
        actions.push(Action::Editing(EditingCommand::SetTool(Tool::Select)));
    }
    if draw && let Ok(gear) = &evaluation.gear {
        let centre = GearCentre::Point(centre.unwrap_or(EntityId::ORIGIN));
        actions.push(
            match gearing::draw(context.model, context.feature, gear, centre) {
                Ok(transaction) => Action::Apply(transaction),
                Err(reason) => Action::Inform(Notice::warning(reason)),
            },
        );
    }
    actions
}

fn fields(ui: &mut Ui, settings: &mut GearSettings, problems: &[(GearField, String)]) {
    widgets::properties(ui, "spur-gear-values", |ui| {
        for field in GearField::ALL {
            let hint = match field {
                GearField::Bore => "none",
                _ => "",
            };
            widgets::property(ui, field.label(), |ui| {
                let Some(text) = settings.text_mut(field) else {
                    return;
                };
                let response = widgets::text_field(ui, |ui| {
                    ui.add(
                        TextEdit::singleline(text)
                            .id(field_id(field))
                            .hint_text(hint)
                            .desired_width(f32::INFINITY),
                    )
                });
                widgets::tie_to_caption(ui, &response);
                response.on_hover_text(field.about());
            });
            if let Some((_, problem)) = problems.iter().find(|(wrong, _)| *wrong == field) {
                widgets::error_row(ui, problem);
            }
        }
    });
}

fn circles(ui: &mut Ui, gear: &SpurGear, unit: LengthUnit) {
    wrapped(ui, widgets::muted(CIRCLES, ui));
    ui.add_space(SPACE_S);
    let circles = gear.circles();
    widgets::properties(ui, "spur-gear-circles", |ui| {
        for (caption, radius) in [
            ("Pitch diameter", circles.pitch),
            ("Tip diameter", circles.tip),
            ("Root diameter", circles.root),
            ("Base diameter", circles.base),
        ] {
            widgets::property(ui, caption, |ui| {
                ui.label(length_text(unit, 2.0 * radius));
            });
        }
    });
}

fn wrapped(ui: &mut Ui, text: impl Into<egui::WidgetText>) {
    ui.add(Label::new(text).wrap_mode(TextWrapMode::Wrap));
}
