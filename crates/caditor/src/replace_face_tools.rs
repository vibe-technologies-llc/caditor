use std::{borrow::Cow, collections::BTreeSet};

use caditor_document::{
    Document, Edit, Evaluation, FeatureId, FeatureKind, PlaneReference, ReplaceFace, Transaction,
};
use caditor_kernel::{FaceId, FaceReference, Solid};

use crate::{
    bodies::{self, FaceKey},
    body_selection,
    commands::{Command, Reason},
    datum_tools::{self, OneChoiceError},
    editing::{self, EditingCommand},
    field::{self, CheckError},
    model::{Action, Model, Notice},
    offset_face_tools::{FacePurpose, FaceToolError},
    queued::{self, Pending},
    selection::{Pickable, Selection},
    shell_tools::FaceSource,
};

pub const TITLE: &str = "Replace face";

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ReplaceToolError {
    #[error("The feature no longer exists")]
    Gone { feature: FeatureId },
    #[error("Select a plane or flat face made before this feature")]
    NoPlane,
    #[error("The faces are already replaced with the selected plane or face")]
    AlreadyPlane,
    #[error(transparent)]
    ChosenPlane(OneChoiceError),
    #[error(transparent)]
    Check(#[from] CheckError),
}

impl Reason for ReplaceToolError {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReplaceSourceError {
    #[error(transparent)]
    Faces(#[from] FaceToolError),
    #[error(
        "Select one plane or face to replace the faces with; several planes or faces of other \
         bodies are selected"
    )]
    SeveralTargets,
}

impl Reason for ReplaceSourceError {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }

    fn pending(&self) -> Option<Pending> {
        match self {
            Self::Faces(error) => error.pending(),
            Self::SeveralTargets => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReplaceSource {
    pub faces: FaceSource,
    pub target: Option<PlaneReference>,
}

fn is_plane_pick(pickable: Pickable) -> bool {
    matches!(
        pickable,
        Pickable::Plane(_) | Pickable::Datum(_) | Pickable::FramePlane { .. }
    )
}

struct Picked {
    body: FeatureId,
    faces: Vec<FaceKey>,
    picks: Vec<Pickable>,
}

fn by_body(picks: &[Pickable]) -> Vec<Picked> {
    let mut groups: Vec<Picked> = Vec::new();
    for pickable in picks {
        let Pickable::Face { body, face } = *pickable else {
            continue;
        };
        match groups.iter_mut().find(|group| group.body == body) {
            Some(group) => {
                group.faces.push(face);
                group.picks.push(*pickable);
            }
            None => groups.push(Picked {
                body,
                faces: vec![face],
                picks: vec![*pickable],
            }),
        }
    }
    if let [first, second] = groups.as_mut_slice()
        && first.faces.len() == 1
        && second.faces.len() > 1
    {
        std::mem::swap(first, second);
    }
    groups
}

pub fn selected_faces(
    model: &Model,
    selection: &Selection,
) -> Result<ReplaceSource, ReplaceSourceError> {
    let no_faces = FaceToolError::NoFaces {
        purpose: FacePurpose::Replace,
    };
    let picks = selection.in_pick_order();
    let mut groups = by_body(&picks).into_iter();
    let replaced = groups.next().ok_or(no_faces)?;
    let mut targets: Vec<Pickable> = groups.flat_map(|group| group.picks).collect();
    targets.extend(
        picks
            .iter()
            .copied()
            .filter(|pickable| is_plane_pick(*pickable)),
    );
    let mut left_out: Vec<Pickable> = picks
        .iter()
        .copied()
        .filter(|pickable| !is_plane_pick(*pickable) && !matches!(pickable, Pickable::Face { .. }))
        .collect();
    bodies::shown(model.evaluation(), replaced.body).ok_or(no_faces)?;
    let target = match targets.as_slice() {
        [] => None,
        [only] => {
            let index = model.document().bar_index();
            let found = datum_tools::plane_reference(model, *only, index);
            if found.is_none() {
                left_out.push(*only);
            }
            found
        }
        [_, _, ..] => return Err(ReplaceSourceError::SeveralTargets),
    };
    Ok(ReplaceSource {
        faces: FaceSource {
            body: replaced.body,
            faces: replaced.faces,
            left_out,
        },
        target,
    })
}

pub fn create(
    document: &Document,
    evaluation: &Evaluation,
    source: &ReplaceSource,
) -> Result<(Transaction, FeatureId), FaceToolError> {
    let chosen = &source.faces;
    let shown = bodies::shown(evaluation, chosen.body).ok_or(FaceToolError::NoShape)?;
    let faces: Vec<FaceReference> = chosen
        .faces
        .iter()
        .filter_map(|key| FaceReference::capture(&shown.solid, bodies::find_face(shown, *key)?))
        .collect();
    if faces.len() < chosen.faces.len() {
        return Err(FaceToolError::FacesGone);
    }
    let name = editing::next_feature_name(document, TITLE);
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(
        name,
        FeatureKind::ReplaceFace(ReplaceFace {
            body: chosen.body,
            faces,
            target: source.target.clone(),
        }),
    );
    Ok((transaction.finish(), feature))
}

pub fn create_actions(
    document: &Document,
    evaluation: &Evaluation,
    source: &ReplaceSource,
) -> Vec<Action> {
    match create(document, evaluation, source) {
        Ok((transaction, feature)) => {
            let told = body_selection::left_out_words(&source.faces.left_out).map(|words| {
                Action::Inform(Notice::warning(format!(
                    "{TITLE} takes faces to replace and one plane or face to replace them with, \
                     so {words}."
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
        Err(reason) => vec![queued::refused(Command::ReplaceFace, TITLE, &reason)],
    }
}

pub fn edit(document: &Document, feature: FeatureId, replace: ReplaceFace) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    Some(Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::ReplaceFace(replace),
        },
    ))
}

pub fn change(
    model: &Model,
    feature: FeatureId,
    replace: ReplaceFace,
) -> Result<Transaction, ReplaceToolError> {
    let document = model.document();
    let transaction = edit(document, feature, replace).ok_or(ReplaceToolError::Gone { feature })?;
    field::checked(document, transaction)
}

pub fn target_change(
    model: &Model,
    feature: FeatureId,
    replace: &ReplaceFace,
    target: PlaneReference,
) -> Result<Transaction, ReplaceToolError> {
    change(
        model,
        feature,
        ReplaceFace {
            target: Some(target),
            ..replace.clone()
        },
    )
}

pub fn plane_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    replace: &ReplaceFace,
) -> Result<Transaction, ReplaceToolError> {
    let index = model
        .document()
        .feature_index(feature)
        .ok_or(ReplaceToolError::Gone { feature })?;
    let plane = datum_tools::chosen_plane(model, selection, index)
        .map_err(ReplaceToolError::ChosenPlane)?
        .ok_or(ReplaceToolError::NoPlane)?
        .plane;
    if replace.target.as_ref() == Some(&plane) {
        return Err(ReplaceToolError::AlreadyPlane);
    }
    target_change(model, feature, replace, plane)
}

fn chosen(solid: &Solid, replace: &ReplaceFace) -> BTreeSet<FaceId> {
    replace
        .resolutions(solid)
        .iter()
        .flat_map(|resolution| resolution.found().iter().copied())
        .collect()
}

pub fn replaced_faces(solid: &Solid, replace: &ReplaceFace) -> BTreeSet<FaceKey> {
    let replaced = chosen(solid, replace);
    bodies::face_keys(solid)
        .into_iter()
        .filter_map(|(id, key)| replaced.contains(&id).then_some(key))
        .collect()
}

pub fn toggle_face(model: &Model, feature: FeatureId, face: FaceKey) -> Option<Transaction> {
    let owner = model.document().feature(feature)?;
    let replace = owner.kind.replace_face()?;
    let input = bodies::input(model.evaluation(), feature)?;
    let clicked = bodies::find_face(input, face)?;
    let solid = &input.solid;
    let mut changed = replace.clone();
    changed.faces = replace
        .faces
        .iter()
        .zip(replace.resolutions(solid))
        .filter(|(_, resolution)| !resolution.found().contains(&clicked))
        .map(|(reference, _)| reference.clone())
        .collect();
    let label = if changed.faces.len() == replace.faces.len() {
        changed.faces.push(FaceReference::capture(solid, clicked)?);
        format!("Replace a face with {}", owner.name)
    } else {
        format!("Leave a face out of {}", owner.name)
    };
    Some(Transaction::single(
        label,
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::ReplaceFace(changed),
        },
    ))
}

pub fn with_selected_faces(
    model: &Model,
    feature: FeatureId,
    selection: &Selection,
) -> Option<Transaction> {
    let owner = model.document().feature(feature)?;
    let replace = owner.kind.replace_face()?;
    let input = bodies::input(model.evaluation(), feature)?;
    let solid = &input.solid;
    let mut taken = chosen(solid, replace);
    let mut changed = replace.clone();
    for pickable in selection.iter() {
        let Pickable::Face { body, face } = pickable else {
            continue;
        };
        let Some(found) = (body == replace.body)
            .then(|| bodies::find_face(input, face))
            .flatten()
            .filter(|found| !taken.contains(found))
        else {
            continue;
        };
        changed.faces.push(FaceReference::capture(solid, found)?);
        taken.insert(found);
    }
    (changed.faces.len() > replace.faces.len()).then(|| {
        Transaction::single(
            format!("Replace the selected faces with {}", owner.name),
            Edit::SetFeatureKind {
                id: feature,
                kind: FeatureKind::ReplaceFace(changed),
            },
        )
    })
}
