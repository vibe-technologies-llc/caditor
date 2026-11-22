use caditor_document::{Document, FeatureId};
use caditor_sketch::{ConstraintId, Sketch};
use egui::{Popup, Response, Ui};

use crate::{
    commands::Command,
    editing::EditingCommand,
    icons,
    model::{Action, Model},
    selection::Pickable,
    sketch_tools::{self, ActivityChange},
    widgets,
};

pub const RESOLVE: &str = "Resolve";
pub const RESOLVE_HOVER: &str = "Select, disable or delete the constraints named here, one at a time; Undo takes a change back.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstraintRemedies {
    feature: FeatureId,
    constraints: Vec<ConstraintId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Remedy {
    Select,
    Disable,
    Delete,
}

impl Remedy {
    fn glyph(self) -> &'static str {
        match self {
            Self::Select => icons::SELECT,
            Self::Disable => icons::command(Command::ToggleConstraintActive),
            Self::Delete => icons::DELETE,
        }
    }

    pub fn label(self, name: &str) -> String {
        let verb = match self {
            Self::Select => "Select",
            Self::Disable => "Disable",
            Self::Delete => "Delete",
        };
        format!("{verb} {name}")
    }
}

impl ConstraintRemedies {
    pub fn of(feature: FeatureId, constraints: Vec<ConstraintId>) -> Option<Self> {
        (!constraints.is_empty()).then_some(Self {
            feature,
            constraints,
        })
    }

    pub fn feature(&self) -> FeatureId {
        self.feature
    }

    pub fn is_offered(&self, document: &Document) -> bool {
        sketch_of(document, self.feature)
            .is_some_and(|sketch| self.present(sketch).next().is_some())
    }

    pub fn present<'a>(&'a self, sketch: &'a Sketch) -> impl Iterator<Item = ConstraintId> + 'a {
        self.constraints
            .iter()
            .copied()
            .filter(|constraint| sketch.constraint(*constraint).is_some())
    }
}

fn offered(sketch: &Sketch, constraint: ConstraintId) -> impl Iterator<Item = Remedy> {
    let active = sketch.is_active(constraint);

    [Remedy::Select, Remedy::Disable, Remedy::Delete]
        .into_iter()
        .filter(move |remedy| *remedy != Remedy::Disable || active)
}

fn taken(
    model: &Model,
    (feature, constraint): (FeatureId, ConstraintId),
    remedy: Remedy,
    edited: bool,
) -> Vec<Action> {
    let Some(sketch) = sketch_of(model.document(), feature) else {
        return Vec::new();
    };
    match remedy {
        Remedy::Select => {
            let select = Action::Select(vec![Pickable::SketchConstraint {
                feature,
                constraint,
            }]);
            if edited {
                vec![select]
            } else {
                vec![Action::Editing(EditingCommand::Enter(feature)), select]
            }
        }
        Remedy::Disable => {
            let change = ActivityChange {
                constraints: vec![constraint],
                active: false,
            };
            vec![Action::Apply(change.transaction(model, feature, sketch))]
        }
        Remedy::Delete => vec![Action::Apply(sketch_tools::remove_items(
            model,
            feature,
            Remedy::Delete.label(&sketch.describe_constraint(constraint)),
            Vec::new(),
            vec![constraint],
        ))],
    }
}

pub fn button(
    ui: &mut Ui,
    model: &Model,
    remedies: &ConstraintRemedies,
    edited: bool,
    actions: &mut Vec<Action>,
) -> Response {
    let button = widgets::small_button(ui, icons::FIX, RESOLVE);
    let response = ui.add(button).on_hover_text(RESOLVE_HOVER);

    Popup::menu(&response).show(|ui| {
        widgets::fitted_menu(ui, |ui| menu(ui, model, remedies, edited, actions));
    });
    response
}

fn menu(
    ui: &mut Ui,
    model: &Model,
    remedies: &ConstraintRemedies,
    edited: bool,
    actions: &mut Vec<Action>,
) {
    let feature = remedies.feature;
    let Some(sketch) = sketch_of(model.document(), feature) else {
        return;
    };

    for (index, constraint) in remedies.present(sketch).enumerate() {
        if index > 0 {
            ui.separator();
        }

        let name = sketch.describe_constraint(constraint);

        for remedy in offered(sketch, constraint) {
            if widgets::menu_item(ui, remedy.glyph(), &remedy.label(&name), None).clicked() {
                actions.extend(taken(model, (feature, constraint), remedy, edited));
                ui.close();
            }
        }
    }
}

fn sketch_of(document: &Document, feature: FeatureId) -> Option<&Sketch> {
    document.feature(feature)?.kind.sketch()
}
