use caditor_document::{FeatureId, Transaction};
use caditor_sketch::{ChainSize, EntityId, GearCentre, Sketch};
use egui::{Id, Label, TextEdit, TextWrapMode, Ui};

use crate::{
    appearance::{SPACE_M, SPACE_S},
    editing::{EditingCommand, Tool},
    gearing::{self, FieldProblem, GearField, GearKind, GearSettings, GearingError, Part},
    guide::Page,
    icons,
    layout::RightPanel,
    model::{Action, Model, Notice},
    modifying::length_text,
    units::Units,
    widgets::{self, PanelTitle, Tone},
};

pub const TITLE: &str = "Spur gear";
pub const CLOSE: &str = "Close Spur gear";
pub const DESCRIPTION: &str = "Choose what to draw and set its values, then click in the sketch \
                               where its centre goes, or draw it at the selected point or the \
                               origin. Every value takes units and expressions using your \
                               parameters.";
pub const EDITING: &str = "Change the selected gear's values, then update it. Its centre stays \
                           where it is and keeps its constraints. A gear drawn here follows \
                           later changes to the parameters its values use.";
pub const EDITING_MESHING: &str = "This gear meshes with another and follows it: its module, \
                                   pressure angle and root fillet are the first gear's, its \
                                   profile shift the opposite, it turns by the tooth ratio and \
                                   sits at the standard centre distance. Change its teeth, hole \
                                   or direction here, and the rest on the first gear.";
pub const NEW_GEAR: &str = "Draw a new gear instead";
pub const CHAIN: &str = "Chain";
pub const CHAIN_ABOUT: &str = "The ISO 606 roller chain the sprocket is cut for: its pitch and \
                               roller diameter set the teeth";
pub const CIRCLES: &str = "Its circles, drawn as construction geometry";
const PANEL: RightPanel = RightPanel { id: "spur-gear" };

pub fn field_id(field: GearField) -> Id {
    Id::unique(("spur-gear-field", field.label()))
}

pub fn chain_id() -> Id {
    Id::unique("spur-gear-chain")
}

pub fn draw_label(sketch: Option<&Sketch>, centre: Option<EntityId>) -> String {
    let words = sketch.map_or_else(
        || "the origin".to_owned(),
        |sketch| gearing::centre_words(sketch, centre),
    );
    format!("Draw at {words}")
}

pub fn update_label(sketch: Option<&Sketch>, gear: EntityId) -> String {
    let name = sketch.map_or_else(
        || format!("Gear {gear}"),
        |sketch| sketch.entity_label(gear),
    );
    format!("Update {name}")
}

enum Pressed {
    Primary,
    NewGear,
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
    settings.load(context.model, context.sketch);
    let evaluation = settings.evaluate(context.model, context.sketch);
    let editing = settings.editing();
    let centre = context
        .sketch
        .and_then(|sketch| gearing::lone_point(sketch, context.selected));
    let label = match editing {
        Some(gear) => update_label(context.sketch, gear),
        None => draw_label(context.sketch, centre),
    };
    let header = PanelTitle {
        glyph: icons::tool(Tool::Gear),
        title: TITLE,
        close: CLOSE,
        help: Some(Page::Gear),
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
                        .add_enabled(evaluation.part.is_ok(), widgets::primary_button(ui, &label))
                        .clicked();
                    let new_gear = editing.is_some() && ui.add(widgets::button(NEW_GEAR)).clicked();
                    match (primary, new_gear) {
                        (true, _) => Some(Pressed::Primary),
                        (false, true) => Some(Pressed::NewGear),
                        (false, false) => None,
                    }
                })
            });
            widgets::panel_scroll(ui, PANEL.id, |ui| {
                let about = match (editing, settings.kind()) {
                    (Some(_), GearKind::Meshing) => EDITING_MESHING,
                    (Some(_), _) => EDITING,
                    (None, _) => DESCRIPTION,
                };
                wrapped(ui, widgets::muted(about, ui));
                ui.add_space(SPACE_S);
                if let Some(note) = editing.and_then(|gear| followers(context.sketch, gear)) {
                    wrapped(ui, widgets::muted(note, ui));
                    ui.add_space(SPACE_S);
                }
                if editing.is_none() {
                    kinds(ui, settings);
                    ui.add_space(SPACE_S);
                }
                fields(ui, settings, &evaluation.problems, context.model.units());
                ui.add_space(SPACE_M);
                match &evaluation.part {
                    Ok(part) => circles(ui, part, context.model.units()),
                    Err(problem) => {
                        widgets::callout(ui, Tone::Error, |ui| wrapped(ui, problem.to_string()));
                    }
                }
            });
            pressed
        },
    );
    if panel.closed {
        actions.push(Action::Editing(EditingCommand::SetTool(Tool::Select)));
    }
    match (panel.inner, &evaluation.part, editing) {
        (Some(Pressed::NewGear), ..) => settings.stop_editing(),
        (Some(Pressed::Primary), Ok(Part::Gear(spur)), Some(gear)) => {
            actions.push(applied(gearing::update(
                context.model,
                context.feature,
                gear,
                spur,
            )));
        }
        (Some(Pressed::Primary), Ok(Part::Pair(pair)), Some(gear)) => {
            actions.push(applied(gearing::update_meshing(
                context.model,
                context.feature,
                gear,
                pair,
            )));
        }
        (Some(Pressed::Primary), Ok(part), None) => {
            let centre = GearCentre::Point(centre.unwrap_or(EntityId::ORIGIN));
            actions.push(applied(gearing::draw(
                context.model,
                context.feature,
                part,
                centre,
            )));
        }
        (Some(Pressed::Primary), ..) | (None, ..) => {}
    }
    actions
}

