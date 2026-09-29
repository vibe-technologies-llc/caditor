use caditor_document::Feature;
use caditor_sketch::{Constraint, ConstraintId, EntityId, Sketch};
use egui::{Frame, Ui};

use crate::{
    appearance,
    commands::{Command, CommandFrame},
    editing::{ActiveSketch, EditingCommand, SketchEditing, Tool},
    feature_tree::count,
    icons,
    model::{Action, Model},
    panels::{Focus, PanelState},
    selection::Selection,
    sketch_status::{self, SketchSummary},
    sketch_tools::{self, ConstraintTool, ConstructionChange},
    widgets::{self, ToolButton},
};

const FINISH_LABEL: &str = "Finish sketch";
const DELETE_LABEL: &str = "Delete";
const TOOL_GAP: f32 = 2.0;
const CONSTRAINT_COLUMNS: usize = 6;
const CONSTRAINT_WIDTH: f32 = 112.0;
const FINISH_HEIGHT: f32 = 32.0;
const SELECT_KEY: &str = "Esc";
const NOTHING_TO_DELETE: &str = "Select sketch geometry or constraints to delete them";
const CONSTRUCTION_LABEL: &str = "Construction";
const START_CONSTRUCTION: &str =
    "Draw construction geometry, which guides the sketch but makes no profile";
const STOP_CONSTRUCTION: &str = "Draw ordinary geometry again";

type Offer = (ConstraintTool, Result<Vec<Constraint>, String>);

pub fn show(
    ui: &mut Ui,
    model: &Model,
    editing: &SketchEditing,
    selection: &Selection,
    commands: &mut CommandFrame<'_>,
    panels: &mut PanelState,
    actions: &mut Vec<Action>,
) {
    let Some(active) = editing.active() else {
        return;
    };
    let Some(feature) = model.document().feature(active.feature) else {
        return;
    };
    let (Some(definition), Some(shown)) = (feature.kind.sketch(), model.displayed_sketch(feature))
    else {
        return;
    };
    let selected = sketch_tools::selected_entities(selection, feature.id());
    let offers: Vec<Offer> = ConstraintTool::ALL
        .into_iter()
        .map(|tool| {
            let candidates = tool
                .candidates(definition, &shown, &selected)
                .map(|constraints| sketch_tools::in_unit(constraints, model.length_unit()));
            (tool, candidates)
        })
        .collect();
    let deletable = Deletable {
        entities: selected
            .iter()
            .copied()
            .filter(|entity| !entity.is_reference())
            .collect(),
        constraints: sketch_tools::selected_constraints(selection, feature.id()),
    };
    let construction = ConstructionChange::of(definition, &selected);

    let mut request = Request::default();
    let tint = appearance::tokens(ui).accent_subtle;
    let frame = Frame::side_top_panel(ui.style()).fill(tint);
    egui::Panel::top("sketch-toolbar")
        .frame(frame)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = TOOL_GAP;
                header(ui, model, feature, panels);
                ui.separator();
                tools(ui, active, commands, actions);
                construction_button(ui, active, construction.as_ref(), commands, &mut request);
                ui.separator();
                constraint_buttons(ui, &offers, commands, &mut request);
                ui.separator();
                delete_button(ui, !deletable.is_empty(), commands, &mut request);
                ui.separator();
                finish_button(ui, commands, actions);
            });
        });

    if let Some((tool, constraints)) = request.constraints {
        let added = sketch_tools::add_constraints(model, feature.id(), tool, constraints);
        if tool.is_dimension()
            && let Some(constraint) = added.constraints.first()
        {
            panels.request_focus(Focus::Dimension {
                feature: feature.id(),
                constraint: *constraint,
            });
        }
        actions.push(Action::Apply(added.transaction));
    }
    if request.construction {
        actions.push(match &construction {
            Some(change) => Action::Apply(change.transaction(model, feature.id(), definition)),
            None => Action::Editing(EditingCommand::DrawConstruction(!active.construction)),
        });
    }
    if request.delete && !deletable.is_empty() {
        let label = deletable.label(definition);
        actions.push(Action::Apply(sketch_tools::remove_items(
            model,
            feature.id(),
            label,
            deletable.entities,
            deletable.constraints,
        )));
    }
}

struct Deletable {
    entities: Vec<EntityId>,
    constraints: Vec<ConstraintId>,
}

impl Deletable {
    fn is_empty(&self) -> bool {
        self.entities.is_empty() && self.constraints.is_empty()
    }

    fn label(&self, sketch: &Sketch) -> String {
        match (self.entities.as_slice(), self.constraints.as_slice()) {
            ([only], []) => format!("Delete {}", sketch.entity_label(*only)),
            ([], [only]) => format!("Delete {}", sketch.describe_constraint(*only)),
            (entities, constraints) => format!(
                "Delete {}",
                count(entities.len() + constraints.len(), "item", "items")
            ),
        }
    }
}

#[derive(Default)]
struct Request {
    constraints: Option<(ConstraintTool, Vec<Constraint>)>,
    construction: bool,
    delete: bool,
}

