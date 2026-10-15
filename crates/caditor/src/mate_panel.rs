use caditor_document::{
    AngleSides, AxisReference, Feature, FeatureId, Mate, MatePair, PlaneReference, PointTarget,
    capitalized, describe_axis, describe_origin, describe_point,
};
use caditor_expression::{Dimension, Expression};
use egui::{Id, Ui};

use crate::{
    datum_tools,
    feature_fields::{self, AxisRow, Choice, Picker, Quantity, Rule, Shown},
    mate_tools::{self, MatePart},
    model::{Action, Model},
    plane_selector,
    reference_picking::Slot,
    selection::Selection,
    widgets,
};

pub const DESCRIPTION: &str = "Places the body by its geometry and keeps it there when that \
                               changes: a face flush on another face or plane, an axis on \
                               another axis, both at once, at an angle, a round face resting on \
                               a plane, a flat face resting on a round face, or a point on a \
                               point or plane";
pub const FACE_SAME_WAY: &str = "Face the same way";
pub const AXIS_OTHER_WAY: &str = "Point the other way";
pub const OTHER_SIDE: &str = "Rest on the other side";
pub const DISTANCE: &str = "Distance";
pub const ANGLE: &str = "Angle";
pub const AXIS_TO_MATE: &str = "Axis to mate";
pub const ONTO_AXIS: &str = "Onto axis";
pub const ONTO: &str = "Onto";
const MOVING_HOVER: &str = "Mate the selected face, axis or point of the body instead";
const TARGET_HOVER: &str = "Mate onto the selected face, plane, axis or point instead";

