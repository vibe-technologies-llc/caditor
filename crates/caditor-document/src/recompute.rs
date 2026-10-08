use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet},
    panic::{self, AssertUnwindSafe},
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

use caditor_expression::{Dimension, EvalError, ParameterId, Quantity};
use caditor_geometry::Point3;
use caditor_kernel::{Interrupt, MeshQuality, Profile, ProfileError, Solid, interruptible};
use caditor_sketch::{
    ConstraintId, DimensionError, EntityId, Sketch, SketchError, SketchSolution, SolveMemo, Solved,
};

use crate::{
    attachment, blend, combine,
    datum::{self, DatumResult},
    document::{Document, Feature, FeatureId, FeatureKind, list_names},
    healing::{self, Healing},
    hole, import, mirror, movement, pattern,
    presenting::{Glimpse, Presentation, SettledBody},
    projection, removal, scaling, shell,
    solid::{self, SketchRegion, SolidFeature, SolidResult, body_part, body_parts},
    split,
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
    Unsuppress(FeatureId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct FeatureError {
    pub reason: String,
    pub remedy: String,
    pub fix: Option<FixTarget>,
    pub constraints: Vec<ConstraintId>,
    pub place: Option<Point3>,
}

pub enum Failure {
    Error(Box<FeatureError>),
    Cancelled,
}

impl Failure {
    pub(crate) fn placed(self, place: Option<Point3>) -> Self {
        match self {
            Self::Error(error) => Self::Error(Box::new(FeatureError { place, ..*error })),
            Self::Cancelled => Self::Cancelled,
        }
    }
}

impl From<FeatureError> for Failure {
    fn from(error: FeatureError) -> Self {
        Self::Error(Box::new(error))
    }
}

#[derive(Debug, Clone)]
pub struct SketchResult {
    pub geometry: Sketch,
    pub solution: SketchSolution,
    pub open_ends: Vec<EntityId>,
    memo: SolveMemo,
    profile: Arc<OnceLock<Box<Result<Profile, ProfileError>>>>,
    regions: Arc<OnceLock<Result<Vec<SketchRegion>, ProfileError>>>,
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
            open_ends: geometry.open_ends(),
            geometry,
            solution,
            memo,
            profile: Arc::default(),
            regions: Arc::default(),
        }
    }

    fn sharing_display_with(mut self, previous: Option<&Self>) -> Self {
        if let Some(previous) = previous
            && previous.geometry.same_geometry(&self.geometry)
        {
            self.profile = Arc::clone(&previous.profile);
            self.regions = Arc::clone(&previous.regions);
        }
        self
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
            (Self::Solid(own), Self::Solid(theirs)) => own.same_shapes(theirs),
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

    pub fn missing_body(&self, body: FeatureId) -> Failure {
        let document = self.document;
        let name = |id: FeatureId| {
            document
                .feature(id)
                .map(|feature| feature.name.clone())
                .unwrap_or_default()
        };
        let body_name = name(body);
        let consumer = document.features().find(|feature| {
            self.features.contains_key(&feature.id())
                && feature.kind.consumed_bodies().contains(&body)
        });
        let error = match consumer {
            Some(consumer) if consumer.kind.combine().is_none() => FeatureError {
                reason: format!(
                    "{} removed the body made by {body_name}, so it no longer stands.",
                    consumer.name
                ),
                remedy: format!(
                    "Move this feature above {}, or suppress or delete {}.",
                    consumer.name, consumer.name
                ),
                fix: Some(FixTarget::Feature(consumer.id())),
                constraints: Vec::new(),
                place: None,
            },
            Some(consumer) => {
                let kept = consumer
                    .kind
                    .combine()
                    .map_or_else(String::new, |combine| name(combine.body));
                FeatureError {
                    reason: format!(
                        "{} combined the body made by {body_name} into the body of {kept}, so it no longer stands on its own.",
                        consumer.name
                    ),
                    remedy: format!(
                        "Use the body of {kept} instead, or move this feature above {}.",
                        consumer.name
                    ),
                    fix: Some(FixTarget::Feature(consumer.id())),
                    constraints: Vec::new(),
                    place: None,
                }
            }
            None => FeatureError {
                reason: format!("The body made by {body_name} has no shape."),
                remedy: format!("Fix {body_name} first."),
                fix: Some(FixTarget::Feature(body)),
                constraints: Vec::new(),
                place: None,
            },
        };
        Failure::Error(Box::new(error))
    }
}

pub trait Evaluator: Send + Sync + 'static {
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
    Suppressed,
    RolledBack,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FeatureStatus {
    pub state: FeatureState,
    pub result: Option<Arc<FeatureResult>>,
    pub healing: Option<Arc<Healing>>,
}

impl FeatureStatus {
    fn without_result(state: FeatureState) -> Self {
        Self {
            state,
            result: None,
            healing: None,
        }
    }

    fn outdated(last_good: Option<Arc<FeatureResult>>) -> Self {
        Self {
            state: FeatureState::Outdated,
            result: last_good,
            healing: None,
        }
    }
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
    pending: BTreeSet<FeatureId>,
    meshed: bool,
}

impl Evaluation {
    pub fn body_before(&self, feature: FeatureId) -> Option<&Arc<FeatureResult>> {
        let state = self.inputs_before.get(&feature)?;
        self.features.get(state)?.result.as_ref()
    }

    pub fn cuts(&self, feature: FeatureId) -> &[Arc<FeatureResult>] {
        self.features
            .get(&feature)
            .and_then(|status| status.result.as_deref())
            .and_then(FeatureResult::solid)
            .map_or(&[], SolidResult::cuts)
    }

    pub fn bodies(&self) -> impl Iterator<Item = (FeatureId, FeatureId)> + '_ {
        self.bodies.iter().map(|(body, state)| (*body, *state))
    }

    pub fn body_seen_by(&self, feature: FeatureId, body: FeatureId) -> Option<&Solid> {
        self.body_result_seen_by(feature, body)
            .map(|result| &result.solid)
    }

    pub fn body_result_seen_by(&self, feature: FeatureId, body: FeatureId) -> Option<&SolidResult> {
        let state = self.seen_bodies.get(&feature)?.get(&body)?;
        body_part(self.features.get(state)?.result.as_ref()?, body)?.solid()
    }

    pub fn is_stale(&self, body: FeatureId) -> bool {
        self.stale_bodies.contains(&body)
    }

    pub fn body(&self, body: FeatureId) -> Option<&Solid> {
        self.body_result(body)?.solid().map(|result| &result.solid)
    }

    pub fn body_result(&self, body: FeatureId) -> Option<&Arc<FeatureResult>> {
        let state = self.bodies.get(&body)?;
        body_part(self.features.get(state)?.result.as_ref()?, body)
    }

    pub fn feature(&self, id: FeatureId) -> Option<&FeatureStatus> {
        self.features.get(&id)
    }

    pub fn is_pending(&self, id: FeatureId) -> bool {
        self.pending.contains(&id)
    }

    pub(crate) fn outdating_pending(mut self) -> Self {
        for id in std::mem::take(&mut self.pending) {
            if let Some(status) = self.features.get_mut(&id) {
                status.state = FeatureState::Outdated;
                status.healing = None;
            } else {
                self.features.insert(id, FeatureStatus::outdated(None));
            }
        }
        self
    }

    pub fn recomputed(&self) -> &[FeatureId] {
        &self.recomputed
    }

    pub fn failures(&self) -> impl Iterator<Item = (FeatureId, &FeatureError)> {
        self.features
            .iter()
            .filter_map(|(id, status)| match &status.state {
                FeatureState::Failed(error) => Some((*id, error)),
                _ => None,
            })
    }

    pub fn failed_count(&self) -> usize {
        self.features
            .values()
            .filter(|status| matches!(status.state, FeatureState::Failed(_)))
            .count()
    }

    pub fn is_complete(&self) -> bool {
        self.meshed
            && self.pending.is_empty()
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
    suppressed_upstream: BTreeSet<FeatureId>,
    state: FeatureState,
    result: Option<Arc<FeatureResult>>,
    healing: Option<Arc<Healing>>,
    retry: bool,
}

impl CacheEntry {
    fn status(&self) -> FeatureStatus {
        FeatureStatus {
            state: self.state.clone(),
            result: self.result.clone(),
            healing: self.healing.clone(),
        }
    }

    fn matches(
        &self,
        definition: &Arc<Feature>,
        parameters: &ParameterFingerprint,
        names: &Names,
        upstream: &[(FeatureId, Option<Arc<FeatureResult>>)],
        suppressed_upstream: &BTreeSet<FeatureId>,
    ) -> bool {
        let same_definition = Arc::ptr_eq(&self.definition, definition)
            || (self.definition.id() == definition.id()
                && self.definition.kind.same_content(&definition.kind));
        let same_message = match self.state {
            FeatureState::Failed(_) => {
                self.names == *names && self.suppressed_upstream == *suppressed_upstream
            }
            FeatureState::UpToDate if self.healing.is_some() => self.names == *names,
            FeatureState::UpToDate
            | FeatureState::Outdated
            | FeatureState::Suppressed
            | FeatureState::RolledBack => true,
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
        !self.retry
            && same_definition
            && same_message
            && self.parameters == *parameters
            && same_upstream
    }
}

const FEATURES_DONE_AFTER: Duration = Duration::from_millis(250);
const MAX_UNSWEPT_REGION_ENTITIES: usize = 2_000;

#[derive(Debug, Clone)]
pub struct Recompute {
    cache: BTreeMap<FeatureId, CacheEntry>,
    mesh_quality: MeshQuality,
    features_done_after: Duration,
}

impl Default for Recompute {
    fn default() -> Self {
        Self::with_mesh_quality(MeshQuality::default())
    }
}

struct Reports<'a> {
    progress: &'a dyn Fn(usize, usize),
    features_done: &'a (dyn Fn(Evaluation) + Sync),
}

impl Recompute {
    pub fn with_mesh_quality(mesh_quality: MeshQuality) -> Self {
        Self {
            cache: BTreeMap::new(),
            mesh_quality,
            features_done_after: FEATURES_DONE_AFTER,
        }
    }

    #[cfg(test)]
    pub(crate) fn report_features_done_after(&mut self, delay: Duration) {
        self.features_done_after = delay;
    }

    pub(crate) fn draft_copy(&self) -> Self {
        Self {
            cache: self.cache.clone(),
            mesh_quality: self.mesh_quality,
            features_done_after: self.features_done_after,
        }
    }

    pub fn mesh_quality(&self) -> MeshQuality {
        self.mesh_quality
    }

    pub fn set_mesh_quality(&mut self, mesh_quality: MeshQuality) {
        if self.mesh_quality != mesh_quality {
            self.mesh_quality = mesh_quality;
            self.clear_cache();
        }
    }

    pub fn retry_failures(&mut self) {
        for entry in self.cache.values_mut() {
            if matches!(entry.state, FeatureState::Failed(_)) {
                entry.retry = true;
            }
        }
    }

    pub(crate) fn clear_cache(&mut self) {
        self.cache.clear();
    }

    pub fn run(
        &mut self,
        document: &Document,
        evaluator: &dyn Evaluator,
        cancel: &CancelToken,
        progress: &dyn Fn(usize, usize),
    ) -> Evaluation {
        self.run_reporting(document, evaluator, cancel, progress, &|_| {})
    }

    pub fn run_reporting(
        &mut self,
        document: &Document,
        evaluator: &dyn Evaluator,
        cancel: &CancelToken,
        progress: &dyn Fn(usize, usize),
        features_done: &(dyn Fn(Evaluation) + Sync),
    ) -> Evaluation {
        let reports = Reports {
            progress,
            features_done,
        };
        self.run_with(document, evaluator, cancel, &reports, Display::Prepared)
    }

    pub fn run_without_display(
        &mut self,
        document: &Document,
        evaluator: &dyn Evaluator,
        cancel: &CancelToken,
    ) -> Evaluation {
        let reports = Reports {
            progress: &|_, _| {},
            features_done: &|_| {},
        };
        self.run_with(document, evaluator, cancel, &reports, Display::Skipped)
    }

    fn run_with(
        &mut self,
        document: &Document,
        evaluator: &dyn Evaluator,
        cancel: &CancelToken,
        reports: &Reports<'_>,
        display: Display,
    ) -> Evaluation {
        let started = Instant::now();
        let shown_from = started
            .checked_add(self.features_done_after)
            .unwrap_or(started);
        let parameters = ParameterValues::evaluate(document);
        let run = Run {
            document,
            parameters: &parameters,
            evaluator,
            cancel,
            progress: reports.progress,
        };
        let walk = match display {
            Display::Prepared => Presentation {
                quality: self.mesh_quality,
                cancel,
                report: reports.features_done,
                from: shown_from,
            }
            .during(|glimpse| self.walk(&run, Some(glimpse))),
            Display::Skipped => self.walk(&run, None),
        };
        let mut evaluation = walk.into_evaluation(document, parameters, BTreeSet::new());
        if display == Display::Prepared {
            self.prepare_display(document, &mut evaluation, cancel, &|evaluation| {
                if !cancel.is_cancelled() && Instant::now() >= shown_from {
                    (reports.features_done)(evaluation.clone());
                }
            });
        }

        let alive: BTreeSet<FeatureId> = document.features().map(Feature::id).collect();
        self.cache.retain(|id, _| alive.contains(id));
        evaluation
    }

    fn walk(&mut self, run: &Run<'_>, glimpse: Option<&dyn Fn(Glimpse)>) -> Walk {
        let Run {
            document,
            parameters,
            evaluator,
            cancel,
            progress,
        } = *run;
        let features = document.feature_handles();
        let tree: Arc<[String]> = features
            .iter()
            .map(|feature| feature.name.clone())
            .collect();
        let bar = document.bar_index();
        let suppressed: BTreeSet<FeatureId> = document
            .features()
            .filter(|feature| feature.suppressed)
            .map(Feature::id)
            .collect();
        let settling = settling(features, bar);
        let mut walk = Walk::default();
        let mut cancelled = false;
        let mut glimpsed = Glimpsed::default();

        for (index, feature) in features.iter().enumerate() {
            progress(index, features.len());
            let id = feature.id();
            if let Some(state) = skipped(feature, index, bar) {
                walk.statuses
                    .insert(id, FeatureStatus::without_result(state));
                continue;
            }
            let used_parameters = feature.kind.parameters();
            let parameter_fingerprint = parameters.fingerprint(&used_parameters);
            let names = Names::of(
                document,
                (feature, index),
                &tree,
                parameters,
                &used_parameters,
            );
            let mut upstream: Vec<(FeatureId, Option<Arc<FeatureResult>>)> = feature
                .kind
                .features()
                .into_iter()
                .map(|used| (used, walk.current.get(&used).cloned()))
                .collect();
            if feature.kind.sketch().is_some() {
                let standing: Vec<(FeatureId, FeatureId)> = walk
                    .bodies
                    .iter()
                    .map(|(body, (state, _))| (*body, *state))
                    .collect();
                walk.seen_bodies.entry(id).or_default().extend(standing);
            }
            for body in feature.kind.bodies_used() {
                if let Some((state, result)) = walk.bodies.get(&body) {
                    upstream.push((*state, Some(Arc::clone(result))));
                    walk.seen_bodies.entry(id).or_default().insert(body, *state);
                    if feature.kind.modifies_body() && feature.kind.body_input() == Some(body) {
                        walk.inputs_before.insert(id, *state);
                    }
                }
            }
            let suppressed_upstream: BTreeSet<FeatureId> = upstream
                .iter()
                .map(|(used, _)| *used)
                .filter(|used| suppressed.contains(used))
                .collect();
            let previous = self.cache.get(&id);

            let reusable = previous
                .filter(|entry| {
                    entry.matches(
                        feature,
                        &parameter_fingerprint,
                        &names,
                        &upstream,
                        &suppressed_upstream,
                    )
                })
                .cloned();
            let entry = if let Some(entry) = reusable {
                entry
            } else if cancelled || cancel.is_cancelled() {
                cancelled = true;
                let last_good = previous.and_then(|entry| entry.result.clone());
                walk.statuses.insert(id, FeatureStatus::outdated(last_good));
                continue;
            } else {
                if let Some(show) = glimpse {
                    self.offer_glimpse(run, &walk, (&settling, index), &mut glimpsed, show);
                }
                let last_good = previous.and_then(|entry| entry.result.clone());
                let inputs = Inputs {
                    document,
                    parameters,
                    features: &walk.current,
                    bodies: &walk.bodies,
                    previous: last_good.as_deref(),
                };
                let outcome = match missing_upstream(document, feature, &upstream) {
                    Some(error) => Err(error.into()),
                    None => evaluate_contained(evaluator, feature, &inputs, cancel),
                };
                let (state, result, healing) = match outcome {
                    Ok(result) => (
                        FeatureState::UpToDate,
                        Some(Arc::new(result)),
                        check_healing(feature, &inputs),
                    ),
                    Err(Failure::Error(error)) => (FeatureState::Failed(*error), last_good, None),
                    Err(Failure::Cancelled) => {
                        cancelled = true;
                        walk.statuses.insert(id, FeatureStatus::outdated(last_good));
                        continue;
                    }
                };
                walk.recomputed.push(id);
                let entry = CacheEntry {
                    definition: Arc::clone(feature),
                    parameters: parameter_fingerprint,
                    names,
                    upstream,
                    suppressed_upstream,
                    state,
                    result,
                    healing,
                    retry: false,
                };
                self.cache.insert(id, entry.clone());
                entry
            };

            if let (FeatureState::UpToDate, Some(result)) = (&entry.state, &entry.result) {
                walk.stand(feature, result);
            }
            walk.statuses.insert(id, entry.status());
        }
        progress(features.len(), features.len());
        walk
    }

    fn offer_glimpse(
        &self,
        run: &Run<'_>,
        walk: &Walk,
        (settling, reached): (&Settling, usize),
        glimpsed: &mut Glimpsed,
        show: &dyn Fn(Glimpse),
    ) {
        let settled: Vec<SettledBody> = settling
            .range(glimpsed.settled_before..reached)
            .flat_map(|(_, bodies)| bodies)
            .filter_map(|body| {
                let (_, result) = walk.bodies.get(body)?;
                let unmeshed = result.solid().is_some_and(|solid| !solid.is_meshed());
                unmeshed.then(|| SettledBody {
                    name: body_name(run.document, *body).to_owned(),
                    result: Arc::clone(result),
                })
            })
            .collect();
        glimpsed.settled_before = reached;
        if walk.recomputed.len() == glimpsed.recomputed && settled.is_empty() {
            return;
        }
        glimpsed.recomputed = walk.recomputed.len();
        show(Glimpse {
            evaluation: self.glimpse(run, walk, reached),
            settled,
        });
    }

    fn glimpse(&self, run: &Run<'_>, walk: &Walk, reached: usize) -> Evaluation {
        let document = run.document;
        let bar = document.bar_index();
        let mut seen = walk.clone();
        let mut pending = BTreeSet::new();
        for (index, feature) in document.feature_handles().iter().enumerate().skip(reached) {
            let id = feature.id();
            if let Some(state) = skipped(feature, index, bar) {
                seen.statuses
                    .insert(id, FeatureStatus::without_result(state));
                continue;
            }
            pending.insert(id);
            let Some(entry) = self.cache.get(&id) else {
                continue;
            };
            if let (FeatureState::UpToDate, Some(result)) = (&entry.state, &entry.result) {
                seen.stand(feature, result);
            }
            seen.statuses.insert(id, entry.status());
        }
        seen.into_evaluation(document, run.parameters.clone(), pending)
    }

    fn prepare_display(
        &self,
        document: &Document,
        evaluation: &mut Evaluation,
        cancel: &CancelToken,
        report: &dyn Fn(&Evaluation),
    ) {
        report(evaluation);
        let swept: BTreeSet<FeatureId> = document
            .active_features()
            .filter_map(|feature| feature.kind.solid().map(SolidFeature::sketch))
            .collect();
        let small: BTreeSet<FeatureId> = document
            .active_features()
            .filter(|feature| {
                feature.kind.sketch().is_some_and(|sketch| {
                    sketch.entities().len() <= MAX_UNSWEPT_REGION_ENTITIES
                })
            })
            .map(Feature::id)
            .collect();
        for sketch in swept.union(&small) {
            if cancel.is_cancelled() {
                break;
            }
            if let Some(result) = evaluation
                .features
                .get(sketch)
                .and_then(|status| status.result.as_deref())
                .and_then(FeatureResult::sketch)
            {
                interruptible(cancel.interrupt(), || result.find_regions());
            }
        }
        for body in evaluation.bodies.keys() {
            if cancel.is_cancelled() {
                break;
            }
            let Some(solid) = evaluation
                .body_result(*body)
                .and_then(|result| result.solid())
            else {
                continue;
            };
            if solid.is_meshed() {
                continue;
            }
            interruptible(cancel.interrupt(), || {
                solid.tessellate(body_name(document, *body), &self.mesh_quality);
            });
            if solid.is_meshed() {
                report(evaluation);
            }
        }
        evaluation.meshed = evaluation.bodies.keys().all(|body| {
            evaluation
                .body_result(*body)
                .and_then(|result| result.solid())
                .is_none_or(SolidResult::is_meshed)
        });
    }
}

struct Run<'a> {
    document: &'a Document,
    parameters: &'a ParameterValues,
    evaluator: &'a dyn Evaluator,
    cancel: &'a CancelToken,
    progress: &'a dyn Fn(usize, usize),
}