fn header(ui: &mut Ui, model: &Model, feature: &Feature, panels: &mut PanelState) {
    let tokens = appearance::tokens(ui);
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            widgets::icon_label(ui, icons::command(Command::NewSketch), tokens.accent_text);
            ui.strong(format!("Editing {}", feature.name));
        });
        ui.horizontal(|ui| {
            let summary = SketchSummary::of(model.evaluation(), feature.id());
            if let Some(focus) = sketch_status::show(ui, &summary) {
                panels.request_focus(focus);
            }
        });
    });
}

fn finish_button(ui: &mut Ui, commands: &mut CommandFrame<'_>, actions: &mut Vec<Action>) {
    let invoked = commands.available(Command::FinishSketch);
    let keys = commands
        .keys(Command::FinishSketch)
        .unwrap_or_else(|| "Esc with nothing selected".to_owned());
    let finish = widgets::primary_button(ui, FINISH_LABEL).min_size(egui::vec2(0.0, FINISH_HEIGHT));
    if ui
        .add(finish)
        .on_hover_text(format!("Leave the sketch. Everything is kept. ({keys})"))
        .clicked()
        || invoked
    {
        actions.push(Action::Editing(EditingCommand::Finish));
    }
}

fn tools(
    ui: &mut Ui,
    active: ActiveSketch,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    for tool in Tool::ALL {
        let command = Command::SketchTool(tool);
        let invoked = commands.available(command);
        let keys = commands
            .keys(command)
            .unwrap_or_else(|| SELECT_KEY.to_owned());
        let button = ToolButton::new(icons::tool(tool), tool.label()).selected(active.tool == tool);
        let response = ui
            .add(button)
            .on_hover_text(format!("{} ({keys})", tool.description()));
        if response.clicked() || invoked {
            actions.push(Action::Editing(EditingCommand::SetTool(tool)));
        }
    }
}

fn construction_button(
    ui: &mut Ui,
    active: ActiveSketch,
    change: Option<&ConstructionChange>,
    commands: &mut CommandFrame<'_>,
    request: &mut Request,
) {
    let invoked = commands.available(Command::Construction);
    let description = match change {
        Some(change) if change.construction => {
            "Make the selected curves construction geometry, which guides the sketch but makes no \
             profile"
        }
        Some(_) => "Make the selected curves ordinary geometry again",
        None if active.construction => STOP_CONSTRUCTION,
        None => START_CONSTRUCTION,
    };
    let button =
        ToolButton::new(icons::CONSTRUCTION, CONSTRUCTION_LABEL).selected(active.construction);
    let response = ui
        .add(button)
        .on_hover_text(commands.with_keys(Command::Construction, description));
    if response.clicked() || invoked {
        request.construction = true;
    }
}

fn constraint_buttons(
    ui: &mut Ui,
    offers: &[Offer],
    commands: &mut CommandFrame<'_>,
    request: &mut Request,
) {
    let muted = appearance::tokens(ui).text_muted;
    let cell = CONSTRAINT_WIDTH + TOOL_GAP;
    let fitting = (ui.max_rect().width() / cell).floor();
    let columns = if fitting >= CONSTRAINT_COLUMNS as f32 {
        CONSTRAINT_COLUMNS
    } else if fitting >= 2.0 {
        2
    } else {
        1
    };
    let remaining = ui.max_rect().right() - ui.cursor().min.x;
    if remaining < cell * columns as f32 {
        ui.end_row();
    }
    egui::Grid::new(("constraint-tools", columns))
        .num_columns(columns)
        .spacing([TOOL_GAP, TOOL_GAP])
        .show(ui, |ui| {
            for (index, (tool, offer)) in offers.iter().enumerate() {
                let command = Command::Constraint(*tool);
                let invoked = commands.invoke(command, offer);
                let button = egui::Button::new((
                    widgets::icon(icons::constraint(*tool)).color(muted),
                    tool.label(),
                    egui::Atom::grow(),
                ))
                .frame_when_inactive(false)
                .min_size(egui::vec2(CONSTRAINT_WIDTH, 0.0));
                let response =
                    ui.add_enabled(offer.is_ok(), widgets::Named::new(button, tool.label()));
                let response = match offer {
                    Ok(_) => {
                        response.on_hover_text(commands.with_keys(command, tool.description()))
                    }
                    Err(reason) => response.on_disabled_hover_text(
                        commands.with_keys(command, &format!("{}. {reason}", tool.description())),
                    ),
                };
                if (response.clicked() || invoked)
                    && let Ok(constraints) = offer
                {
                    request.constraints = Some((*tool, constraints.clone()));
                }
                if (index + 1) % columns == 0 {
                    ui.end_row();
                }
            }
        });
}

fn delete_button(
    ui: &mut Ui,
    enabled: bool,
    commands: &mut CommandFrame<'_>,
    request: &mut Request,
) {
    let availability = if enabled {
        Ok(())
    } else {
        Err(NOTHING_TO_DELETE)
    };
    let invoked = commands.invoke(Command::DeleteSelection, &availability);
    let button = ToolButton::new(icons::DELETE, DELETE_LABEL);
    let response = ui.add_enabled(enabled, button);
    let response = if enabled {
        response.on_hover_text(commands.with_keys(
            Command::DeleteSelection,
            "Delete the selected geometry and constraints, and the constraints on that geometry",
        ))
    } else {
        response.on_disabled_hover_text(NOTHING_TO_DELETE)
    };
    if response.clicked() || invoked {
        request.delete = true;
    }
}
