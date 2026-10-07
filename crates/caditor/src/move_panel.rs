use caditor_document::{Feature, FeatureId, Move, MoveAxis, Transaction};
use caditor_expression::Dimension;
use egui::{Id, Ui};

use crate::{
    feature_fields::{self, Quantity, Rule},
    field,
    model::{Action, Model},
    move_tools, widgets,
};

pub const DESCRIPTION: &str = "Turns the body about the axes through the origin, then shifts it";
pub const COPY_DESCRIPTION: &str = "Makes a new body from a copy of the body, turned about the axes through the origin, then \
     shifted; the original stays where it is";
pub const MAKE_A_COPY: &str = "Make a copy";

fn change(model: &Model, feature: FeatureId, movement: Move) -> Result<Transaction, String> {
    let document = model.document();
    let transaction = move_tools::edit(document, feature, movement)
        .ok_or_else(|| "The feature no longer exists".to_owned())?;
    field::checked(document, transaction)
}

struct Panel<'a> {
    model: &'a Model,
    feature: &'a Feature,
    movement: &'a Move,
    actions: &'a mut Vec<Action>,
}

impl Panel<'_> {
    fn distance_row(&mut self, ui: &mut Ui, axis: MoveAxis) {
        let id = self.feature.id();
        let movement = self.movement;
        let quantity = Quantity {
            id: Id::new(("move-field", "offset", axis.index(), id)),
            expression: axis.of(&movement.offset),
            dimension: Dimension::LENGTH,
            rule: Rule::Any,
        };
        let model = self.model;
        let committed = feature_fields::expression_row(
            ui,
            model,
            &format!("Move along {}", axis.name()),
            quantity,
            |value| {
                let mut changed = movement.clone();
                *axis.of_mut(&mut changed.offset) = value;
                change(model, id, changed)
            },
        );
        self.actions.extend(committed.map(Action::Apply));
    }

    fn turn_row(&mut self, ui: &mut Ui, axis: MoveAxis) {
        let id = self.feature.id();
        let movement = self.movement;
        let quantity = Quantity {
            id: Id::new(("move-field", "turn", axis.index(), id)),
            expression: axis.of(&movement.turn),
            dimension: Dimension::ANGLE,
            rule: Rule::Any,
        };
        let model = self.model;
        let committed = feature_fields::expression_row(
            ui,
            model,
            &format!("Turn about {}", axis.name()),
            quantity,
            |value| {
                let mut changed = movement.clone();
                *axis.of_mut(&mut changed.turn) = value;
                change(model, id, changed)
            },
        );
        self.actions.extend(committed.map(Action::Apply));
    }
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    actions: &mut Vec<Action>,
    feature: &Feature,
    movement: &Move,
) {
    let mut panel = Panel {
        model,
        feature,
        movement,
        actions,
    };
    widgets::properties(ui, ("move-properties", feature.id()), |ui| {
        let description = if movement.copy {
            COPY_DESCRIPTION
        } else {
            DESCRIPTION
        };
        feature_fields::description_row(ui, description);
        for axis in MoveAxis::ALL {
            panel.turn_row(ui, axis);
        }
        for axis in MoveAxis::ALL {
            panel.distance_row(ui, axis);
        }
        feature_fields::feature_row(ui, model.document(), "Body", movement.body);
        if let Some(copy) = feature_fields::reverse_row(ui, MAKE_A_COPY, movement.copy) {
            let change = change(
                model,
                feature.id(),
                Move {
                    copy,
                    ..movement.clone()
                },
            );
            panel
                .actions
                .push(feature_fields::applied(&feature.name, change));
        }
    });
}
