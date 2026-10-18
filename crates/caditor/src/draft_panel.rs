use caditor_document::{Draft, Feature, FeatureId, Transaction, displayed_plane};
use caditor_expression::{Dimension, Expression, Unit};
use caditor_geometry::Vector3;
use egui::{Id, Ui};

use crate::{
    bodies, datum_tools,
    draft_tools::{self, DEFAULT_ANGLE, DraftToolError},
    editing::EditingCommand,
    feature_fields::{self, ListChoosing, Picker, Quantity, Rule},
    feature_tree::count,
    icons,
    model::{Action, Model},
    offset_face_panel, plane_selector,
    reference_picking::{Picking, Slot},
    reference_rows::{ReferenceRows, RowCache},
    selection::Selection,
    widgets,
};

pub const DESCRIPTION: &str = "Tilts the chosen flat faces about the line where each meets the \
                               neutral plane, so each makes the draft angle with the pull \
                               direction, the faces beside them following";
pub const FLIP: &str = "Flip the pull direction";
pub const TANGENT: &str = "Draft the faces tangent to these too";
pub const SPLIT: &str = "Split the faces at the neutral plane";
pub const OWN_ANGLE: &str = "Own angle";
pub const ADD_OWN_ANGLE: &str = "Choose faces";
const ADD_OWN_ANGLE_HOVER: &str =
    "Click flat faces in the view to give each its own draft angle, or the draft's again";
const NO_OWN_ANGLE: &str = "None: every face takes the draft angle";
const DRAFTS_ANGLE: &str = "Give this face the draft angle";
const GONE: &str = "A face that is no longer there";
const ANGLE: &str = "Draft angle";
const BEYOND: &str = "Angle beyond";
const PICK_HOVER: &str = "Draft from the selected plane or flat face instead";
const NO_SHAPE_YET: &str = "A face of a body that has no shape yet";
const ALIGNED: f64 = 1.0 - 1e-9;

pub struct FacesRow<'a> {
    pub model: &'a Model,
    pub selection: &'a Selection,
    pub feature: &'a Feature,
    pub draft: &'a Draft,
    pub opened: bool,
}

struct Panel<'a, 'b> {
    row: &'a FacesRow<'b>,
    actions: &'a mut Vec<Action>,
    described: Vec<String>,
}

fn axis_words(direction: Vector3) -> Option<&'static str> {
    [
        (Vector3::X, "+X"),
        (Vector3::NEG_X, "-X"),
        (Vector3::Y, "+Y"),
        (Vector3::NEG_Y, "-Y"),
        (Vector3::Z, "+Z"),
        (Vector3::NEG_Z, "-Z"),
    ]
    .into_iter()
    .find(|(axis, _)| axis.dot(direction) >= ALIGNED)
    .map(|(_, words)| words)
}

impl Panel<'_, '_> {
    fn id(&self) -> FeatureId {
        self.row.feature.id()
    }

    fn change(&self, draft: Draft) -> Result<Transaction, DraftToolError> {
        draft_tools::change(self.row.model, self.id(), draft)
    }

    fn apply(&mut self, change: Result<Transaction, DraftToolError>) {
        self.actions
            .push(feature_fields::applied(&self.row.feature.name, change));
    }

