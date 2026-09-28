use caditor_document::Feature;
use caditor_sketch::{Constraint, ConstraintId, EntityId, Sketch};
use egui::{Button, Ui};

use crate::{
    commands::{Command, CommandFrame},
    editing::{ActiveSketch, EditingCommand, SketchEditing, Tool},
    feature_tree::count,
    model::{Action, Model},
    panels::{Focus, PanelState},
    scene,
    selection::Selection,
    sketch_status::{self, SketchSummary},
    sketch_tools::{self, ConstraintTool},
};

const FINISH_LABEL: &str = "Finish sketch";
const SELECT_KEY: &str = "Esc";
const NOTHING_TO_DELETE: &str = "Select sketch geometry or constraints to delete them";

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
    let (Some(definition), Some(shown)) = (
        feature.kind.sketch(),
        scene::displayed_sketch(model.evaluation(), feature),
    ) else {
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

    let mut request = Request::default();
    egui::Panel::top("sketch-toolbar").show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            header(ui, model, feature, commands, panels, actions);
            ui.separator();
            tools(ui, active, commands, actions);
            ui.separator();
            constraint_buttons(ui, &offers, commands, &mut request);
            ui.separator();
            delete_button(ui, !deletable.is_empty(), commands, &mut request);
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
    delete: bool,
}

fn header(
    ui: &mut Ui,
    model: &Model,
    feature: &Feature,
    commands: &mut CommandFrame<'_>,
    panels: &mut PanelState,
    actions: &mut Vec<Action>,
) {
    ui.strong(format!("Editing {}", feature.name));
    let invoked = commands.available(Command::FinishSketch);
    let keys = commands
        .keys(Command::FinishSketch)
        .unwrap_or_else(|| "Esc with nothing selected".to_owned());
    if ui
        .button(FINISH_LABEL)
        .on_hover_text(format!("Leave the sketch. Everything is kept. ({keys})"))
        .clicked()
        || invoked
    {
        actions.push(Action::Editing(EditingCommand::Finish));
    }
    let summary = SketchSummary::of(model.evaluation(), feature.id());
    if let Some(focus) = sketch_status::show(ui, &summary) {
        panels.request_focus(focus);
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
        let response = ui
            .add(Button::selectable(active.tool == tool, tool.button_text()))
            .on_hover_text(format!("{} ({keys})", tool.description()));
        if response.clicked() || invoked {
            actions.push(Action::Editing(EditingCommand::SetTool(tool)));
        }
    }
}

fn constraint_buttons(
    ui: &mut Ui,
    offers: &[Offer],
    commands: &mut CommandFrame<'_>,
    request: &mut Request,
) {
    for (tool, offer) in offers {
        let command = Command::Constraint(*tool);
        let invoked = commands.invoke(command, offer);
        let response = ui.add_enabled(offer.is_ok(), Button::new(tool.label()));
        let response = match offer {
            Ok(_) => response.on_hover_text(commands.with_keys(command, tool.description())),
            Err(reason) => response.on_disabled_hover_text(
                commands.with_keys(command, &format!("{}. {reason}", tool.description())),
            ),
        };
        if (response.clicked() || invoked)
            && let Ok(constraints) = offer
        {
            request.constraints = Some((*tool, constraints.clone()));
        }
    }
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
    let response = ui.add_enabled(enabled, Button::new("Delete"));
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
