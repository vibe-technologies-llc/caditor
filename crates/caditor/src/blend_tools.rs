use std::{borrow::Cow, collections::BTreeSet};

use caditor_document::{
    Blend, BlendKind, ChamferForm, Document, EdgeGroup, EdgeRule, Edit, Evaluation, Feature,
    FeatureId, FeatureKind, GroupResolution, Resolution, SolidResult, Transaction, origin_feature,
};
use caditor_expression::{Expression, Unit};
use caditor_kernel::{
    EdgeId, EdgeName, EdgeNaming, EdgeReference, FaceId, FaceReference, Solid, blend_chain,
};

use crate::{
    blend_panel,
    bodies::{self, FaceKey},
    body_selection,
    commands::{Command, Reason},
    editing::{self, EditingCommand},
    field::{self, CheckError},
    last_values::{self, Remembered, Starts},
    model::{Action, Model, Notice},
    queued::{self, Pending},
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

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum BlendError {
    #[error("{}", NOTHING_TO_BLEND)]
    NothingToBlend,
    #[error("Select edges or faces of one body only")]
    SeveralBodies,
    #[error("{}", Pending::BodyShape)]
    NoShape { body: FeatureId },
    #[error("A selected face is no longer part of the model")]
    FaceGone,
    #[error("The selected faces or body have no edges to round")]
    NoEdges,
    #[error("The selected edges are no longer part of the model")]
    EdgesGone,
    #[error("Some of the selected edges are no longer part of the model")]
    SomeEdgesGone,
    #[error(
        "A selected edge does not lie between two different faces, so it could not be followed \
         when the model changes; choose an edge between two faces"
    )]
    EdgeNotNameable,
    #[error("The feature is not a fillet or a chamfer")]
    NotABlend { feature: FeatureId },
    #[error("{name} already takes {rule}")]
    RuleTaken { name: String, rule: String },
    #[error("{body} has no edges to match {rule}, so there is nothing to add")]
    RuleEmpty { body: String, rule: String },
    #[error("Select a face of the body made by the feature whose edges to add first")]
    NoFaceOfFeature,
    #[error("The feature is not a chamfer")]
    NotAChamfer { feature: FeatureId },
    #[error("{name} is already {form}")]
    AlreadyForm { name: String, form: String },
    #[error(
        "A chamfer by one distance is the same on both faces; choose two distances or an angle \
         first"
    )]
    NothingToFlip,
    #[error("The feature no longer exists")]
    Gone { feature: FeatureId },
    #[error(transparent)]
    Checked(#[from] CheckError),
}

impl Reason for BlendError {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }

    fn pending(&self) -> Option<Pending> {
        matches!(self, Self::NoShape { .. }).then_some(Pending::BodyShape)
    }
}

fn nameable_edges(
    naming: &EdgeNaming,
    located: &[EdgeId],
) -> Result<Vec<EdgeReference>, BlendError> {
    located
        .iter()
        .map(|edge| EdgeReference::capture_in(naming, *edge).ok_or(BlendError::EdgeNotNameable))
        .collect()
}

pub fn selected_edges(
    model: &Model,
    selection: &Selection,
    tree: &[FeatureId],
) -> Result<EdgeSource, BlendError> {
    match tree {
        [] => edges_of_selection(model, selection),
        [body] => edges_of_body(model, *body),
        [_, _, ..] => Err(BlendError::SeveralBodies),
    }
}

fn edges_of_body(model: &Model, body: FeatureId) -> Result<EdgeSource, BlendError> {
    let shown = bodies::shown(model.evaluation(), body).ok_or(BlendError::NoShape { body })?;
    let edges = edge_names(shown, &GroupResolution::Body.edges(&shown.solid));
    if edges.is_empty() {
        return Err(BlendError::NoEdges);
    }
    Ok(EdgeSource {
        body,
        edges,
        picked: Vec::new(),
        groups: vec![GroupSource::Body],
        left_out: Vec::new(),
    })
}

