use std::{
    collections::BTreeSet,
    panic::{self, AssertUnwindSafe},
    sync::OnceLock,
};

use caditor_expression::{Dimension, EvalError, Expression, ParameterId};
use caditor_geometry::{Aabb2, Plane, Point2, Vector2};
use caditor_kernel::{
    AngularExtent, Axis2, BooleanError, BooleanOperation, GeometryError, LinearExtent, MAX_SIZE,
    Mesh, Profile, ProfileCurve, ProfileError, Region, RegionKey, RegionMesh, SamplingTolerance,
    Selection, Solid, SweepError, TessellationError, boolean, extrude, revolve,
};
use caditor_sketch::{Entity, EntityId, Reference, Sketch};

use crate::{
    datum::{AxisReference, Resolver, capitalized, describe_axis},
    document::{Feature, FeatureId},
    recompute::{
        CancelToken, Failure, FeatureError, FeatureResult, FixTarget, Inputs, SketchResult,
    },
    tolerance,
    values::ParameterValues,
};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RegionChoice {
    #[default]
    All,
    Chosen(Vec<RegionKey>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BodyOperation {
    NewBody,
    Add(FeatureId),
    Remove(FeatureId),
    Intersect(FeatureId),
}

impl BodyOperation {
    pub fn target(self) -> Option<FeatureId> {
        match self {
            Self::NewBody => None,
            Self::Add(body) | Self::Remove(body) | Self::Intersect(body) => Some(body),
        }
    }

    #[must_use]
    pub fn with_target(self, body: FeatureId) -> Self {
        match self {
            Self::NewBody => Self::NewBody,
            Self::Add(_) => Self::Add(body),
            Self::Remove(_) => Self::Remove(body),
            Self::Intersect(_) => Self::Intersect(body),
        }
    }

    pub fn verb(self) -> &'static str {
        match self {
            Self::NewBody => "New body",
            Self::Add(_) => "Add",
            Self::Remove(_) => "Remove",
            Self::Intersect(_) => "Intersect",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExtrudeExtent {
    OneSide {
        distance: Expression,
        reversed: bool,
    },
    Symmetric {
        distance: Expression,
    },
    TwoSides {
        forward: Expression,
        backward: Expression,
    },
}

impl ExtrudeExtent {
    fn expressions(&self) -> Vec<&Expression> {
        match self {
            Self::OneSide { distance, .. } | Self::Symmetric { distance } => vec![distance],
            Self::TwoSides { forward, backward } => vec![forward, backward],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RevolveExtent {
    Full,
    OneSide { angle: Expression, reversed: bool },
    Symmetric { angle: Expression },
}

impl RevolveExtent {
    fn expressions(&self) -> Vec<&Expression> {
        match self {
            Self::Full => Vec::new(),
            Self::OneSide { angle, .. } | Self::Symmetric { angle } => vec![angle],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Extrude {
    pub sketch: FeatureId,
    pub regions: RegionChoice,
    pub extent: ExtrudeExtent,
    pub operation: BodyOperation,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RevolveAxis {
    Sketch(EntityId),
    Model(AxisReference),
}

impl RevolveAxis {
    pub fn line(&self) -> Option<EntityId> {
        match self {
            Self::Sketch(line) => Some(*line),
            Self::Model(_) => None,
        }
    }

    pub fn model(&self) -> Option<&AxisReference> {
        match self {
            Self::Model(axis) => Some(axis),
            Self::Sketch(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Revolve {
    pub sketch: FeatureId,
    pub regions: RegionChoice,
    pub axis: RevolveAxis,
    pub extent: RevolveExtent,
    pub operation: BodyOperation,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SolidFeature {
    Extrude(Extrude),
    Revolve(Revolve),
}

impl SolidFeature {
    pub fn sketch(&self) -> FeatureId {
        match self {
            Self::Extrude(extrude) => extrude.sketch,
            Self::Revolve(revolve) => revolve.sketch,
        }
    }

    pub fn regions(&self) -> &RegionChoice {
        match self {
            Self::Extrude(extrude) => &extrude.regions,
            Self::Revolve(revolve) => &revolve.regions,
        }
    }

    pub fn operation(&self) -> BodyOperation {
        match self {
            Self::Extrude(extrude) => extrude.operation,
            Self::Revolve(revolve) => revolve.operation,
        }
    }

    pub fn axis(&self) -> Option<&RevolveAxis> {
        match self {
            Self::Extrude(_) => None,
            Self::Revolve(revolve) => Some(&revolve.axis),
        }
    }

    pub fn axis_line(&self) -> Option<EntityId> {
        self.axis().and_then(RevolveAxis::line)
    }

    pub fn axis_body(&self) -> Option<FeatureId> {
        self.axis()
            .and_then(RevolveAxis::model)
            .and_then(AxisReference::body)
    }

    pub fn axis_datum(&self) -> Option<FeatureId> {
        self.axis()
            .and_then(RevolveAxis::model)
            .and_then(AxisReference::datum)
    }

    pub fn shape(&self) -> &'static str {
        match self {
            Self::Extrude(_) => "extrusion",
            Self::Revolve(_) => "revolution",
        }
    }

    pub fn same_kind(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::Extrude(_), Self::Extrude(_)) | (Self::Revolve(_), Self::Revolve(_))
        )
    }

    fn expressions(&self) -> Vec<&Expression> {
        match self {
            Self::Extrude(extrude) => extrude.extent.expressions(),
            Self::Revolve(revolve) => revolve.extent.expressions(),
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

    pub fn features(&self) -> BTreeSet<FeatureId> {
        std::iter::once(self.sketch())
            .chain(self.operation().target())
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SolidResult {
    pub body: FeatureId,
    pub solid: Solid,
    mesh: OnceLock<Option<Mesh>>,
}

impl SolidResult {
    pub fn new(body: FeatureId, solid: Solid) -> Self {
        Self {
            body,
            solid,
            mesh: OnceLock::new(),
        }
    }

    pub fn mesh(&self) -> Option<&Mesh> {
        self.mesh.get().and_then(Option::as_ref)
    }

    pub fn is_meshed(&self) -> bool {
        self.mesh.get().is_some()
    }

    pub fn mesh_failed(&self) -> bool {
        matches!(self.mesh.get(), Some(None))
    }

    pub(crate) fn tessellate(&self, name: &str) {
        if self.is_meshed() {
            return;
        }
        let tessellated = panic::catch_unwind(AssertUnwindSafe(|| {
            self.solid.tessellate(&self.solid.default_tolerance())
        }));
        let mesh = match tessellated {
            Ok(Ok(mesh)) => Some(mesh),
            Ok(Err(TessellationError::Cancelled(_))) => return,
            Ok(Err(error)) => {
                log::warn!("the body of {name} could not be meshed: {error}");
                None
            }
            Err(_) => {
                log::error!("meshing the body of {name} panicked");
                None
            }
        };
        let _ = self.mesh.set(mesh);
    }
}

pub(crate) fn profile_curves(sketch: &Sketch) -> Vec<ProfileCurve> {
    sketch
        .entities()
        .filter_map(|(id, entity)| {
            let raw = id.raw();
            match entity {
                Entity::Point(_) => None,
                Entity::Line { .. } => sketch
                    .line_endpoints(id)
                    .map(|(start, end)| ProfileCurve::line(raw, start, end)),
                Entity::Circle { .. } => sketch
                    .circle(id)
                    .map(|(center, radius)| ProfileCurve::circle(raw, center, radius)),
                Entity::Arc { center, start, end } => Some(ProfileCurve::arc(
                    raw,
                    sketch.point(*center)?,
                    sketch.point(*start)?,
                    sketch.point(*end)?,
                )),
                Entity::Spline { .. } => sketch.spline(id).map(|spline| {
                    ProfileCurve::spline(
                        raw,
                        spline.degree(),
                        spline.knots().to_vec(),
                        spline.control_points().to_vec(),
                    )
                }),
            }
        })
        .collect()
}

pub fn sketch_regions(sketch: &Sketch) -> Result<Vec<Region>, ProfileError> {
    Ok(Profile::new(&profile_curves(sketch))?.regions().to_vec())
}

#[derive(Debug, Clone, PartialEq)]
pub struct SketchRegion {
    pub region: Region,
    pub mesh: Option<RegionMesh>,
    pub even_depth: bool,
}

pub(crate) fn display_regions(profile: &Profile) -> Result<Vec<SketchRegion>, ProfileError> {
    let extent = profile
        .regions()
        .iter()
        .filter_map(Region::bounds)
        .reduce(Aabb2::union)
        .map_or(1.0, |bounds| bounds.size().length());
    let tolerance = SamplingTolerance::for_extent(extent);
    Ok(profile
        .regions()
        .iter()
        .map(|region| SketchRegion {
            mesh: region.triangulate(&tolerance),
            even_depth: region.depth() % 2 == 0,
            region: region.clone(),
        })
        .collect())
}

struct Context<'a> {
    feature: &'a Feature,
    sketch_name: String,
    sketch_id: FeatureId,
    sketch: &'a Sketch,
}

impl Context<'_> {
    fn error(&self, reason: String, remedy: String, fix: FixTarget) -> Failure {
        Failure::Error(FeatureError {
            reason,
            remedy,
            fix: Some(fix),
            constraints: Vec::new(),
        })
    }

    fn own(&self) -> FixTarget {
        FixTarget::Feature(self.feature.id())
    }

    fn in_sketch(&self) -> FixTarget {
        FixTarget::Feature(self.sketch_id)
    }

    fn curves(&self, entities: &[u64]) -> String {
        let labels: Vec<String> = entities
            .iter()
            .map(|raw| self.sketch.entity_label(EntityId::from_raw(*raw)))
            .collect();
        crate::document::list_names(&labels)
    }
}

pub(crate) fn evaluate(
    feature: &Feature,
    solid: &SolidFeature,
    inputs: &Inputs<'_>,
    cancel: &CancelToken,
) -> Result<FeatureResult, Failure> {
    let sketch_id = solid.sketch();
    let sketch_name = inputs
        .document
        .feature(sketch_id)
        .map(|sketch| sketch.name.clone())
        .unwrap_or_default();
    let Some(FeatureResult::Sketch(sketch)) = inputs.features.get(&sketch_id).map(AsRef::as_ref)
    else {
        return Err(Failure::Error(FeatureError {
            reason: format!("It needs the shape of {sketch_name}, which is not available."),
            remedy: format!("Fix {sketch_name} first."),
            fix: Some(FixTarget::Feature(sketch_id)),
            constraints: Vec::new(),
        }));
    };
    let context = Context {
        feature,
        sketch_name,
        sketch_id,
        sketch: &sketch.geometry,
    };
    let regions = chosen_regions(&context, sketch, solid.regions())?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let plane = sketch.geometry.plane();
    let raw = feature.id().raw();
    let tool = match solid {
        SolidFeature::Extrude(definition) => {
            let extent = linear_extent(&context, &definition.extent, inputs.parameters)?;
            extrude(&plane, &regions, extent, raw)
        }
        SolidFeature::Revolve(definition) => {
            let axis = match &definition.axis {
                RevolveAxis::Sketch(line) => revolution_axis(&context, *line)?,
                RevolveAxis::Model(reference) => {
                    model_axis(&context, feature, inputs, &plane, reference)?
                }
            };
            let extent = angular_extent(&context, &definition.extent, inputs.parameters)?;
            revolve(&plane, &regions, axis, extent, raw)
        }
    }
    .map_err(|error| sweep_failure(&context, solid.shape(), &error))?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let (body, solid) = match solid.operation() {
        BodyOperation::NewBody => (feature.id(), tool),
        operation @ (BodyOperation::Add(body)
        | BodyOperation::Remove(body)
        | BodyOperation::Intersect(body)) => {
            let current = inputs
                .body(body)
                .ok_or_else(|| missing_body(inputs, body))?;
            let kernel_operation = match operation {
                BodyOperation::Remove(_) => BooleanOperation::Difference,
                BodyOperation::Intersect(_) => BooleanOperation::Intersection,
                BodyOperation::NewBody | BodyOperation::Add(_) => BooleanOperation::Union,
            };
            let combined = boolean(current, &tool, kernel_operation)
                .map_err(|error| boolean_failure(&context, inputs, body, operation, &error))?;
            (body, combined)
        }
    };
    Ok(FeatureResult::Solid(SolidResult::new(body, solid)))
}

fn missing_body(inputs: &Inputs<'_>, body: FeatureId) -> Failure {
    let name = inputs
        .document
        .feature(body)
        .map(|feature| feature.name.clone())
        .unwrap_or_default();
    Failure::Error(FeatureError {
        reason: format!("The body made by {name} has no shape."),
        remedy: format!("Fix {name} first."),
        fix: Some(FixTarget::Feature(body)),
        constraints: Vec::new(),
    })
}

fn chosen_regions(
    context: &Context<'_>,
    sketch: &SketchResult,
    choice: &RegionChoice,
) -> Result<Vec<Region>, Failure> {
    let selection = match choice {
        RegionChoice::All => Selection::EvenDepth,
        RegionChoice::Chosen(keys) => Selection::Regions(keys.clone()),
    };
    sketch
        .profile()
        .map_err(|error| profile_failure(context, error))?
        .select(&selection)
        .map_err(|error| profile_failure(context, &error))
}

fn profile_failure(context: &Context<'_>, error: &ProfileError) -> Failure {
    let sketch = &context.sketch_name;
    match error {
        ProfileError::NoClosedProfile => context.error(
            format!("{sketch} has no closed shape to sweep."),
            format!("Close the outline in {sketch}, for example by joining the ends of its lines."),
            context.in_sketch(),
        ),
        ProfileError::EmptySelection => context.error(
            "No region of the sketch is chosen.".to_owned(),
            "Choose at least one region.".to_owned(),
            context.own(),
        ),
        ProfileError::MissingRegion(_) => context.error(
            format!("A chosen region of {sketch} no longer exists, because the sketch changed."),
            "Choose the regions again.".to_owned(),
            context.own(),
        ),
        ProfileError::Unresolved { entities } if entities.is_empty() => context.error(
            format!("The curves of {sketch} could not be divided into closed regions."),
            format!("Simplify where curves meet or cross in {sketch}."),
            context.in_sketch(),
        ),
        ProfileError::DuplicateEntity { .. }
        | ProfileError::Degenerate { .. }
        | ProfileError::InvalidCurve { .. }
        | ProfileError::SelfOverlap { .. }
        | ProfileError::Overlap { .. }
        | ProfileError::TooIntricate { .. }
        | ProfileError::Unresolved { .. } => {
            let entities = error.entities();
            let curves = context.curves(&entities);
            let problem = match error {
                ProfileError::Degenerate { .. } => "has no length",
                ProfileError::InvalidCurve {
                    error: GeometryError::BeyondMaximum(_),
                    ..
                } => "reaches farther than caditor models",
                ProfileError::SelfOverlap { .. } => "runs back over itself",
                ProfileError::Overlap { .. } => "run along each other",
                ProfileError::TooIntricate { .. } => "cross too often to be divided into regions",
                ProfileError::Unresolved { .. } => "could not be divided into closed regions",
                _ => "cannot be used",
            };
            let which = if entities.len() > 1 {
                "one of them"
            } else {
                "it"
            };
            context.error(
                format!("In {sketch}, {curves} {problem}."),
                format!("Change or delete {which} in {sketch}."),
                context.in_sketch(),
            )
        }
    }
}

fn sweep_failure(context: &Context<'_>, shape: &str, error: &SweepError) -> Failure {
    let sketch = &context.sketch_name;
    match error {
        SweepError::NoRegions => context.error(
            "No region of the sketch is chosen.".to_owned(),
            "Choose at least one region.".to_owned(),
            context.own(),
        ),
        SweepError::NonFinite | SweepError::ZeroLength => context.error(
            format!("The {shape} has no length."),
            "Enter a distance other than zero.".to_owned(),
            context.own(),
        ),
        SweepError::TooLong => context.error(
            format!("The {shape} reaches farther than {} m.", MAX_SIZE / 1_000.0),
            "Enter a shorter distance.".to_owned(),
            context.own(),
        ),
        SweepError::ZeroAngle => context.error(
            "The revolution has no angle.".to_owned(),
            "Enter an angle other than zero.".to_owned(),
            context.own(),
        ),
        SweepError::BeyondFullTurn => context.error(
            "The revolution turns more than once.".to_owned(),
            "Enter an angle of at most 360°.".to_owned(),
            context.own(),
        ),
        SweepError::DegenerateAxis => context.error(
            "The revolution axis has no direction.".to_owned(),
            "Choose a line as the axis.".to_owned(),
            context.own(),
        ),
        SweepError::BothSidesOfAxis { left, right } => {
            let apart = if right.len() <= left.len() {
                right
            } else {
                left
            };
            let verb = if apart.len() == 1 { "lies" } else { "lie" };
            let curves = context.curves(apart);
            context.error(
                format!(
                    "In {sketch}, {curves} {verb} on the other side of the revolution axis from the \
                     rest of the profile."
                ),
                "Choose only the regions on one side of the axis, or choose another axis."
                    .to_owned(),
                context.own(),
            )
        }
        SweepError::CrossesAxis { .. } => {
            let curves = context.curves(&error.entities());
            context.error(
                format!("In {sketch}, {curves} would cut through the revolution axis."),
                format!(
                    "Keep the profile on one side of the axis in {sketch}, or choose another axis."
                ),
                context.in_sketch(),
            )
        }
        SweepError::OnAxis => context.error(
            format!("The profile of {sketch} lies on the revolution axis."),
            "Choose another axis.".to_owned(),
            context.own(),
        ),
        SweepError::Unassembled | SweepError::Geometry(_) | SweepError::Invalid(_) => {
            log::warn!("{} could not be built: {error}", context.feature.name);
            context.error(
                format!("The {shape} of {sketch} could not be built."),
                format!("Undo the last change, or simplify the regions of {sketch}."),
                context.own(),
            )
        }
    }
}

fn boolean_failure(
    context: &Context<'_>,
    inputs: &Inputs<'_>,
    body: FeatureId,
    operation: BodyOperation,
    error: &BooleanError,
) -> Failure {
    let body_name = inputs
        .document
        .feature(body)
        .map(|feature| feature.name.clone())
        .unwrap_or_default();
    match error {
        BooleanError::Cancelled(_) => Failure::Cancelled,
        BooleanError::Empty => {
            let reason = match operation {
                BodyOperation::Intersect(_) => {
                    format!(
                        "The shape does not overlap the body of {body_name}, so nothing would be left."
                    )
                }
                _ => format!("This removes all of the body of {body_name}."),
            };
            context.error(
                reason,
                "Change the distance or the sketch so that part of the body stays.".to_owned(),
                context.own(),
            )
        }
        BooleanError::NonManifold => context.error(
            format!(
                "The body of {body_name} would be left with parts that meet only along an edge."
            ),
            "Move or resize the shape so it overlaps more or stays clear.".to_owned(),
            context.own(),
        ),
        BooleanError::Intersection(_)
        | BooleanError::Split
        | BooleanError::Ambiguous
        | BooleanError::Open
        | BooleanError::Invalid(_) => {
            log::warn!("{} could not be combined: {error}", context.feature.name);
            context.error(
                format!("The shape could not be combined with the body of {body_name}."),
                "Move or resize the shape slightly; faces or edges that exactly touch can cause this."
                    .to_owned(),
                context.own(),
            )
        }
    }
}

fn evaluate_value(
    context: &Context<'_>,
    expression: &Expression,
    dimension: Dimension,
    what: &str,
    parameters: &ParameterValues,
) -> Result<f64, Failure> {
    expression
        .evaluate_as(dimension, &|id| parameters.value(id))
        .map_err(|error| {
            let (remedy, fix) = match &error {
                EvalError::ParameterFailed { id, name } => (
                    format!("Fix {name} under Parameters, or edit the {what}."),
                    FixTarget::Parameter(*id),
                ),
                EvalError::WrongKind { .. } if dimension == Dimension::ANGLE => (
                    format!("Edit the {what} so it gives an angle, such as 90 deg."),
                    context.own(),
                ),
                EvalError::WrongKind { .. } => (
                    format!("Edit the {what} so it gives a length, such as 10 mm."),
                    context.own(),
                ),
                _ => (
                    format!("Edit the {what} or the parameters it uses."),
                    context.own(),
                ),
            };
            context.error(
                format!("The {what} cannot be evaluated: {error}."),
                remedy,
                fix,
            )
        })
}

fn positive(context: &Context<'_>, value: f64, what: &str) -> Result<f64, Failure> {
    if value > 0.0 {
        Ok(value)
    } else {
        Err(context.error(
            format!("The {what} must be more than zero."),
            format!("Enter a {what} above zero, or flip the direction instead."),
            context.own(),
        ))
    }
}

fn within_reach(context: &Context<'_>, value: f64, what: &str) -> Result<f64, Failure> {
    if value <= MAX_SIZE {
        return Ok(value);
    }
    let most = MAX_SIZE / 1_000.0;
    Err(context.error(
        format!("The {what} cannot be more than {most} m."),
        format!("Enter a {what} of at most {most} m."),
        context.own(),
    ))
}

fn linear_extent(
    context: &Context<'_>,
    extent: &ExtrudeExtent,
    parameters: &ParameterValues,
) -> Result<LinearExtent, Failure> {
    let length = |expression: &Expression, what: &str| {
        evaluate_value(context, expression, Dimension::LENGTH, what, parameters)
            .and_then(|value| positive(context, value, what))
            .and_then(|value| within_reach(context, value, what))
    };
    let built = match extent {
        ExtrudeExtent::OneSide { distance, reversed } => {
            let distance = length(distance, "distance")?;
            LinearExtent::one_side(if *reversed { -distance } else { distance })
        }
        ExtrudeExtent::Symmetric { distance } => {
            LinearExtent::symmetric(length(distance, "distance")?)
        }
        ExtrudeExtent::TwoSides { forward, backward } => LinearExtent::two_sided(
            length(forward, "forward distance")?,
            length(backward, "backward distance")?,
        ),
    };
    built.map_err(|_| {
        context.error(
            "The extrusion has no length.".to_owned(),
            "Enter distances that do not cancel each other out.".to_owned(),
            context.own(),
        )
    })
}

fn angular_extent(
    context: &Context<'_>,
    extent: &RevolveExtent,
    parameters: &ParameterValues,
) -> Result<AngularExtent, Failure> {
    let angle = |expression: &Expression| {
        evaluate_value(context, expression, Dimension::ANGLE, "angle", parameters)
            .and_then(|degrees| positive(context, degrees, "angle"))
            .map(f64::to_radians)
    };
    let built = match extent {
        RevolveExtent::Full => Ok(AngularExtent::full()),
        RevolveExtent::OneSide {
            angle: value,
            reversed,
        } => {
            let radians = angle(value)?;
            AngularExtent::one_side(if *reversed { -radians } else { radians })
        }
        RevolveExtent::Symmetric { angle: value } => AngularExtent::symmetric(angle(value)?),
    };
    built.map_err(|error| sweep_failure(context, "revolution", &error))
}

fn model_axis(
    context: &Context<'_>,
    feature: &Feature,
    inputs: &Inputs<'_>,
    plane: &Plane,
    reference: &AxisReference,
) -> Result<Axis2, Failure> {
    let axis = Resolver { feature, inputs }.axis(reference)?;
    let in_plane = tolerance::along_plane(axis, plane);
    let origin = plane.to_local(axis.origin());
    let direction = plane.to_local(axis.origin() + axis.direction()) - origin;
    match Axis2::new(origin, direction) {
        Ok(found) if in_plane => Ok(found),
        _ => Err(context.error(
            format!(
                "{} does not lie in the plane of {}, so the sketch cannot turn about it.",
                capitalized(&describe_axis(inputs.document, reference)),
                context.sketch_name
            ),
            "Choose an axis that lies in the sketch plane, or a line of the sketch.".to_owned(),
            context.own(),
        )),
    }
}

fn revolution_axis(context: &Context<'_>, axis: EntityId) -> Result<Axis2, Failure> {
    let sketch = context.sketch;
    let found = match axis.reference() {
        Some(Reference::HorizontalAxis) => Axis2::new(Point2::ZERO, Vector2::X).ok(),
        Some(Reference::VerticalAxis) => Axis2::new(Point2::ZERO, Vector2::Y).ok(),
        Some(Reference::Origin) => None,
        None => sketch
            .line_endpoints(axis)
            .and_then(|(start, end)| Axis2::through(start, end).ok()),
    };
    found.ok_or_else(|| {
        context.error(
            format!(
                "The revolution axis, {}, is not a line of {}.",
                sketch.entity_label(axis),
                context.sketch_name
            ),
            "Choose a line or one of the sketch axes as the revolution axis.".to_owned(),
            context.own(),
        )
    })
}
