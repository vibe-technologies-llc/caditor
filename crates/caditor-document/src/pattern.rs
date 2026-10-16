use std::collections::BTreeSet;

use caditor_expression::{Dimension, Expression, ParameterId, format_number};
use caditor_geometry::{Plane, Ray, RigidTransform, Vector2, Vector3};
use caditor_kernel::{
    BooleanError, BooleanOperation, LINEAR_RESOLUTION, MAX_SIZE, PatternCopy, PatternError, Solid,
    boolean, pattern, pattern_copies,
};
use caditor_sketch::{EntityId, Sketch};

use crate::{
    blend::BlendRecipe,
    datum::{AxisReference, PointReference, Resolver, feature_name},
    document::{Feature, FeatureId, FeatureKind},
    failure::Cause,
    pattern_path::{CurvePath, PathError, Station, curve_path},
    recompute::{
        CancelToken, Failure, FeatureError, FeatureResult, FixTarget, Inputs, MissingInput,
    },
    repeat_blend::{self, CopiedNames, Repeat, RepeatCause, Repeater},
    repeat_shell,
    shell::ShellRecipe,
    solid::{BodyOperation, SolidResult},
    tolerance,
    trouble::{self, Adjust, Trouble},
    values::ParameterValues,
};

pub const MAX_PATTERN_INSTANCES: u32 = 100;
const WHOLE_TOLERANCE: f64 = 1e-9;
const FULL_TURN: f64 = 360.0;
const ANGLE_TOLERANCE: f64 = 1e-9;
const MAX_NAMED_COPIES: usize = 3;
pub const ORIGINAL_INSTANCE: Instance = [0, 0];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LinearSpacing {
    #[default]
    BetweenCopies,
    Total,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LinearDirection {
    pub axis: AxisReference,
    pub count: Expression,
    pub spacing: Expression,
    pub measured: LinearSpacing,
    pub reversed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CircularPattern {
    pub axis: Option<AxisReference>,
    pub count: Expression,
    pub angle: Expression,
    pub reversed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CurveSpacing {
    #[default]
    Spread,
    Distance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CopyOrientation {
    #[default]
    Kept,
    Following,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CurvePattern {
    pub sketch: FeatureId,
    pub count: Expression,
    pub spacing: Expression,
    pub measured: CurveSpacing,
    pub orientation: CopyOrientation,
    pub reversed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PointsPattern {
    pub sketch: FeatureId,
    pub base: PointReference,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PatternKind {
    Linear {
        first: LinearDirection,
        second: Option<LinearDirection>,
    },
    Circular(CircularPattern),
    Curve(CurvePattern),
    Points(PointsPattern),
}

pub type Instance = [u32; 2];

#[derive(Debug, Clone, PartialEq)]
pub struct Pattern {
    pub body: FeatureId,
    pub kind: PatternKind,
    pub skipped: BTreeSet<Instance>,
    pub repeated: Vec<FeatureId>,
}

pub fn repeatable_on(kind: &FeatureKind) -> Option<FeatureId> {
    if let Some(hole) = kind.hole() {
        return Some(hole.body);
    }
    if let Some(blend) = kind.blend() {
        return Some(blend.body);
    }
    if let Some(shell) = kind.shell() {
        return Some(shell.body);
    }
    let operation = if let Some(primitive) = kind.primitive() {
        primitive.operation
    } else if let Some(pipe) = kind.pipe() {
        pipe.operation
    } else if let Some(coil) = kind.coil() {
        coil.operation
    } else if let Some(loft) = kind.loft() {
        loft.operation
    } else {
        kind.solid()?.operation()
    };
    match operation {
        BodyOperation::Add(body) | BodyOperation::Remove(body) => Some(body),
        BodyOperation::NewBody | BodyOperation::Intersect(_) => None,
    }
}

impl PatternKind {
    pub fn heap_size(&self) -> usize {
        match self {
            Self::Linear { first, second } => {
                first.heap_size() + second.as_ref().map_or(0, LinearDirection::heap_size)
            }
            Self::Circular(circular) => {
                circular.axis.as_ref().map_or(0, AxisReference::heap_size)
                    + circular.count.heap_size()
                    + circular.angle.heap_size()
            }
            Self::Curve(curve) => curve.count.heap_size() + curve.spacing.heap_size(),
            Self::Points(points) => points.base.heap_size(),
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            Self::Linear { .. } => "Linear pattern",
            Self::Circular(_) => "Circular pattern",
            Self::Curve(_) => "Curve pattern",
            Self::Points(_) => "Point pattern",
        }
    }

    pub fn is_linear(&self) -> bool {
        matches!(self, Self::Linear { .. })
    }

    pub fn sketch(&self) -> Option<FeatureId> {
        match self {
            Self::Curve(curve) => Some(curve.sketch),
            Self::Points(points) => Some(points.sketch),
            Self::Linear { .. } | Self::Circular(_) => None,
        }
    }

    pub fn base(&self) -> Option<&PointReference> {
        match self {
            Self::Points(points) => Some(&points.base),
            Self::Linear { .. } | Self::Circular(_) | Self::Curve(_) => None,
        }
    }
}

pub fn point_instance(point: EntityId) -> Option<Instance> {
    let key = point.raw().checked_add(1)?;
    Some([(key & u64::from(u32::MAX)) as u32, (key >> 32) as u32])
}

pub fn instance_point([low, high]: Instance) -> Option<EntityId> {
    let key = u64::from(low) | (u64::from(high) << 32);
    key.checked_sub(1).map(EntityId::from_raw)
}

pub fn point_copy_name(instance: Instance) -> String {
    match (instance == ORIGINAL_INSTANCE, instance_point(instance)) {
        (false, Some(point)) => format!("the copy at point {point}"),
        _ => instance_name(instance),
    }
}

impl LinearSpacing {
    pub fn what(self) -> &'static str {
        match self {
            Self::BetweenCopies => "spacing",
            Self::Total => "total length",
        }
    }
}

impl LinearDirection {
    pub fn heap_size(&self) -> usize {
        self.axis.heap_size() + self.count.heap_size() + self.spacing.heap_size()
    }
}

impl Pattern {
    pub(crate) fn settle_skipped(&mut self) {
        let points = matches!(self.kind, PatternKind::Points(_));
        self.skipped.retain(|instance| {
            *instance != ORIGINAL_INSTANCE
                && (points || instance.iter().all(|step| *step < MAX_PATTERN_INSTANCES))
        });
    }

    pub fn new(body: FeatureId, kind: PatternKind) -> Self {
        Self {
            body,
            kind,
            skipped: BTreeSet::new(),
            repeated: Vec::new(),
        }
    }

    #[must_use]
    pub fn repeating(mut self, features: Vec<FeatureId>) -> Self {
        self.repeated = features;
        self
    }

    pub fn repeats_features(&self) -> bool {
        !self.repeated.is_empty()
    }

    pub fn heap_size(&self) -> usize {
        self.kind.heap_size()
            + self.skipped.len() * size_of::<Instance>()
            + self.repeated.len() * size_of::<FeatureId>()
    }

    pub fn instances(&self, values: &ParameterValues) -> Option<[u32; 2]> {
        let count = |expression: &Expression| {
            let value = values.evaluate_expression(expression).ok()?.value.round();
            (1.0..=f64::from(MAX_PATTERN_INSTANCES))
                .contains(&value)
                .then_some(value as u32)
        };
        match &self.kind {
            PatternKind::Linear { first, second } => {
                let across = match second {
                    Some(second) => count(&second.count)?,
                    None => 1,
                };
                Some([count(&first.count)?, across])
            }
            PatternKind::Circular(circular) => Some([count(&circular.count)?, 1]),
            PatternKind::Curve(curve) => Some([count(&curve.count)?, 1]),
            PatternKind::Points(_) => None,
        }
    }

    pub fn reversed_directions(&self) -> (bool, bool) {
        match &self.kind {
            PatternKind::Linear { first, second } => (
                first.reversed,
                second.as_ref().is_some_and(|second| second.reversed),
            ),
            PatternKind::Circular(_) | PatternKind::Curve(_) | PatternKind::Points(_) => {
                (false, false)
            }
        }
    }

    pub fn instance_words(&self, instance: Instance) -> String {
        match self.kind {
            PatternKind::Points(_) => point_copy_name(instance),
            PatternKind::Linear { .. } | PatternKind::Circular(_) | PatternKind::Curve(_) => {
                instance_name(instance)
            }
        }
    }

    pub fn skipped_within(&self, [columns, rows]: [u32; 2]) -> usize {
        self.skipped
            .iter()
            .filter(|[column, row]| *column < columns && *row < rows)
            .count()
    }

    pub fn is_skipped(&self, instance: Instance) -> bool {
        instance != ORIGINAL_INSTANCE && self.skipped.contains(&instance)
    }

    pub fn toggled(&self, instance: Instance) -> Option<Self> {
        if instance == ORIGINAL_INSTANCE {
            return None;
        }
        let mut toggled = self.clone();
        if !toggled.skipped.remove(&instance) {
            toggled.skipped.insert(instance);
        }
        Some(toggled)
    }

    pub fn title(&self) -> &'static str {
        self.kind.title()
    }

    fn expressions(&self) -> Vec<&Expression> {
        match &self.kind {
            PatternKind::Linear { first, second } => std::iter::once(first)
                .chain(second)
                .flat_map(|direction| [&direction.count, &direction.spacing])
                .collect(),
            PatternKind::Circular(circular) => vec![&circular.count, &circular.angle],
            PatternKind::Curve(curve) => vec![&curve.count, &curve.spacing],
            PatternKind::Points(_) => Vec::new(),
        }
    }

    pub fn axes(&self) -> Vec<&AxisReference> {
        match &self.kind {
            PatternKind::Linear { first, second } => std::iter::once(first)
                .chain(second)
                .map(|direction| &direction.axis)
                .collect(),
            PatternKind::Circular(circular) => circular.axis.iter().collect(),
            PatternKind::Curve(_) | PatternKind::Points(_) => Vec::new(),
        }
    }

    pub fn parameters(&self) -> BTreeSet<ParameterId> {
        self.expressions()
            .into_iter()
            .flat_map(Expression::parameters)
            .collect()
    }

    pub fn uses_parameter(&self, parameter: ParameterId) -> bool {
        self.expressions()
            .into_iter()
            .any(|expression| expression.uses(parameter))
    }

    pub fn axis_datums(&self) -> BTreeSet<FeatureId> {
        self.axes()
            .into_iter()
            .filter_map(AxisReference::datum)
            .collect()
    }

    pub fn point_datums(&self) -> BTreeSet<FeatureId> {
        self.kind
            .base()
            .and_then(PointReference::datum)
            .into_iter()
            .collect()
    }

    pub fn frames(&self) -> BTreeSet<FeatureId> {
        self.axes()
            .into_iter()
            .filter_map(AxisReference::frame)
            .chain(self.kind.base().and_then(PointReference::frame))
            .collect()
    }

    pub fn bodies(&self) -> BTreeSet<FeatureId> {
        self.axes()
            .into_iter()
            .filter_map(AxisReference::body)
            .chain(self.kind.base().and_then(PointReference::body))
            .collect()
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        self.axes()
            .into_iter()
            .flat_map(AxisReference::origin_features)
            .chain(
                self.kind
                    .base()
                    .map(PointReference::origin_features)
                    .unwrap_or_default(),
            )
            .collect()
    }

    pub fn sketch_entities(&self) -> Vec<(FeatureId, EntityId)> {
        self.axes()
            .into_iter()
            .filter_map(AxisReference::sketch_entity)
            .chain(self.kind.base().and_then(PointReference::sketch_entity))
            .collect()
    }

    pub fn skipped_points(&self) -> Vec<(FeatureId, EntityId)> {
        let PatternKind::Points(points) = &self.kind else {
            return Vec::new();
        };
        self.skipped
            .iter()
            .filter(|instance| **instance != ORIGINAL_INSTANCE)
            .filter_map(|instance| instance_point(*instance))
            .map(|point| (points.sketch, point))
            .collect()
    }

    pub fn sketches(&self) -> BTreeSet<FeatureId> {
        self.axes()
            .into_iter()
            .filter_map(AxisReference::sketch)
            .chain(self.kind.sketch())
            .chain(self.kind.base().and_then(PointReference::sketch))
            .collect()
    }

    pub fn features(&self) -> BTreeSet<FeatureId> {
        let mut used = BTreeSet::from([self.body]);
        used.extend(self.repeated.iter().copied());
        used.extend(self.axis_datums());
        used.extend(self.point_datums());
        used.extend(self.bodies());
        used.extend(self.sketches());
        used
    }
}

struct Context<'a> {
    resolver: Resolver<'a>,
    subject: String,
    at_points: bool,
}

struct Placed<'a> {
    sketch: &'a Sketch,
    name: String,
}

struct Steps {
    count: u32,
    offset: Vector3,
}

impl Context<'_> {
    fn error(&self, cause: PatternCause) -> Failure {
        Failure::from(
            FeatureError::new(cause).fixing(FixTarget::Feature(self.resolver.feature.id())),
        )
    }

    fn error_at(&self, cause: PatternCause, fix: FeatureId) -> Failure {
        Failure::from(FeatureError::new(cause).fixing(FixTarget::Feature(fix)))
    }

    fn count(&self, expression: &Expression, what: &str) -> Result<u32, Failure> {
        let value = self.resolver.value(expression, what, Dimension::NONE)?;
        let whole = value.round();
        if (value - whole).abs() > WHOLE_TOLERANCE {
            return Err(self.error(PatternCause::NotWhole {
                what: what.to_owned(),
                value,
            }));
        }
        if whole < 1.0 {
            return Err(self.error(PatternCause::CountBelowOne {
                what: what.to_owned(),
            }));
        }
        if whole > f64::from(MAX_PATTERN_INSTANCES) {
            return Err(self.too_many(whole));
        }
        Ok(whole as u32)
    }

    fn too_many(&self, instances: f64) -> Failure {
        self.error(PatternCause::TooMany {
            instances,
            subject: self.subject.clone(),
        })
    }

    fn steps(&self, direction: &LinearDirection, which: &str) -> Result<(Ray, Steps), Failure> {
        let count = self.count(&direction.count, &format!("{which}count"))?;
        let spacing = self.resolver.value(
            &direction.spacing,
            &format!("{which}{}", direction.measured.what()),
            Dimension::LENGTH,
        )?;
        if spacing <= 0.0 {
            return Err(self.error(PatternCause::SpacingNotPositive {
                what: format!("{which}{}", direction.measured.what()),
            }));
        }
        let spacing = match (direction.measured, count) {
            (LinearSpacing::BetweenCopies, _) | (LinearSpacing::Total, 1) => spacing,
            (LinearSpacing::Total, count) => spacing / f64::from(count - 1),
        };
        if spacing * f64::from(count.saturating_sub(1)) > MAX_SIZE {
            return Err(self.error(PatternCause::ReachTooFar));
        }
        let ray = self.resolver.axis(&direction.axis)?;
        let sign = if direction.reversed { -1.0 } else { 1.0 };
        Ok((
            ray,
            Steps {
                count,
                offset: ray.direction() * spacing * sign,
            },
        ))
    }

    fn linear(
        &self,
        first: &LinearDirection,
        second: Option<&LinearDirection>,
    ) -> Result<Vec<PatternCopy>, Failure> {
        let (first_ray, along) = self.steps(first, "")?;
        let across = match second {
            Some(direction) => {
                let (second_ray, across) = self.steps(direction, "second ")?;
                if tolerance::parallel(first_ray.direction(), second_ray.direction()) {
                    return Err(self.error(PatternCause::Parallel));
                }
                across
            }
            None => Steps {
                count: 1,
                offset: Vector3::ZERO,
            },
        };
        let instances = u64::from(along.count) * u64::from(across.count);
        if instances > u64::from(MAX_PATTERN_INSTANCES) {
            return Err(self.too_many(instances as f64));
        }
        let mut copies = Vec::new();
        for row in 0..across.count {
            for column in 0..along.count {
                if row == 0 && column == 0 {
                    continue;
                }
                let offset = along.offset * f64::from(column) + across.offset * f64::from(row);
                copies.push(self.copy([column, row], RigidTransform::translation(offset))?);
            }
        }
        Ok(copies)
    }

    fn circular(&self, circular: &CircularPattern) -> Result<Vec<PatternCopy>, Failure> {
        let count = self.count(&circular.count, "count")?;
        let angle = self
            .resolver
            .value(&circular.angle, "angle", Dimension::ANGLE)?;
        if angle <= 0.0 || angle > FULL_TURN + ANGLE_TOLERANCE {
            return Err(self.error(PatternCause::AngleRange));
        }
        let Some(axis) = &circular.axis else {
            return Err(Failure::Waiting(MissingInput::Axis));
        };
        let axis = self.resolver.axis(axis)?;
        let full_turn = (angle - FULL_TURN).abs() <= ANGLE_TOLERANCE;
        let step = match (full_turn, count) {
            (true, _) => FULL_TURN / f64::from(count),
            (false, 1) => 0.0,
            (false, _) => angle / f64::from(count - 1),
        };
        let sign = if circular.reversed { -1.0 } else { 1.0 };
        (1..count)
            .map(|index| {
                let turn = (step * f64::from(index) * sign).to_radians();
                self.copy(
                    [index, 0],
                    RigidTransform::rotation_about(axis.origin(), axis.direction(), turn),
                )
            })
            .collect()
    }

    fn sketch(&self, sketch: FeatureId) -> Result<Placed<'_>, Failure> {
        let inputs = self.resolver.inputs;
        let name = feature_name(inputs.document, sketch);
        match inputs
            .features
            .get(&sketch)
            .and_then(|result| result.sketch())
        {
            Some(result) => Ok(Placed {
                sketch: &result.geometry,
                name,
            }),
            None => Err(self.error_at(PatternCause::SketchUnavailable { name }, sketch)),
        }
    }

    fn path_failure(&self, placed: &Placed<'_>, sketch: FeatureId, error: &PathError) -> Failure {
        let name = placed.name.clone();
        let cause = match error {
            PathError::NoCurves => PatternCause::PathEmpty { name },
            PathError::NotOneChain => PatternCause::PathBroken { name },
            PathError::NoLength { entity } => PatternCause::PathNoLength {
                name,
                curve: entity_label(placed.sketch, *entity),
            },
            PathError::Curve(_) => PatternCause::PathUnfollowable { name },
        };
        self.error_at(cause, sketch)
    }

    fn curve(&self, curve: &CurvePattern) -> Result<Vec<PatternCopy>, Failure> {
        let count = self.count(&curve.count, "count")?;
        let placed = self.sketch(curve.sketch)?;
        let path = curve_path(placed.sketch)
            .map_err(|error| self.path_failure(&placed, curve.sketch, &error))?;
        let step = self.curve_step(curve, count, &path)?;
        let plane = placed.sketch.plane();
        let start = path
            .station(0.0, curve.reversed)
            .ok_or_else(|| self.unplaceable())?;
        (1..count)
            .map(|index| {
                let station = path
                    .station(step * f64::from(index), curve.reversed)
                    .ok_or_else(|| self.unplaceable())?;
                self.copy(
                    [index, 0],
                    along_curve(&plane, start, station, curve.orientation),
                )
            })
            .collect()
    }

    fn curve_step(
        &self,
        curve: &CurvePattern,
        count: u32,
        path: &CurvePath,
    ) -> Result<f64, Failure> {
        let length = path.length();
        let steps = f64::from(count.saturating_sub(1));
        let step = match curve.measured {
            CurveSpacing::Spread if path.is_closed() => length / f64::from(count),
            CurveSpacing::Spread if count > 1 => length / steps,
            CurveSpacing::Spread => 0.0,
            CurveSpacing::Distance => {
                let spacing = self
                    .resolver
                    .value(&curve.spacing, "spacing", Dimension::LENGTH)?;
                if spacing <= 0.0 {
                    return Err(self.error(PatternCause::SpacingNotPositive {
                        what: "spacing".to_owned(),
                    }));
                }
                spacing
            }
        };
        let reach = step * steps;
        let past_end = if path.is_closed() {
            reach >= length - LINEAR_RESOLUTION
        } else {
            reach > length + LINEAR_RESOLUTION
        };
        if past_end {
            return Err(self.error(PatternCause::PastEnd {
                closed: path.is_closed(),
                length,
            }));
        }
        Ok(step)
    }

    fn unplaceable(&self) -> Failure {
        self.error(PatternCause::Unplaceable)
    }

    fn points(&self, points: &PointsPattern) -> Result<Vec<PatternCopy>, Failure> {
        let base = self.resolver.point(&points.base)?;
        let placed = self.sketch(points.sketch)?;
        let plane = placed.sketch.plane();
        let ids = placed.sketch.layout_points();
        if ids.is_empty() {
            let name = &placed.name;
            return Err(self.error_at(PatternCause::NoPoints { name: name.clone() }, points.sketch));
        }
        let mut copies = Vec::with_capacity(ids.len());
        for id in ids {
            let (Some(point), Some(index)) = (placed.sketch.point(id), point_instance(id)) else {
                continue;
            };
            let target = plane.to_world(point);
            if target.distance(base) <= LINEAR_RESOLUTION {
                continue;
            }
            copies.push(self.copy(index, RigidTransform::translation(target - base))?);
        }
        if copies.len() >= MAX_PATTERN_INSTANCES as usize {
            return Err(self.too_many((copies.len() + 1) as f64));
        }
        Ok(copies)
    }

    fn copy(
        &self,
        index: [u32; 2],
        placement: Option<RigidTransform>,
    ) -> Result<PatternCopy, Failure> {
        let placement = placement.ok_or_else(|| self.error(PatternCause::CopyUnplaced))?;
        Ok(PatternCopy {
            index,
            placement: placement.into(),
        })
    }

    fn describe(&self, copies: &[[u32; 2]]) -> String {
        let name = if self.at_points {
            point_copy_name
        } else {
            instance_name
        };
        describe_copies(copies, name)
    }

    fn failure(&self, error: &PatternError) -> Failure {
        let subject = self.subject.clone();
        match error {
            PatternError::Cancelled(_) => Failure::Cancelled,
            PatternError::Placement { .. } => self.error(PatternCause::FarAway { subject }),
            PatternError::Union {
                copies,
                error: BooleanError::NonManifold(_),
            } => self.error(PatternCause::Edgewise {
                copies: self.describe(copies),
                subject,
            }),
            PatternError::Union {
                copies,
                error: BooleanError::Ambiguous(_),
            } => self.error(PatternCause::Touching {
                copies: self.describe(copies),
                subject,
            }),
            PatternError::Union { copies, .. } => {
                log::warn!("{} could not be built: {error}", self.resolver.feature.name);
                self.error(PatternCause::Unjoined {
                    copies: self.describe(copies),
                    subject,
                })
            }
        }
    }
}

pub(crate) struct Seed<'a> {
    pub name: String,
    pub operation: BooleanOperation,
    pub tools: Vec<&'a Solid>,
}

