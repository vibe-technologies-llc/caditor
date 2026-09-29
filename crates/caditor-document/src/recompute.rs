use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet},
    panic::{self, AssertUnwindSafe},
    sync::{Arc, OnceLock},
};

use caditor_expression::{Dimension, EvalError, ParameterId, Quantity};
use caditor_kernel::{Interrupt, Profile, ProfileError, Solid, interruptible};
use caditor_sketch::{
    ConstraintId, DimensionError, EntityId, Sketch, SketchError, SketchSolution, SolveMemo, Solved,
};

use crate::{
    attachment, blend,
    datum::{self, DatumResult},
    document::{Document, Feature, FeatureId, FeatureKind, list_names},
    import, shell,
    solid::{self, SketchRegion, SolidFeature, SolidResult},
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

    pub fn interrupt(&self) -> Interrupt {
        Arc::clone(&self.0)
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

#[derive(Debug, Clone)]
pub struct SketchResult {
    pub geometry: Sketch,
    pub solution: SketchSolution,
    memo: SolveMemo,
    profile: OnceLock<Box<Result<Profile, ProfileError>>>,
    regions: OnceLock<Result<Vec<SketchRegion>, ProfileError>>,
}

impl PartialEq for SketchResult {
    fn eq(&self, other: &Self) -> bool {
        self.geometry == other.geometry
            && self.solution == other.solution
            && self.regions == other.regions
    }
}

impl SketchResult {
    pub fn new(geometry: Sketch, solution: SketchSolution) -> Self {
        Self::remembering(geometry, solution, SolveMemo::default())
    }

    fn remembering(geometry: Sketch, solution: SketchSolution, memo: SolveMemo) -> Self {
        Self {
            geometry,
            solution,
            memo,
            profile: OnceLock::new(),
            regions: OnceLock::new(),
        }
    }

    pub fn memo(&self) -> &SolveMemo {
        &self.memo
    }

    pub fn regions(&self) -> Option<&Result<Vec<SketchRegion>, ProfileError>> {
        self.regions.get()
    }

    pub(crate) fn profile(&self) -> Result<&Profile, ProfileError> {
        if let Some(built) = self.profile.get() {
            return built.as_ref().as_ref().map_err(Clone::clone);
        }
        let built = panic::catch_unwind(AssertUnwindSafe(|| {
            Profile::new(&solid::profile_curves(&self.geometry))
        }))
        .unwrap_or_else(|_| {
            log::error!("dividing a sketch into regions panicked");
            Err(ProfileError::unresolved())
        });
        if let Err(cancelled @ ProfileError::Cancelled(_)) = built {
            return Err(cancelled);
        }
        self.profile
            .get_or_init(|| Box::new(built))
            .as_ref()
            .as_ref()
            .map_err(Clone::clone)
    }

    pub(crate) fn find_regions(&self) {
        if self.regions.get().is_some() {
            return;
        }
        let found = self.profile().and_then(|profile| {
            panic::catch_unwind(AssertUnwindSafe(|| solid::display_regions(profile)))
                .unwrap_or_else(|_| {
                    log::error!("triangulating the regions of a sketch panicked");
                    Err(ProfileError::unresolved())
                })
        });
        if !matches!(found, Err(ProfileError::Cancelled(_))) {
            self.regions.get_or_init(|| found);
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FeatureResult {
    Sketch(SketchResult),
    Solid(SolidResult),
    Datum(DatumResult),
}

impl FeatureResult {
    fn same_for_dependents(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Sketch(own), Self::Sketch(theirs)) => {
                own.geometry.same_geometry(&theirs.geometry)
            }
            (Self::Datum(own), Self::Datum(theirs)) => own == theirs,
            (Self::Solid(own), Self::Solid(theirs)) => {
                own.body == theirs.body && own.solid == theirs.solid
            }
            _ => false,
        }
    }

    pub fn sketch(&self) -> Option<&SketchResult> {
        match self {
            Self::Sketch(sketch) => Some(sketch),
            Self::Solid(_) | Self::Datum(_) => None,
        }
    }

    pub fn solid(&self) -> Option<&SolidResult> {
        match self {
            Self::Solid(solid) => Some(solid),
            Self::Sketch(_) | Self::Datum(_) => None,
        }
    }

    pub fn datum(&self) -> Option<&DatumResult> {
        match self {
            Self::Datum(datum) => Some(datum),
            Self::Sketch(_) | Self::Solid(_) => None,
        }
    }
}

type BodyStates = BTreeMap<FeatureId, (FeatureId, Arc<FeatureResult>)>;

pub struct Inputs<'a> {
    pub document: &'a Document,
    pub parameters: &'a ParameterValues,
    pub features: &'a BTreeMap<FeatureId, Arc<FeatureResult>>,
    pub bodies: &'a BTreeMap<FeatureId, (FeatureId, Arc<FeatureResult>)>,
    pub previous: Option<&'a FeatureResult>,
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
    stale_bodies: BTreeSet<FeatureId>,
    inputs_before: BTreeMap<FeatureId, FeatureId>,
    seen_bodies: BTreeMap<FeatureId, BTreeMap<FeatureId, FeatureId>>,
    meshed: bool,
}

impl Evaluation {
    pub fn body_before(&self, feature: FeatureId) -> Option<&Arc<FeatureResult>> {
        let state = self.inputs_before.get(&feature)?;
        self.features.get(state)?.result.as_ref()
    }

    pub fn bodies(&self) -> impl Iterator<Item = (FeatureId, FeatureId)> + '_ {
        self.bodies.iter().map(|(body, state)| (*body, *state))
    }

    pub fn body_seen_by(&self, feature: FeatureId, body: FeatureId) -> Option<&Solid> {
        let state = self.seen_bodies.get(&feature)?.get(&body)?;
        self.features
            .get(state)?
            .result
            .as_deref()?
            .solid()
            .map(|result| &result.solid)
    }

    pub fn is_stale(&self, body: FeatureId) -> bool {
        self.stale_bodies.contains(&body)
    }

    pub fn body(&self, body: FeatureId) -> Option<&Solid> {
        self.body_result(body)?.solid().map(|result| &result.solid)
    }

    pub fn body_result(&self, body: FeatureId) -> Option<&Arc<FeatureResult>> {
        let state = self.bodies.get(&body)?;
        self.features.get(state)?.result.as_ref()
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
        self.meshed
            && self
                .features
                .values()
                .all(|status| status.state != FeatureState::Outdated)
    }
}

type ParameterFingerprint = Vec<(ParameterId, Option<Quantity>)>;

#[derive(Debug, Clone)]
struct Names {
    feature: String,
    features: Vec<String>,
    parameters: Vec<String>,
    tree: Arc<[String]>,
    earlier: usize,
}

impl PartialEq for Names {
    fn eq(&self, other: &Self) -> bool {
        self.feature == other.feature
            && self.features == other.features
            && self.parameters == other.parameters
            && self.tree.get(..self.earlier) == other.tree.get(..other.earlier)
    }
}

impl Names {
    fn of(
        document: &Document,
        (feature, earlier): (&Feature, usize),
        tree: &Arc<[String]>,
        parameters: &ParameterValues,
        used: &BTreeSet<ParameterId>,
    ) -> Self {
        Self {
            tree: Arc::clone(tree),
            earlier,
            feature: feature.name.clone(),
            features: feature
                .kind
                .features()
                .into_iter()
                .map(|used| {
                    document
                        .feature(used)
                        .map(|feature| feature.name.clone())
                        .unwrap_or_default()
                })
                .collect(),
            parameters: parameters.names(used),
        }
    }
}

#[derive(Debug, Clone)]
struct CacheEntry {
    definition: Arc<Feature>,
    parameters: ParameterFingerprint,
    names: Names,
    upstream: Vec<(FeatureId, Option<Arc<FeatureResult>>)>,
    state: FeatureState,
    result: Option<Arc<FeatureResult>>,
}

impl CacheEntry {
    fn matches(
        &self,
        definition: &Arc<Feature>,
        parameters: &ParameterFingerprint,
        names: &Names,
        upstream: &[(FeatureId, Option<Arc<FeatureResult>>)],
    ) -> bool {
        let same_definition = Arc::ptr_eq(&self.definition, definition)
            || (self.definition.id() == definition.id()
                && self.definition.kind.same_content(&definition.kind));
        let same_message = match self.state {
            FeatureState::Failed(_) => self.names == *names,
            FeatureState::UpToDate | FeatureState::Outdated => true,
        };
        let same_upstream = self.upstream.len() == upstream.len()
            && self.upstream.iter().zip(upstream).all(
                |((id, result), (other_id, other_result))| {
                    id == other_id
                        && match (result, other_result) {
                            (Some(result), Some(other)) => {
                                Arc::ptr_eq(result, other) || result.same_for_dependents(other)
                            }
                            (None, None) => true,
                            (Some(_), None) | (None, Some(_)) => false,
                        }
                },
            );
        same_definition && same_message && self.parameters == *parameters && same_upstream
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
        let tree: Arc<[String]> = features
            .iter()
            .map(|feature| feature.name.clone())
            .collect();
        let mut statuses = BTreeMap::new();
        let mut current: BTreeMap<FeatureId, Arc<FeatureResult>> = BTreeMap::new();
        let mut bodies: BodyStates = BTreeMap::new();
        let mut recomputed = Vec::new();
        let mut inputs_before = BTreeMap::new();
        let mut seen_bodies: BTreeMap<FeatureId, BTreeMap<FeatureId, FeatureId>> = BTreeMap::new();
        let mut cancelled = false;

        for (index, feature) in features.iter().enumerate() {
            progress(index, features.len());
            let id = feature.id();
            let used_parameters = feature.kind.parameters();
            let parameter_fingerprint = parameters.fingerprint(&used_parameters);
            let names = Names::of(
                document,
                (feature, index),
                &tree,
                &parameters,
                &used_parameters,
            );
            let mut upstream: Vec<(FeatureId, Option<Arc<FeatureResult>>)> = feature
                .kind
                .features()
                .into_iter()
                .map(|used| (used, current.get(&used).cloned()))
                .collect();
            for body in feature.kind.bodies_used() {
                if let Some((state, result)) = bodies.get(&body) {
                    upstream.push((*state, Some(Arc::clone(result))));
                    seen_bodies.entry(id).or_default().insert(body, *state);
                    if feature.kind.modifies_body() && feature.kind.body_input() == Some(body) {
                        inputs_before.insert(id, *state);
                    }
                }
            }
            let previous = self.cache.get(&id);

            let reusable = previous
                .filter(|entry| entry.matches(feature, &parameter_fingerprint, &names, &upstream))
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
                            previous: last_good.as_deref(),
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
                    names,
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
        let swept: BTreeSet<FeatureId> = features
            .iter()
            .filter_map(|feature| feature.kind.solid().map(SolidFeature::sketch))
            .collect();
        for sketch in &swept {
            if cancel.is_cancelled() {
                break;
            }
            if let Some(result) = statuses
                .get(sketch)
                .and_then(|status: &FeatureStatus| status.result.as_deref())
                .and_then(FeatureResult::sketch)
            {
                interruptible(cancel.interrupt(), || result.find_regions());
            }
        }
        let mut shown: BTreeMap<FeatureId, FeatureId> = bodies
            .iter()
            .map(|(body, (state, _))| (*body, *state))
            .collect();
        let stale_bodies = last_good_bodies(document, &statuses, &shown);
        for (body, state) in &stale_bodies {
            shown.insert(*body, *state);
        }
        for (body, state) in &shown {
            if cancel.is_cancelled() {
                break;
            }
            let meshable = statuses
                .get(state)
                .and_then(|status: &FeatureStatus| status.result.as_deref())
                .and_then(FeatureResult::solid);
            if let Some(solid) = meshable {
                let name = document
                    .feature(*body)
                    .map_or("a feature", |feature| feature.name.as_str());
                interruptible(cancel.interrupt(), || solid.tessellate(name));
            }
        }
        let meshed = shown.values().all(|state| {
            statuses
                .get(state)
                .and_then(|status| status.result.as_deref())
                .and_then(FeatureResult::solid)
                .is_none_or(SolidResult::is_meshed)
        });

        let alive: BTreeSet<FeatureId> = features.iter().map(|feature| feature.id()).collect();
        self.cache.retain(|id, _| alive.contains(id));
        Evaluation {
            parameters,
            features: statuses,
            recomputed,
            bodies: shown,
            stale_bodies: stale_bodies.into_keys().collect(),
            inputs_before,
            seen_bodies,
            meshed,
        }
    }
}

fn last_good_bodies(
    document: &Document,
    statuses: &BTreeMap<FeatureId, FeatureStatus>,
    current: &BTreeMap<FeatureId, FeatureId>,
) -> BTreeMap<FeatureId, FeatureId> {
    let made: BTreeSet<FeatureId> = document
        .features()
        .filter(|feature| feature.makes_body())
        .map(Feature::id)
        .collect();
    let mut stale = BTreeMap::new();
    for feature in document.features() {
        let last_good = statuses
            .get(&feature.id())
            .and_then(|status| status.result.as_deref())
            .and_then(FeatureResult::solid);
        if let Some(solid) = last_good
            && made.contains(&solid.body)
            && !current.contains_key(&solid.body)
        {
            stale.insert(solid.body, feature.id());
        }
    }
    stale
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

const _: () = assert!(
    cfg!(panic = "unwind"),
    "a panicking feature is contained by unwinding, so caditor must be built with panic = \"unwind\""
);

fn evaluate_contained(
    evaluator: &dyn Evaluator,
    feature: &Feature,
    inputs: &Inputs<'_>,
    cancel: &CancelToken,
) -> Result<FeatureResult, Failure> {
    let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
        interruptible(cancel.interrupt(), || {
            evaluator.evaluate(feature, inputs, cancel)
        })
    }));
    match outcome {
        Ok(Err(Failure::Error(_))) if cancel.is_cancelled() => Err(Failure::Cancelled),
        Ok(result) => result,
        Err(payload) => Err(panicked(feature, payload.as_ref())),
    }
}

