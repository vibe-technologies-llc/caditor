use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet},
    panic::{self, AssertUnwindSafe},
    sync::Arc,
};

use caditor_expression::{EvalError, ParameterId, Quantity};
use caditor_sketch::{ConstraintId, DimensionError, Sketch, SketchError, SketchSolution};

use crate::{
    document::{Document, Feature, FeatureId, FeatureKind},
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureError {
    pub reason: String,
    pub remedy: String,
    pub fix: Option<FixTarget>,
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
}

pub struct Inputs<'a> {
    pub document: &'a Document,
    pub parameters: &'a ParameterValues,
    pub features: &'a BTreeMap<FeatureId, Arc<FeatureResult>>,
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
}

impl Evaluation {
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
        let mut recomputed = Vec::new();
        let mut cancelled = false;

        for (index, feature) in features.iter().enumerate() {
            progress(index, features.len());
            let id = feature.id();
            let parameter_fingerprint = parameters.fingerprint(&feature.kind.parameters());
            let upstream: Vec<(FeatureId, Option<Arc<FeatureResult>>)> = feature
                .kind
                .features()
                .into_iter()
                .map(|used| (used, current.get(&used).cloned()))
                .collect();
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
        });
    };
    Some(FeatureError {
        reason: format!("It uses {name}, which has an error."),
        remedy: format!("Fix {name} first."),
        fix: Some(FixTarget::Feature(*missing)),
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
        _cancel: &CancelToken,
    ) -> Result<FeatureResult, Failure> {
        match &feature.kind {
            FeatureKind::Sketch(sketch) => sketch
                .evaluate(&|id| inputs.parameters.value(id))
                .map(|solution| {
                    FeatureResult::Sketch(SketchResult {
                        geometry: sketch.clone(),
                        solution,
                    })
                })
                .map_err(|error| Failure::Error(sketch_error(feature.id(), sketch, &error))),
        }
    }
}

fn sketch_error(feature: FeatureId, sketch: &Sketch, error: &SketchError) -> FeatureError {
    let SketchError::Dimension { constraint, reason } = error else {
        return FeatureError {
            reason: format!("The sketch could not be evaluated: {error}."),
            remedy: "Undo the last change.".to_owned(),
            fix: None,
        };
    };
    let dimension = FixTarget::Dimension {
        feature,
        constraint: *constraint,
    };
    let (remedy, fix) = match reason {
        DimensionError::Evaluation(EvalError::ParameterFailed { id, name }) => (
            format!("Fix {name} under Parameters, or edit this dimension."),
            FixTarget::Parameter(*id),
        ),
        DimensionError::Evaluation(EvalError::WrongKind { .. }) => (
            "Edit the dimension so it gives a length, such as 10 mm.".to_owned(),
            dimension,
        ),
        DimensionError::Negative => (
            "Edit the dimension so it gives zero or more.".to_owned(),
            dimension,
        ),
        DimensionError::Evaluation(_) => (
            "Edit the dimension or the parameters it uses.".to_owned(),
            dimension,
        ),
    };
    FeatureError {
        reason: format!(
            "{} cannot be evaluated: {reason}.",
            sketch.describe_constraint(*constraint)
        ),
        remedy,
        fix: Some(fix),
    }
}