pub(crate) enum Repeated<'a> {
    Tools(Seed<'a>),
    Blend {
        name: String,
        recipe: &'a BlendRecipe,
    },
    Shell(RepeatedShell<'a>),
}

pub(crate) struct RepeatedShell<'a> {
    pub name: String,
    pub recipe: &'a ShellRecipe,
    pub before: &'a Solid,
}

pub(crate) struct Seeds<'a> {
    pub repeated: Vec<(FeatureId, Repeated<'a>)>,
    pub shell: Option<RepeatedShell<'a>>,
}

impl<'a> Seeds<'a> {
    pub(crate) fn of(
        inputs: &'a Inputs<'_>,
        features: &[FeatureId],
        body: FeatureId,
        repeater: Repeater,
    ) -> Result<Self, Failure> {
        let mut seeds = Self {
            repeated: Vec::with_capacity(features.len()),
            shell: None,
        };
        for &feature in features {
            match seed(inputs, feature, body, repeater)? {
                Repeated::Shell(shell) => seeds.shell = Some(shell),
                repeated => seeds.repeated.push((feature, repeated)),
            }
        }
        Ok(seeds)
    }

    pub(crate) fn start<'b>(&self, solid: &'b Solid) -> &'b Solid
    where
        'a: 'b,
    {
        self.shell.as_ref().map_or(solid, |shell| shell.before)
    }
}

