use std::collections::BTreeSet;

use caditor_expression::{Dimension, Expression, ParameterId, format_number};
use caditor_geometry::{Ray, RigidTransform, Vector3};
use caditor_kernel::{
    BooleanError, BooleanOperation, MAX_SIZE, PatternCopy, PatternError, Solid, boolean, pattern,
    pattern_copies,
};

use crate::{
    datum::{AxisReference, Resolver},
    document::{Feature, FeatureId, FeatureKind},
    recompute::{CancelToken, Failure, FeatureError, FeatureResult, FixTarget, Inputs},
    solid::{BodyOperation, SolidResult},
    tolerance, trouble,
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
    pub axis: AxisReference,
    pub count: Expression,
    pub angle: Expression,
    pub reversed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PatternKind {
    Linear {
        first: LinearDirection,
        second: Option<LinearDirection>,
    },
    Circular(CircularPattern),
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
    match kind.solid()?.operation() {
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
                circular.axis.heap_size() + circular.count.heap_size() + circular.angle.heap_size()
            }
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            Self::Linear { .. } => "Linear pattern",
            Self::Circular(_) => "Circular pattern",
        }
    }

    pub fn is_linear(&self) -> bool {
        matches!(self, Self::Linear { .. })
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
        }
    }

    pub fn axes(&self) -> Vec<&AxisReference> {
        match &self.kind {
            PatternKind::Linear { first, second } => std::iter::once(first)
                .chain(second)
                .map(|direction| &direction.axis)
                .collect(),
            PatternKind::Circular(circular) => vec![&circular.axis],
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

    pub fn axis_frames(&self) -> BTreeSet<FeatureId> {
        self.axes()
            .into_iter()
            .filter_map(AxisReference::frame)
            .collect()
    }

    pub fn axis_bodies(&self) -> BTreeSet<FeatureId> {
        self.axes()
            .into_iter()
            .filter_map(AxisReference::body)
            .collect()
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        self.axes()
            .into_iter()
            .flat_map(AxisReference::origin_features)
            .collect()
    }

    pub fn axis_sketches(&self) -> BTreeSet<FeatureId> {
        self.axes()
            .into_iter()
            .filter_map(AxisReference::sketch)
            .collect()
    }

    pub fn features(&self) -> BTreeSet<FeatureId> {
        let mut used = BTreeSet::from([self.body]);
        used.extend(self.repeated.iter().copied());
        used.extend(self.axis_datums());
        used.extend(self.axis_bodies());
        used.extend(self.axis_sketches());
        used
    }
}

struct Context<'a> {
    resolver: Resolver<'a>,
    subject: String,
}

struct Steps {
    count: u32,
    offset: Vector3,
}

impl Context<'_> {
    fn error(&self, reason: String, remedy: &str) -> Failure {
        Failure::Error(Box::new(FeatureError {
            reason,
            remedy: remedy.to_owned(),
            fix: Some(FixTarget::Feature(self.resolver.feature.id())),
            constraints: Vec::new(),
            place: None,
        }))
    }

    fn count(&self, expression: &Expression, what: &str) -> Result<u32, Failure> {
        let value = self.resolver.value(expression, what, Dimension::NONE)?;
        let whole = value.round();
        if (value - whole).abs() > WHOLE_TOLERANCE {
            return Err(self.error(
                format!(
                    "The {what} must be a whole number, and {} is not.",
                    format_number(value)
                ),
                "Enter a whole number, such as 4.",
            ));
        }
        if whole < 1.0 {
            return Err(self.error(
                format!("The {what} must be at least 1."),
                "Enter a count of 1 or more; 1 leaves the body as it is.",
            ));
        }
        if whole > f64::from(MAX_PATTERN_INSTANCES) {
            return Err(self.too_many(whole));
        }
        Ok(whole as u32)
    }

    fn too_many(&self, instances: f64) -> Failure {
        self.error(
            format!(
                "The pattern would make {} instances of {}, and at most \
                 {MAX_PATTERN_INSTANCES} are allowed.",
                format_number(instances),
                self.subject
            ),
            "Lower the count.",
        )
    }

    fn steps(&self, direction: &LinearDirection, which: &str) -> Result<(Ray, Steps), Failure> {
        let count = self.count(&direction.count, &format!("{which}count"))?;
        let spacing = self.resolver.value(
            &direction.spacing,
            &format!("{which}{}", direction.measured.what()),
            Dimension::LENGTH,
        )?;
        if spacing <= 0.0 {
            return Err(self.error(
                format!(
                    "The {which}{} must be more than zero.",
                    direction.measured.what()
                ),
                "Enter a length above zero, and tick Reversed to go the other way.",
            ));
        }
        let spacing = match (direction.measured, count) {
            (LinearSpacing::BetweenCopies, _) | (LinearSpacing::Total, 1) => spacing,
            (LinearSpacing::Total, count) => spacing / f64::from(count - 1),
        };
        if spacing * f64::from(count.saturating_sub(1)) > MAX_SIZE {
            return Err(self.error(
                "The copies would reach farther than a kilometre, the largest size caditor \
                 models."
                    .to_owned(),
                "Lower the count or the spacing.",
            ));
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
                    return Err(self.error(
                        "The two directions are parallel, so the copies would fall on one line."
                            .to_owned(),
                        "Choose a second direction across the first, or remove it.",
                    ));
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
            return Err(self.error(
                "The angle must be more than 0° and at most 360°.".to_owned(),
                "Enter an angle up to 360 deg; a full turn spaces the copies evenly around the \
                 axis.",
            ));
        }
        let axis = self.resolver.axis(&circular.axis)?;
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

    fn copy(
        &self,
        index: [u32; 2],
        placement: Option<RigidTransform>,
    ) -> Result<PatternCopy, Failure> {
        let placement = placement.ok_or_else(|| {
            self.error(
                "A copy could not be placed.".to_owned(),
                "Change the spacing, the angle or the axis.",
            )
        })?;
        Ok(PatternCopy {
            index,
            placement: placement.into(),
        })
    }

    fn failure(&self, error: &PatternError) -> Failure {
        let body = &self.subject;
        match error {
            PatternError::Cancelled(_) => Failure::Cancelled,
            PatternError::Placement { .. } => self.error(
                format!("A copy of {body} could not be placed that far away."),
                "Lower the count or the spacing.",
            ),
            PatternError::Union {
                copies,
                error: BooleanError::NonManifold(_),
            } => self.error(
                format!(
                    "{} of {body} meet only along an edge or at a corner, and could not be \
                     kept as separate shells.",
                    describe_copies(copies)
                ),
                "Change the spacing or the angle so the copies overlap or stand apart.",
            ),
            PatternError::Union {
                copies,
                error: BooleanError::Ambiguous(_),
            } => self.error(
                format!(
                    "{} of {body} touch where it cannot be told which side is inside.",
                    describe_copies(copies)
                ),
                "Change the spacing or the angle slightly.",
            ),
            PatternError::Union { copies, .. } => {
                log::warn!("{} could not be built: {error}", self.resolver.feature.name);
                self.error(
                    format!(
                        "{} of {body} could not be joined together.",
                        describe_copies(copies)
                    ),
                    "Change the spacing or the angle slightly, or lower the count.",
                )
            }
        }
    }
}

