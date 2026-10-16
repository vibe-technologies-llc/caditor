use caditor_document::{DeleteFace, Document, Feature, FeatureId, SolidResult, Transaction};
use egui::Ui;

use crate::{
    bodies, delete_face_tools,
    editing::EditingCommand,
    feature_fields::{self, ListChoosing},
    feature_tree::count,
    field::{self, CheckError},
    model::{Action, Model},
    offset_face_panel::face_row,
    reference_rows::{ReferenceRows, RowCache},
    selection::Selection,
    widgets,
};

pub const DESCRIPTION: &str =
    "Removes the chosen faces and closes the gap by extending the faces around them";
const NO_SHAPE_YET: &str = "A face of a body that has no shape yet";

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
enum DeleteFaceChangeError {
    #[error("The feature no longer exists")]
    Gone { feature: FeatureId },
    #[error(transparent)]
    Checked(#[from] CheckError),
}

fn change(
    model: &Model,
    feature: FeatureId,
    deletion: DeleteFace,
) -> Result<Transaction, DeleteFaceChangeError> {
    let document = model.document();
    let transaction = delete_face_tools::edit(document, feature, deletion)
        .ok_or(DeleteFaceChangeError::Gone { feature })?;
    field::checked(document, transaction)
}

fn face_rows(
    document: &Document,
    input: Option<&SolidResult>,
    deletion: &DeleteFace,
) -> ReferenceRows {
    let summary = count(deletion.faces.len(), "face", "faces");
    let rows = match input {
        Some(input) => deletion
            .resolutions(&input.solid)
            .iter()
            .map(|resolution| face_row(document, input, resolution))
            .collect(),
        None => vec![NO_SHAPE_YET.to_owned(); deletion.faces.len()],
    };
    ReferenceRows { summary, rows }
}

pub struct FacesRow<'a> {
    pub model: &'a Model,
    pub selection: &'a Selection,
    pub feature: &'a Feature,
    pub deletion: &'a DeleteFace,
    pub opened: bool,
}

fn faces_row(ui: &mut Ui, row: &FacesRow<'_>, cache: &mut RowCache, actions: &mut Vec<Action>) {
    let FacesRow {
        model,
        selection,
        feature,
        deletion,
        opened,
    } = *row;
    let id = feature.id();
    widgets::caption(ui, "Faces to delete");
    let evaluation = model.evaluation();
    let mut previewed = Vec::new();
    let listed = cache.rows(id, evaluation.body_before(id), model.revision(), || {
        face_rows(model.document(), bodies::input(evaluation, id), deletion)
    });
    ui.vertical(|ui| {
        if deletion.faces.is_empty() {
            ui.label(widgets::muted("None: nothing is deleted", ui));
        } else {
            ui.label(&listed.summary);
        }
        for (index, text) in listed.rows.iter().enumerate() {
            let text = widgets::muted(text, ui);
            let row = widgets::removable_row_hovered(ui, text, "Keep this face");
            if row.hovered && opened {
                previewed = bodies::input(evaluation, id)
                    .map(|input| {
                        bodies::face_pickables(
                            id,
                            &input.solid,
                            deletion.resolutions(&input.solid).get(index),
                        )
                    })
                    .unwrap_or_default();
            }
            if row.removed {
                let mut changed = deletion.clone();
                changed.faces.remove(index);
                actions.push(feature_fields::applied(
                    &feature.name,
                    change(model, id, changed),
                ));
            }
        }
        let chosen = feature_fields::list_in_view(
            ui,
            opened,
            "Click faces in the view to delete them or keep them again.",
            "Show the body as it is with this feature so you can click faces",
        );
        match chosen {
            Some(ListChoosing::Start) => {
                if let Some(transaction) =
                    delete_face_tools::with_selected_faces(model, id, selection)
                {
                    actions.push(Action::Apply(transaction));
                }
                actions.push(Action::Editing(EditingCommand::OpenSolid(id)));
            }
            Some(ListChoosing::Stop) => actions.push(Action::Editing(EditingCommand::CloseSolid)),
            None => {}
        }
    });
    if !previewed.is_empty() {
        cache.preview(previewed);
    }
    ui.end_row();
}

pub fn show(ui: &mut Ui, row: &FacesRow<'_>, cache: &mut RowCache, actions: &mut Vec<Action>) {
    let FacesRow {
        model,
        feature,
        deletion,
        ..
    } = *row;
    widgets::properties(ui, ("delete-face-properties", feature.id()), |ui| {
        feature_fields::description_row(ui, DESCRIPTION);
        faces_row(ui, row, cache, actions);
        feature_fields::feature_row(ui, model.document(), "Body", deletion.body);
    });
}
