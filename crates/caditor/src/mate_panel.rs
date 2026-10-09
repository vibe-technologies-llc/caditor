use caditor_document::{
    Feature, FeatureId, Mate, MatePair, capitalized, describe_axis, describe_origin, describe_plane,
};
use caditor_expression::{Dimension, Expression};
use egui::{Id, Ui};

use crate::{
    feature_fields::{self, Picker, Quantity, Rule, Shown},
    mate_tools,
    model::{Action, Model},
    reference_picking::Slot,
    selection::Selection,
    widgets,
};

pub const DESCRIPTION: &str = "Places the body so its face lies on the other face or plane, flush \
                               or at a distance, or its axis on the other axis, and keeps it \
                               there when they change";
pub const FACE_SAME_WAY: &str = "Face the same way";
pub const AXIS_OTHER_WAY: &str = "Point the other way";
pub const DISTANCE: &str = "Distance";
const MOVING_HOVER: &str = "Mate the selected face or axis of the body instead";
const TARGET_HOVER: &str = "Mate onto the selected face, plane or axis instead";

struct Panel<'a> {
    model: &'a Model,
    selection: &'a Selection,
    feature: &'a Feature,
    mate: &'a Mate,
    actions: &'a mut Vec<Action>,
}

impl Panel<'_> {
    fn id(&self) -> FeatureId {
        self.feature.id()
    }

    fn reference_row(&mut self, ui: &mut Ui, caption: &str, shown: String, slot: Slot) {
        let (model, selection, id, mate) = (self.model, self.selection, self.id(), self.mate);
        let target = slot == Slot::MateTarget;
        let picker = Picker {
            feature: id,
            slot,
            selected: feature_fields::offered_change(
                ui.ctx(),
                model,
                selection,
                (id, slot),
                || {
                    if target {
                        mate_tools::target_change(model, selection, id, mate)
                    } else {
                        mate_tools::moving_change(model, selection, id, mate)
                    }
                },
            ),
            hover: if target { TARGET_HOVER } else { MOVING_HOVER },
        };
        feature_fields::reference_row(
            ui,
            model,
            caption,
            Shown::Named(capitalized(&shown)),
            picker,
            None,
            self.actions,
        );
    }

    fn distance_row(&mut self, ui: &mut Ui, distance: &Expression) {
        let (model, id, mate) = (self.model, self.id(), self.mate);
        let quantity = Quantity {
            id: Id::new(("mate-field", "distance", id)),
            expression: distance,
            dimension: Dimension::LENGTH,
            rule: Rule::Any,
        };
        let committed = feature_fields::expression_row(ui, model, DISTANCE, quantity, |value| {
            let mut changed = mate.clone();
            if let MatePair::Faces(faces) = &mut changed.pair {
                faces.distance = value;
            }
            mate_tools::change(model, id, changed)
        });
        self.actions.extend(committed.map(Action::Apply));
    }

    fn flip_row(&mut self, ui: &mut Ui, label: &str) {
        if let Some(flipped) = feature_fields::reverse_row(ui, label, self.mate.flipped) {
            let change = mate_tools::change(
                self.model,
                self.id(),
                Mate {
                    flipped,
                    ..self.mate.clone()
                },
            );
            self.actions
                .push(feature_fields::applied(&self.feature.name, change));
        }
    }
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    selection: &Selection,
    actions: &mut Vec<Action>,
    feature: &Feature,
    mate: &Mate,
) {
    let document = model.document();
    let mut panel = Panel {
        model,
        selection,
        feature,
        mate,
        actions,
    };
    widgets::properties(ui, ("mate-properties", feature.id()), |ui| {
        feature_fields::description_row(ui, DESCRIPTION);
        match &mate.pair {
            MatePair::Faces(faces) => {
                let moving = describe_origin(document, faces.face.origin());
                panel.reference_row(ui, "Face to mate", moving, Slot::MateMoving);
                let target = describe_plane(document, &faces.target);
                panel.reference_row(ui, "Onto", target, Slot::MateTarget);
                panel.distance_row(ui, &faces.distance);
                panel.flip_row(ui, FACE_SAME_WAY);
            }
            MatePair::Axes(axes) => {
                let moving = describe_axis(document, &axes.axis);
                panel.reference_row(ui, "Axis to mate", moving, Slot::MateMoving);
                let target = describe_axis(document, &axes.target);
                panel.reference_row(ui, "Onto", target, Slot::MateTarget);
                panel.flip_row(ui, AXIS_OTHER_WAY);
            }
        }
        feature_fields::feature_row(ui, document, "Body", mate.body);
    });
}