#[derive(Clone, Default)]
struct Walk {
    statuses: BTreeMap<FeatureId, FeatureStatus>,
    current: BTreeMap<FeatureId, Arc<FeatureResult>>,
    bodies: BodyStates,
    consumed: BTreeSet<FeatureId>,
    recomputed: Vec<FeatureId>,
    inputs_before: BTreeMap<FeatureId, FeatureId>,
    seen_bodies: BTreeMap<FeatureId, BTreeMap<FeatureId, FeatureId>>,
}

impl Walk {
    fn stand(&mut self, feature: &Feature, result: &Arc<FeatureResult>) {
        let id = feature.id();
        self.current.insert(id, Arc::clone(result));
        for part in body_parts(result) {
            if let Some(solid) = part.solid() {
                self.bodies.insert(solid.body, (id, Arc::clone(part)));
            }
        }
        for body in feature.kind.consumed_bodies() {
            self.bodies.remove(&body);
            self.consumed.insert(body);
        }
    }

    fn into_evaluation(
        self,
        document: &Document,
        parameters: ParameterValues,
        pending: BTreeSet<FeatureId>,
    ) -> Evaluation {
        let mut shown: BTreeMap<FeatureId, FeatureId> = self
            .bodies
            .iter()
            .map(|(body, (state, _))| (*body, *state))
            .collect();
        let stale_bodies = last_good_bodies(document, &self.statuses, &shown, &self.consumed);
        for (body, state) in &stale_bodies {
            shown.insert(*body, *state);
        }
        Evaluation {
            parameters,
            features: self.statuses,
            recomputed: self.recomputed,
            bodies: shown,
            stale_bodies: stale_bodies.into_keys().collect(),
            inputs_before: self.inputs_before,
            seen_bodies: self.seen_bodies,
            pending,
            meshed: false,
        }
    }
}

