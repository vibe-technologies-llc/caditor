use std::collections::BTreeSet;

use caditor_document::{
    Blend, BlendKind, ChamferForm, Document, Edit, Evaluation, Feature, FeatureId, FeatureKind,
    SolidResult, Transaction,
};
use caditor_expression::{Expression, Unit};
use caditor_kernel::{EdgeId, EdgeName, EdgeNaming, EdgeReference, Solid, blend_chain};

use crate::{
    bodies, body_selection,
    editing::{self, EditingCommand},
    model::{Action, Model, Notice},
    selection::{Pickable, Selection},
    units::LengthUnit,
};

pub const DEFAULT_SIZE: f64 = 1.0;
const NO_SHAPE: &str = "The body has no shape yet; recompute the model, then try again";

pub const KINDS: [BlendKind; 2] = [BlendKind::Fillet, BlendKind::Chamfer];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeSource {
    pub body: FeatureId,
    pub edges: Vec<EdgeName>,
    pub left_out: Vec<Pickable>,
}

pub const NOTHING_TO_BLEND: &str =
    "Select edges or faces of a body, or choose a body in the tree, first";
pub const SEVERAL_BODIES: &str = "Select edges or faces of one body only";
const FACE_GONE: &str = "A selected face is no longer part of the model";
const NO_EDGES: &str = "The selected faces or body have no edges to round";

pub fn selected_edges(
    model: &Model,
    selection: &Selection,
    tree: &[FeatureId],
) -> Result<EdgeSource, &'static str> {
    match tree {
        [] => edges_of_selection(model, selection),
        [body] => edges_of_body(model, *body),
        [_, _, ..] => Err(SEVERAL_BODIES),
    }
}

fn edges_of_body(model: &Model, body: FeatureId) -> Result<EdgeSource, &'static str> {
    let shown = bodies::shown(model.evaluation(), body).ok_or(NO_SHAPE)?;
    let edges = edge_names(shown, shown.solid.edges().map(|(id, _)| id));
    if edges.is_empty() {
        return Err(NO_EDGES);
    }
    Ok(EdgeSource {
        body,
        edges,
        left_out: Vec::new(),
    })
}

fn edges_of_selection(model: &Model, selection: &Selection) -> Result<EdgeSource, &'static str> {
    let mut body = None;
    let mut edges = Vec::new();
    let mut left_out = Vec::new();
    for pickable in selection.iter() {
        let owner = match pickable {
            Pickable::Edge { body, .. } | Pickable::Face { body, .. } => body,
            _ => {
                left_out.push(pickable);
                continue;
            }
        };
        match body {
            Some(known) if known != owner => return Err(SEVERAL_BODIES),
            _ => body = Some(owner),
        }
        match pickable {
            Pickable::Face { face, .. } => {
                let shown = bodies::shown(model.evaluation(), owner).ok_or(NO_SHAPE)?;
                let found = bodies::find_face(shown, face).ok_or(FACE_GONE)?;
                edges.extend(edge_names(
                    shown,
                    body_selection::face_boundary(&shown.solid, found),
                ));
            }
            Pickable::Edge { edge, .. } => edges.push(edge),
            _ => {}
        }
    }
    let body = body.ok_or(NOTHING_TO_BLEND)?;
    let mut seen = BTreeSet::new();
    edges.retain(|edge| seen.insert(*edge));
    if edges.is_empty() {
        return Err(NO_EDGES);
    }
    Ok(EdgeSource {
        body,
        edges,
        left_out,
    })
}

fn edge_names(shown: &SolidResult, edges: impl IntoIterator<Item = EdgeId>) -> Vec<EdgeName> {
    edges
        .into_iter()
        .filter(|edge| !bodies::is_seam(&shown.solid, *edge))
        .filter_map(|edge| shown.solid.edge(edge))
        .map(|edge| edge.name())
        .collect()
}

pub fn create(
    document: &Document,
    evaluation: &Evaluation,
    kind: BlendKind,
    source: &EdgeSource,
    unit: LengthUnit,
) -> Result<(Transaction, FeatureId), &'static str> {
    let shown = bodies::shown(evaluation, source.body).ok_or(NO_SHAPE)?;
    let naming = EdgeNaming::new(&shown.solid);
    let edges: Vec<EdgeReference> = source
        .edges
        .iter()
        .filter_map(|name| {
            let edge = bodies::find_edge(shown, *name)?;
            EdgeReference::capture_in(&naming, edge)
        })
        .collect();
    if edges.is_empty() {
        return Err("The selected edges are no longer part of the model");
    }
    if edges.len() < source.edges.len() {
        return Err("Some of the selected edges are no longer part of the model");
    }
    let name = editing::next_feature_name(document, kind.title());
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(
        name,
        FeatureKind::Blend(Blend {
            kind,
            body: source.body,
            edges,
            size: unit.default_length(DEFAULT_SIZE),
            form: ChamferForm::Equal,
            flipped: false,
        }),
    );
    Ok((transaction.finish(), feature))
}