fn panicked(feature: &Feature, payload: &(dyn Any + Send)) -> Failure {
    {
        log::error!(
            "recomputing {} panicked: {}",
            feature.name,
            panic_message(payload)
        );
        Failure::Error(FeatureError {
            reason: "caditor ran into an internal error while recomputing this feature.".to_owned(),
            remedy:
                "Your model is unchanged. Undo the last change, and please report this problem."
                    .to_owned(),
            fix: None,
            constraints: Vec::new(),
        })
    }
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
            FeatureKind::Sketch(definition) => {
                let plane = definition
                    .attachment
                    .as_ref()
                    .map(|attachment| attachment::attached_plane(feature, attachment, inputs))
                    .transpose()?;
                let sketch = &definition.sketch;
                let memo = inputs
                    .previous
                    .and_then(FeatureResult::sketch)
                    .map(|previous| &previous.memo);
                let solved = sketch.solve_from(
                    &|id| inputs.parameters.value(id),
                    &|| cancel.is_cancelled(),
                    &[],
                    memo,
                );
                match solved {
                    Ok(Solved {
                        mut geometry,
                        solution,
                        memo,
                    }) => {
                        if let Some(plane) = plane {
                            geometry.set_plane(plane);
                        }
                        Ok(FeatureResult::Sketch(SketchResult::remembering(
                            geometry, solution, memo,
                        )))
                    }
                    Err(SketchError::Cancelled) => Err(Failure::Cancelled),
                    Err(error) => Err(Failure::Error(sketch_error(feature.id(), sketch, &error))),
                }
            }
            FeatureKind::Solid(solid) => solid::evaluate(feature, solid, inputs, cancel),
            FeatureKind::Blend(definition) => blend::evaluate(feature, definition, inputs, cancel),
            FeatureKind::Shell(definition) => shell::evaluate(feature, definition, inputs, cancel),
            FeatureKind::Datum(definition) => datum::evaluate(feature, definition, inputs),
            FeatureKind::Import(definition) => import::evaluate(feature, definition),
        }
    }
}

