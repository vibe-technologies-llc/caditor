use caditor_document::{Feature, FeatureId, MoveFace, Transaction, capitalized, describe_axis};
use caditor_expression::{Dimension, Expression};
use egui::{Id, Ui};

use crate::{
    bodies,
    editing::EditingCommand,
    feature_fields::{self, AxisRow, Choice, ListChoosing, Picker, Quantity, Rule},
    feature_tree::count,
    model::{Action, Model},
    move_face_tools::{self, MoveFaceError, Reference},
    offset_face_panel,
    reference_picking::Slot,
    reference_rows::{ReferenceRows, RowCache},
    selection::Selection,
    widgets,
};

pub const DESCRIPTION: &str = "Shifts the chosen faces along a direction and tilts them about an \
                               axis, the faces beside them following";
pub const SQUARE_TO_FACE: &str = "Square to a flat face";
pub const NO_TILT: &str = "None: no tilt";
pub const DISTANCE: &str = "Distance";
pub const ANGLE: &str = "Angle";
const DIRECTION: &str = "Direction";
const TILT_AXIS: &str = "Tilt about";
const DIRECTION_HOVER: &str =
    "Shift the faces along the selected axis, straight edge or round face";
const AXIS_HOVER: &str = "Tilt the faces about the selected axis or straight edge";
const NO_SHAPE_YET: &str = "A face of a body that has no shape yet";

pub struct FacesRow<'a> {
    pub model: &'a Model,
    pub selection: &'a Selection,
    pub feature: &'a Feature,
    pub moving: &'a MoveFace,
    pub opened: bool,
}

struct Panel<'a, 'b> {
    row: &'a FacesRow<'b>,
    actions: &'a mut Vec<Action>,
}

impl Panel<'_, '_> {
    fn id(&self) -> FeatureId {
        self.row.feature.id()
    }

    fn change(&self, moving: MoveFace) -> Result<Transaction, MoveFaceError> {
        move_face_tools::change(self.row.model, self.id(), moving)
    }

    fn faces_row(&mut self, ui: &mut Ui, cache: &mut RowCache) {
        let FacesRow {
            model,
            selection,
            feature,
            moving,
            opened,
        } = *self.row;
        let id = feature.id();
        widgets::caption(ui, "Faces to move");
        let evaluation = model.evaluation();
        let mut previewed = Vec::new();
        let listed = cache.rows(id, evaluation.body_before(id), model.revision(), || {
            let summary = count(moving.faces.len(), "face", "faces");
            let rows = match bodies::input(evaluation, id) {
                Some(input) => moving
                    .resolutions(&input.solid)
                    .iter()
                    .map(|resolution| {
                        offset_face_panel::face_row(model.document(), input, resolution)
                    })
                    .collect(),
                None => vec![NO_SHAPE_YET.to_owned(); moving.faces.len()],
            };
            ReferenceRows { summary, rows }
        });
        ui.vertical(|ui| {
            if moving.faces.is_empty() {
                ui.label(widgets::muted("None: nothing moves", ui));
            } else {
                ui.label(&listed.summary);
            }
            for (index, text) in listed.rows.iter().enumerate() {
                let text = widgets::muted(text, ui);
                let row = widgets::removable_row_hovered(ui, text, "Leave this face out");
                if row.hovered && opened {
                    previewed = bodies::input(evaluation, id)
                        .map(|input| {
                            bodies::face_pickables(
                                id,
                                &input.solid,
                                moving.resolutions(&input.solid).get(index),
                            )
                        })
                        .unwrap_or_default();
                }
                if row.removed {
                    let mut changed = moving.clone();
                    changed.faces.remove(index);
                    self.actions
                        .push(feature_fields::applied(&feature.name, self.change(changed)));
                }
            }
            let chosen = feature_fields::list_in_view(
                ui,
                opened,
                "Click faces in the view to move them or leave them out again.",
                "Show the body as it is with this feature so you can click faces",
            );
            match chosen {
                Some(ListChoosing::Start) => {
                    if let Some(transaction) =
                        move_face_tools::with_selected_faces(model, id, selection)
                    {
                        self.actions.push(Action::Apply(transaction));
                    }
                    self.actions
                        .push(Action::Editing(EditingCommand::OpenSolid(id)));
                }
                Some(ListChoosing::Stop) => self
                    .actions
                    .push(Action::Editing(EditingCommand::CloseSolid)),
                None => {}
            }
        });
        if !previewed.is_empty() {
            cache.preview(previewed);
        }
        ui.end_row();
    }

