use std::collections::BTreeSet;

use caditor_document::{
    Blend, BlendKind, ChamferForm, Document, EdgeGroup, Edit, Evaluation, Feature, FeatureId,
    FeatureKind, GroupResolution, Resolution, SolidResult, Transaction,
};
use caditor_expression::{Expression, Unit};
use caditor_kernel::{
    EdgeId, EdgeName, EdgeNaming, EdgeReference, FaceId, FaceReference, Solid, blend_chain,
};

use crate::{
    bodies::{self, FaceKey},
    body_selection,
    editing::{self, EditingCommand},
    last_values::{self, Remembered, Starts},
    model::{Action, Model, Notice},
    selection::{Pickable, Selection},
};

pub const DEFAULT_SIZE: f64 = 1.0;
fn size_slot(kind: BlendKind) -> Remembered {
    match kind {
        BlendKind::Fillet => Remembered::FilletSize,
        BlendKind::Chamfer => Remembered::ChamferSize,
    }
}

fn starting_form(kind: BlendKind, starts: &Starts) -> ChamferForm {
    if kind != BlendKind::Chamfer {
        return ChamferForm::Equal;
    }
    match starts.form(Remembered::ChamferForm) {
        Some(last_values::FORM_TWO_DISTANCES) => ChamferForm::TwoDistances {
            second: starts.length(Remembered::ChamferSecond, DEFAULT_SIZE),
        },
        Some(last_values::FORM_DISTANCE_ANGLE) => ChamferForm::DistanceAngle {
            angle: starts
                .remembered(Remembered::ChamferAngle)
                .unwrap_or_else(|| Expression::measure(DEFAULT_ANGLE_DEGREES, Unit::Degree)),
        },
        Some(_) | None => ChamferForm::Equal,
    }
}

const NO_SHAPE: &str = "The body has no shape yet; recompute the model, then try again";

pub const KINDS: [BlendKind; 2] = [BlendKind::Fillet, BlendKind::Chamfer];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupSource {
    Face(FaceKey),
    Body,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeSource {
    pub body: FeatureId,
    pub edges: Vec<EdgeName>,
    pub picked: Vec<EdgeName>,
    pub groups: Vec<GroupSource>,
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
    let edges = edge_names(shown, &GroupResolution::Body.edges(&shown.solid));
    if edges.is_empty() {
        return Err(NO_EDGES);
    }
    Ok(EdgeSource {
        body,
        edges,
        picked: Vec::new(),
        groups: vec![GroupSource::Body],
        left_out: Vec::new(),
    })
}

fn edges_of_selection(model: &Model, selection: &Selection) -> Result<EdgeSource, &'static str> {
    let mut body = None;
    let mut around = Vec::new();
    let mut picked = Vec::new();
    let mut groups = Vec::new();
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
                let resolution = GroupResolution::Face(Resolution::One(found));
                around.extend(edge_names(shown, &resolution.edges(&shown.solid)));
                if !groups.contains(&GroupSource::Face(face)) {
                    groups.push(GroupSource::Face(face));
                }
            }
            Pickable::Edge { edge, .. } => picked.push(edge),
            _ => {}
        }
    }
    let body = body.ok_or(NOTHING_TO_BLEND)?;
    let mut seen: BTreeSet<EdgeName> = around.iter().copied().collect();
    picked.retain(|edge| seen.insert(*edge));
    let mut listed = BTreeSet::new();
    let edges: Vec<EdgeName> = around
        .into_iter()
        .chain(picked.iter().copied())
        .filter(|edge| listed.insert(*edge))
        .collect();
    if edges.is_empty() {
        return Err(NO_EDGES);
    }
    Ok(EdgeSource {
        body,
        edges,
        picked,
        groups,
        left_out,
    })
}

fn edge_names(shown: &SolidResult, edges: &[EdgeId]) -> Vec<EdgeName> {
    edges
        .iter()
        .filter_map(|edge| shown.solid.edge(*edge))
        .map(|edge| edge.name())
        .collect()
}

