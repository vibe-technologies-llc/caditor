use std::collections::BTreeSet;

use caditor_document::{
    AxisReference, Document, Edit, Evaluation, FeatureId, FeatureKind, MoveFace, Transaction,
};
use caditor_expression::{Expression, Unit};
use caditor_kernel::{FaceId, FaceReference, Solid};

use crate::{
    bodies::{self, FaceKey},
    body_selection,
    commands::Command,
    datum_tools,
    editing::{self, EditingCommand},
    feature_fields::ChoiceRefusal,
    field::{self, CheckError},
    model::{Action, Model, Notice},
    offset_face_tools::{self, FacePurpose, FaceToolError},
    queued,
    selection::{Pickable, Selection},
    shell_tools::FaceSource,
};

pub const TITLE: &str = "Move face";
pub const DEFAULT_DISTANCE: f64 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reference {
    Direction,
    Axis,
}

impl Reference {
    fn of(self, moving: &MoveFace) -> Option<&AxisReference> {
        match self {
            Self::Direction => moving.direction.as_ref(),
            Self::Axis => moving.axis.as_ref(),
        }
    }

    pub fn with(self, moving: &MoveFace, reference: Option<AxisReference>) -> MoveFace {
        match self {
            Self::Direction => MoveFace {
                direction: reference,
                ..moving.clone()
            },
            Self::Axis => MoveFace {
                axis: reference,
                ..moving.clone()
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MoveFaceError {
    #[error("The feature no longer exists")]
    Gone { feature: FeatureId },
    #[error("Select an axis, straight edge, round face or sketch line made before this move")]
    NoAxis,
    #[error(transparent)]
    AxisSelection(datum_tools::OneChoiceError),
    #[error(transparent)]
    Choice(#[from] ChoiceRefusal),
    #[error(transparent)]
    Check(#[from] CheckError),
}

pub fn selected_faces(model: &Model, selection: &Selection) -> Result<FaceSource, FaceToolError> {
    offset_face_tools::faces_of_one_body(model, selection, FacePurpose::Move)
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
        FeatureKind::MoveFace(MoveFace {
            body: source.body,
            faces,
            distance: Expression::Measure(DEFAULT_DISTANCE, Unit::Millimetre),
            direction: None,
            angle: Expression::Measure(0.0, Unit::Degree),
            axis: None,
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
        Err(reason) => vec![queued::refused(Command::MoveFace, TITLE, &reason)],
    }
}

pub fn edit(document: &Document, feature: FeatureId, moving: MoveFace) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    Some(Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::MoveFace(moving),
        },
    ))
}

pub fn change(
    model: &Model,
    feature: FeatureId,
    moving: MoveFace,
) -> Result<Transaction, MoveFaceError> {
    let document = model.document();
    let transaction = edit(document, feature, moving).ok_or(MoveFaceError::Gone { feature })?;
    field::checked(document, transaction)
}

pub fn reference_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    moving: &MoveFace,
    reference: Reference,
) -> Result<Transaction, MoveFaceError> {
    let index = model
        .document()
        .feature_index(feature)
        .ok_or(MoveFaceError::Gone { feature })?;
    let axis = datum_tools::only_axis(model, selection, index)
        .map_err(MoveFaceError::AxisSelection)?
        .ok_or(MoveFaceError::NoAxis)?;
    if reference.of(moving) == Some(&axis) {
        return Err(ChoiceRefusal::AlreadyChosen.into());
    }
    change(model, feature, reference.with(moving, Some(axis)))
}

fn chosen(solid: &Solid, moving: &MoveFace) -> BTreeSet<FaceId> {
    moving
        .resolutions(solid)
        .iter()
        .flat_map(|resolution| resolution.found().iter().copied())
        .collect()
}

pub fn moved_faces(solid: &Solid, moving: &MoveFace) -> BTreeSet<FaceKey> {
    let moved = chosen(solid, moving);
    bodies::face_keys(solid)
        .into_iter()
        .filter_map(|(id, key)| moved.contains(&id).then_some(key))
        .collect()
}

pub fn toggle_face(model: &Model, feature: FeatureId, face: FaceKey) -> Option<Transaction> {
    let document = model.document();
    let owner = document.feature(feature)?;
    let moving = owner.kind.move_face()?;
    let input = bodies::input(model.evaluation(), feature)?;
    let clicked = bodies::find_face(input, face)?;
    let solid = &input.solid;
    let mut changed = moving.clone();
    changed.faces = moving
        .faces
        .iter()
        .zip(moving.resolutions(solid))
        .filter(|(_, resolution)| !resolution.found().contains(&clicked))
        .map(|(reference, _)| reference.clone())
        .collect();
    let label = if changed.faces.len() == moving.faces.len() {
        changed.faces.push(FaceReference::capture(solid, clicked)?);
        format!("Move a face with {}", owner.name)
    } else {
        format!("Leave a face out of {}", owner.name)
    };
    Some(Transaction::single(
        label,
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::MoveFace(changed),
        },
    ))
}

pub fn with_selected_faces(
    model: &Model,
    feature: FeatureId,
    selection: &Selection,
) -> Option<Transaction> {
    let owner = model.document().feature(feature)?;
    let moving = owner.kind.move_face()?;
    let input = bodies::input(model.evaluation(), feature)?;
    let solid = &input.solid;
    let mut taken = chosen(solid, moving);
    let mut changed = moving.clone();
    for pickable in selection.iter() {
        let Pickable::Face { body, face } = pickable else {
            continue;
        };
        let Some(found) = (body == moving.body)
            .then(|| bodies::find_face(input, face))
            .flatten()
            .filter(|found| !taken.contains(found))
        else {
            continue;
        };
        changed.faces.push(FaceReference::capture(solid, found)?);
        taken.insert(found);
    }
    (changed.faces.len() > moving.faces.len()).then(|| {
        Transaction::single(
            format!("Move the selected faces with {}", owner.name),
            Edit::SetFeatureKind {
                id: feature,
                kind: FeatureKind::MoveFace(changed),
            },
        )
    })
}
