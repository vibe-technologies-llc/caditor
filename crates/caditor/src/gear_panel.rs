use caditor_document::FeatureId;
use caditor_sketch::{ChainSize, EntityId, GearCentre, Sketch};
use egui::{Id, Label, TextEdit, TextWrapMode, Ui};

use crate::{
    appearance::{SPACE_M, SPACE_S},
    editing::{EditingCommand, Tool},
    gearing::{self, FieldProblem, GearField, GearKind, GearSettings, Part},
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
            let draw = widgets::panel_footer(ui, PANEL.id, |ui| {
                widgets::footer(ui, |ui| {
                    ui.add_enabled(evaluation.part.is_ok(), widgets::primary_button(ui, &label))
                        .clicked()
                })
            });
            widgets::panel_scroll(ui, PANEL.id, |ui| {
                wrapped(ui, widgets::muted(DESCRIPTION, ui));
                ui.add_space(SPACE_S);
                kinds(ui, settings);
                ui.add_space(SPACE_S);
                fields(ui, settings, &evaluation.problems, context.model.units());
                ui.add_space(SPACE_M);
                match &evaluation.part {
                    Ok(part) => circles(ui, part, context.model.units()),
                    Err(problem) => {
                        widgets::callout(ui, Tone::Error, |ui| wrapped(ui, problem.to_string()));
                    }
                }
            });
            draw
        },
    );
    if panel.closed {
        actions.push(Action::Editing(EditingCommand::SetTool(Tool::Select)));
    }
    if panel.inner
        && let Ok(part) = &evaluation.part
    {
        let centre = GearCentre::Point(centre.unwrap_or(EntityId::ORIGIN));
        actions.push(
            match gearing::draw(context.model, context.feature, part, centre) {
                Ok(transaction) => Action::Apply(transaction),
                Err(reason) => Action::Inform(Notice::warning(reason.to_string())),
            },
        );
    }
    actions
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
        Part::Gear(gear) => {
            let circles = gear.circles();
            vec![
                ("Pitch diameter", 2.0 * circles.pitch),
                ("Tip diameter", 2.0 * circles.tip),
                ("Root diameter", 2.0 * circles.root),
                ("Base diameter", 2.0 * circles.base),
            ]
        }
        Part::Pair(pair) => {
            let first = pair.driver.circles();
            let second = pair.partner().circles();
            vec![
                ("Centre distance", pair.centre_distance()),
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