    fn value_row(&mut self, ui: &mut Ui, reference: Reference) {
        let (id, moving) = (self.id(), self.row.moving);
        let (caption, expression, dimension, rule) = match reference {
            Reference::Direction => (DISTANCE, &moving.distance, Dimension::LENGTH, Rule::Any),
            Reference::Axis => (ANGLE, &moving.angle, Dimension::ANGLE, Rule::Any),
        };
        let quantity = Quantity {
            feature: id,
            id: Id::unique(("move-face-value", caption, id)),
            expression,
            dimension,
            rule,
        };
        let drafting = feature_fields::expression_row_drafting(
            ui,
            self.row.model,
            caption,
            quantity,
            |value: Expression| {
                let mut changed = moving.clone();
                match reference {
                    Reference::Direction => changed.distance = value,
                    Reference::Axis => changed.angle = value,
                }
                self.change(changed)
            },
        );
        self.actions.extend(drafting.into_actions(id));
    }

    fn reference_row(&mut self, ui: &mut Ui, reference: Reference) {
        let FacesRow {
            model,
            selection,
            moving,
            ..
        } = *self.row;
        let id = self.id();
        let (caption, current, slot, hover, unchosen) = match reference {
            Reference::Direction => (
                DIRECTION,
                moving.direction.as_ref(),
                Slot::MoveFaceDirection,
                DIRECTION_HOVER,
                SQUARE_TO_FACE,
            ),
            Reference::Axis => (
                TILT_AXIS,
                moving.axis.as_ref(),
                Slot::MoveFaceAxis,
                AXIS_HOVER,
                NO_TILT,
            ),
        };
        let shown = Some(current.map_or_else(
            || unchosen.to_owned(),
            |axis| capitalized(&describe_axis(model.document(), axis)),
        ));
        let row = AxisRow {
            caption,
            shown,
            picker: Picker {
                feature: id,
                slot,
                selected: feature_fields::offered_change(
                    ui.ctx(),
                    model,
                    selection,
                    (id, slot),
                    || move_face_tools::reference_change(model, selection, id, moving, reference),
                ),
                hover,
            },
        };
        let listed = || {
            let none = Choice {
                label: unchosen.to_owned(),
                selected: current.is_none(),
                change: self.change(reference.with(moving, None)).map(Action::Apply),
            };
            let axes = feature_fields::model_axis_choices(model, id, current, &|axis| {
                self.change(reference.with(moving, Some(axis)))
            });
            std::iter::once(none).chain(axes).collect::<Vec<_>>()
        };
        let mut actions = Vec::new();
        feature_fields::axis_row(ui, model, row, Some(&listed), &mut actions);
        self.actions.extend(actions);
    }
}

pub fn show(ui: &mut Ui, row: &FacesRow<'_>, cache: &mut RowCache, actions: &mut Vec<Action>) {
    let mut panel = Panel { row, actions };
    let id = panel.id();
    widgets::properties(ui, ("move-face-properties", id), |ui| {
        feature_fields::description_row(ui, DESCRIPTION);
        panel.faces_row(ui, cache);
        panel.value_row(ui, Reference::Direction);
        panel.reference_row(ui, Reference::Direction);
        panel.value_row(ui, Reference::Axis);
        panel.reference_row(ui, Reference::Axis);
        feature_fields::feature_row(ui, row.model.document(), "Body", row.moving.body);
    });
}