pub(crate) struct Seed<'a> {
    pub name: String,
    pub operation: BooleanOperation,
    pub tools: Vec<&'a Solid>,
}

pub(crate) struct SeedWords {
    pub feature: &'static str,
    pub repeats: &'static str,
    pub places: &'static str,
    pub repeated: &'static str,
    pub verb: &'static str,
}

const PATTERN_WORDS: SeedWords = SeedWords {
    feature: "pattern",
    repeats: "repeats",
    places: "repeats it on",
    repeated: "repeated",
    verb: "pattern",
};

fn seed_error(seed: FeatureId, reason: String, remedy: &str) -> Failure {
    Failure::Error(Box::new(FeatureError {
        reason,
        remedy: remedy.to_owned(),
        fix: Some(FixTarget::Feature(seed)),
        constraints: Vec::new(),
        place: None,
    }))
}

pub(crate) fn seed<'a>(
    inputs: &'a Inputs<'_>,
    seed: FeatureId,
    body: FeatureId,
    words: &SeedWords,
) -> Result<Seed<'a>, Failure> {
    let SeedWords {
        feature,
        repeats,
        places,
        repeated,
        verb,
    } = words;
    let name = inputs.document.feature(seed).map_or_else(
        || "a deleted feature".to_owned(),
        |found| found.name.clone(),
    );
    let Some(result) = inputs.features.get(&seed).and_then(|result| result.solid()) else {
        return Err(seed_error(
            seed,
            format!("It {repeats} {name}, whose shape is not available."),
            &format!("Fix {name} first, or leave it out of the {feature}."),
        ));
    };
    if result.body != body {
        return Err(seed_error(
            seed,
            format!("{name} changes another body than the one this {feature} {places}."),
            &format!("Leave it out of the {feature}, or {verb} the body it changes."),
        ));
    }
    let (operation, parts) = match (result.cuts(), result.joins()) {
        ([], []) => {
            return Err(seed_error(
                seed,
                format!(
                    "{name} neither adds to nor removes from a body, so it cannot be {repeated}."
                ),
                &format!("Leave it out of the {feature}, or {verb} the whole body."),
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
    Ok(Seed {
        name,
        operation,
        tools,
    })
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
        let mut body = solid.clone();
        let mut cuts = Vec::new();
        for &feature in &definition.repeated {
            let seed = seed(inputs, feature, definition.body, &PATTERN_WORDS)?;
            let subject = Context {
                resolver: Resolver {
                    feature: self.resolver.feature,
                    inputs,
                },
                subject: seed.name.clone(),
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
        let headline = match (error, seed.operation) {
            (BooleanError::Cancelled(_), _) => return Failure::Cancelled,
            (BooleanError::Empty, _) => {
                return self.error(
                    format!("The copies of {name} would leave nothing of {body}."),
                    "Lower the count or change the spacing.",
                );
            }
            (_, BooleanOperation::Difference) => {
                format!("The copies of {name} could not be cut into {body}.")
            }
            (_, _) => format!("The copies of {name} could not be joined to {body}."),
        };
        log::warn!("{} could not be built: {error}", self.resolver.feature.name);
        let trouble = trouble::boolean_trouble(
            inputs.document,
            operands,
            error,
            "Change the spacing or the angle slightly",
        );
        self.error(trouble.reason(headline), &trouble.remedy)
            .placed(trouble.place)
    }
}

pub fn instance_name(instance: Instance) -> String {
    match instance {
        [0, 0] => "the original".to_owned(),
        [index, 0] => format!("copy {index}"),
        [first, second] => format!("copy ({first}, {second})"),
    }
}

fn describe_copies(copies: &[[u32; 2]]) -> String {
    let named: Vec<String> = copies
        .iter()
        .take(MAX_NAMED_COPIES)
        .map(|copy| instance_name(*copy))
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
        .feature(definition.body)
        .map(|body| body.name.clone())
        .unwrap_or_default();
    let context = Context {
        resolver: Resolver { feature, inputs },
        subject: format!("the body of {body_name}"),
    };
    let mut copies = match &definition.kind {
        PatternKind::Linear { first, second } => context.linear(first, second.as_ref())?,
        PatternKind::Circular(circular) => context.circular(circular)?,
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