pub fn create(
    document: &Document,
    evaluation: &Evaluation,
    kind: BlendKind,
    source: &EdgeSource,
    starts: &Starts,
) -> Result<(Transaction, FeatureId), &'static str> {
    let shown = bodies::shown(evaluation, source.body).ok_or(NO_SHAPE)?;
    let naming = EdgeNaming::new(&shown.solid);
    let edges: Vec<EdgeReference> = source
        .picked
        .iter()
        .filter_map(|name| {
            let edge = bodies::find_edge(shown, *name)?;
            EdgeReference::capture_in(&naming, edge)
        })
        .collect();
    let groups: Vec<EdgeGroup> = source
        .groups
        .iter()
        .filter_map(|group| match group {
            GroupSource::Face(key) => {
                FaceReference::capture(&shown.solid, bodies::find_face(shown, *key)?)
                    .map(EdgeGroup::Face)
            }
            GroupSource::Body => Some(EdgeGroup::Body),
        })
        .collect();
    if edges.is_empty() && groups.is_empty() {
        return Err("The selected edges are no longer part of the model");
    }
    if edges.len() < source.picked.len() {
        return Err("Some of the selected edges are no longer part of the model");
    }
    if groups.len() < source.groups.len() {
        return Err(FACE_GONE);
    }
    let name = editing::next_feature_name(document, kind.title());
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(
        name,
        FeatureKind::Blend(Blend {
            kind,
            body: source.body,
            edges,
            groups,
            size: starts.length(size_slot(kind), DEFAULT_SIZE),
            form: starting_form(kind, starts),
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
    starts: &Starts,
) -> Vec<Action> {
    match create(document, evaluation, kind, source, starts) {
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
    let mut seen = BTreeSet::new();
    let explicit: Vec<EdgeId> = blend
        .entry_edges(solid)
        .into_iter()
        .flatten()
        .filter(|edge| seen.insert(*edge))
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
    let naming = EdgeNaming::new(solid);
    let reaches = |found: &[EdgeId]| blend_chain(solid, found).contains(&clicked);
    let entries = blend.entry_edges(solid);
    let (edge_entries, group_entries) = entries.split_at(blend.edges.len().min(entries.len()));
    let mut changed = blend.clone();
    changed.edges = blend
        .edges
        .iter()
        .zip(edge_entries)
        .filter(|(_, found)| !reaches(found))
        .map(|(reference, _)| *reference)
        .collect();
    let mut opened = Vec::new();
    changed.groups = blend
        .groups
        .iter()
        .zip(group_entries)
        .filter_map(|(group, found)| {
            if reaches(found) {
                opened.extend(found.iter().copied());
                None
            } else {
                Some(group.clone())
            }
        })
        .collect();
    let left_out = changed.entry_count() < blend.entry_count();
    let label = if left_out {
        let mut taken: BTreeSet<EdgeId> =
            changed.entry_edges(solid).into_iter().flatten().collect();
        for found in opened {
            if !reaches(&[found]) && taken.insert(found) {
                changed
                    .edges
                    .push(EdgeReference::capture_in(&naming, found)?);
            }
        }
        format!("Leave an edge out of {}", owner.name)
    } else {
        changed
            .edges
            .push(EdgeReference::capture_in(&naming, clicked)?);
        format!("Add an edge to {}", owner.name)
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
        .entry_edges(solid)
        .iter()
        .flat_map(|found| blend_chain(solid, found))
        .collect();
    let mut grouped: BTreeSet<FaceId> = blend
        .group_resolutions(solid)
        .iter()
        .filter_map(|resolution| match resolution {
            GroupResolution::Face(face) => Some(face.found().to_vec()),
            GroupResolution::Body => None,
        })
        .flatten()
        .collect();
    let mut changed = blend.clone();
    let mut picked = Vec::new();
    for pickable in selection.iter() {
        if pickable.body() != Some(blend.body) {
            continue;
        }
        match pickable {
            Pickable::Edge { edge, .. } => picked.extend(bodies::find_edge(input, edge)),
            Pickable::Face { face, .. } => {
                let Some(found) = bodies::find_face(input, face) else {
                    continue;
                };
                if !grouped.insert(found) {
                    continue;
                }
                let around = GroupResolution::Face(Resolution::One(found)).edges(solid);
                if around.iter().all(|edge| taken.contains(edge)) {
                    continue;
                }
                changed
                    .groups
                    .push(EdgeGroup::Face(FaceReference::capture(solid, found)?));
                taken.extend(blend_chain(solid, &around));
            }
            _ => {}
        }
    }
    for found in picked {
        if taken.contains(&found) {
            continue;
        }
        changed
            .edges
            .push(EdgeReference::capture_in(&naming, found)?);
        taken.extend(blend_chain(solid, &[found]));
    }
    (changed.entry_count() > blend.entry_count()).then(|| {
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
    use crate::{samples::Sample, units::LengthUnit};

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
                    picked: edges.clone(),
                    edges,
                    groups: Vec::new(),
                    left_out: Vec::new(),
                },
                &Starts::defaults(LengthUnit::Millimetre),
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
