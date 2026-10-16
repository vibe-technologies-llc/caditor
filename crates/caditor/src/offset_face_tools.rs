use std::{
    borrow::Cow,
    collections::BTreeSet,
    fmt::{self, Display, Formatter},
};

use caditor_document::{
    Document, Edit, Evaluation, FeatureId, FeatureKind, OffsetFace, Transaction,
};
use caditor_kernel::{FaceId, FaceReference, Solid, tangent_faces};

use crate::{
    bodies::{self, FaceKey},
    body_selection,
    commands::{Command, Reason},
    editing::{self, EditingCommand},
    last_values::{Remembered, Starts},
    model::{Action, Model, Notice},
    queued,
    selection::{Pickable, Selection},
    shell_tools::FaceSource,
};

pub const DEFAULT_DISTANCE: f64 = 1.0;
pub const TITLE: &str = "Offset face";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FacePurpose {
    Move,
    Delete,
}

impl Display for FacePurpose {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Move => "move",
            Self::Delete => "delete",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FaceToolError {
    #[error("Select faces of one body only")]
    SeveralBodies,
    #[error("Select the faces of a body to {purpose}")]
    NoFaces { purpose: FacePurpose },
    #[error("{}", queued::BODY_SHAPE)]
    NoShape,
    #[error("Some of the selected faces are no longer part of the model")]
    FacesGone,
}

impl Reason for FaceToolError {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

pub fn selected_faces(model: &Model, selection: &Selection) -> Result<FaceSource, FaceToolError> {
    faces_of_one_body(model, selection, FacePurpose::Move)
}

pub fn faces_of_one_body(
    model: &Model,
    selection: &Selection,
    purpose: FacePurpose,
) -> Result<FaceSource, FaceToolError> {
    let mut body = None;
    let mut faces = Vec::new();
    let mut left_out = Vec::new();
    for pickable in selection.iter() {
        if let Pickable::Face { body: owner, face } = pickable {
            match body {
                Some(known) if known != owner => return Err(FaceToolError::SeveralBodies),
                _ => body = Some(owner),
            }
            faces.push(face);
        } else {
            left_out.push(pickable);
        }
    }
    let no_faces = FaceToolError::NoFaces { purpose };
    let body = body.ok_or(no_faces)?;
    bodies::shown(model.evaluation(), body).ok_or(no_faces)?;
    Ok(FaceSource {
        body,
        faces,
        left_out,
    })
}

pub fn create(
    document: &Document,
    evaluation: &Evaluation,
    source: &FaceSource,
    starts: &Starts,
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
        FeatureKind::OffsetFace(OffsetFace {
            body: source.body,
            faces,
            distance: starts.length(Remembered::OffsetDistance, DEFAULT_DISTANCE),
            tangent: false,
        }),
    );
    Ok((transaction.finish(), feature))
}

pub fn create_actions(
    document: &Document,
    evaluation: &Evaluation,
    source: &FaceSource,
    starts: &Starts,
) -> Vec<Action> {
    match create(document, evaluation, source, starts) {
        Ok((transaction, feature)) => {
            let told = body_selection::left_out_words(&source.left_out).map(|words| {
                Action::Inform(Notice::warning(format!(
                    "{TITLE} takes faces to move only, so {words}."
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
        Err(reason) => vec![queued::refused(
            Command::OffsetFace,
            TITLE,
            &reason.to_string(),
        )],
    }
}

pub fn edit(document: &Document, feature: FeatureId, offset: OffsetFace) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    Some(Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::OffsetFace(offset),
        },
    ))
}

pub fn moved_faces(solid: &Solid, offset: &OffsetFace) -> BTreeSet<FaceKey> {
    let chosen: Vec<FaceId> = offset
        .resolutions(solid)
        .iter()
        .flat_map(|resolution| resolution.found().iter().copied())
        .collect();
    let moved: BTreeSet<FaceId> = if offset.tangent {
        tangent_faces(solid, &chosen).into_iter().collect()
    } else {
        chosen.into_iter().collect()
    };
    bodies::face_keys(solid)
        .into_iter()
        .filter_map(|(id, key)| moved.contains(&id).then_some(key))
        .collect()
}

pub fn toggle_face(model: &Model, feature: FeatureId, face: FaceKey) -> Option<Transaction> {
    let document = model.document();
    let owner = document.feature(feature)?;
    let offset = owner.kind.offset_face()?;
    let input = bodies::input(model.evaluation(), feature)?;
    let clicked = bodies::find_face(input, face)?;
    let solid = &input.solid;
    let mut changed = offset.clone();
    changed.faces = offset
        .faces
        .iter()
        .zip(offset.resolutions(solid))
        .filter(|(_, resolution)| !resolution.found().contains(&clicked))
        .map(|(reference, _)| reference.clone())
        .collect();
    let label = if changed.faces.len() == offset.faces.len() {
        changed.faces.push(FaceReference::capture(solid, clicked)?);
        format!("Move a face with {}", owner.name)
    } else {
        format!("Leave a face out of {}", owner.name)
    };
    Some(Transaction::single(
        label,
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::OffsetFace(changed),
        },
    ))
}

pub fn with_selected_faces(
    model: &Model,
    feature: FeatureId,
    selection: &Selection,
) -> Option<Transaction> {
    let owner = model.document().feature(feature)?;
    let offset = owner.kind.offset_face()?;
    let input = bodies::input(model.evaluation(), feature)?;
    let solid = &input.solid;
    let mut taken: BTreeSet<FaceId> = offset
        .resolutions(solid)
        .iter()
        .flat_map(|resolution| resolution.found().iter().copied())
        .collect();
    let mut changed = offset.clone();
    for pickable in selection.iter() {
        let Pickable::Face { body, face } = pickable else {
            continue;
        };
        let Some(found) = (body == offset.body)
            .then(|| bodies::find_face(input, face))
            .flatten()
            .filter(|found| !taken.contains(found))
        else {
            continue;
        };
        changed.faces.push(FaceReference::capture(solid, found)?);
        taken.insert(found);
    }
    (changed.faces.len() > offset.faces.len()).then(|| {
        Transaction::single(
            format!("Move the selected faces with {}", owner.name),
            Edit::SetFeatureKind {
                id: feature,
                kind: FeatureKind::OffsetFace(changed),
            },
        )
    })
}