fn seed_error(seed: FeatureId, cause: RepeatCause) -> Failure {
    Failure::from(FeatureError::new(cause).fixing(FixTarget::Feature(seed)))
}

pub(crate) fn seed<'a>(
    inputs: &'a Inputs<'_>,
    seed: FeatureId,
    body: FeatureId,
    repeater: Repeater,
) -> Result<Repeated<'a>, Failure> {
    let name = inputs.document.feature(seed).map_or_else(
        || "a deleted feature".to_owned(),
        |found| found.name.clone(),
    );
    let Some(result) = inputs.features.get(&seed).and_then(|result| result.solid()) else {
        return Err(seed_error(
            seed,
            RepeatCause::SeedUnavailable { repeater, name },
        ));
    };
    if result.body != body {
        return Err(seed_error(
            seed,
            RepeatCause::SeedOtherBody { repeater, name },
        ));
    }
    if let Some(recipe) = result.blended() {
        return Ok(Repeated::Blend { name, recipe });
    }
    if let Some(recipe) = result.shelled() {
        let last = inputs.bodies.get(&body).map(|(state, _)| *state);
        let before = recipe.before().filter(|_| last == Some(seed));
        let Some(before) = before else {
            let after = last
                .and_then(|state| inputs.document.feature(state))
                .map_or_else(|| "another feature".to_owned(), |found| found.name.clone());
            return Err(seed_error(
                seed,
                RepeatCause::SeedShellLate {
                    repeater,
                    name,
                    after,
                },
            ));
        };
        return Ok(Repeated::Shell(RepeatedShell {
            name,
            recipe,
            before,
        }));
    }
    let (operation, parts) = match (result.cuts(), result.joins()) {
        ([], []) => {
            return Err(seed_error(
                seed,
                RepeatCause::SeedNeither { repeater, name },
            ));
        }
        ([], joins) => (BooleanOperation::Union, joins),
        (cuts, _) => (BooleanOperation::Difference, cuts),
    };
    let tools = parts
        .iter()
        .filter_map(|part| part.solid())
        .map(|part| &part.solid)
        .collect();
    Ok(Repeated::Tools(Seed {
        name,
        operation,
        tools,
    }))
}

