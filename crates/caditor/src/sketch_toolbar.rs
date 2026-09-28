use caditor_document::Feature;
use caditor_sketch::{Constraint, ConstraintId, EntityId, Sketch};
use egui::{Button, Key, KeyboardShortcut, Modifiers, Ui};

use crate::{
    editing::{ActiveSketch, EditingCommand, SketchEditing, Tool},
    feature_tree::count,
    model::{Action, Model, Notice},
    panels::{Focus, PanelState},
    scene,
    selection::Selection,
    sketch_status::{self, SketchSummary},
    sketch_tools::{self, ConstraintTool},
};

const DELETE_KEYS: [Key; 2] = [Key::Delete, Key::Backspace];
const FINISH_LABEL: &str = "Finish sketch";
const SELECT_KEY: &str = "Esc";

type Offer = (ConstraintTool, Result<Vec<Constraint>, String>);

pub struct SketchInput<'a> {
    pub selection: &'a Selection,
    pub keys_free: bool,
    pub drawing: bool,
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    editing: &SketchEditing,
    input: &SketchInput<'_>,
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
    let selected = sketch_tools::selected_entities(input.selection, feature.id());
    let offers: Vec<Offer> = ConstraintTool::ALL
        .into_iter()
        .map(|tool| (tool, tool.candidates(definition, &shown, &selected)))
        .collect();
    let deletable = Deletable {
        entities: selected
            .iter()
            .copied()
            .filter(|entity| !entity.is_reference())
            .collect(),
        constraints: sketch_tools::selected_constraints(input.selection, feature.id()),
    };

    let mut request = Request::default();
    egui::Panel::top("sketch-toolbar").show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            header(ui, model, feature, panels, actions);
            ui.separator();
            tools(ui, active, actions);
            ui.separator();
            constraint_buttons(ui, &offers, &mut request);
            ui.separator();
            delete_button(ui, !deletable.is_empty(), &mut request);
        });
    });
    if input.keys_free {
        shortcuts(ui, &offers, input.drawing, &mut request, actions);
    }

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
    panels: &mut PanelState,
    actions: &mut Vec<Action>,
) {
    ui.strong(format!("Editing {}", feature.name));
    if ui
        .button(FINISH_LABEL)
        .on_hover_text("Leave the sketch. Everything is kept. (Esc with nothing selected)")
        .clicked()
    {
        actions.push(Action::Editing(EditingCommand::Finish));
    }
    let summary = SketchSummary::of(model.evaluation(), feature.id());
    if let Some(focus) = sketch_status::show(ui, &summary) {
        panels.request_focus(focus);
    }
}

fn tools(ui: &mut Ui, active: ActiveSketch, actions: &mut Vec<Action>) {
    for tool in Tool::ALL {
        let keys = tool_shortcut(tool).map_or_else(
            || SELECT_KEY.to_owned(),
            |shortcut| ui.ctx().format_shortcut(&shortcut),
        );
        let response = ui
            .add(Button::selectable(active.tool == tool, tool.button_text()))
            .on_hover_text(format!("{} ({keys})", tool.description()));
        if response.clicked() {
            actions.push(Action::Editing(EditingCommand::SetTool(tool)));
        }
    }
}

fn constraint_buttons(ui: &mut Ui, offers: &[Offer], request: &mut Request) {
    for (tool, offer) in offers {
        let keys = ui.ctx().format_shortcut(&shortcut(*tool));
        let response = ui.add_enabled(offer.is_ok(), Button::new(tool.label()));
        let response = match offer {
            Ok(_) => response.on_hover_text(format!("{} ({keys})", tool.description())),
            Err(reason) => response
                .on_disabled_hover_text(format!("{}. {reason} ({keys})", tool.description())),
        };
        if response.clicked()
            && let Ok(constraints) = offer
        {
            request.constraints = Some((*tool, constraints.clone()));
        }
    }
}

fn delete_button(ui: &mut Ui, enabled: bool, request: &mut Request) {
    let response = ui.add_enabled(enabled, Button::new("Delete"));
    let response = if enabled {
        response.on_hover_text(
            "Delete the selected geometry and constraints, and the constraints on that geometry \
             (Del)",
        )
    } else {
        response.on_disabled_hover_text("Select sketch geometry or constraints to delete them")
    };
    if response.clicked() {
        request.delete = true;
    }
}

fn shortcuts(
    ui: &mut Ui,
    offers: &[Offer],
    drawing: bool,
    request: &mut Request,
    actions: &mut Vec<Action>,
) {
    for (tool, offer) in offers {
        if !ui.input_mut(|input| input.consume_shortcut(&shortcut(*tool))) {
            continue;
        }
        match offer {
            Ok(constraints) => request.constraints = Some((*tool, constraints.clone())),
            Err(reason) => actions.push(Action::Inform(Notice::info(format!(
                "{}: {reason}",
                tool.label()
            )))),
        }
    }
    let delete = !drawing
        && ui.input_mut(|input| {
            DELETE_KEYS
                .into_iter()
                .any(|key| input.consume_key(Modifiers::NONE, key))
        });
    if delete {
        request.delete = true;
    }
    for tool in Tool::ALL {
        let pressed = tool_shortcut(tool)
            .is_some_and(|shortcut| ui.input_mut(|input| input.consume_shortcut(&shortcut)));
        if pressed {
            actions.push(Action::Editing(EditingCommand::SetTool(tool)));
        }
    }
}

fn shortcut(tool: ConstraintTool) -> KeyboardShortcut {
    KeyboardShortcut::new(Modifiers::SHIFT, tool.key())
}

fn tool_shortcut(tool: Tool) -> Option<KeyboardShortcut> {
    tool.key()
        .map(|key| KeyboardShortcut::new(Modifiers::NONE, key))
}