#[derive(Clone, Copy)]
enum End {
    Moving(MatePart),
    Target(MatePart),
}

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

    fn picker(&self, ctx: &egui::Context, end: End) -> Picker<'static> {
        let (model, selection, id, mate) = (self.model, self.selection, self.id(), self.mate);
        let (target, part) = match end {
            End::Moving(part) => (false, part),
            End::Target(part) => (true, part),
        };
        let slot = if target {
            Slot::MateTarget(part)
        } else {
            Slot::MateMoving(part)
        };
        Picker {
            feature: id,
            slot,
            selected: feature_fields::offered_change(ctx, model, selection, (id, slot), || {
                if target {
                    mate_tools::target_change(model, selection, id, mate, part)
                } else {
                    mate_tools::moving_change(model, selection, id, mate, part)
                }
            }),
            hover: if target { TARGET_HOVER } else { MOVING_HOVER },
        }
    }

    fn reference_row(&mut self, ui: &mut Ui, caption: &str, shown: String, end: End) {
        let picker = self.picker(ui.ctx(), end);
        let model = self.model;
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

    fn axis_row(&mut self, ui: &mut Ui, caption: &str, axis: &AxisReference, end: End) {
        let (model, id, mate) = (self.model, self.id(), self.mate);
        let row = AxisRow {
            caption,
            shown: Some(capitalized(&describe_axis(model.document(), axis))),
            picker: self.picker(ui.ctx(), end),
        };
        let mut actions = Vec::new();
        match end {
            End::Moving(_) => feature_fields::axis_row(
                ui,
                model,
                row,
                None::<&dyn Fn() -> Vec<Choice>>,
                &mut actions,
            ),
            End::Target(part) => {
                let listed = || {
                    feature_fields::model_axis_choices(model, id, Some(axis), &|axis| {
                        mate_tools::target_axis_change(model, id, mate, part, axis)
                    })
                };
                feature_fields::axis_row(ui, model, row, Some(&listed), &mut actions);
            }
        }
        self.actions.extend(actions);
    }

    fn plane_row(&mut self, ui: &mut Ui, caption: &str, plane: &PlaneReference) {
        let (model, id, mate) = (self.model, self.id(), self.mate);
        let row = plane_selector::Row {
            caption,
            id: Id::new(("mate-plane", caption, id)),
            planes: datum_tools::listed_planes(model.document(), id),
            current: Some(plane),
            picker: self.picker(ui.ctx(), End::Target(MatePart::Main)),
            remove: None,
        };
        let mut actions = Vec::new();
        plane_selector::row(
            ui,
            model,
            row,
            |plane| mate_tools::target_plane_change(model, id, mate, plane.clone()),
            &mut actions,
        );
        self.actions.extend(actions);
    }

    fn moving(&mut self, ui: &mut Ui, caption: &str, shown: String) {
        self.reference_row(ui, caption, shown, End::Moving(MatePart::Main));
    }

    fn onto(&mut self, ui: &mut Ui, shown: String) {
        self.reference_row(ui, ONTO, shown, End::Target(MatePart::Main));
    }

    fn distance_row(&mut self, ui: &mut Ui, distance: &Expression) {
        let (model, id, mate) = (self.model, self.id(), self.mate);
        let quantity = Quantity {
            feature: id,
            id: Id::new(("mate-field", "distance", id)),
            expression: distance,
            dimension: Dimension::LENGTH,
            rule: Rule::Any,
        };
        let drafting =
            feature_fields::expression_row_drafting(ui, model, DISTANCE, quantity, |value| {
                let mut changed = mate.clone();
                for distance in changed.lengths_mut() {
                    *distance = value.clone();
                }
                mate_tools::change(model, id, changed)
            });
        self.actions.extend(drafting.into_actions(id));
    }

    fn angle_row(&mut self, ui: &mut Ui, angle: &Expression) {
        let (model, id, mate) = (self.model, self.id(), self.mate);
        let quantity = Quantity {
            feature: id,
            id: Id::new(("mate-field", "angle", id)),
            expression: angle,
            dimension: Dimension::ANGLE,
            rule: Rule::MateAngle,
        };
        let drafting =
            feature_fields::expression_row_drafting(ui, model, ANGLE, quantity, |value| {
                let mut changed = mate.clone();
                if let MatePair::Angle(angle) = &mut changed.pair {
                    angle.angle = value;
                }
                mate_tools::change(model, id, changed)
            });
        self.actions.extend(drafting.into_actions(id));
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
                panel.moving(
                    ui,
                    "Face to mate",
                    describe_origin(document, faces.face.origin()),
                );
                panel.plane_row(ui, ONTO, &faces.target);
                panel.distance_row(ui, &faces.distance);
                panel.flip_row(ui, FACE_SAME_WAY);
            }
            MatePair::Axes(axes) => {
                panel.axis_row(ui, AXIS_TO_MATE, &axes.axis, End::Moving(MatePart::Main));
                panel.axis_row(ui, ONTO, &axes.target, End::Target(MatePart::Main));
                panel.flip_row(ui, AXIS_OTHER_WAY);
            }
            MatePair::FaceAxis(both) => {
                let face = describe_origin(document, both.faces.face.origin());
                panel.moving(ui, "Face to mate", face);
                panel.plane_row(ui, ONTO, &both.faces.target);
                panel.axis_row(
                    ui,
                    AXIS_TO_MATE,
                    &both.axes.axis,
                    End::Moving(MatePart::Axis),
                );
                panel.axis_row(
                    ui,
                    ONTO_AXIS,
                    &both.axes.target,
                    End::Target(MatePart::Axis),
                );
                panel.distance_row(ui, &both.faces.distance);
                panel.flip_row(ui, FACE_SAME_WAY);
            }
            MatePair::Angle(angle) => {
                match &angle.sides {
                    AngleSides::Faces(faces) => {
                        let face = describe_origin(document, faces.face.origin());
                        panel.moving(ui, "Face to turn", face);
                        panel.plane_row(ui, ONTO, &faces.target);
                    }
                    AngleSides::Axes(axes) => {
                        let moving = End::Moving(MatePart::Main);
                        panel.axis_row(ui, "Axis to turn", &axes.axis, moving);
                        panel.axis_row(ui, ONTO, &axes.target, End::Target(MatePart::Main));
                    }
                }
                panel.angle_row(ui, &angle.angle);
            }
            MatePair::Tangent(tangent) => {
                let face = describe_origin(document, tangent.face.origin());
                panel.moving(ui, "Round face", face);
                panel.plane_row(ui, "Rests on", &tangent.target);
                panel.flip_row(ui, OTHER_SIDE);
            }
            MatePair::FaceOnRound(resting) => {
                let face = describe_origin(document, resting.face.origin());
                panel.moving(ui, "Flat face", face);
                panel.reference_row(
                    ui,
                    "Rests on",
                    describe_origin(document, resting.round.face.origin()),
                    End::Target(MatePart::Main),
                );
                panel.flip_row(ui, OTHER_SIDE);
            }
            MatePair::Point(point) => {
                panel.moving(ui, "Point to mate", describe_point(document, &point.point));
                match &point.target {
                    PointTarget::Point(target) => panel.onto(ui, describe_point(document, target)),
                    PointTarget::Plane(plane) => panel.plane_row(ui, ONTO, plane),
                }
            }
        }
        feature_fields::feature_row(ui, document, "Body", mate.body);
    });
}