impl Context<'_> {
    fn repeat(
        &self,
        definition: &Pattern,
        solid: &Solid,
        copies: &[PatternCopy],
        cancel: &CancelToken,
    ) -> Result<SolidResult, Failure> {
        let inputs = self.resolver.inputs;
        let raw = self.resolver.feature.id().raw();
        let seeds = Seeds::of(
            inputs,
            &definition.repeated,
            definition.body,
            Repeater::Pattern,
        )?;
        let mut body = seeds.start(solid).clone();
        let mut cuts = Vec::new();
        let mut names = CopiedNames::new(self.resolver.feature.id());
        let instances: Vec<Instance> = copies.iter().map(|copy| copy.index).collect();
        let copy_words = |instance: Instance| definition.instance_words(instance);
        let base = Repeat {
            feature: self.resolver.feature.id(),
            repeater: Repeater::Pattern,
            copy: &copy_words,
        };
        for (feature, repeated) in seeds.repeated {
            let seed = match repeated {
                Repeated::Tools(seed) => seed,
                Repeated::Blend { name, recipe } => {
                    body = names
                        .blend(body, recipe, &instances, cancel)
                        .map_err(|error| repeat_blend::failure(&error, recipe, &base, &name))?;
                    continue;
                }
                Repeated::Shell(_) => continue,
            };
            let subject = Context {
                resolver: Resolver {
                    feature: self.resolver.feature,
                    inputs,
                },
                subject: seed.name.clone(),
                at_points: self.at_points,
            };
            for tool in &seed.tools {
                if cancel.is_cancelled() {
                    return Err(Failure::Cancelled);
                }
                let placed = pattern_copies(tool, copies, raw).map_err(|error| {
                    subject.failure(&error).placed(trouble::union_place(&error))
                })?;
                let Some(placed) = placed else {
                    continue;
                };
                body = boolean(&body, &placed, seed.operation).map_err(|error| {
                    self.combine_failure(inputs, [&body, &placed], &seed, &error)
                })?;
                if seed.operation == BooleanOperation::Difference {
                    cuts.push(placed);
                }
            }
            names.copying_tools_of(feature);
        }
        if let Some(shell) = &seeds.shell {
            body = names
                .shell(&body, shell.recipe, &instances, cancel)
                .map_err(|error| repeat_shell::failure(&error, &base, &shell.name))?;
        }
        Ok(SolidResult::new(definition.body, body).cutting(cuts))
    }

    fn combine_failure(
        &self,
        inputs: &Inputs<'_>,
        operands: [&Solid; 2],
        seed: &Seed<'_>,
        error: &BooleanError,
    ) -> Failure {
        let name = &seed.name;
        let body = &self.subject;
        let cutting = match (error, seed.operation) {
            (BooleanError::Cancelled(_), _) => return Failure::Cancelled,
            (BooleanError::Empty, _) => {
                return self.error(PatternCause::CopiesVanish {
                    name: name.clone(),
                    subject: body.clone(),
                });
            }
            (_, BooleanOperation::Difference) => true,
            (_, _) => false,
        };
        log::warn!("{} could not be built: {error}", self.resolver.feature.name);
        let trouble = trouble::boolean_trouble(
            inputs.document,
            operands,
            error,
            Adjust::ChangeTheSpacingOrTheAngleSlightly,
        );
        let place = trouble.place;
        self.error(PatternCause::CopiesUncombined {
            name: name.clone(),
            subject: body.clone(),
            cutting,
            trouble,
        })
        .placed(place)
    }
}