pub fn create_actions(
    document: &Document,
    evaluation: &Evaluation,
    kind: BlendKind,
    source: &EdgeSource,
    unit: LengthUnit,
) -> Vec<Action> {
    match create(document, evaluation, kind, source, unit) {
        Ok((transaction, feature)) => {
            let told = body_selection::left_out_words(&source.left_out).map(|words| {
                Action::Inform(Notice::warning(format!(
                    "{} takes edges, faces and bodies only, so {words}.",
                    kind.title()
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
            "{}: {reason}.",
            kind.title()
        )))],
    }
}

pub fn edit(document: &Document, feature: FeatureId, blend: Blend) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    Some(Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Blend(blend),
        },
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormChoice {
    Equal,
    TwoDistances,
    DistanceAngle,
}

pub const FORMS: [FormChoice; 3] = [
    FormChoice::Equal,
    FormChoice::TwoDistances,
    FormChoice::DistanceAngle,
];

const DEFAULT_ANGLE_DEGREES: f64 = 45.0;
const NOT_A_CHAMFER: &str = "The feature is not a chamfer";
const NOTHING_TO_FLIP: &str =
    "A chamfer by one distance is the same on both faces; choose two distances or an angle first";

impl FormChoice {
    pub fn of(form: &ChamferForm) -> Self {
        match form {
            ChamferForm::Equal => Self::Equal,
            ChamferForm::TwoDistances { .. } => Self::TwoDistances,
            ChamferForm::DistanceAngle { .. } => Self::DistanceAngle,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Equal => "Equal",
            Self::TwoDistances => "Two distances",
            Self::DistanceAngle => "Distance and angle",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            Self::Equal => "Equal",
            Self::TwoDistances => "Two",
            Self::DistanceAngle => "Angle",
        }
    }

    pub fn hover(self) -> &'static str {
        match self {
            Self::Equal => "Bevel both faces by the same distance",
            Self::TwoDistances => "Bevel each face by its own distance",
            Self::DistanceAngle => {
                "Bevel one face by a distance, the cut turned by an angle from it"
            }
        }
    }

    pub fn applied_to(self, blend: &Blend) -> Blend {
        let form = match (self, &blend.form) {
            (Self::Equal, _) => ChamferForm::Equal,
            (Self::TwoDistances, ChamferForm::TwoDistances { second }) => {
                ChamferForm::TwoDistances {
                    second: second.clone(),
                }
            }
            (Self::TwoDistances, _) => ChamferForm::TwoDistances {
                second: blend.size.clone(),
            },
            (Self::DistanceAngle, ChamferForm::DistanceAngle { angle }) => {
                ChamferForm::DistanceAngle {
                    angle: angle.clone(),
                }
            }
            (Self::DistanceAngle, _) => ChamferForm::DistanceAngle {
                angle: Expression::measure(DEFAULT_ANGLE_DEGREES, Unit::Degree),
            },
        };
        Blend {
            kind: BlendKind::Chamfer,
            form,
            ..blend.clone()
        }
    }
}

fn chamfer_of(feature: &Feature) -> Result<&Blend, String> {
    feature
        .kind
        .blend()
        .filter(|blend| blend.kind == BlendKind::Chamfer)
        .ok_or_else(|| NOT_A_CHAMFER.to_owned())
}

pub fn form_change(
    document: &Document,
    feature: &Feature,
    choice: FormChoice,
) -> Result<Transaction, String> {
    let blend = chamfer_of(feature)?;
    if FormChoice::of(&blend.form) == choice {
        return Err(format!(
            "{} is already {}",
            feature.name,
            choice.label().to_lowercase()
        ));
    }
    edit(document, feature.id(), choice.applied_to(blend)).ok_or_else(|| NOT_A_CHAMFER.to_owned())
}

pub fn flip_change(document: &Document, feature: &Feature) -> Result<Transaction, String> {
    let blend = chamfer_of(feature)?;
    if blend.form.is_equal() {
        return Err(NOTHING_TO_FLIP.to_owned());
    }
    edit(
        document,
        feature.id(),
        Blend {
            flipped: !blend.flipped,
            ..blend.clone()
        },
    )
    .ok_or_else(|| NOT_A_CHAMFER.to_owned())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChosenEdges {
    pub explicit: BTreeSet<EdgeName>,
    pub followed: BTreeSet<EdgeName>,
}

pub fn chosen_edges(solid: &Solid, blend: &Blend) -> ChosenEdges {
    let name = |edge: &EdgeId| solid.edge(*edge).map(|edge| edge.name());
    let explicit: Vec<EdgeId> = blend
        .resolutions(solid)
        .iter()
        .flat_map(|resolution| resolution.found().iter().copied())
        .collect();
    let followed = blend_chain(solid, &explicit)
        .iter()
        .filter_map(name)
        .collect();
    ChosenEdges {
        explicit: explicit.iter().filter_map(name).collect(),
        followed,
    }
}

pub fn toggle_edge(model: &Model, feature: FeatureId, edge: EdgeName) -> Option<Transaction> {
    let document = model.document();
    let owner = document.feature(feature)?;
    let blend = owner.kind.blend()?;
    let input = bodies::input(model.evaluation(), feature)?;
    let clicked = bodies::find_edge(input, edge)?;
    let solid = &input.solid;
    let mut changed = blend.clone();
    changed.edges = blend
        .edges
        .iter()
        .zip(blend.resolutions(solid))
        .filter(|(_, resolution)| !blend_chain(solid, resolution.found()).contains(&clicked))
        .map(|(reference, _)| *reference)
        .collect();
    let label = if changed.edges.len() == blend.edges.len() {
        changed
            .edges
            .push(EdgeReference::capture_in(&EdgeNaming::new(solid), clicked)?);
        format!("Add an edge to {}", owner.name)
    } else {
        format!("Leave an edge out of {}", owner.name)
    };
    Some(Transaction::single(
        label,
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Blend(changed),
        },
    ))
}

pub fn with_selected_edges(
    model: &Model,
    feature: FeatureId,
    selection: &Selection,
) -> Option<Transaction> {
    let owner = model.document().feature(feature)?;
    let blend = owner.kind.blend()?;
    let input = bodies::input(model.evaluation(), feature)?;
    let solid = &input.solid;
    let naming = EdgeNaming::new(solid);
    let mut taken: BTreeSet<EdgeId> = blend
        .resolutions(solid)
        .iter()
        .flat_map(|resolution| blend_chain(solid, resolution.found()))
        .collect();
    let mut changed = blend.clone();
    let picked: Vec<EdgeId> = selection
        .iter()
        .filter(|pickable| pickable.body() == Some(blend.body))
        .flat_map(|pickable| match pickable {
            Pickable::Edge { edge, .. } => bodies::find_edge(input, edge).into_iter().collect(),
            Pickable::Face { face, .. } => bodies::find_face(input, face)
                .map(|face| body_selection::face_boundary(solid, face))
                .unwrap_or_default()
                .into_iter()
                .filter(|edge| !bodies::is_seam(solid, *edge))
                .collect(),
            _ => Vec::new(),
        })
        .collect();
    for found in picked {
        if taken.contains(&found) {
            continue;
        }
        changed
            .edges
            .push(EdgeReference::capture_in(&naming, found)?);
        taken.extend(blend_chain(solid, &[found]));
    }
    (changed.edges.len() > blend.edges.len()).then(|| {
        Transaction::single(
            format!("Add the selected edges to {}", owner.name),
            Edit::SetFeatureKind {
                id: feature,
                kind: FeatureKind::Blend(changed),
            },
        )
    })
}

#[cfg(test)]
mod tests {
    use caditor_document::{CancelToken, ModelEvaluator, Recompute};

    use super::*;
    use crate::samples::Sample;

    #[test]
    fn a_fillet_is_refused_when_any_selected_edge_is_gone() {
        let document = Sample::Plate.document().unwrap();

        let evaluation = Recompute::default().run(
            &document,
            &ModelEvaluator,
            &CancelToken::never(),
            &|_, _| {},
        );
        let (body, _) = evaluation.bodies().next().unwrap();
        let solid = evaluation.body(body).unwrap();
        let present = solid.edges().next().map(|(_, edge)| edge.name()).unwrap();
        let gone = EdgeName::from_digest(7);
        let create = |edges: Vec<EdgeName>| {
            create(
                &document,
                &evaluation,
                BlendKind::Fillet,
                &EdgeSource {
                    body,
                    edges,
                    left_out: Vec::new(),
                },
                LengthUnit::Millimetre,
            )
            .map(|_| ())
        };

        assert_eq!(create(vec![present]), Ok(()));
        assert_eq!(
            create(vec![present, gone]),
            Err("Some of the selected edges are no longer part of the model")
        );
        assert_eq!(
            create(vec![gone]),
            Err("The selected edges are no longer part of the model")
        );
    }
}