fn sketch_error(feature: FeatureId, sketch: &Sketch, error: &SketchError) -> FeatureError {
    match error {
        SketchError::Dimension { constraint, reason } => {
            dimension_error(feature, sketch, *constraint, reason)
        }
        SketchError::Conflict { constraints } => conflict_error(feature, sketch, constraints),
        SketchError::NoLength { label, .. } => FeatureError {
            reason: format!("{label} has no length, so the sketch cannot be solved."),
            remedy: format!(
                "Delete {label}, or move its ends apart and remove any constraint that holds them \
                 together."
            ),
            fix: Some(FixTarget::Feature(feature)),
            constraints: Vec::new(),
        },
        SketchError::Unsolvable { entities, newest } => {
            unsolvable_error(feature, sketch, entities, *newest)
        }
        _ => FeatureError {
            reason: format!("The sketch could not be evaluated: {error}."),
            remedy: "Undo the last change.".to_owned(),
            fix: None,
            constraints: Vec::new(),
        },
    }
}

const UNSOLVED_NAMED: usize = 3;

pub(crate) fn unsolvable_error(
    feature: FeatureId,
    sketch: &Sketch,
    entities: &[EntityId],
    newest: Option<ConstraintId>,
) -> FeatureError {
    let mut names: Vec<String> = entities
        .iter()
        .take(UNSOLVED_NAMED)
        .map(|entity| sketch.entity_label(*entity))
        .collect();
    let unnamed = entities.len().saturating_sub(UNSOLVED_NAMED);
    if unnamed > 0 {
        names.push(format!("{unnamed} more"));
    }
    let (subject, possessive, them) = match entities.len() {
        0 => ("The sketch".to_owned(), "its", "it"),
        1 => (list_names(&names), "its", "it"),
        _ => (list_names(&names), "their", "them"),
    };
    let reason = format!("{subject} could not be solved from {possessive} current shape.");
    match newest {
        Some(constraint) => FeatureError {
            reason,
            remedy: format!(
                "Delete or change {}, the newest constraint on {them}, or undo the last change.",
                sketch.describe_constraint(constraint)
            ),
            fix: Some(FixTarget::Constraint {
                feature,
                constraint,
            }),
            constraints: Vec::new(),
        },
        None if entities.is_empty() => FeatureError {
            reason,
            remedy: "Undo the last change.".to_owned(),
            fix: Some(FixTarget::Feature(feature)),
            constraints: Vec::new(),
        },
        None => FeatureError {
            reason,
            remedy: format!("Delete and redraw {them}, or undo the last change."),
            fix: Some(FixTarget::Feature(feature)),
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
        return unsolvable_error(feature, sketch, &[], None);
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
        DimensionError::NotPositive | DimensionError::DiameterNotPositive => (
            "Edit the dimension so it gives more than zero.".to_owned(),
            dimension,
        ),
        DimensionError::TooLong => (
            "Edit the dimension so it gives a length the model can hold.".to_owned(),
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