pub fn instance_name(instance: Instance) -> String {
    match instance {
        [0, 0] => "the original".to_owned(),
        [index, 0] => format!("copy {index}"),
        [first, second] => format!("copy ({first}, {second})"),
    }
}

pub(crate) fn along_curve(
    plane: &Plane,
    start: Station,
    station: Station,
    orientation: CopyOrientation,
) -> Option<RigidTransform> {
    let from = plane.to_world(start.point);
    let shift = RigidTransform::translation(plane.to_world(station.point) - from)?;
    match orientation {
        CopyOrientation::Kept => Some(shift),
        CopyOrientation::Following => {
            let turn = turn_between(plane, start.tangent, station.tangent);
            let turned = RigidTransform::rotation_about(from, plane.normal(), turn)?;
            Some(turned.then(&shift))
        }
    }
}

fn turn_between(plane: &Plane, from: Vector2, to: Vector2) -> f64 {
    let world = |direction: Vector2| plane.x_axis() * direction.x + plane.y_axis() * direction.y;
    let (from, to) = (world(from), world(to));
    plane.normal().dot(from.cross(to)).atan2(from.dot(to))
}

fn entity_label(sketch: &Sketch, entity: u64) -> String {
    sketch.entity_label(EntityId::from_raw(entity))
}