#[derive(Default)]
struct Glimpsed {
    recomputed: usize,
    settled_before: usize,
}

type Settling = BTreeMap<usize, Vec<FeatureId>>;

fn settling(features: &[Arc<Feature>], bar: usize) -> Settling {
    let mut last_change: BTreeMap<FeatureId, usize> = BTreeMap::new();
    for (index, feature) in features.iter().enumerate().take(bar) {
        if feature.suppressed {
            continue;
        }
        for body in feature
            .bodies()
            .into_iter()
            .chain(feature.kind.consumed_bodies())
        {
            last_change.insert(body, index);
        }
    }
    let mut settling = Settling::new();
    for (body, index) in last_change {
        settling.entry(index).or_default().push(body);
    }
    settling
}

fn skipped(feature: &Feature, index: usize, bar: usize) -> Option<FeatureState> {
    if index >= bar {
        Some(FeatureState::RolledBack)
    } else if feature.suppressed {
        Some(FeatureState::Suppressed)
    } else {
        None
    }
}

fn body_name(document: &Document, body: FeatureId) -> &str {
    document
        .feature(body)
        .map_or("a feature", |feature| feature.name.as_str())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Display {
    Prepared,
    Skipped,
}

fn last_good_bodies(
    document: &Document,
    statuses: &BTreeMap<FeatureId, FeatureStatus>,
    current: &BTreeMap<FeatureId, FeatureId>,
    consumed: &BTreeSet<FeatureId>,
) -> BTreeMap<FeatureId, FeatureId> {
    let made: BTreeSet<FeatureId> = document
        .active_features()
        .filter(|feature| feature.makes_body())
        .map(Feature::id)
        .collect();
    let mut stale = BTreeMap::new();
    for feature in document.features() {
        let Some(last_good) = statuses
            .get(&feature.id())
            .and_then(|status| status.result.as_ref())
        else {
            continue;
        };
        for solid in body_parts(last_good).filter_map(|part| part.solid()) {
            if made.contains(&solid.body)
                && !current.contains_key(&solid.body)
                && !consumed.contains(&solid.body)
            {
                stale.insert(solid.body, feature.id());
            }
        }
    }
    stale
}

fn missing_upstream(
    document: &Document,
    feature: &Feature,
    upstream: &[(FeatureId, Option<Arc<FeatureResult>>)],
) -> Option<FeatureError> {
    let (missing, _) = upstream.iter().find(|(_, result)| result.is_none())?;
    let Some(used) = document.feature(*missing) else {
        let remedy = if feature.kind.attachment().is_some() {
            "Detach the sketch to keep it where it is, or undo the deletion."
        } else {
            "Undo the deletion, or delete this feature too."
        };
        return Some(FeatureError {
            reason: "It uses a feature that no longer exists.".to_owned(),
            remedy: remedy.to_owned(),
            fix: None,
            constraints: Vec::new(),
            place: None,
        });
    };
    let name = used.name.clone();
    if used.suppressed {
        return Some(FeatureError {
            reason: format!("It uses {name}, which is suppressed."),
            remedy: format!("Unsuppress {name}, or suppress this feature too."),
            fix: Some(FixTarget::Unsuppress(*missing)),
            constraints: Vec::new(),
            place: None,
        });
    }
    Some(FeatureError {
        reason: format!("It uses {name}, which has an error."),
        remedy: format!("Fix {name} first."),
        fix: Some(FixTarget::Feature(*missing)),
        constraints: Vec::new(),
        place: None,
    })
}

const _: () = assert!(
    cfg!(panic = "unwind"),
    "a panicking feature is contained by unwinding, so caditor must be built with panic = \"unwind\""
);

fn check_healing(feature: &Arc<Feature>, inputs: &Inputs<'_>) -> Option<Arc<Healing>> {
    panic::catch_unwind(AssertUnwindSafe(|| healing::check(feature, inputs)))
        .unwrap_or_else(|_| {
            log::error!("checking the references of {} panicked", feature.name);
            None
        })
        .map(Arc::new)
}

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
        Failure::Error(Box::new(FeatureError {
            reason: "caditor ran into an internal error while recomputing this feature.".to_owned(),
            remedy:
                "Your model is unchanged. Undo the last change, and please report this problem."
                    .to_owned(),
            fix: None,
            constraints: Vec::new(),
            place: None,
        }))
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
                let sketch = &*projection::refreshed(
                    feature,
                    definition,
                    &plane.unwrap_or_else(|| definition.sketch.plane()),
                    inputs,
                )?;
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
                        let result = SketchResult::remembering(geometry, solution, memo)
                            .sharing_display_with(inputs.previous.and_then(FeatureResult::sketch));
                        Ok(FeatureResult::Sketch(result))
                    }
                    Err(SketchError::Cancelled) => Err(Failure::Cancelled),
                    Err(error) => Err(sketch_error(feature.id(), sketch, &error).into()),
                }
            }
            FeatureKind::Solid(solid) => solid::evaluate(feature, solid, inputs, cancel),
            FeatureKind::Blend(definition) => blend::evaluate(feature, definition, inputs, cancel),
            FeatureKind::Shell(definition) => shell::evaluate(feature, definition, inputs, cancel),
            FeatureKind::Hole(definition) => hole::evaluate(feature, definition, inputs, cancel),
            FeatureKind::Move(definition) => {
                movement::evaluate(feature, definition, inputs, cancel)
            }
            FeatureKind::Mirror(definition) => {
                mirror::evaluate(feature, definition, inputs, cancel)
            }
            FeatureKind::Split(definition) => split::evaluate(feature, definition, inputs, cancel),
            FeatureKind::Scale(definition) => {
                scaling::evaluate(feature, definition, inputs, cancel)
            }
            FeatureKind::Combine(definition) => {
                combine::evaluate(feature, definition, inputs, cancel)
            }
            FeatureKind::Pattern(definition) => {
                pattern::evaluate(feature, definition, inputs, cancel)
            }
            FeatureKind::Datum(definition) => datum::evaluate(feature, definition, inputs),
            FeatureKind::Import(definition) => import::evaluate(feature, definition),
            FeatureKind::Remove(definition) => removal::evaluate(feature, definition, inputs),
        }
    }
}