    fn faces_row(&mut self, ui: &mut Ui, cache: &mut RowCache) {
        let FacesRow {
            model,
            selection,
            feature,
            draft,
            opened,
        } = *self.row;
        let id = feature.id();
        widgets::caption(ui, "Faces to draft");
        let evaluation = model.evaluation();
        let mut previewed = Vec::new();
        let listed = cache.rows(id, evaluation.body_before(id), model.revision(), || {
            let summary = count(draft.faces.len(), "face", "faces");
            let rows = match bodies::input(evaluation, id) {
                Some(input) => draft
                    .resolutions(&input.solid)
                    .iter()
                    .chain(&draft.own_resolutions(&input.solid))
                    .map(|resolution| {
                        offset_face_panel::face_row(model.document(), input, resolution)
                    })
                    .collect(),
                None => vec![NO_SHAPE_YET.to_owned(); draft.faces.len() + draft.own.len()],
            };
            ReferenceRows { summary, rows }
        });
        self.described = listed
            .rows
            .iter()
            .skip(draft.faces.len())
            .cloned()
            .collect();
        ui.vertical(|ui| {
            if draft.faces.is_empty() {
                ui.label(widgets::muted("None: nothing is drafted", ui));
            } else {
                ui.label(&listed.summary);
            }
            for (index, text) in listed.rows.iter().take(draft.faces.len()).enumerate() {
                let text = widgets::muted(text, ui);
                let row = widgets::removable_row_hovered(ui, text, "Leave this face out");
                if row.hovered && opened {
                    previewed = bodies::input(evaluation, id)
                        .map(|input| {
                            bodies::face_pickables(
                                id,
                                &input.solid,
                                draft.resolutions(&input.solid).get(index),
                            )
                        })
                        .unwrap_or_default();
                }
                if row.removed {
                    let mut changed = draft.clone();
                    changed.faces.remove(index);
                    let change = self.change(changed);
                    self.apply(change);
                }
            }
            let chosen = feature_fields::list_in_view(
                ui,
                opened,
                "Click faces in the view to draft them or leave them out again.",
                "Show the body as it is with this feature so you can click faces",
            );
            match chosen {
                Some(ListChoosing::Start) => {
                    if let Some(transaction) =
                        draft_tools::with_selected_faces(model, id, selection)
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

    fn angle_row(&mut self, ui: &mut Ui, beyond: bool) {
        let (id, draft) = (self.id(), self.row.draft);
        let (caption, expression) = match (beyond, &draft.beyond) {
            (true, Some(expression)) => (BEYOND, expression),
            (true, None) => return,
            (false, _) => (ANGLE, &draft.angle),
        };
        let quantity = Quantity {
            feature: id,
            id: Id::unique(("draft-angle", caption, id)),
            expression,
            dimension: Dimension::ANGLE,
            rule: Rule::Any,
        };
        let drafting = feature_fields::expression_row_drafting(
            ui,
            self.row.model,
            caption,
            quantity,
            |value: Expression| {
                let mut changed = draft.clone();
                if beyond {
                    changed.beyond = Some(value);
                } else {
                    changed.angle = value;
                }
                self.change(changed)
            },
        );
        self.actions.extend(drafting.into_actions(id));
    }

    fn own_angle_list(&mut self, ui: &mut Ui) {
        let FacesRow { model, draft, .. } = *self.row;
        let id = self.id();
        widgets::caption(ui, OWN_ANGLE);
        ui.vertical(|ui| {
            if draft.own.is_empty() {
                ui.label(widgets::muted(NO_OWN_ANGLE, ui));
            }
            let slot = (id, Slot::DraftOwnAngle);
            let stop = feature_fields::STOP_CHOOSING;
            if !feature_fields::choosing_prompt(ui, model, slot, stop, self.actions) {
                let add = widgets::small_button(ui, icons::ADD, ADD_OWN_ANGLE);
                if ui.add(add).on_hover_text(ADD_OWN_ANGLE_HOVER).clicked() {
                    self.actions
                        .push(Action::Editing(EditingCommand::Pick(Picking::new(
                            id,
                            Slot::DraftOwnAngle,
                        ))));
                }
            }
        });
        ui.end_row();
    }

    fn own_angle_rows(&mut self, ui: &mut Ui, cache: &mut RowCache) {
        let FacesRow {
            model,
            draft,
            opened,
            ..
        } = *self.row;
        let id = self.id();
        let evaluation = model.evaluation();
        for (index, own) in draft.own.iter().enumerate() {
            let caption = self.described.get(index).map_or(GONE, String::as_str);
            let owned_as = own_angle_caption(caption);
            let quantity = Quantity {
                feature: id,
                id: Id::unique(("draft-own-angle", id, index)),
                expression: &own.angle,
                dimension: Dimension::ANGLE,
                rule: Rule::Any,
            };
            let value = feature_fields::expression_row_removable(
                ui,
                model,
                (caption, &owned_as),
                quantity,
                |angle| {
                    let mut changed = draft.clone();
                    if let Some(entry) = changed.own.get_mut(index) {
                        entry.angle = angle;
                    }
                    self.change(changed)
                },
                DRAFTS_ANGLE,
            );
            self.actions.extend(value.drafting.into_actions(id));
            if value.hovered && opened {
                let previewed = bodies::input(evaluation, id)
                    .map(|input| {
                        bodies::face_pickables(
                            id,
                            &input.solid,
                            draft.own_resolutions(&input.solid).get(index),
                        )
                    })
                    .unwrap_or_default();
                cache.preview(previewed);
            }
            if value.removed {
                let mut changed = draft.clone();
                if index < changed.own.len() {
                    let entry = changed.own.remove(index);
                    changed.faces.push(entry.face);
                }
                let change = self.change(changed);
                self.apply(change);
            }
        }
    }

    fn neutral_rows(&mut self, ui: &mut Ui) {
        let FacesRow {
            model,
            selection,
            draft,
            ..
        } = *self.row;
        let document = model.document();
        let id = self.id();
        widgets::caption(ui, "Neutral plane");
        let chosen = plane_selector::combo(
            ui,
            Id::unique(("draft-neutral", id)),
            document,
            datum_tools::listed_planes(document, id),
            Some(&draft.neutral),
            |reference| draft_tools::neutral_change(model, id, draft, reference.clone()),
        );
        ui.end_row();
        self.actions.extend(chosen.map(Action::Apply));
        ui.label("");
        let picker = Picker {
            feature: id,
            slot: Slot::DraftNeutral,
            selected: feature_fields::offered_change(
                ui.ctx(),
                model,
                selection,
                (id, Slot::DraftNeutral),
                || draft_tools::plane_change(model, selection, id, draft),
            ),
            hover: PICK_HOVER,
        };
        ui.vertical(|ui| {
            feature_fields::reference_picker(ui, model, picker, self.actions);
        });
        ui.end_row();
    }

    fn pull_rows(&mut self, ui: &mut Ui) {
        let FacesRow { model, draft, .. } = *self.row;
        widgets::caption(ui, "Pull direction");
        let pull = displayed_plane(model.evaluation(), self.id(), &draft.neutral).map(|plane| {
            if draft.flipped {
                -plane.normal()
            } else {
                plane.normal()
            }
        });
        let words = match pull {
            Some(pull) => match axis_words(pull) {
                Some(axis) => format!("Toward {axis}: the faces narrow the body that way"),
                None => {
                    "Square to the neutral plane: the faces narrow the body that way".to_owned()
                }
            },
            None => "Not known until the neutral plane is placed".to_owned(),
        };
        ui.label(widgets::muted(&words, ui));
        ui.end_row();
        if let Some(flipped) = feature_fields::reverse_row(ui, FLIP, draft.flipped) {
            let change = self.change(Draft {
                flipped,
                ..draft.clone()
            });
            self.apply(change);
        }
    }

    fn option_rows(&mut self, ui: &mut Ui) {
        let draft = self.row.draft;
        if let Some(tangent) = feature_fields::reverse_row(ui, TANGENT, draft.tangent) {
            let change = self.change(Draft {
                tangent,
                ..draft.clone()
            });
            self.apply(change);
        }
        if let Some(split) = feature_fields::reverse_row(ui, SPLIT, draft.beyond.is_some()) {
            let beyond = split.then_some(Expression::Measure(DEFAULT_ANGLE, Unit::Degree));
            let change = self.change(Draft {
                beyond,
                ..draft.clone()
            });
            self.apply(change);
        }
    }
}

pub fn own_angle_caption(face: &str) -> String {
    format!("Angle of {face}")
}

pub fn show(ui: &mut Ui, row: &FacesRow<'_>, cache: &mut RowCache, actions: &mut Vec<Action>) {
    let mut panel = Panel {
        row,
        actions,
        described: Vec::new(),
    };
    let id = panel.id();
    widgets::properties(ui, ("draft-properties", id), |ui| {
        feature_fields::description_row(ui, DESCRIPTION);
        panel.faces_row(ui, cache);
        panel.angle_row(ui, false);
        panel.own_angle_list(ui);
        panel.own_angle_rows(ui, cache);
        panel.neutral_rows(ui);
        panel.pull_rows(ui);
        panel.option_rows(ui);
        panel.angle_row(ui, true);
        feature_fields::feature_row(ui, row.model.document(), "Body", row.draft.body);
    });
}
