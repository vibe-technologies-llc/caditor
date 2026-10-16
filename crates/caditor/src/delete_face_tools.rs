use std::collections::BTreeSet;

use caditor_document::{
    DeleteFace, Document, Edit, Evaluation, FeatureId, FeatureKind, Transaction,
};
use caditor_kernel::{FaceId, FaceReference, Solid};

use crate::{
    bodies::{self, FaceKey},
    body_selection,
    commands::Command,
    editing::{self, EditingCommand},
    model::{Action, Model, Notice},
    offset_face_tools::{self, FacePurpose, FaceToolError},
    queued,
    selection::{Pickable, Selection},
    shell_tools::FaceSource,
};

pub const TITLE: &str = "Delete face";

pub fn selected_faces(model: &Model, selection: &Selection) -> Result<FaceSource, FaceToolError> {
    offset_face_tools::faces_of_one_body(model, selection, FacePurpose::Delete)
}

pub fn create(
    document: &Document,
    evaluation: &Evaluation,
    source: &FaceSource,
) -> Result<(Transaction, FeatureId), FaceToolError> {
    let shown = bodies::shown(evaluation, source.body).ok_or(FaceToolError::NoShape)?;
    let faces: Vec<FaceReference> = source
        .faces
        .iter()
        .filter_map(|key| FaceReference::capture(&shown.solid, bodies::find_face(shown, *key)?))
        .collect();
    if faces.len() < source.faces.len() {
        return Err(FaceToolError::FacesGone);
    }
    let name = editing::next_feature_name(document, TITLE);
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(
        name,
        FeatureKind::DeleteFace(DeleteFace {
            body: source.body,
            faces,
        }),
    );
    Ok((transaction.finish(), feature))
}

pub fn create_actions(
    document: &Document,
    evaluation: &Evaluation,
    source: &FaceSource,
) -> Vec<Action> {
    match create(document, evaluation, source) {
        Ok((transaction, feature)) => {
            let told = body_selection::left_out_words(&source.left_out).map(|words| {
                Action::Inform(Notice::warning(format!(
                    "{TITLE} takes faces to delete only, so {words}."
                )))
            });
            [
                Action::Apply(transaction),
                Action::Editing(EditingCommand::OpenSolid(feature)),
            ]
            .into_iter()
            .chain(told)
            .collect()
        }
        Err(reason) => vec![queued::refused(Command::DeleteFace, TITLE, &reason)],
    }
}

pub fn edit(document: &Document, feature: FeatureId, deletion: DeleteFace) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    Some(Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::DeleteFace(deletion),
        },
    ))
}

fn chosen(solid: &Solid, deletion: &DeleteFace) -> BTreeSet<FaceId> {
    deletion
        .resolutions(solid)
        .iter()
        .flat_map(|resolution| resolution.found().iter().copied())
        .collect()
}

pub fn deleted_faces(solid: &Solid, deletion: &DeleteFace) -> BTreeSet<FaceKey> {
    let deleted = chosen(solid, deletion);
    bodies::face_keys(solid)
        .into_iter()
        .filter_map(|(id, key)| deleted.contains(&id).then_some(key))
        .collect()
}

pub fn toggle_face(model: &Model, feature: FeatureId, face: FaceKey) -> Option<Transaction> {
    let owner = model.document().feature(feature)?;
    let deletion = owner.kind.delete_face()?;
    let input = bodies::input(model.evaluation(), feature)?;
    let clicked = bodies::find_face(input, face)?;
    let solid = &input.solid;
    let mut changed = deletion.clone();
    changed.faces = deletion
        .faces
        .iter()
        .zip(deletion.resolutions(solid))
        .filter(|(_, resolution)| !resolution.found().contains(&clicked))
        .map(|(reference, _)| reference.clone())
        .collect();
    let label = if changed.faces.len() == deletion.faces.len() {
        changed.faces.push(FaceReference::capture(solid, clicked)?);
        format!("Delete a face with {}", owner.name)
    } else {
        format!("Keep a face out of {}", owner.name)
    };
    Some(Transaction::single(
        label,
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::DeleteFace(changed),
        },
    ))
}

pub fn with_selected_faces(
    model: &Model,
    feature: FeatureId,
    selection: &Selection,
) -> Option<Transaction> {
    let owner = model.document().feature(feature)?;
    let deletion = owner.kind.delete_face()?;
    let input = bodies::input(model.evaluation(), feature)?;
    let solid = &input.solid;
    let mut taken = chosen(solid, deletion);
    let mut changed = deletion.clone();
    for pickable in selection.iter() {
        let Pickable::Face { body, face } = pickable else {
            continue;
        };
        let Some(found) = (body == deletion.body)
            .then(|| bodies::find_face(input, face))
            .flatten()
            .filter(|found| !taken.contains(found))
        else {
            continue;
        };
        changed.faces.push(FaceReference::capture(solid, found)?);
        taken.insert(found);
    }
    (changed.faces.len() > deletion.faces.len()).then(|| {
        Transaction::single(
            format!("Delete the selected faces with {}", owner.name),
            Edit::SetFeatureKind {
                id: feature,
                kind: FeatureKind::DeleteFace(changed),
            },
        )
    })
}
