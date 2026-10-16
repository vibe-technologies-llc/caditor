use std::{borrow::Cow, collections::BTreeSet};

use caditor_document::{
    Document, Edit, Evaluation, Feature, FeatureId, FeatureKind, PlaneReference, PrincipalPlane,
    SplitAlong, SplitCarry, SplitFace, Transaction, on_one_unrolled_surface,
};
use caditor_kernel::{FaceId, FaceReference, Solid};

use crate::{
    bodies::{self, FaceKey},
    body_selection,
    commands::{Command, Reason},
    datum_tools,
    editing::{self, EditingCommand},
    field::{self, CheckError},
    model::{Action, Model, Notice},
    queued,
    selection::{Pickable, Selection},
    split_tools::{self, SplitError},
};

pub const TITLE: &str = "Split face";
const DEFAULT_PLANE: PlaneReference = PlaneReference::Principal(PrincipalPlane::Yz);

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SplitFaceError {
    #[error("Select the faces of a body to split")]
    ChooseFaces,
    #[error("Select faces of one body only")]
    OneBody,
    #[error("{}", queued::BODY_SHAPE)]
    NoShape { body: FeatureId },
    #[error("Some of the selected faces are no longer part of the model")]
    FacesGone,
    #[error("Select a plane, flat face, sketch curve or another body made before this feature")]
    NoTool,
    #[error("The faces are already split along the selected plane, curve or body")]
    Already,
    #[error("Several other bodies are selected; select only the one to split along")]
    SeveralTools,
    #[error("The feature no longer exists")]
    Gone { feature: FeatureId },
    #[error("Select an edge, axis, round face or sketch line to carry the curves along")]
    NoDirection,
    #[error("The curves are already carried along the selected edge or axis")]
    AlreadyCarried,
    #[error("Only a sketch's curves can be wrapped round the faces; split along one first")]
    NotAlongSketch,
    #[error("The curves are already wrapped round the faces")]
    AlreadyWrapped,
    #[error("Only faces of one cylinder or cone can have curves wrapped round them")]
    NotOneSurface,
    #[error("{name} does not split faces")]
    DoesNotSplitFaces { name: String },
    #[error("{0}")]
    ChosenAxis(String),
    #[error(transparent)]
    Along(#[from] SplitError),
    #[error(transparent)]
    Checked(#[from] CheckError),
}

impl From<SplitFaceError> for String {
    fn from(error: SplitFaceError) -> Self {
        error.to_string()
    }
}

impl Reason for SplitFaceError {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SplitFaceSource {
    pub body: FeatureId,
    pub faces: Vec<FaceKey>,
    pub along: Option<SplitAlong>,
    pub left_out: Vec<Pickable>,
}

fn without_faces(selection: &Selection) -> Selection {
    let mut rest = selection.clone();
    rest.retain(|pickable| !matches!(pickable, Pickable::Face { .. }));
    rest
}

pub fn source(model: &Model, selection: &Selection) -> Result<SplitFaceSource, SplitFaceError> {
    let mut body = None;
    let mut faces = Vec::new();
    for pickable in selection.iter() {
        if let Pickable::Face { body: owner, face } = pickable {
            match body {
                Some(known) if known != owner => return Err(SplitFaceError::OneBody),
                _ => body = Some(owner),
            }
            faces.push(face);
        }
    }
    let body = body.ok_or(SplitFaceError::ChooseFaces)?;
    bodies::shown(model.evaluation(), body).ok_or(SplitFaceError::ChooseFaces)?;
    let rest = without_faces(selection);
    let along = split_tools::chosen_along(model, &rest, model.document().bar_index())?
        .map(|(along, _)| along);
    let left_out = rest
        .iter()
        .filter(|pickable| {
            along.is_none()
                || !matches!(
                    pickable,
                    Pickable::SketchEntity { .. }
                        | Pickable::SketchRegion { .. }
                        | Pickable::Plane(_)
                        | Pickable::Datum(_)
                        | Pickable::FramePlane { .. }
                )
        })
        .collect();
    Ok(SplitFaceSource {
        body,
        faces,
        along,
        left_out,
    })
}

pub fn create(
    document: &Document,
    evaluation: &Evaluation,
    source: &SplitFaceSource,
) -> Result<(Transaction, FeatureId), SplitFaceError> {
    let shown = bodies::shown(evaluation, source.body)
        .ok_or(SplitFaceError::NoShape { body: source.body })?;
    let faces: Vec<FaceReference> = source
        .faces
        .iter()
        .filter_map(|key| FaceReference::capture(&shown.solid, bodies::find_face(shown, *key)?))
        .collect();
    if faces.len() < source.faces.len() {
        return Err(SplitFaceError::FacesGone);
    }
    let name = editing::next_feature_name(document, TITLE);
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(
        name,
        FeatureKind::SplitFace(SplitFace {
            body: source.body,
            faces,
            along: source
                .along
                .clone()
                .unwrap_or(SplitAlong::Plane(DEFAULT_PLANE)),
            carry: SplitCarry::Square,
        }),
    );
    Ok((transaction.finish(), feature))
}

pub fn create_actions(
    document: &Document,
    evaluation: &Evaluation,
    source: &SplitFaceSource,
) -> Vec<Action> {
    match create(document, evaluation, source) {
        Ok((transaction, feature)) => {
            let told = body_selection::left_out_words(&source.left_out).map(|words| {
                Action::Inform(Notice::warning(format!(
                    "{TITLE} takes the faces to split and one plane or sketch to split them \
                     along, so {words}."
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
            Command::SplitFace,
            TITLE,
            &reason.to_string(),
        )],
    }
}

pub fn edit(document: &Document, feature: FeatureId, split: SplitFace) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    Some(Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::SplitFace(split),
        },
    ))
}

pub fn change(
    model: &Model,
    feature: FeatureId,
    split: SplitFace,
) -> Result<Transaction, SplitFaceError> {
    let document = model.document();
    let transaction = edit(document, feature, split).ok_or(SplitFaceError::Gone { feature })?;
    field::checked(document, transaction)
}

pub fn choices(model: &Model, feature: FeatureId, split: &SplitFace) -> Vec<SplitAlong> {
    let document = model.document();
    let index = document.feature_index(feature).unwrap_or(0);
    let planes = datum_tools::listed_planes(document, feature)
        .into_iter()
        .map(SplitAlong::Plane);
    let bodies = document
        .bodies_before_in(feature, model.evaluation())
        .into_iter()
        .filter(|body| *body != split.body)
        .map(SplitAlong::Body);
    let sketches = document
        .features()
        .take(index)
        .filter(|earlier| matches!(earlier.kind, FeatureKind::Sketch(_)))
        .map(|sketch| SplitAlong::Sketch(sketch.id()));
    planes.chain(bodies).chain(sketches).collect()
}

pub fn along_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    split: &SplitFace,
) -> Result<Transaction, SplitFaceError> {
    let document = model.document();
    let index = document
        .feature_index(feature)
        .ok_or(SplitFaceError::Gone { feature })?;
    let along = match split_tools::chosen_along(model, selection, index)? {
        Some((along, _)) => along,
        None => {
            let earlier = document.bodies_before_in(feature, model.evaluation());
            let tools: Vec<FeatureId> = body_selection::bodies_in(selection)
                .into_iter()
                .filter(|body| *body != split.body && earlier.contains(body))
                .collect();
            match tools.as_slice() {
                [] => return Err(SplitFaceError::NoTool),
                [tool] => SplitAlong::Body(*tool),
                [_, _, ..] => return Err(SplitFaceError::SeveralTools),
            }
        }
    };
    if along == split.along {
        return Err(SplitFaceError::Already);
    }
    change(model, feature, split.with_along(along))
}

pub fn direction_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    split: &SplitFace,
) -> Result<Transaction, SplitFaceError> {
    let index = model
        .document()
        .feature_index(feature)
        .ok_or(SplitFaceError::Gone { feature })?;
    let axis = datum_tools::only_axis(model, selection, index)
        .map_err(|error| SplitFaceError::ChosenAxis(error.into()))?
        .ok_or(SplitFaceError::NoDirection)?;
    if split.direction() == Some(&axis) {
        return Err(SplitFaceError::AlreadyCarried);
    }
    change(
        model,
        feature,
        SplitFace {
            carry: SplitCarry::Along(Box::new(axis)),
            ..split.clone()
        },
    )
}

fn on_one_unrolled(model: &Model, feature: FeatureId, split: &SplitFace) -> bool {
    let Some(input) = bodies::input(model.evaluation(), feature) else {
        return true;
    };
    let solid = &input.solid;
    split
        .resolve(solid)
        .map_or(true, |faces| on_one_unrolled_surface(solid, &faces))
}

pub fn wrapped_change(
    model: &Model,
    feature: FeatureId,
    split: &SplitFace,
) -> Result<Transaction, SplitFaceError> {
    if split.along.sketch().is_none() {
        return Err(SplitFaceError::NotAlongSketch);
    }
    if split.is_wrapped() {
        return Err(SplitFaceError::AlreadyWrapped);
    }
    if !on_one_unrolled(model, feature, split) {
        return Err(SplitFaceError::NotOneSurface);
    }
    change(
        model,
        feature,
        SplitFace {
            carry: SplitCarry::Wrapped,
            ..split.clone()
        },
    )
}

pub fn wrap_command(model: &Model, feature: &Feature) -> Result<Transaction, SplitFaceError> {
    match feature.kind.split_face() {
        Some(split) => wrapped_change(model, feature.id(), split),
        None => Err(SplitFaceError::DoesNotSplitFaces {
            name: feature.name.clone(),
        }),
    }
}

pub fn chosen_faces(solid: &Solid, split: &SplitFace) -> BTreeSet<FaceKey> {
    let chosen: BTreeSet<FaceId> = split
        .resolutions(solid)
        .iter()
        .flat_map(|resolution| resolution.found().iter().copied())
        .collect();
    bodies::face_keys(solid)
        .into_iter()
        .filter_map(|(id, key)| chosen.contains(&id).then_some(key))
        .collect()
}

pub fn toggle_face(model: &Model, feature: FeatureId, face: FaceKey) -> Option<Transaction> {
    let document = model.document();
    let owner = document.feature(feature)?;
    let split = owner.kind.split_face()?;
    let input = bodies::input(model.evaluation(), feature)?;
    let clicked = bodies::find_face(input, face)?;
    let solid = &input.solid;
    let mut changed = split.clone();
    changed.faces = split
        .faces
        .iter()
        .zip(split.resolutions(solid))
        .filter(|(_, resolution)| !resolution.found().contains(&clicked))
        .map(|(reference, _)| reference.clone())
        .collect();
    let label = if changed.faces.len() == split.faces.len() {
        changed.faces.push(FaceReference::capture(solid, clicked)?);
        format!("Split a face with {}", owner.name)
    } else {
        format!("Leave a face out of {}", owner.name)
    };
    Some(Transaction::single(
        label,
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::SplitFace(changed),
        },
    ))
}

pub fn with_selected_faces(
    model: &Model,
    feature: FeatureId,
    selection: &Selection,
) -> Option<Transaction> {
    let owner = model.document().feature(feature)?;
    let split = owner.kind.split_face()?;
    let input = bodies::input(model.evaluation(), feature)?;
    let solid = &input.solid;
    let mut taken: BTreeSet<FaceId> = split
        .resolutions(solid)
        .iter()
        .flat_map(|resolution| resolution.found().iter().copied())
        .collect();
    let mut changed = split.clone();
    for pickable in selection.iter() {
        let Pickable::Face { body, face } = pickable else {
            continue;
        };
        let Some(found) = (body == split.body)
            .then(|| bodies::find_face(input, face))
            .flatten()
            .filter(|found| !taken.contains(found))
        else {
            continue;
        };
        changed.faces.push(FaceReference::capture(solid, found)?);
        taken.insert(found);
    }
    (changed.faces.len() > split.faces.len()).then(|| {
        Transaction::single(
            format!("Split the selected faces with {}", owner.name),
            Edit::SetFeatureKind {
                id: feature,
                kind: FeatureKind::SplitFace(changed),
            },
        )
    })
}