fn followers(sketch: Option<&Sketch>, gear: EntityId) -> Option<String> {
    let sketch = sketch?;
    let partners = sketch.partners_of(gear);
    if partners.is_empty() {
        return None;
    }
    let names: Vec<String> = partners
        .into_iter()
        .map(|partner| sketch.entity_label(partner))
        .collect();
    Some(format!(
        "{} meshes with it and follows its values.",
        names.join(", ")
    ))
}

fn applied(outcome: Result<Transaction, GearingError>) -> Action {
    match outcome {
        Ok(transaction) => Action::Apply(transaction),
        Err(reason) => Action::Inform(Notice::warning(reason.to_string())),
    }
}

fn kinds(ui: &mut Ui, settings: &mut GearSettings) {
    let choices = GearKind::ALL.map(|kind| (kind.label(), kind.about()));
    let chosen = GearKind::ALL
        .iter()
        .position(|kind| *kind == settings.kind())
        .unwrap_or_default();
    if let Some(kind) =
        widgets::segmented(ui, &choices, chosen).and_then(|index| GearKind::ALL.get(index).copied())
    {
        settings.set_kind(kind);
    }
}

fn fields(
    ui: &mut Ui,
    settings: &mut GearSettings,
    problems: &[(GearField, FieldProblem)],
    unit: Units,
) {
    let kind = settings.kind();
    widgets::properties(ui, "spur-gear-values", |ui| {
        if kind == GearKind::Sprocket {
            widgets::property(ui, CHAIN, |ui| chain(ui, settings, unit));
        }
        for field in kind.fields() {
            let field = *field;
            widgets::property(ui, field.label(), |ui| {
                let Some(text) = settings.text_mut(field) else {
                    return;
                };
                let response = widgets::text_field(ui, |ui| {
                    ui.add(
                        TextEdit::singleline(text)
                            .id(field_id(field))
                            .hint_text(field.hint())
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

fn chain(ui: &mut Ui, settings: &mut GearSettings, unit: Units) {
    let current = settings.chain();
    let mut chosen = None;
    widgets::combo(chain_id(), gearing::chain_label(current, unit))
        .hover(CHAIN_ABOUT)
        .show(ui, |ui| {
            for size in ChainSize::ALL {
                let label = gearing::chain_label(size, unit);
                if widgets::menu_option(ui, size == current, &label).clicked() {
                    chosen = Some(size);
                }
            }
        });
    if let Some(size) = chosen {
        settings.set_chain(size);
    }
}

fn circles(ui: &mut Ui, part: &Part, unit: Units) {
    wrapped(ui, widgets::muted(CIRCLES, ui));
    ui.add_space(SPACE_S);
    let rows: Vec<(&str, f64)> = match part {
        Part::Gear(spur) => {
            let circles = spur.values.circles();
            vec![
                ("Pitch diameter", 2.0 * circles.pitch),
                ("Tip diameter", 2.0 * circles.tip),
                ("Root diameter", 2.0 * circles.root),
                ("Base diameter", 2.0 * circles.base),
            ]
        }
        Part::Pair(pair) => {
            let first = pair.placing.driver.circles();
            let second = pair.placing.partner().circles();
            vec![
                ("Centre distance", pair.placing.centre_distance()),
                ("Pitch diameter", 2.0 * first.pitch),
                ("Tip diameter", 2.0 * first.tip),
                ("Meshing pitch diameter", 2.0 * second.pitch),
                ("Meshing tip diameter", 2.0 * second.tip),
            ]
        }
        Part::Sprocket(sprocket) => {
            let circles = sprocket.circles();
            vec![
                ("Pitch diameter", 2.0 * circles.pitch),
                ("Tip diameter", 2.0 * circles.tip),
                ("Root diameter", 2.0 * circles.root),
                ("Roller diameter", sprocket.chain.roller),
            ]
        }
    };
    widgets::properties(ui, "spur-gear-circles", |ui| {
        for (caption, length) in rows {
            widgets::property(ui, caption, |ui| {
                ui.label(length_text(unit, length));
            });
        }
    });
}

fn wrapped(ui: &mut Ui, text: impl Into<egui::WidgetText>) {
    ui.add(Label::new(text).wrap_mode(TextWrapMode::Wrap));
}