fn describe_copies(copies: &[[u32; 2]], name: fn(Instance) -> String) -> String {
    let named: Vec<String> = copies
        .iter()
        .take(MAX_NAMED_COPIES)
        .map(|copy| name(*copy))
        .collect();
    let more = copies.len().saturating_sub(MAX_NAMED_COPIES);
    let text = match more {
        0 => named.join(", "),
        more => format!("{} and {more} more", named.join(", ")),
    };
    let mut characters = text.chars();
    characters
        .next()
        .map(|first| first.to_uppercase().chain(characters).collect())
        .unwrap_or_default()
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &Pattern,
    inputs: &Inputs<'_>,
    cancel: &CancelToken,
) -> Result<FeatureResult, Failure> {
    let body_name = inputs
        .document
        .body_label(definition.body)
        .unwrap_or_default();
    let context = Context {
        resolver: Resolver { feature, inputs },
        subject: format!("the body of {body_name}"),
        at_points: matches!(definition.kind, PatternKind::Points(_)),
    };
    let mut copies = match &definition.kind {
        PatternKind::Linear { first, second } => context.linear(first, second.as_ref())?,
        PatternKind::Circular(circular) => context.circular(circular)?,
        PatternKind::Curve(curve) => context.curve(curve)?,
        PatternKind::Points(points) => context.points(points)?,
    };
    copies.retain(|copy| !definition.is_skipped(copy.index));
    let Some(solid) = inputs.body(definition.body) else {
        return Err(inputs.missing_body(definition.body));
    };
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    if definition.repeats_features() {
        return context
            .repeat(definition, solid, &copies, cancel)
            .map(FeatureResult::Solid);
    }
    let result = pattern(solid, &copies, feature.id().raw())
        .map_err(|error| context.failure(&error).placed(trouble::union_place(&error)))?;
    Ok(FeatureResult::Solid(SolidResult::new(
        definition.body,
        result,
    )))
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PatternCause {
    NotWhole {
        what: String,
        value: f64,
    },
    CountBelowOne {
        what: String,
    },
    TooMany {
        instances: f64,
        subject: String,
    },
    SpacingNotPositive {
        what: String,
    },
    ReachTooFar,
    Parallel,
    AngleRange,
    SketchUnavailable {
        name: String,
    },
    PathEmpty {
        name: String,
    },
    PathBroken {
        name: String,
    },
    PathNoLength {
        name: String,
        curve: String,
    },
    PathUnfollowable {
        name: String,
    },
    PastEnd {
        closed: bool,
        length: f64,
    },
    Unplaceable,
    NoPoints {
        name: String,
    },
    CopyUnplaced,
    FarAway {
        subject: String,
    },
    Edgewise {
        copies: String,
        subject: String,
    },
    Touching {
        copies: String,
        subject: String,
    },
    Unjoined {
        copies: String,
        subject: String,
    },
    CopiesVanish {
        name: String,
        subject: String,
    },
    CopiesUncombined {
        name: String,
        subject: String,
        cutting: bool,
        trouble: Trouble,
    },
}

impl PatternCause {
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::NotWhole { what, value } => format!(
                "The {what} must be a whole number, and {} is not.",
                format_number(*value)
            ),
            Self::CountBelowOne { what } => format!("The {what} must be at least 1."),
            Self::TooMany { instances, subject } => format!(
                "The pattern would make {} instances of {subject}, and at most \
                 {MAX_PATTERN_INSTANCES} are allowed.",
                format_number(*instances)
            ),
            Self::SpacingNotPositive { what } => format!("The {what} must be more than zero."),
            Self::ReachTooFar => {
                "The copies would reach farther than a kilometre, the largest size caditor models."
                    .to_owned()
            }
            Self::Parallel => {
                "The two directions are parallel, so the copies would fall on one line.".to_owned()
            }
            Self::AngleRange => "The angle must be more than 0° and at most 360°.".to_owned(),
            Self::SketchUnavailable { name } => {
                format!("It follows {name}, which has an error or is not a sketch.")
            }
            Self::PathEmpty { name } => format!("{name} has no curve for the copies to follow."),
            Self::PathBroken { name } => format!(
                "The curves of {name} do not join end to end into one chain, so it is unclear \
                 which way the copies run."
            ),
            Self::PathNoLength { name, curve } => format!("{curve} in {name} has no length."),
            Self::PathUnfollowable { name } => {
                format!("A curve of {name} could not be followed.")
            }
            Self::PastEnd {
                closed: true,
                length,
            } => format!(
                "The copies would go all the way round the curve, which is {} mm long, and land \
                 on the original.",
                format_number(*length)
            ),
            Self::PastEnd {
                closed: false,
                length,
            } => format!(
                "The copies would run past the end of the curve, which is {} mm long.",
                format_number(*length)
            ),
            Self::Unplaceable => "A copy could not be placed on the curve.".to_owned(),
            Self::NoPoints { name } => format!("{name} has no lone points to place copies at."),
            Self::CopyUnplaced => "A copy could not be placed.".to_owned(),
            Self::FarAway { subject } => {
                format!("A copy of {subject} could not be placed that far away.")
            }
            Self::Edgewise { copies, subject } => format!(
                "{copies} of {subject} meet only along an edge or at a corner, and could not be \
                 kept as separate shells."
            ),
            Self::Touching { copies, subject } => {
                format!("{copies} of {subject} touch where it cannot be told which side is inside.")
            }
            Self::Unjoined { copies, subject } => {
                format!("{copies} of {subject} could not be joined together.")
            }
            Self::CopiesVanish { name, subject } => {
                format!("The copies of {name} would leave nothing of {subject}.")
            }
            Self::CopiesUncombined {
                name,
                subject,
                cutting,
                trouble,
            } => trouble.reason(if *cutting {
                format!("The copies of {name} could not be cut into {subject}.")
            } else {
                format!("The copies of {name} could not be joined to {subject}.")
            }),
        }
    }

    pub(crate) fn remedy(&self) -> String {
        match self {
            Self::NotWhole { .. } => "Enter a whole number, such as 4.".to_owned(),
            Self::CountBelowOne { .. } => {
                "Enter a count of 1 or more; 1 leaves the body as it is.".to_owned()
            }
            Self::TooMany { .. } => "Lower the count.".to_owned(),
            Self::SpacingNotPositive { .. } => {
                "Enter a length above zero, and tick Reversed to go the other way.".to_owned()
            }
            Self::ReachTooFar | Self::FarAway { .. } => {
                "Lower the count or the spacing.".to_owned()
            }
            Self::Parallel => {
                "Choose a second direction across the first, or remove it.".to_owned()
            }
            Self::AngleRange => {
                "Enter an angle up to 360 deg; a full turn spaces the copies evenly around the \
                 axis."
                    .to_owned()
            }
            Self::SketchUnavailable { name } => {
                format!("Fix {name} first, or choose another sketch.")
            }
            Self::PathEmpty { name } => {
                format!("Draw a line, arc or spline in {name}, or choose another sketch.")
            }
            Self::PathBroken { name } => format!(
                "Join the curves of {name} end to end without branches, or make the others \
                 construction geometry."
            ),
            Self::PathNoLength { name, .. } => {
                format!("Give it a length or delete it from {name}.")
            }
            Self::PathUnfollowable { name } => format!("Redraw the curves of {name}."),
            Self::PastEnd { .. } => {
                "Lower the count or the spacing, or spread the copies over the whole curve."
                    .to_owned()
            }
            Self::Unplaceable => "Redraw the curve so it has a direction everywhere.".to_owned(),
            Self::NoPoints { name } => format!(
                "Add points to {name} with the Point tool; the ends of curves do not count."
            ),
            Self::CopyUnplaced => "Change the spacing, the angle or the axis.".to_owned(),
            Self::Edgewise { .. } => {
                "Change the spacing or the angle so the copies overlap or stand apart.".to_owned()
            }
            Self::Touching { .. } => "Change the spacing or the angle slightly.".to_owned(),
            Self::Unjoined { .. } => {
                "Change the spacing or the angle slightly, or lower the count.".to_owned()
            }
            Self::CopiesVanish { .. } => "Lower the count or change the spacing.".to_owned(),
            Self::CopiesUncombined { trouble, .. } => trouble.remedy(),
        }
    }
}

impl From<PatternCause> for Cause {
    fn from(cause: PatternCause) -> Self {
        Self::Pattern(cause)
    }
}
