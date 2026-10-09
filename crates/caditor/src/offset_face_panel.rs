use caditor_document::{
    Document, Feature, FeatureId, OffsetFace, Resolution, SolidResult, Transaction,
};
use caditor_expression::Dimension;
use caditor_kernel::FaceId;
use egui::{Id, Ui};

use crate::{
    bodies,
    editing::EditingCommand,
    feature_fields::{self, Quantity, Rule},
    feature_tree::count,
    field,
    model::{Action, Model},
    offset_face_tools,
    reference_rows::{ReferenceRows, RowCache},
    widgets,
};

pub const DESCRIPTION: &str =
    "Moves the chosen faces along their normals, the faces beside them following";
const TANGENT: &str = "Move the faces tangent to these too";

fn change(model: &Model, feature: FeatureId, offset: OffsetFace) -> Result<Transaction, String> {
    let document = model.document();
    let transaction = offset_face_tools::edit(document, feature, offset)
        .ok_or_else(|| "The feature no longer exists".to_owned())?;
    field::checked(document, transaction)
}

fn distance_row(
    ui: &mut Ui,
    model: &Model,
    feature: FeatureId,
    offset: &OffsetFace,
    actions: &mut Vec<Action>,
) {
    let quantity = Quantity {
        feature,
        id: Id::new(("offset-face-distance", feature)),
        expression: &offset.distance,
        dimension: Dimension::LENGTH,
        rule: Rule::Any,
    };
    let drafting =
        feature_fields::expression_row_drafting(ui, model, "Distance", quantity, |distance| {
            change(
                model,
                feature,
                OffsetFace {
                    distance,
                    ..offset.clone()
                },
            )
        });
    actions.extend(drafting.into_actions(feature));
    feature_fields::draft_failure_row(ui, model, feature);
}

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

fn face_rows(
    document: &Document,
    input: Option<&SolidResult>,
    offset: &OffsetFace,
) -> ReferenceRows {
    let summary = count(offset.faces.len(), "face", "faces");
    let rows = match input {
        Some(input) => offset
            .resolutions(&input.solid)
            .iter()
            .map(|resolution| face_row(document, input, resolution))
            .collect(),
        None => vec![NO_SHAPE_YET.to_owned(); offset.faces.len()],
    };
    ReferenceRows { summary, rows }
}

struct FacesRow<'a> {
    model: &'a Model,
    feature: &'a Feature,
    offset: &'a OffsetFace,
    opened: bool,
}

fn faces_row(ui: &mut Ui, row: &FacesRow<'_>, cache: &mut RowCache, actions: &mut Vec<Action>) {
    let FacesRow {
        model,
        feature,
        offset,
        opened,
    } = *row;
    let id = feature.id();
    widgets::caption(ui, "Faces to move");
    let evaluation = model.evaluation();
    let mut previewed = Vec::new();
    let listed = cache.rows(id, evaluation.body_before(id), model.revision(), || {
        face_rows(model.document(), bodies::input(evaluation, id), offset)
    });
    ui.vertical(|ui| {
        if offset.faces.is_empty() {
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
                            offset.resolutions(&input.solid).get(index),
                        )
                    })
                    .unwrap_or_default();
            }
            if row.removed {
                let mut changed = offset.clone();
                changed.faces.remove(index);
                actions.push(feature_fields::applied(
                    &feature.name,
                    change(model, id, changed),
                ));
            }
        }
        if widgets::choose_in_view(
            ui,
            feature_fields::choosing_list(ui, opened),
            "Click faces in the view to move them or leave them out again.",
            "Show the body as it is with this feature so you can click faces",
        ) {
            actions.push(Action::Editing(EditingCommand::OpenSolid(id)));
        }
    });
    if !previewed.is_empty() {
        cache.preview(previewed);
    }
    ui.end_row();
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    cache: &mut RowCache,
    actions: &mut Vec<Action>,
    feature: &Feature,
    offset: &OffsetFace,
    opened: bool,
) {
    let id = feature.id();
    let row = FacesRow {
        model,
        feature,
        offset,
        opened,
    };
    widgets::properties(ui, ("offset-face-properties", id), |ui| {
        feature_fields::description_row(ui, DESCRIPTION);
        faces_row(ui, &row, cache, actions);
        distance_row(ui, model, id, offset, actions);
        if let Some(tangent) = feature_fields::reverse_row(ui, TANGENT, offset.tangent) {
            actions.push(feature_fields::applied(
                &feature.name,
                change(
                    model,
                    id,
                    OffsetFace {
                        tangent,
                        ..offset.clone()
                    },
                ),
            ));
        }
        feature_fields::feature_row(ui, model.document(), "Body", offset.body);
    });
}
