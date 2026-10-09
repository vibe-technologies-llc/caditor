use std::{
    collections::BTreeSet,
    f64::consts::{FRAC_PI_2, PI, TAU},
};

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
const BASE: u64 = 3;
const TOP: u64 = 4;
const WEDGE_BOTTOM: u64 = 1;
const WEDGE_SLOPE: u64 = 2;
const WEDGE_TOP: u64 = 3;
const WEDGE_LEFT: u64 = 4;
const WHOLE_TOLERANCE: f64 = 1e-9;
pub const MIN_PRISM_SIDES: u32 = 3;
pub const MAX_PRISM_SIDES: u32 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cap {
    Start,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeRule {
    AboveZero,
    ZeroOrMore,
    Sides,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrimitiveKind {
    Box,
    Cylinder,
    Sphere,
    Torus,
    Cone,
    Wedge,
    Prism,
}

impl PrimitiveKind {
    pub const ALL: [Self; 7] = [
        Self::Box,
        Self::Cylinder,
        Self::Sphere,
        Self::Torus,
        Self::Cone,
        Self::Wedge,
        Self::Prism,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Box => "Box",
            Self::Cylinder => "Cylinder",
            Self::Sphere => "Sphere",
            Self::Torus => "Torus",
            Self::Cone => "Cone",
            Self::Wedge => "Wedge",
            Self::Prism => "Prism",
        }
    }

    pub fn noun(self) -> &'static str {
        match self {
            Self::Box => "box",
            Self::Cylinder => "cylinder",
            Self::Sphere => "sphere",
            Self::Torus => "torus",
            Self::Cone => "cone",
            Self::Wedge => "wedge",
            Self::Prism => "prism",
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
    Cone {
        bottom: Expression,
        top: Expression,
        height: Expression,
    },
    Wedge {
        length: Expression,
        width: Expression,
        height: Expression,
        top: Expression,
    },
    Prism {
        sides: Expression,
        diameter: Expression,
        height: Expression,
    },
}

impl PrimitiveShape {
    pub fn kind(&self) -> PrimitiveKind {
        match self {
            Self::Box { .. } => PrimitiveKind::Box,
            Self::Cylinder { .. } => PrimitiveKind::Cylinder,
            Self::Sphere { .. } => PrimitiveKind::Sphere,
            Self::Torus { .. } => PrimitiveKind::Torus,
            Self::Cone { .. } => PrimitiveKind::Cone,
            Self::Wedge { .. } => PrimitiveKind::Wedge,
            Self::Prism { .. } => PrimitiveKind::Prism,
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
            Self::Cone {
                bottom,
                top,
                height,
            } => vec![
                ("bottom diameter", bottom),
                ("top diameter", top),
                ("height", height),
            ],
            Self::Wedge {
                length,
                width,
                height,
                top,
            } => vec![
                ("length", length),
                ("width", width),
                ("height", height),
                ("top length", top),
            ],
            Self::Prism {
                sides,
                diameter,
                height,
            } => vec![
                ("number of sides", sides),
                ("diameter", diameter),
                ("height", height),
            ],
        }
    }

    pub fn rules(&self) -> Vec<SizeRule> {
        match self {
            Self::Box { .. } => vec![SizeRule::AboveZero; 3],
            Self::Cylinder { .. } | Self::Torus { .. } => vec![SizeRule::AboveZero; 2],
            Self::Sphere { .. } => vec![SizeRule::AboveZero],
            Self::Cone { .. } => vec![
                SizeRule::ZeroOrMore,
                SizeRule::ZeroOrMore,
                SizeRule::AboveZero,
            ],
            Self::Wedge { .. } => vec![
                SizeRule::AboveZero,
                SizeRule::AboveZero,
                SizeRule::AboveZero,
                SizeRule::ZeroOrMore,
            ],
            Self::Prism { .. } => vec![SizeRule::Sides, SizeRule::AboveZero, SizeRule::AboveZero],
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
            Self::Cone {
                bottom,
                top,
                height,
            } => vec![bottom, top, height],
            Self::Wedge {
                length,
                width,
                height,
                top,
            } => vec![length, width, height, top],
            Self::Prism {
                sides,
                diameter,
                height,
            } => vec![sides, diameter, height],
        }
    }

    pub fn side_name(&self, entity: u64) -> &'static str {
        match (self, entity) {
            (Self::Box { .. }, FRONT) => "front face",
            (Self::Box { .. }, RIGHT) => "right face",
            (Self::Box { .. }, BACK) => "back face",
            (Self::Box { .. }, LEFT) => "left face",
            (Self::Box { .. } | Self::Prism { .. }, _) => "side face",
            (Self::Cone { .. }, BASE) => "bottom face",
            (Self::Cone { .. }, TOP) => "top face",
            (Self::Cylinder { .. } | Self::Cone { .. }, _) => "wall",
            (Self::Sphere { .. } | Self::Torus { .. }, _) => "surface",
            (Self::Wedge { .. }, WEDGE_BOTTOM) => "bottom face",
            (Self::Wedge { .. }, WEDGE_SLOPE) => "sloped face",
            (Self::Wedge { .. }, WEDGE_TOP) => "top face",
            (Self::Wedge { .. }, WEDGE_LEFT) => "left face",
            (Self::Wedge { .. }, _) => "side face",
        }
    }

    pub fn cap_name(&self, cap: Cap) -> &'static str {
        match (self, cap) {
            (Self::Wedge { .. }, Cap::Start) => "front face",
            (Self::Wedge { .. }, Cap::End) => "back face",
            (_, Cap::Start) => "bottom face",
            (_, Cap::End) => "top face",
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
        self.value_as(expression, what, Dimension::LENGTH)
    }

    fn value_as(
        &self,
        expression: &Expression,
        what: &str,
        dimension: Dimension,
    ) -> Result<f64, Failure> {
        let parameters = self.resolver.inputs.parameters;
        let example = if dimension == Dimension::LENGTH {
            "a length, such as 10 mm"
        } else {
            "a plain number, such as 6"
        };
        expression
            .evaluate_as(dimension, &|id| parameters.value(id))
            .map_err(|error| {
                let (remedy, fix) = match &error {
                    EvalError::ParameterFailed { id, name } => (
                        format!("Fix {name} under Parameters, or edit the {what}."),
                        FixTarget::Parameter(*id),
                    ),
                    EvalError::WrongKind { .. } => (
                        format!("Edit the {what} so it gives {example}."),
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

    fn size_or_zero(&self, expression: &Expression, what: &str) -> Result<f64, Failure> {
        let value = self.value(expression, what)?;
        if value < 0.0 {
            return Err(self.error(
                format!("The {what} of the {} cannot be negative.", self.shape),
                format!("Enter a {what} of zero or more."),
            ));
        }
        self.within_reach(value, what)
    }

    fn sides(&self, expression: &Expression) -> Result<u32, Failure> {
        let what = "number of sides";
        let value = self.value_as(expression, what, Dimension::NONE)?;
        let whole = value.round();
        if (value - whole).abs() > WHOLE_TOLERANCE
            || whole < f64::from(MIN_PRISM_SIDES)
            || whole > f64::from(MAX_PRISM_SIDES)
        {
            return Err(self.error(
                format!(
                    "The {what} of the prism must be a whole number from {MIN_PRISM_SIDES} to \
                     {MAX_PRISM_SIDES}."
                ),
                format!("Enter a whole number from {MIN_PRISM_SIDES} to {MAX_PRISM_SIDES}."),
            ));
        }
        Ok(whole as u32)
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
    Cylinder {
        radius: f64,
        height: f64,
    },
    Sphere {
        radius: f64,
    },
    Torus {
        ring: f64,
        tube: f64,
    },
    Cone {
        bottom: f64,
        top: f64,
        height: f64,
    },
    Wedge {
        extents: [f64; 3],
        top: f64,
    },
    Prism {
        sides: u32,
        radius: f64,
        height: f64,
    },
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
            PrimitiveShape::Cone {
                bottom,
                top,
                height,
            } => {
                let bottom = context.size_or_zero(bottom, "bottom diameter")? / 2.0;
                let top = context.size_or_zero(top, "top diameter")? / 2.0;
                let height = context.size(height, "height")?;
                if bottom <= 0.0 && top <= 0.0 {
                    return Err(context.error(
                        "The bottom and top diameters of the cone cannot both be zero.".to_owned(),
                        "Enter a diameter above zero for the bottom or the top.".to_owned(),
                    ));
                }
                Self::Cone {
                    bottom,
                    top,
                    height,
                }
            }
            PrimitiveShape::Wedge {
                length,
                width,
                height,
                top,
            } => {
                let extents = [
                    context.size(length, "length")?,
                    context.size(width, "width")?,
                    context.size(height, "height")?,
                ];
                let top = context.size_or_zero(top, "top length")?;
                if top > extents[0] {
                    return Err(context.error(
                        "The top length of the wedge cannot be more than its length.".to_owned(),
                        "Make the top shorter or the wedge longer.".to_owned(),
                    ));
                }
                Self::Wedge { extents, top }
            }
            PrimitiveShape::Prism {
                sides,
                diameter,
                height,
            } => Self::Prism {
                sides: context.sides(sides)?,
                radius: context.size(diameter, "diameter")? / 2.0,
                height: context.size(height, "height")?,
            },
        })
    }

    fn footprint(&self) -> [f64; 3] {
        match *self {
            Self::Box(extents) => extents,
            Self::Cylinder { radius, height } => [2.0 * radius, 2.0 * radius, height],
            Self::Sphere { radius } => [2.0 * radius; 3],
            Self::Torus { ring, tube } => [2.0 * (ring + tube), 2.0 * (ring + tube), 2.0 * tube],
            Self::Cone {
                bottom,
                top,
                height,
            } => {
                let widest = 2.0 * bottom.max(top);
                [widest, widest, height]
            }
            Self::Wedge { extents, .. } => extents,
            Self::Prism { radius, height, .. } => [2.0 * radius, 2.0 * radius, height],
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
        Sizes::Cone {
            bottom,
            top,
            height,
        } => {
            let widest = bottom.max(top);
            let centre = placement.low + (x + y) * widest;
            let mut curves = vec![ProfileCurve::line(
                ROUND,
                Point2::new(bottom, 0.0),
                Point2::new(top, height),
            )];
            if top > 0.0 {
                curves.push(ProfileCurve::line(
                    TOP,
                    Point2::new(top, height),
                    Point2::new(0.0, height),
                ));
            }
            curves.push(ProfileCurve::line(
                ON_AXIS,
                Point2::new(0.0, height),
                Point2::ZERO,
            ));
            if bottom > 0.0 {
                curves.push(ProfileCurve::line(
                    BASE,
                    Point2::ZERO,
                    Point2::new(bottom, 0.0),
                ));
            }
            revolved(context, centre, x, up, &curves, raw)
        }
        Sizes::Wedge {
            extents: [length, width, height],
            top,
        } => {
            let across = x.cross(up);
            let frame = Plane::from_frame(placement.low, across, x)
                .ok_or_else(|| unbuildable(&"the plane has no direction"))?;
            let mut curves = vec![
                ProfileCurve::line(WEDGE_BOTTOM, Point2::ZERO, Point2::new(length, 0.0)),
                ProfileCurve::line(
                    WEDGE_SLOPE,
                    Point2::new(length, 0.0),
                    Point2::new(top, height),
                ),
            ];
            if top > 0.0 {
                curves.push(ProfileCurve::line(
                    WEDGE_TOP,
                    Point2::new(top, height),
                    Point2::new(0.0, height),
                ));
            }
            curves.push(ProfileCurve::line(
                WEDGE_LEFT,
                Point2::new(0.0, height),
                Point2::ZERO,
            ));
            let profile = Profile::new(&curves).map_err(|error| unbuildable(&error))?;
            let extent = LinearExtent::new(0.0, width * y.dot(across))
                .map_err(|error| unbuildable(&error))?;
            extrude(&frame, profile.regions(), extent, raw).map_err(|error| unbuildable(&error))
        }
        Sizes::Prism {
            sides,
            radius,
            height,
        } => {
            let frame = Plane::from_frame(placement.low, normal, x)
                .ok_or_else(|| unbuildable(&"the plane has no direction"))?;
            let centre = Point2::new(radius, radius);
            let step = TAU / f64::from(sides);
            let first = -FRAC_PI_2 - PI / f64::from(sides);
            let corners: Vec<Point2> = (0..sides)
                .map(|corner| {
                    let angle = first + step * f64::from(corner);
                    centre + Vector2::new(angle.cos(), angle.sin()) * radius
                })
                .collect();
            let curves: Vec<ProfileCurve> = (1..=u64::from(sides))
                .zip(corners.iter().zip(corners.iter().cycle().skip(1)))
                .map(|(entity, (start, end))| ProfileCurve::line(entity, *start, *end))
                .collect();
            let profile = Profile::new(&curves).map_err(|error| unbuildable(&error))?;
            let extent = LinearExtent::new(0.0, height * up.dot(normal))
                .map_err(|error| unbuildable(&error))?;
            extrude(&frame, profile.regions(), extent, raw).map_err(|error| unbuildable(&error))
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
