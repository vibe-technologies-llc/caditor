use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet},
    panic::{self, AssertUnwindSafe},
    sync::Arc,
};

use caditor_expression::{Dimension, EvalError, ParameterId, Quantity};
use caditor_kernel::Solid;
use caditor_sketch::{ConstraintId, DimensionError, Sketch, SketchError, SketchSolution, Solved};

use crate::{
    document::{Document, Feature, FeatureId, FeatureKind, list_names},
    solid::{self, SolidResult},
    values::ParameterValues,
};

#[derive(Clone)]
pub struct CancelToken(Arc<dyn Fn() -> bool + Send + Sync>);

impl CancelToken {
    pub fn new(is_cancelled: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        Self(Arc::new(is_cancelled))
    }

    pub fn never() -> Self {
        Self::new(|| false)
    }

    pub fn is_cancelled(&self) -> bool {
        (self.0)()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FixTarget {
    Parameter(ParameterId),
    Dimension {
        feature: FeatureId,
        constraint: ConstraintId,
    },
    Feature(FeatureId),
    Constraint {
        feature: FeatureId,
        constraint: ConstraintId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureError {
    pub reason: String,
    pub remedy: String,
    pub fix: Option<FixTarget>,
    pub constraints: Vec<ConstraintId>,
}

pub enum Failure {
    Error(FeatureError),
    Cancelled,
}

impl From<FeatureError> for Failure {
    fn from(error: FeatureError) -> Self {
        Self::Error(error)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SketchResult {
    pub geometry: Sketch,
    pub solution: SketchSolution,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FeatureResult {
    Sketch(SketchResult),
    Solid(SolidResult),
}

impl FeatureResult {
    pub fn sketch(&self) -> Option<&SketchResult> {
        match self {
            Self::Sketch(sketch) => Some(sketch),
            Self::Solid(_) => None,
        }
    }

    pub fn solid(&self) -> Option<&SolidResult> {
        match self {
            Self::Sketch(_) => None,
            Self::Solid(solid) => Some(solid),
        }
    }
}

type BodyStates = BTreeMap<FeatureId, (FeatureId, Arc<FeatureResult>)>;

pub struct Inputs<'a> {
    pub document: &'a Document,
    pub parameters: &'a ParameterValues,
    pub features: &'a BTreeMap<FeatureId, Arc<FeatureResult>>,
    pub bodies: &'a BTreeMap<FeatureId, (FeatureId, Arc<FeatureResult>)>,
}

impl Inputs<'_> {
    pub fn body(&self, body: FeatureId) -> Option<&Solid> {
        let (_, result) = self.bodies.get(&body)?;
        result.solid().map(|result| &result.solid)
    }
}

pub trait Evaluator: Send + 'static {
    fn evaluate(
        &self,
        feature: &Feature,
        inputs: &Inputs<'_>,
        cancel: &CancelToken,
    ) -> Result<FeatureResult, Failure>;
}

#[derive(Debug, Clone, PartialEq)]
pub enum FeatureState {
    UpToDate,
    Failed(FeatureError),
    Outdated,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FeatureStatus {
    pub state: FeatureState,
    pub result: Option<Arc<FeatureResult>>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Evaluation {
    pub parameters: ParameterValues,
    features: BTreeMap<FeatureId, FeatureStatus>,
    recomputed: Vec<FeatureId>,
    bodies: BTreeMap<FeatureId, FeatureId>,
}

impl Evaluation {
    pub fn bodies(&self) -> impl Iterator<Item = (FeatureId, FeatureId)> + '_ {
        self.bodies.iter().map(|(body, state)| (*body, *state))
    }

    pub fn body(&self, body: FeatureId) -> Option<&Solid> {
        let state = self.bodies.get(&body)?;
        let result = self.features.get(state)?.result.as_deref()?;
        result.solid().map(|result| &result.solid)
    }

    pub fn feature(&self, id: FeatureId) -> Option<&FeatureStatus> {
        self.features.get(&id)
    }

    pub fn recomputed(&self) -> &[FeatureId] {
        &self.recomputed
    }

    pub fn failed_count(&self) -> usize {
        self.features
            .values()
            .filter(|status| matches!(status.state, FeatureState::Failed(_)))
            .count()
    }

    pub fn is_complete(&self) -> bool {
        self.features
            .values()
            .all(|status| status.state != FeatureState::Outdated)
    }
}

type ParameterFingerprint = Vec<(ParameterId, String, Option<Quantity>)>;

#[derive(Debug, Clone)]
struct CacheEntry {
    definition: Arc<Feature>,
    parameters: ParameterFingerprint,
    upstream: Vec<(FeatureId, Option<Arc<FeatureResult>>)>,
    state: FeatureState,
    result: Option<Arc<FeatureResult>>,
}

impl CacheEntry {
    fn matches(
        &self,
        definition: &Arc<Feature>,
        parameters: &ParameterFingerprint,
        upstream: &[(FeatureId, Option<Arc<FeatureResult>>)],
    ) -> bool {
        let same_definition =
            Arc::ptr_eq(&self.definition, definition) || *self.definition == **definition;
        let same_upstream = self.upstream.len() == upstream.len()
            && self.upstream.iter().zip(upstream).all(
                |((id, result), (other_id, other_result))| {
                    id == other_id
                        && match (result, other_result) {
                            (Some(result), Some(other)) => Arc::ptr_eq(result, other),
                            (None, None) => true,
                            (Some(_), None) | (None, Some(_)) => false,
                        }
                },
            );
        same_definition && self.parameters == *parameters && same_upstream
    }
}

#[derive(Debug, Clone, Default)]
pub struct Recompute {
    cache: BTreeMap<FeatureId, CacheEntry>,
}

impl Recompute {
    pub fn run(
        &mut self,
        document: &Document,
        evaluator: &dyn Evaluator,
        cancel: &CancelToken,
        progress: &dyn Fn(usize, usize),
    ) -> Evaluation {
        let parameters = ParameterValues::evaluate(document);
        let features = document.feature_handles();
        let mut statuses = BTreeMap::new();
        let mut current: BTreeMap<FeatureId, Arc<FeatureResult>> = BTreeMap::new();
        let mut bodies: BodyStates = BTreeMap::new();
        let mut recomputed = Vec::new();
        let mut cancelled = false;

        for (index, feature) in features.iter().enumerate() {
            progress(index, features.len());
            let id = feature.id();
            let parameter_fingerprint = parameters.fingerprint(&feature.kind.parameters());
            let mut upstream: Vec<(FeatureId, Option<Arc<FeatureResult>>)> = feature
                .kind
                .features()
                .into_iter()
                .map(|used| (used, current.get(&used).cloned()))
                .collect();
            let changed_body = feature
                .kind
                .solid()
                .and_then(|solid| solid.operation().target());
            if let Some((state, result)) = changed_body.and_then(|body| bodies.get(&body)) {
                upstream.push((*state, Some(Arc::clone(result))));
            }
            let previous = self.cache.get(&id);

            let reusable = previous
                .filter(|entry| entry.matches(feature, &parameter_fingerprint, &upstream))
                .cloned();
            let entry = if let Some(entry) = reusable {
                entry
            } else if cancelled || cancel.is_cancelled() {
                cancelled = true;
                statuses.insert(
                    id,
                    FeatureStatus {
                        state: FeatureState::Outdated,
                        result: previous.and_then(|entry| entry.result.clone()),
                    },
                );
                continue;
            } else {
                let last_good = previous.and_then(|entry| entry.result.clone());
                let outcome = match missing_upstream(document, &upstream) {
                    Some(error) => Err(Failure::Error(error)),
                    None => evaluate_contained(
                        evaluator,
                        feature,
                        &Inputs {
                            document,
                            parameters: &parameters,
                            features: &current,
                            bodies: &bodies,
                        },
                        cancel,
                    ),
                };
                let (state, result) = match outcome {
                    Ok(result) => (FeatureState::UpToDate, Some(Arc::new(result))),
                    Err(Failure::Error(error)) => (FeatureState::Failed(error), last_good),
                    Err(Failure::Cancelled) => {
                        cancelled = true;
                        statuses.insert(
                            id,
                            FeatureStatus {
                                state: FeatureState::Outdated,
                                result: last_good,
                            },
                        );
                        continue;
                    }
                };
                recomputed.push(id);
                let entry = CacheEntry {
                    definition: Arc::clone(feature),
                    parameters: parameter_fingerprint,
                    upstream,
                    state,
                    result,
                };
                self.cache.insert(id, entry.clone());
                entry
            };

            if let (FeatureState::UpToDate, Some(result)) = (&entry.state, &entry.result) {
                current.insert(id, Arc::clone(result));
                if let Some(solid) = result.solid() {
                    bodies.insert(solid.body, (id, Arc::clone(result)));
                }
            }
            statuses.insert(
                id,
                FeatureStatus {
                    state: entry.state,
                    result: entry.result,
                },
            );
        }
        progress(features.len(), features.len());

        let alive: BTreeSet<FeatureId> = features.iter().map(|feature| feature.id()).collect();
        self.cache.retain(|id, _| alive.contains(id));
        Evaluation {
            parameters,
            features: statuses,
            recomputed,
            bodies: bodies
                .into_iter()
                .map(|(body, (state, _))| (body, state))
                .collect(),
        }
    }
}

fn missing_upstream(
    document: &Document,
    upstream: &[(FeatureId, Option<Arc<FeatureResult>>)],
) -> Option<FeatureError> {
    let (missing, _) = upstream.iter().find(|(_, result)| result.is_none())?;
    let Some(name) = document
        .feature(*missing)
        .map(|feature| feature.name.clone())
    else {
        return Some(FeatureError {
            reason: "It uses a feature that no longer exists.".to_owned(),
            remedy: "Edit it so it no longer uses the missing feature.".to_owned(),
            fix: None,
            constraints: Vec::new(),
        });
    };
    Some(FeatureError {
        reason: format!("It uses {name}, which has an error."),
        remedy: format!("Fix {name} first."),
        fix: Some(FixTarget::Feature(*missing)),
        constraints: Vec::new(),
    })
}

fn evaluate_contained(
    evaluator: &dyn Evaluator,
    feature: &Feature,
    inputs: &Inputs<'_>,
    cancel: &CancelToken,
) -> Result<FeatureResult, Failure> {
    panic::catch_unwind(AssertUnwindSafe(|| {
        evaluator.evaluate(feature, inputs, cancel)
    }))
    .unwrap_or_else(|payload| {
        log::error!(
            "recomputing {} panicked: {}",
            feature.name,
            panic_message(payload.as_ref())
        );
        Err(Failure::Error(FeatureError {
            reason: "caditor ran into an internal error while recomputing this feature.".to_owned(),
            remedy:
                "Your model is unchanged. Undo the last change, and please report this problem."
                    .to_owned(),
            fix: None,
            constraints: Vec::new(),
        }))
    })
}

fn panic_message(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("unknown panic")
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ModelEvaluator;

impl Evaluator for ModelEvaluator {
    fn evaluate(
        &self,
        feature: &Feature,
        inputs: &Inputs<'_>,
        cancel: &CancelToken,
    ) -> Result<FeatureResult, Failure> {
        match &feature.kind {
            FeatureKind::Sketch(sketch) => {
                let solved =
                    sketch.solve(&|id| inputs.parameters.value(id), &|| cancel.is_cancelled());
                match solved {
                    Ok(Solved { geometry, solution }) => {
                        Ok(FeatureResult::Sketch(SketchResult { geometry, solution }))
                    }
                    Err(SketchError::Cancelled) => Err(Failure::Cancelled),
                    Err(error) => Err(Failure::Error(sketch_error(feature.id(), sketch, &error))),
                }
            }
            FeatureKind::Solid(solid) => solid::evaluate(feature, solid, inputs, cancel),
        }
    }
}

fn sketch_error(feature: FeatureId, sketch: &Sketch, error: &SketchError) -> FeatureError {
    match error {
        SketchError::Dimension { constraint, reason } => {
            dimension_error(feature, sketch, *constraint, reason)
        }
        SketchError::Conflict { constraints } => conflict_error(feature, sketch, constraints),
        SketchError::Unsolvable => FeatureError {
            reason: "The sketch could not be solved from its current shape.".to_owned(),
            remedy: "Undo the last change, or remove constraints until the sketch solves."
                .to_owned(),
            fix: None,
            constraints: Vec::new(),
        },
        _ => FeatureError {
            reason: format!("The sketch could not be evaluated: {error}."),
            remedy: "Undo the last change.".to_owned(),
            fix: None,
            constraints: Vec::new(),
        },
    }
}

fn conflict_error(
    feature: FeatureId,
    sketch: &Sketch,
    constraints: &[ConstraintId],
) -> FeatureError {
    let Some((newest, older)) = constraints.split_last() else {
        return sketch_error(feature, sketch, &SketchError::Unsolvable);
    };
    let newest_label = sketch.describe_constraint(*newest);
    let reason = if older.is_empty() {
        format!("{newest_label} cannot be satisfied.")
    } else {
        let older: Vec<String> = older
            .iter()
            .map(|constraint| sketch.describe_constraint(*constraint))
            .collect();
        format!("{newest_label} conflicts with {}.", list_names(&older))
    };
    let remedy = if older.is_empty() {
        "Delete or change this constraint, or undo the last change."
    } else {
        "Delete or change one of these constraints, or undo the last change."
    };
    FeatureError {
        reason,
        remedy: remedy.to_owned(),
        fix: Some(FixTarget::Constraint {
            feature,
            constraint: *newest,
        }),
        constraints: constraints.to_vec(),
    }
}

fn dimension_error(
    feature: FeatureId,
    sketch: &Sketch,
    constraint: ConstraintId,
    reason: &DimensionError,
) -> FeatureError {
    let dimension = FixTarget::Dimension {
        feature,
        constraint,
    };
    let example = match sketch
        .constraint(constraint)
        .and_then(|definition| definition.dimension_kind())
    {
        Some(kind) if kind == Dimension::ANGLE => "an angle, such as 30 deg",
        _ => "a length, such as 10 mm",
    };
    let (remedy, fix) = match reason {
        DimensionError::Evaluation(EvalError::ParameterFailed { id, name }) => (
            format!("Fix {name} under Parameters, or edit this dimension."),
            FixTarget::Parameter(*id),
        ),
        DimensionError::Evaluation(EvalError::WrongKind { .. }) => (
            format!("Edit the dimension so it gives {example}."),
            dimension,
        ),
        DimensionError::Negative => (
            "Edit the dimension so it gives zero or more.".to_owned(),
            dimension,
        ),
        DimensionError::NotPositive => (
            "Edit the dimension so it gives more than zero.".to_owned(),
            dimension,
        ),
        DimensionError::Evaluation(_) | DimensionError::NotFinite => (
            "Edit the dimension or the parameters it uses.".to_owned(),
            dimension,
        ),
    };
    FeatureError {
        reason: format!(
            "{} cannot be evaluated: {reason}.",
            sketch.describe_constraint(constraint)
        ),
        remedy,
        fix: Some(fix),
        constraints: Vec::new(),
    }
}