fn edges_of_selection(model: &Model, selection: &Selection) -> Result<EdgeSource, BlendError> {
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
            Some(known) if known != owner => return Err(BlendError::SeveralBodies),
            _ => body = Some(owner),
        }
        match pickable {
            Pickable::Face { face, .. } => {
                let shown = bodies::shown(model.evaluation(), owner)
                    .ok_or(BlendError::NoShape { body: owner })?;
                let found = bodies::find_face(shown, face).ok_or(BlendError::FaceGone)?;
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
    let body = body.ok_or(BlendError::NothingToBlend)?;
    let mut seen: BTreeSet<EdgeName> = around.iter().copied().collect();
    picked.retain(|edge| seen.insert(*edge));
    let mut listed = BTreeSet::new();
    let edges: Vec<EdgeName> = around
        .into_iter()
        .chain(picked.iter().copied())
        .filter(|edge| listed.insert(*edge))
        .collect();
    if edges.is_empty() {
        return Err(BlendError::NoEdges);
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
) -> Result<(Transaction, FeatureId), BlendError> {
    let shown =
        bodies::shown(evaluation, source.body).ok_or(BlendError::NoShape { body: source.body })?;
    let naming = EdgeNaming::new(&shown.solid);
    let located: Vec<EdgeId> = source
        .picked
        .iter()
        .filter_map(|name| bodies::find_edge(shown, *name))
        .collect();
    let edges = nameable_edges(&naming, &located)?;
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
    if located.is_empty() && groups.is_empty() {
        return Err(BlendError::EdgesGone);
    }
    if located.len() < source.picked.len() {
        return Err(BlendError::SomeEdgesGone);
    }
    if groups.len() < source.groups.len() {
        return Err(BlendError::FaceGone);
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
        Err(reason) => {
            let command = match kind {
                BlendKind::Fillet => Command::Fillet,
                BlendKind::Chamfer => Command::Chamfer,
            };
            vec![queued::refused(command, kind.title(), &reason)]
        }
    }
}

pub fn edit(document: &Document, feature: FeatureId, blend: Blend) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    let fields = blend_panel::captioned(&blend);
    let transaction = Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Blend(blend.clone()),
        },
    );
    Some(document.carrying_owners(transaction, feature, fields))
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

    pub fn applied_to(self, document: &Document, feature: FeatureId, blend: &Blend) -> Blend {
        let form = match (self, &blend.form) {
            (Self::Equal, _) => ChamferForm::Equal,
            (Self::TwoDistances, ChamferForm::TwoDistances { second }) => {
                ChamferForm::TwoDistances {
                    second: second.clone(),
                }
            }
            (Self::TwoDistances, _) => ChamferForm::TwoDistances {
                second: field::copied_value(document, feature, &blend.size),
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

fn chamfer_of(feature: &Feature) -> Result<&Blend, BlendError> {
    feature
        .kind
        .blend()
        .filter(|blend| blend.kind == BlendKind::Chamfer)
        .ok_or(BlendError::NotAChamfer {
            feature: feature.id(),
        })
}

pub fn form_change(
    document: &Document,
    feature: &Feature,
    choice: FormChoice,
) -> Result<Transaction, BlendError> {
    let blend = chamfer_of(feature)?;
    if FormChoice::of(&blend.form) == choice {
        return Err(BlendError::AlreadyForm {
            name: feature.name.clone(),
            form: choice.label().to_lowercase(),
        });
    }
    edit(
        document,
        feature.id(),
        choice.applied_to(document, feature.id(), blend),
    )
    .ok_or(BlendError::NotAChamfer {
        feature: feature.id(),
    })
}

pub fn flip_change(document: &Document, feature: &Feature) -> Result<Transaction, BlendError> {
    let blend = chamfer_of(feature)?;
    if blend.form.is_equal() {
        return Err(BlendError::NothingToFlip);
    }
    edit(
        document,
        feature.id(),
        Blend {
            flipped: !blend.flipped,
            ..blend.clone()
        },
    )
    .ok_or(BlendError::NotAChamfer {
        feature: feature.id(),
    })
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChosenEdges {
    pub explicit: BTreeSet<EdgeName>,
    pub followed: BTreeSet<EdgeName>,
}

pub fn chosen_edges(document: &Document, solid: &Solid, blend: &Blend) -> ChosenEdges {
    let name = |edge: &EdgeId| solid.edge(*edge).map(|edge| edge.name());
    let mut seen = BTreeSet::new();
    let explicit: Vec<EdgeId> = blend
        .entry_edges(document, solid)
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
    let entries = blend.entry_edges(document, solid);
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
        let mut taken: BTreeSet<EdgeId> = changed
            .entry_edges(document, solid)
            .into_iter()
            .flatten()
            .collect();
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
        .entry_edges(model.document(), solid)
        .iter()
        .flat_map(|found| blend_chain(solid, found))
        .collect();
    let mut grouped: BTreeSet<FaceId> = blend
        .group_resolutions(model.document(), solid)
        .iter()
        .filter_map(|resolution| match resolution {
            GroupResolution::Face(face) => Some(face.found().to_vec()),
            GroupResolution::Body | GroupResolution::Rule { .. } => None,
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

pub fn rule_words(document: &Document, rule: EdgeRule) -> String {
    match rule {
        EdgeRule::Convex => "every convex edge".to_owned(),
        EdgeRule::Concave => "every concave edge".to_owned(),
        EdgeRule::MadeBy(feature) => format!(
            "every edge made by {}",
            document
                .feature(feature)
                .map_or("a deleted feature", |made| made.name.as_str())
        ),
    }
}

pub fn rule_added(blend: &Blend, rule: EdgeRule) -> Blend {
    let mut groups = blend.groups.clone();
    groups.push(EdgeGroup::Rule(rule));
    Blend {
        groups,
        ..blend.clone()
    }
}

pub fn rule_taken(blend: &Blend, rule: EdgeRule) -> bool {
    blend.groups.contains(&EdgeGroup::Rule(rule))
}

pub fn rule_check(
    model: &Model,
    feature: &Feature,
    blend: &Blend,
    rule: EdgeRule,
) -> Result<(), BlendError> {
    let document = model.document();
    let words = || rule_words(document, rule);
    if rule_taken(blend, rule) {
        return Err(BlendError::RuleTaken {
            name: feature.name.clone(),
            rule: words(),
        });
    }
    let input = bodies::input(model.evaluation(), feature.id())
        .ok_or(BlendError::NoShape { body: blend.body })?;
    if rule.edges(document, &input.solid).is_empty() {
        return Err(BlendError::RuleEmpty {
            body: document
                .body_name(blend.body)
                .map_or_else(|| feature.name.clone(), Cow::into_owned),
            rule: words(),
        });
    }
    Ok(())
}

fn blend_of(feature: &Feature) -> Result<&Blend, BlendError> {
    feature.kind.blend().ok_or(BlendError::NotABlend {
        feature: feature.id(),
    })
}

pub fn add_rule(
    model: &Model,
    feature: &Feature,
    rules: &[EdgeRule],
) -> Result<Transaction, BlendError> {
    let blend = blend_of(feature)?;
    let mut changed = blend.clone();
    for rule in rules {
        rule_check(model, feature, &changed, *rule)?;
        changed = rule_added(&changed, *rule);
    }
    let document = model.document();
    let words: Vec<String> = rules
        .iter()
        .map(|rule| rule_words(document, *rule))
        .collect();
    Ok(Transaction::single(
        format!("Add {} to {}", words.join(" and "), feature.name),
        Edit::SetFeatureKind {
            id: feature.id(),
            kind: FeatureKind::Blend(changed),
        },
    ))
}

pub fn made_by_selected(
    model: &Model,
    selection: &Selection,
    feature: &Feature,
) -> Result<Transaction, BlendError> {
    let blend = blend_of(feature)?;
    let input = bodies::input(model.evaluation(), feature.id())
        .ok_or(BlendError::NoShape { body: blend.body })?;
    let mut makers = Vec::new();
    for pickable in selection.iter() {
        let Pickable::Face { body, face } = pickable else {
            continue;
        };
        if body != blend.body {
            continue;
        }
        let maker = bodies::find_face(input, face)
            .and_then(|found| input.solid.face(found)?.origin())
            .map(origin_feature);
        if let Some(maker) = maker
            && !makers.contains(&maker)
        {
            makers.push(maker);
        }
    }
    if makers.is_empty() {
        return Err(BlendError::NoFaceOfFeature);
    }
    let rules: Vec<EdgeRule> = makers.into_iter().map(EdgeRule::MadeBy).collect();
    add_rule(model, feature, &rules)
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
        assert_eq!(create(vec![present, gone]), Err(BlendError::SomeEdgesGone));
        assert_eq!(create(vec![gone]), Err(BlendError::EdgesGone));
    }

    #[test]
    fn an_edge_the_naming_cannot_capture_is_refused_in_words_that_are_true_of_it() {
        let recomputed = |sample: Sample| {
            let document = sample.document().unwrap();
            Recompute::default().run(
                &document,
                &ModelEvaluator,
                &CancelToken::never(),
                &|_, _| {},
            )
        };
        let plate = recomputed(Sample::Plate);
        let spool = recomputed(Sample::Spool);
        let solid_of = |evaluation: &Evaluation| {
            let (body, _) = evaluation.bodies().next().unwrap();
            evaluation.body(body).unwrap().clone()
        };
        let (plate, spool) = (solid_of(&plate), solid_of(&spool));
        let (larger, smaller) = if plate.edges().count() > spool.edges().count() {
            (plate, spool)
        } else {
            (spool, plate)
        };
        let naming = EdgeNaming::new(&smaller);
        let foreign: Vec<EdgeId> = larger
            .edges()
            .map(|(id, _)| id)
            .filter(|id| smaller.edge(*id).is_none())
            .collect();

        let refused = nameable_edges(&naming, &foreign);

        assert!(!foreign.is_empty());
        assert_eq!(refused, Err(BlendError::EdgeNotNameable));
        assert!(
            !BlendError::EdgeNotNameable
                .to_string()
                .contains("no longer")
        );
        assert_eq!(nameable_edges(&naming, &[]), Ok(Vec::new()));
    }
}
