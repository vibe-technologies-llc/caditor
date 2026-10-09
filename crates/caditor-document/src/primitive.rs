use std::collections::BTreeSet;

use caditor_expression::{Dimension, EvalError, Expression, ParameterId};
use caditor_geometry::{Plane, Point2, Point3, Vector2, Vector3};
use caditor_kernel::{
    AngularExtent, Axis2, BooleanError, BooleanOperation, LinearExtent, MAX_SIZE, Profile,
    ProfileCurve, Solid, boolean, extrude, revolve,
};

use crate::{
    datum::{PlaneReference, Resolver},
    document::{Feature, FeatureId},
    recompute::{CancelToken, Failure, FeatureError, FeatureResult, FixTarget, Inputs},
    solid::{BodyOperation, SolidResult},
    trouble::{self, boolean_trouble},
};

const FRONT: u64 = 1;
const RIGHT: u64 = 2;
const BACK: u64 = 3;
const LEFT: u64 = 4;
const ROUND: u64 = 1;
const ON_AXIS: u64 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrimitiveKind {
    Box,
    Cylinder,
    Sphere,
    Torus,
}

impl PrimitiveKind {
    pub const ALL: [Self; 4] = [Self::Box, Self::Cylinder, Self::Sphere, Self::Torus];

    pub fn name(self) -> &'static str {
        match self {
            Self::Box => "Box",
            Self::Cylinder => "Cylinder",
            Self::Sphere => "Sphere",
            Self::Torus => "Torus",
        }
    }

    pub fn noun(self) -> &'static str {
        match self {
            Self::Box => "box",
            Self::Cylinder => "cylinder",
            Self::Sphere => "sphere",
            Self::Torus => "torus",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PrimitiveShape {
    Box {
        length: Expression,
        width: Expression,
        height: Expression,
    },
    Cylinder {
        diameter: Expression,
        height: Expression,
    },
    Sphere {
        diameter: Expression,
    },
    Torus {
        diameter: Expression,
        tube: Expression,
    },
}

impl PrimitiveShape {
    pub fn kind(&self) -> PrimitiveKind {
        match self {
            Self::Box { .. } => PrimitiveKind::Box,
            Self::Cylinder { .. } => PrimitiveKind::Cylinder,
            Self::Sphere { .. } => PrimitiveKind::Sphere,
            Self::Torus { .. } => PrimitiveKind::Torus,
        }
    }

    pub fn sizes(&self) -> Vec<(&'static str, &Expression)> {
        match self {
            Self::Box {
                length,
                width,
                height,
            } => vec![("length", length), ("width", width), ("height", height)],
            Self::Cylinder { diameter, height } => {
                vec![("diameter", diameter), ("height", height)]
            }
            Self::Sphere { diameter } => vec![("diameter", diameter)],
            Self::Torus { diameter, tube } => {
                vec![("diameter", diameter), ("tube diameter", tube)]
            }
        }
    }

    pub fn sizes_mut(&mut self) -> Vec<&mut Expression> {
        match self {
            Self::Box {
                length,
                width,
                height,
            } => vec![length, width, height],
            Self::Cylinder { diameter, height } => vec![diameter, height],
            Self::Sphere { diameter } => vec![diameter],
            Self::Torus { diameter, tube } => vec![diameter, tube],
        }
    }

    pub fn side_name(&self, entity: u64) -> &'static str {
        match (self, entity) {
            (Self::Box { .. }, FRONT) => "front face",
            (Self::Box { .. }, RIGHT) => "right face",
            (Self::Box { .. }, BACK) => "back face",
            (Self::Box { .. }, LEFT) => "left face",
            (Self::Box { .. }, _) => "side face",
            (Self::Cylinder { .. }, _) => "wall",
            (Self::Sphere { .. } | Self::Torus { .. }, _) => "surface",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PrimitiveAnchor {
    Corner,
    #[default]
    BaseCentre,
    Centre,
}

impl PrimitiveAnchor {
    pub const ALL: [Self; 3] = [Self::Corner, Self::BaseCentre, Self::Centre];

    pub fn name(self) -> &'static str {
        match self {
            Self::Corner => "Corner",
            Self::BaseCentre => "Base centre",
            Self::Centre => "Centre",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Primitive {
    pub shape: PrimitiveShape,
    pub plane: PlaneReference,
    pub at: [Expression; 2],
    pub anchor: PrimitiveAnchor,
    pub reversed: bool,
    pub operation: BodyOperation,
}

impl Primitive {
    pub fn expressions(&self) -> Vec<&Expression> {
        self.shape
            .sizes()
            .into_iter()
            .map(|(_, size)| size)
            .chain(self.at.iter())
            .collect()
    }

    pub fn expressions_mut(&mut self) -> Vec<&mut Expression> {
        let mut expressions = self.shape.sizes_mut();
        expressions.extend(self.at.iter_mut());
        expressions
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
        let mut used: BTreeSet<FeatureId> = self.operation.target().into_iter().collect();
        used.extend(self.plane.datum());
        used.extend(self.plane.body());
        used
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        self.plane.origin_features()
    }

    pub fn heap_size(&self) -> usize {
        self.plane.heap_size()
            + self
                .expressions()
                .into_iter()
                .map(Expression::heap_size)
                .sum::<usize>()
    }
}

struct Context<'a> {
    resolver: Resolver<'a>,
    shape: &'static str,
}

impl Context<'_> {
    fn feature(&self) -> &Feature {
        self.resolver.feature
    }

    fn error(&self, reason: String, remedy: String) -> Failure {
        Failure::Error(Box::new(FeatureError {
            reason,
            remedy,
            fix: Some(FixTarget::Feature(self.feature().id())),
            constraints: Vec::new(),
            place: None,
        }))
    }

    fn value(&self, expression: &Expression, what: &str) -> Result<f64, Failure> {
        let parameters = self.resolver.inputs.parameters;
        expression
            .evaluate_as(Dimension::LENGTH, &|id| parameters.value(id))
            .map_err(|error| {
                let (remedy, fix) = match &error {
                    EvalError::ParameterFailed { id, name } => (
                        format!("Fix {name} under Parameters, or edit the {what}."),
                        FixTarget::Parameter(*id),
                    ),
                    EvalError::WrongKind { .. } => (
                        format!("Edit the {what} so it gives a length, such as 10 mm."),
                        FixTarget::Feature(self.feature().id()),
                    ),
                    _ => (
                        format!("Edit the {what} or the parameters it uses."),
                        FixTarget::Feature(self.feature().id()),
                    ),
                };
                Failure::Error(Box::new(FeatureError {
                    reason: format!("The {what} cannot be evaluated: {error}."),
                    remedy,
                    fix: Some(fix),
                    constraints: Vec::new(),
                    place: None,
                }))
            })
    }

    fn within_reach(&self, value: f64, what: &str) -> Result<f64, Failure> {
        if value.abs() <= MAX_SIZE {
            return Ok(value);
        }
        let most = MAX_SIZE / 1_000.0;
        Err(self.error(
            format!("The {what} cannot be more than {most} m."),
            format!("Enter a {what} of at most {most} m."),
        ))
    }

    fn size(&self, expression: &Expression, what: &str) -> Result<f64, Failure> {
        let value = self.value(expression, what)?;
        if value <= 0.0 {
            return Err(self.error(
                format!("The {what} of the {} must be more than zero.", self.shape),
                format!("Enter a {what} above zero."),
            ));
        }
        self.within_reach(value, what)
    }

    fn unbuildable(&self, error: &dyn std::fmt::Display) -> Failure {
        log::warn!("{} could not be built: {error}", self.feature().name);
        self.error(
            format!("The {} cannot be built with these sizes here.", self.shape),
            "Change its sizes or place it nearer the origin.".to_owned(),
        )
    }
}

enum Sizes {
    Box([f64; 3]),
    Cylinder { radius: f64, height: f64 },
    Sphere { radius: f64 },
    Torus { ring: f64, tube: f64 },
}

impl Sizes {
    fn of(context: &Context<'_>, shape: &PrimitiveShape) -> Result<Self, Failure> {
        Ok(match shape {
            PrimitiveShape::Box {
                length,
                width,
                height,
            } => Self::Box([
                context.size(length, "length")?,
                context.size(width, "width")?,
                context.size(height, "height")?,
            ]),
            PrimitiveShape::Cylinder { diameter, height } => Self::Cylinder {
                radius: context.size(diameter, "diameter")? / 2.0,
                height: context.size(height, "height")?,
            },
            PrimitiveShape::Sphere { diameter } => Self::Sphere {
                radius: context.size(diameter, "diameter")? / 2.0,
            },
            PrimitiveShape::Torus { diameter, tube } => {
                let ring = context.size(diameter, "diameter")? / 2.0;
                let tube = context.size(tube, "tube diameter")? / 2.0;
                if tube >= ring {
                    return Err(context.error(
                        "The tube diameter of the torus must be less than its diameter, \
                         measured across the middle of the tube."
                            .to_owned(),
                        "Make the tube thinner or the torus wider.".to_owned(),
                    ));
                }
                Self::Torus { ring, tube }
            }
        })
    }

    fn footprint(&self) -> [f64; 3] {
        match *self {
            Self::Box(extents) => extents,
            Self::Cylinder { radius, height } => [2.0 * radius, 2.0 * radius, height],
            Self::Sphere { radius } => [2.0 * radius; 3],
            Self::Torus { ring, tube } => [2.0 * (ring + tube), 2.0 * (ring + tube), 2.0 * tube],
        }
    }
}

struct Placement {
    plane: Plane,
    up: Vector3,
    low: Point3,
}

impl Placement {
    fn new(
        plane: Plane,
        at: Point2,
        anchor: PrimitiveAnchor,
        reversed: bool,
        size: [f64; 3],
    ) -> Self {
        let [length, width, height] = size;
        let up = if reversed {
            -plane.normal()
        } else {
            plane.normal()
        };
        let (shift, rise) = match anchor {
            PrimitiveAnchor::Corner => (Vector2::ZERO, 0.0),
            PrimitiveAnchor::BaseCentre => (Vector2::new(length, width) / 2.0, 0.0),
            PrimitiveAnchor::Centre => (Vector2::new(length, width) / 2.0, height / 2.0),
        };
        let corner = at - shift;
        Self {
            plane,
            up,
            low: plane.to_world(corner) - up * rise,
        }
    }
}

fn tool(
    context: &Context<'_>,
    sizes: &Sizes,
    placement: &Placement,
    raw: u64,
) -> Result<Solid, Failure> {
    let plane = &placement.plane;
    let normal = plane.normal();
    let x = plane.x_axis();
    let y = plane.y_axis();
    let up = placement.up;
    let unbuildable = |error: &dyn std::fmt::Display| context.unbuildable(error);
    match *sizes {
        Sizes::Box([length, width, height]) => {
            let frame = Plane::from_frame(placement.low, normal, x)
                .ok_or_else(|| unbuildable(&"the plane has no direction"))?;
            let corners = [
                Point2::ZERO,
                Point2::new(length, 0.0),
                Point2::new(length, width),
                Point2::new(0.0, width),
            ];
            let curves: Vec<ProfileCurve> = [FRONT, RIGHT, BACK, LEFT]
                .into_iter()
                .zip(corners.iter().zip(corners.iter().cycle().skip(1)))
                .map(|(entity, (start, end))| ProfileCurve::line(entity, *start, *end))
                .collect();
            let profile = Profile::new(&curves).map_err(|error| unbuildable(&error))?;
            let extent = LinearExtent::new(0.0, height * up.dot(normal))
                .map_err(|error| unbuildable(&error))?;
            extrude(&frame, profile.regions(), extent, raw).map_err(|error| unbuildable(&error))
        }
        Sizes::Cylinder { radius, height } => {
            let frame = Plane::from_frame(placement.low, normal, x)
                .ok_or_else(|| unbuildable(&"the plane has no direction"))?;
            let curves = [ProfileCurve::circle(
                ROUND,
                Point2::new(radius, radius),
                radius,
            )];
            let profile = Profile::new(&curves).map_err(|error| unbuildable(&error))?;
            let extent = LinearExtent::new(0.0, height * up.dot(normal))
                .map_err(|error| unbuildable(&error))?;
            extrude(&frame, profile.regions(), extent, raw).map_err(|error| unbuildable(&error))
        }
        Sizes::Sphere { radius } => {
            let centre = placement.low + (x + y) * radius + up * radius;
            let curves = [
                ProfileCurve::arc(
                    ROUND,
                    Point2::ZERO,
                    Point2::new(0.0, -radius),
                    Point2::new(0.0, radius),
                ),
                ProfileCurve::line(ON_AXIS, Point2::new(0.0, radius), Point2::new(0.0, -radius)),
            ];
            revolved(context, centre, x, normal, &curves, raw)
        }
        Sizes::Torus { ring, tube } => {
            let outer = ring + tube;
            let centre = placement.low + (x + y) * outer + up * tube;
            let curves = [ProfileCurve::circle(ROUND, Point2::new(ring, 0.0), tube)];
            revolved(context, centre, x, normal, &curves, raw)
        }
    }
}

fn revolved(
    context: &Context<'_>,
    centre: Point3,
    across: Vector3,
    up: Vector3,
    curves: &[ProfileCurve],
    raw: u64,
) -> Result<Solid, Failure> {
    let unbuildable = |error: &dyn std::fmt::Display| context.unbuildable(error);
    let plane = Plane::from_frame(centre, across.cross(up), across)
        .ok_or_else(|| unbuildable(&"the plane has no direction"))?;
    let profile = Profile::new(curves).map_err(|error| unbuildable(&error))?;
    let axis = Axis2::new(Point2::ZERO, Vector2::Y).map_err(|error| unbuildable(&error))?;
    revolve(&plane, profile.regions(), axis, AngularExtent::full(), raw)
        .map_err(|error| unbuildable(&error))
}

fn combine_failure(
    context: &Context<'_>,
    operands: [&Solid; 2],
    body: FeatureId,
    operation: BodyOperation,
    error: &BooleanError,
) -> Failure {
    let inputs = context.resolver.inputs;
    let shape = context.shape;
    let body_name = inputs
        .document
        .feature(body)
        .map(|feature| feature.name.clone())
        .unwrap_or_default();
    let failure = match error {
        BooleanError::Cancelled(_) => Failure::Cancelled,
        BooleanError::Empty => {
            let reason = match operation {
                BodyOperation::Intersect(_) => format!(
                    "The {shape} does not overlap the body of {body_name}, so nothing would be \
                     left."
                ),
                _ => format!("The {shape} removes all of the body of {body_name}."),
            };
            context.error(
                reason,
                format!("Move or resize the {shape} so that part of the body stays."),
            )
        }
        BooleanError::NonManifold(_) => context.error(
            format!(
                "The {shape} and the body of {body_name} meet only along an edge or at a point."
            ),
            format!(
                "Move or resize the {shape} so it overlaps the body, or choose New body instead."
            ),
        ),
        BooleanError::Intersection { .. }
        | BooleanError::Split(_)
        | BooleanError::Ambiguous(_)
        | BooleanError::Open(_)
        | BooleanError::Invalid(_) => {
            log::warn!("{} could not be combined: {error}", context.feature().name);
            let trouble = boolean_trouble(
                inputs.document,
                operands,
                error,
                &format!("Move or resize the {shape}"),
            );
            context.error(
                trouble.reason(format!(
                    "The {shape} could not be combined with the body of {body_name}."
                )),
                trouble.remedy,
            )
        }
    };
    failure.placed(trouble::place(error))
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &Primitive,
    inputs: &Inputs<'_>,
    cancel: &CancelToken,
) -> Result<FeatureResult, Failure> {
    let context = Context {
        resolver: Resolver { feature, inputs },
        shape: definition.shape.kind().noun(),
    };
    let sizes = Sizes::of(&context, &definition.shape)?;
    let [along, across] = &definition.at;
    let at = Point2::new(
        context.within_reach(
            context.value(along, "position along X")?,
            "position along X",
        )?,
        context.within_reach(
            context.value(across, "position along Y")?,
            "position along Y",
        )?,
    );
    let plane = context.resolver.plane(&definition.plane)?;
    let placement = Placement::new(
        plane,
        at,
        definition.anchor,
        definition.reversed,
        sizes.footprint(),
    );
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let made = tool(&context, &sizes, &placement, feature.id().raw())?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let operation = definition.operation;
    let Some(body) = operation.target() else {
        return Ok(FeatureResult::Solid(SolidResult::new(feature.id(), made)));
    };
    let current = inputs.body(body).ok_or_else(|| inputs.missing_body(body))?;
    let kernel_operation = match operation {
        BodyOperation::Remove(_) => BooleanOperation::Difference,
        BodyOperation::Intersect(_) => BooleanOperation::Intersection,
        BodyOperation::NewBody | BodyOperation::Add(_) => BooleanOperation::Union,
    };
    let combined = boolean(current, &made, kernel_operation)
        .map_err(|error| combine_failure(&context, [current, &made], body, operation, &error))?;
    let result = SolidResult::new(body, combined);
    Ok(FeatureResult::Solid(match operation {
        BodyOperation::Remove(_) => result.cutting([made]),
        BodyOperation::Add(_) => result.joining([made]),
        BodyOperation::NewBody | BodyOperation::Intersect(_) => result,
    }))
}