fn sketch_error(feature: FeatureId, sketch: &Sketch, error: &SketchError) -> FeatureError {
    match error {
        SketchError::Dimension { constraint, reason } => {
            dimension_error(feature, sketch, *constraint, reason)
        }
        SketchError::Conflict { constraints } => conflict_error(feature, sketch, constraints),
        SketchError::Several(parts) => several_errors(feature, sketch, parts),
        SketchError::NoLength { label, .. } => FeatureError {
            reason: format!("{label} has no length, so the sketch cannot be solved."),
            remedy: format!(
                "Delete {label}, or move its ends apart and remove any constraint that holds them \
                 together."
            ),
            fix: Some(FixTarget::Feature(feature)),
            constraints: Vec::new(),
            place: None,
        },
        SketchError::Unsolvable { entities, newest } => {
            unsolvable_error(feature, sketch, entities, *newest)
        }
        _ => FeatureError {
            reason: format!("The sketch could not be evaluated: {error}."),
            remedy: "Undo the last change.".to_owned(),
            fix: None,
            constraints: Vec::new(),
            place: None,
        },
    }
}

fn several_errors(feature: FeatureId, sketch: &Sketch, parts: &[SketchError]) -> FeatureError {
    let errors: Vec<FeatureError> = parts
        .iter()
        .map(|part| sketch_error(feature, sketch, part))
        .collect();
    let numbered = |pick: fn(&FeatureError) -> &str| {
        errors
            .iter()
            .enumerate()
            .map(|(index, error)| format!("({}) {}", index + 1, pick(error)))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut constraints: Vec<ConstraintId> = Vec::new();
    for constraint in errors.iter().flat_map(|error| &error.constraints) {
        if !constraints.contains(constraint) {
            constraints.push(*constraint);
        }
    }
    FeatureError {
        reason: format!(
            "The sketch has {} separate problems. {}",
            errors.len(),
            numbered(|error| &error.reason)
        ),
        remedy: format!("Fix each of them. {}", numbered(|error| &error.remedy)),
        fix: errors.iter().find_map(|error| error.fix),
        constraints,
        place: None,
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
            place: None,
        },
        None if entities.is_empty() => FeatureError {
            reason,
            remedy: "Undo the last change.".to_owned(),
            fix: Some(FixTarget::Feature(feature)),
            constraints: Vec::new(),
            place: None,
        },
        None => FeatureError {
            reason,
            remedy: format!("Delete and redraw {them}, or undo the last change."),
            fix: Some(FixTarget::Feature(feature)),
            constraints: Vec::new(),
            place: None,
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
        place: None,
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
        DimensionError::NotPositive
        | DimensionError::DiameterNotPositive
        | DimensionError::ArcLengthNotPositive => (
            "Edit the dimension so it gives more than zero.".to_owned(),
            dimension,
        ),
        DimensionError::SweepOutsideTurn => (
            "Edit the dimension so it gives an angle between 0 and 360 deg.".to_owned(),
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
        place: None,
    }
}
