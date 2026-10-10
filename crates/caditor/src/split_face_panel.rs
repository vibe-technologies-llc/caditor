use caditor_document::{
    Document, Feature, Resolution, SolidResult, SplitCarry, SplitFace, capitalized, describe_axis,
};
use caditor_kernel::FaceId;
use egui::{Id, Ui};

use crate::{
    bodies,
    editing::EditingCommand,
    feature_fields::{self, Choice, Picker, Shown},
    feature_tree::count,
    model::{Action, Model},
    reference_picking::{self, Picking, Slot},
    reference_rows::{ReferenceRows, RowCache},
    selection::Selection,
    split_face_tools, split_tools, widgets,
};

pub const DESCRIPTION: &str = "Divides the chosen faces along a plane, a sketch's curves carried \
                               through them or wrapped round a cylinder, or another body, without \
                               changing the shape, so a piece can take its own colour, draft or \
                               fillet";
pub const CARRIED: &str = "Carried";
pub const SQUARE: &str = "Square to the sketch";
pub const ALONG: &str = "Along an edge or axis";
pub const WRAPPED: &str = "Wrapped round the faces";
const DIRECTION_HOVER: &str = "Carry the curves along the selected edge, axis or line instead";
const PICK_HOVER: &str =
    "Split along the selected plane, flat face, sketch curve or other body instead";
const NO_SHAPE_YET: &str = "A face of a body that has no shape yet";
const GONE: &str = "A face that is no longer there";

fn face_row(document: &Document, input: &SolidResult, resolution: &Resolution<FaceId>) -> String {
    match resolution {
        Resolution::One(face) => bodies::describe_face_id(document, input, *face),
        Resolution::Pieces(pieces) => match pieces.first().and_then(|face| input.solid.face(*face))
        {
            Some(first) => format!(
                "{}, split into {} pieces",
                bodies::describe_origin(document, first.origin()),
                pieces.len()
            ),
            None => GONE.to_owned(),
        },
        Resolution::Tied(candidates) => format!(
            "A face that now matches {} separate faces; leave it out and choose it again",
            candidates.len()
        ),
        Resolution::Missing => GONE.to_owned(),
    }
}

fn face_rows(document: &Document, input: Option<&SolidResult>, split: &SplitFace) -> ReferenceRows {
    let summary = count(split.faces.len(), "face", "faces");
    let rows = match input {
        Some(input) => split
            .resolutions(&input.solid)
            .iter()
            .map(|resolution| face_row(document, input, resolution))
            .collect(),
        None => vec![NO_SHAPE_YET.to_owned(); split.faces.len()],
    };
    ReferenceRows { summary, rows }
}

pub struct FacesRow<'a> {
    pub model: &'a Model,
    pub selection: &'a Selection,
    pub feature: &'a Feature,
    pub split: &'a SplitFace,
    pub opened: bool,
}

fn faces_row(ui: &mut Ui, row: &FacesRow<'_>, cache: &mut RowCache, actions: &mut Vec<Action>) {
    let FacesRow {
        model,
        selection,
        feature,
        split,
        opened,
    } = *row;
    let id = feature.id();
    widgets::caption(ui, "Faces to split");
    let evaluation = model.evaluation();
    let mut previewed = Vec::new();
    let listed = cache.rows(id, evaluation.body_before(id), model.revision(), || {
        face_rows(model.document(), bodies::input(evaluation, id), split)
    });
    ui.vertical(|ui| {
        if split.faces.is_empty() {
            ui.label(widgets::muted("None: nothing is split", ui));
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
                            split.resolutions(&input.solid).get(index),
                        )
                    })
                    .unwrap_or_default();
            }
            if row.removed {
                let mut changed = split.clone();
                changed.faces.remove(index);
                actions.push(feature_fields::applied(
                    &feature.name,
                    split_face_tools::change(model, id, changed),
                ));
            }
        }
        if widgets::choose_in_view(
            ui,
            feature_fields::choosing_list(ui, opened),
            "Click faces in the view to split them or leave them out again.",
            "Show the body as it is before this feature so you can click faces",
        ) {
            if let Some(transaction) = split_face_tools::with_selected_faces(model, id, selection) {
                actions.push(Action::Apply(transaction));
            }
            actions.push(Action::Editing(EditingCommand::OpenSolid(id)));
        }
    });
    if !previewed.is_empty() {
        cache.preview(previewed);
    }
    ui.end_row();
}

