use std::collections::BTreeSet;

use caditor_document::{
    Document, Edit, Evaluation, FeatureId, FeatureKind, PlaneReference, PrincipalPlane, SplitAlong,
    SplitFace, Transaction,
};
use caditor_kernel::{FaceId, FaceReference, Solid};

use crate::{
    bodies::{self, FaceKey},
    body_selection, datum_tools,
    editing::{self, EditingCommand},
    field,
    model::{Action, Model, Notice},
    selection::{Pickable, Selection},
    split_tools,
};

pub const TITLE: &str = "Split face";
const CHOOSE_FACES: &str = "Select the faces of a body to split";
const ONE_BODY: &str = "Select faces of one body only";
const NO_TOOL: &str =
    "Select a plane, flat face, sketch curve or another body made before this feature";
const ALREADY: &str = "The faces are already split along the selected plane, curve or body";
const SEVERAL_TOOLS: &str = "Several other bodies are selected; select only the one to split along";
const GONE: &str = "The feature no longer exists";
const NO_DIRECTION: &str =
    "Select an edge, axis, round face or sketch line to carry the curves along";
const ALREADY_CARRIED: &str = "The curves are already carried along the selected edge or axis";
const DEFAULT_PLANE: PlaneReference = PlaneReference::Principal(PrincipalPlane::Yz);

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

pub fn source(model: &Model, selection: &Selection) -> Result<SplitFaceSource, &'static str> {
    let mut body = None;
    let mut faces = Vec::new();
    for pickable in selection.iter() {
        if let Pickable::Face { body: owner, face } = pickable {
            match body {
                Some(known) if known != owner => return Err(ONE_BODY),
                _ => body = Some(owner),
            }
            faces.push(face);
        }
    }
    let body = body.ok_or(CHOOSE_FACES)?;
    bodies::shown(model.evaluation(), body).ok_or(CHOOSE_FACES)?;
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
) -> Result<(Transaction, FeatureId), &'static str> {
    let shown = bodies::shown(evaluation, source.body)
        .ok_or("The body has no shape yet; recompute the model, then try again")?;
    let faces: Vec<FaceReference> = source
        .faces
        .iter()
        .filter_map(|key| FaceReference::capture(&shown.solid, bodies::find_face(shown, *key)?))
        .collect();
    if faces.len() < source.faces.len() {
        return Err("Some of the selected faces are no longer part of the model");
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
            direction: None,
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
        Err(reason) => vec![Action::Inform(Notice::warning(format!(
            "{TITLE}: {reason}."
        )))],
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

pub fn change(model: &Model, feature: FeatureId, split: SplitFace) -> Result<Transaction, String> {
    let document = model.document();
    let transaction = edit(document, feature, split).ok_or_else(|| GONE.to_owned())?;
    field::checked(document, transaction)
}

pub fn choices(model: &Model, feature: FeatureId, split: &SplitFace) -> Vec<SplitAlong> {
    let document = model.document();
    let index = document.feature_index(feature).unwrap_or(0);
    let planes = datum_tools::listed_planes(document, feature)
        .into_iter()
        .map(SplitAlong::Plane);
    let bodies = document
        .bodies_before(feature)
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
) -> Result<Transaction, String> {
    let document = model.document();
    let index = document
        .feature_index(feature)
        .ok_or_else(|| GONE.to_owned())?;
    let along = match split_tools::chosen_along(model, selection, index)? {
        Some((along, _)) => along,
        None => {
            let earlier = document.bodies_before(feature);
            let tools: Vec<FeatureId> = body_selection::bodies_in(selection)
                .into_iter()
                .filter(|body| *body != split.body && earlier.contains(body))
                .collect();
            match tools.as_slice() {
                [] => return Err(NO_TOOL.to_owned()),
                [tool] => SplitAlong::Body(*tool),
                [_, _, ..] => return Err(SEVERAL_TOOLS.to_owned()),
            }
        }
    };
    if along == split.along {
        return Err(ALREADY.to_owned());
    }
    change(model, feature, split.with_along(along))
}

pub fn direction_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    split: &SplitFace,
) -> Result<Transaction, String> {
    let index = model
        .document()
        .feature_index(feature)
        .ok_or_else(|| GONE.to_owned())?;
    let axis =
        datum_tools::only_axis(model, selection, index)?.ok_or_else(|| NO_DIRECTION.to_owned())?;
    if split.direction() == Some(&axis) {
        return Err(ALREADY_CARRIED.to_owned());
    }
    change(
        model,
        feature,
        SplitFace {
            direction: Some(Box::new(axis)),
            ..split.clone()
        },
    )
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
