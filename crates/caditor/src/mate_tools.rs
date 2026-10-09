use caditor_document::{
    AxisMate, AxisReference, Document, Edit, FaceMate, FeatureId, FeatureKind, Mate, MatePair,
    PlaneReference, Transaction,
};
use caditor_kernel::FaceReference;

use crate::{
    datum_tools,
    editing::{self, EditingCommand},
    field,
    model::{Action, Model},
    move_tools,
    selection::{Pickable, Selection},
};

const NAME: &str = "Mate";
const NO_PAIR: &str = "Select a flat face or axis of the body to move, then the face, plane or \
                       axis to mate it onto";
const SAME_BODY: &str =
    "Select what to mate onto on another body, or a plane or axis; not on the body that moves";
const NO_FACE: &str = "Select a flat face of the body this mate moves";
const NO_AXIS: &str = "Select a straight edge or round face of the body this mate moves";
const NO_TARGET_PLANE: &str =
    "Select a flat face of another body, or a plane, made before this feature";
const NO_TARGET_AXIS: &str =
    "Select an axis, or a straight edge or round face of another body, made before this feature";
const ALREADY: &str = "The mate already uses the selected face or axis";
const GONE: &str = "The feature no longer exists";

#[derive(Debug, Clone, PartialEq)]
pub struct MateSource {
    pub body: FeatureId,
    pub pair: MatePair,
}

fn moving_face(
    model: &Model,
    pickable: Pickable,
    index: usize,
) -> Option<(FeatureId, FaceReference)> {
    match datum_tools::plane_reference(model, pickable, index)? {
        PlaneReference::Face(attachment) => Some((attachment.body, attachment.face)),
        PlaneReference::Principal(_) | PlaneReference::Datum(_) => None,
    }
}

fn moving_axis(
    model: &Model,
    pickable: Pickable,
    index: usize,
) -> Option<(FeatureId, AxisReference)> {
    let axis = datum_tools::axis_reference(model, pickable, index)?;
    Some((axis.body()?, axis))
}

pub fn source(model: &Model, selection: &Selection) -> Result<MateSource, &'static str> {
    let index = model.document().bar_index();
    let picked = selection.in_pick_order();
    let [first, second] = picked.as_slice() else {
        return Err(NO_PAIR);
    };
    let body = first.body().ok_or(NO_PAIR)?;
    if second.body() == Some(body) {
        return Err(SAME_BODY);
    }
    let shown = |body| move_tools::chosen_body(model, &Selection::default(), &[body], NO_PAIR);
    let face = moving_face(model, *first, index);
    let target = datum_tools::plane_reference(model, *second, index);
    if let (Some((body, face)), Some(target)) = (face, target) {
        return Ok(MateSource {
            body: shown(body)?,
            pair: MatePair::Faces(Box::new(FaceMate {
                face,
                target,
                distance: model.length_unit().default_length(0.0),
            })),
        });
    }
    let axis = moving_axis(model, *first, index);
    let target = datum_tools::axis_reference(model, *second, index);
    match (axis, target) {
        (Some((body, axis)), Some(target)) => Ok(MateSource {
            body: shown(body)?,
            pair: MatePair::Axes(Box::new(AxisMate { axis, target })),
        }),
        _ => Err(NO_PAIR),
    }
}

pub fn create(document: &Document, source: &MateSource) -> (Transaction, FeatureId) {
    let name = editing::next_feature_name(document, NAME);
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(
        name,
        FeatureKind::Mate(Mate {
            body: source.body,
            pair: source.pair.clone(),
            flipped: false,
        }),
    );
    (transaction.finish(), feature)
}

pub fn create_actions(model: &Model, source: &MateSource) -> Vec<Action> {
    let (transaction, feature) = create(model.document(), source);
    vec![
        Action::Apply(transaction),
        Action::Editing(EditingCommand::OpenSolid(feature)),
    ]
}

pub fn edit(document: &Document, feature: FeatureId, mate: Mate) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    Some(Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Mate(mate),
        },
    ))
}

pub fn change(model: &Model, feature: FeatureId, mate: Mate) -> Result<Transaction, String> {
    let document = model.document();
    let transaction = edit(document, feature, mate).ok_or_else(|| GONE.to_owned())?;
    field::checked(document, transaction)
}

fn index_of(model: &Model, feature: FeatureId) -> Result<usize, String> {
    model
        .document()
        .feature_index(feature)
        .ok_or_else(|| GONE.to_owned())
}

fn changed(
    model: &Model,
    feature: FeatureId,
    mate: &Mate,
    pair: MatePair,
) -> Result<Transaction, String> {
    if pair == mate.pair {
        return Err(ALREADY.to_owned());
    }
    change(
        model,
        feature,
        Mate {
            pair,
            ..mate.clone()
        },
    )
}

pub fn moving_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    mate: &Mate,
) -> Result<Transaction, String> {
    let index = index_of(model, feature)?;
    let on_body = selection
        .in_pick_order()
        .into_iter()
        .filter(|pickable| pickable.body() == Some(mate.body));
    let pair = match &mate.pair {
        MatePair::Faces(faces) => {
            let face = on_body
                .filter_map(|pickable| moving_face(model, pickable, index))
                .next()
                .ok_or(NO_FACE)?
                .1;
            MatePair::Faces(Box::new(FaceMate {
                face,
                ..faces.as_ref().clone()
            }))
        }
        MatePair::Axes(axes) => {
            let axis = on_body
                .filter_map(|pickable| moving_axis(model, pickable, index))
                .next()
                .ok_or(NO_AXIS)?
                .1;
            MatePair::Axes(Box::new(AxisMate {
                axis,
                ..axes.as_ref().clone()
            }))
        }
    };
    changed(model, feature, mate, pair)
}

pub fn target_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    mate: &Mate,
) -> Result<Transaction, String> {
    let index = index_of(model, feature)?;
    let elsewhere = selection
        .in_pick_order()
        .into_iter()
        .filter(|pickable| pickable.body() != Some(mate.body));
    let pair = match &mate.pair {
        MatePair::Faces(faces) => {
            let target = elsewhere
                .filter_map(|pickable| datum_tools::plane_reference(model, pickable, index))
                .next()
                .ok_or(NO_TARGET_PLANE)?;
            MatePair::Faces(Box::new(FaceMate {
                target,
                ..faces.as_ref().clone()
            }))
        }
        MatePair::Axes(axes) => {
            let target = elsewhere
                .filter_map(|pickable| datum_tools::axis_reference(model, pickable, index))
                .next()
                .ok_or(NO_TARGET_AXIS)?;
            MatePair::Axes(Box::new(AxisMate {
                target,
                ..axes.as_ref().clone()
            }))
        }
    };
    changed(model, feature, mate, pair)
}