fn along_rows(ui: &mut Ui, row: &FacesRow<'_>, actions: &mut Vec<Action>) {
    let FacesRow {
        model,
        selection,
        feature,
        split,
        ..
    } = *row;
    let id = feature.id();
    let document = model.document();
    let current = split_tools::describe(document, &split.along);
    widgets::caption(ui, "Split along");
    let chosen = feature_fields::combo(ui, Id::new(("split-face-along", id)), current, || {
        split_face_tools::choices(model, id, split)
            .into_iter()
            .map(|along| Choice {
                label: split_tools::describe(document, &along),
                selected: split.along == along,
                change: split_face_tools::change(model, id, split.with_along(along))
                    .map(Action::Apply),
            })
            .collect()
    });
    ui.end_row();
    actions.extend(chosen);
    ui.label("");
    let picker = Picker {
        feature: id,
        slot: Slot::SplitPlane,
        selected: feature_fields::offered_change(
            ui.ctx(),
            model,
            selection,
            (id, Slot::SplitPlane),
            || split_face_tools::along_change(model, selection, id, split),
        ),
        hover: PICK_HOVER,
    };
    ui.vertical(|ui| {
        feature_fields::reference_picker(ui, model, picker, actions);
    });
    ui.end_row();
}

fn direction_rows(ui: &mut Ui, row: &FacesRow<'_>, actions: &mut Vec<Action>) {
    let FacesRow {
        model,
        selection,
        feature,
        split,
        ..
    } = *row;
    if split.along.sketch().is_none() {
        return;
    }
    let id = feature.id();
    let along = split.direction();
    let square = SplitFace {
        carry: SplitCarry::Square,
        ..split.clone()
    };
    widgets::caption(ui, CARRIED);
    let shown = match split.carry {
        SplitCarry::Square => SQUARE,
        SplitCarry::Along(_) => ALONG,
        SplitCarry::Wrapped => WRAPPED,
    };
    let chosen = feature_fields::combo(ui, Id::new(("split-face-direction", id)), shown, || {
        vec![
            Choice {
                label: SQUARE.to_owned(),
                selected: matches!(split.carry, SplitCarry::Square),
                change: split_face_tools::change(model, id, square.clone()).map(Action::Apply),
            },
            Choice {
                label: ALONG.to_owned(),
                selected: along.is_some(),
                change: Ok(
                    match split_face_tools::direction_change(model, selection, id, split) {
                        Ok(transaction) => Action::Apply(transaction),
                        Err(_) => Action::Editing(EditingCommand::Pick(Picking::new(
                            id,
                            Slot::SplitDirection,
                        ))),
                    },
                ),
            },
            Choice {
                label: WRAPPED.to_owned(),
                selected: split.is_wrapped(),
                change: split_face_tools::wrapped_change(model, id, split).map(Action::Apply),
            },
        ]
    });
    ui.end_row();
    actions.extend(chosen);
    let picking = reference_picking::current(ui.ctx())
        .is_some_and(|picking| picking.is_for(id, Slot::SplitDirection));
    let shown = match along {
        Some(axis) => Shown::Named(capitalized(&describe_axis(model.document(), axis))),
        None if picking => Shown::NoneChosen,
        None => return,
    };
    let picker = Picker {
        feature: id,
        slot: Slot::SplitDirection,
        selected: feature_fields::offered_change(
            ui.ctx(),
            model,
            selection,
            (id, Slot::SplitDirection),
            || split_face_tools::direction_change(model, selection, id, split),
        ),
        hover: DIRECTION_HOVER,
    };
    let removed = feature_fields::reference_row(
        ui,
        model,
        "Along",
        shown,
        picker,
        along.map(|_| "Carry the curves square to the sketch again"),
        actions,
    );
    if removed {
        actions.push(feature_fields::applied(
            &feature.name,
            split_face_tools::change(model, id, square),
        ));
    }
}

pub fn show(ui: &mut Ui, row: &FacesRow<'_>, cache: &mut RowCache, actions: &mut Vec<Action>) {
    let FacesRow {
        model,
        feature,
        split,
        ..
    } = *row;
    widgets::properties(ui, ("split-face-properties", feature.id()), |ui| {
        feature_fields::description_row(ui, DESCRIPTION);
        faces_row(ui, row, cache, actions);
        along_rows(ui, row, actions);
        direction_rows(ui, row, actions);
        feature_fields::feature_row(ui, model.document(), "Body", split.body);
    });
}
