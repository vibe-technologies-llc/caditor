use std::collections::BTreeSet;

use caditor_expression::{Dimension, EvalError, Expression, ParameterId, Unit};
use caditor_geometry::{Plane, Point3, Ray, RigidTransform, Vector3};
use caditor_kernel::Solid;

use crate::{
    datum::{AxisReference, Resolver},
    document::{Feature, FeatureId},
    recompute::{CancelToken, Failure, FeatureError, FeatureResult, FixTarget, Inputs},
    solid::SolidResult,
    values::ParameterValues,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MoveAxis {
    X,
    Y,
    Z,
}

impl MoveAxis {
    pub const ALL: [Self; 3] = [Self::X, Self::Y, Self::Z];

    pub fn name(self) -> &'static str {
        match self {
            Self::X => "X",
            Self::Y => "Y",
            Self::Z => "Z",
        }
    }

    pub fn index(self) -> usize {
        match self {
            Self::X => 0,
            Self::Y => 1,
            Self::Z => 2,
        }
    }

    pub fn of<T>(self, values: &[T; 3]) -> &T {
        let [x, y, z] = values;
        match self {
            Self::X => x,
            Self::Y => y,
            Self::Z => z,
        }
    }

    pub fn of_mut<T>(self, values: &mut [T; 3]) -> &mut T {
        let [x, y, z] = values;
        match self {
            Self::X => x,
            Self::Y => y,
            Self::Z => z,
        }
    }

    pub fn direction(self) -> Vector3 {
        match self {
            Self::X => Vector3::X,
            Self::Y => Vector3::Y,
            Self::Z => Vector3::Z,
        }
    }

    pub fn direction_in(self, frame: Option<&Plane>) -> Vector3 {
        match (frame, self) {
            (None, _) => self.direction(),
            (Some(frame), Self::X) => frame.x_axis(),
            (Some(frame), Self::Y) => frame.y_axis(),
            (Some(frame), Self::Z) => frame.normal(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum TurnCentre {
    #[default]
    Origin,
    Body,
    Axis(Box<AxisTurn>),
}

impl TurnCentre {
    pub fn axis_turn(&self) -> Option<&AxisTurn> {
        match self {
            Self::Axis(turn) => Some(turn),
            Self::Origin | Self::Body => None,
        }
    }

    pub fn axis(&self) -> Option<&AxisReference> {
        self.axis_turn().map(|turn| &turn.axis)
    }

    pub fn heap_size(&self) -> usize {
        self.axis_turn().map_or(0, |turn| {
            size_of::<AxisTurn>() + turn.axis.heap_size() + turn.angle.heap_size()
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AxisTurn {
    pub axis: AxisReference,
    pub angle: Expression,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pivot {
    Point(Point3),
    Axis(Ray),
}

impl Pivot {
    pub fn point(self) -> Point3 {
        match self {
            Self::Point(point) => point,
            Self::Axis(axis) => axis.origin(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Move {
    pub body: FeatureId,
    pub offset: [Expression; 3],
    pub turn: [Expression; 3],
    pub copy: bool,
    pub about: TurnCentre,
    pub frame: Option<FeatureId>,
}

impl Move {
    pub fn expressions(&self) -> impl Iterator<Item = &Expression> {
        self.offset
            .iter()
            .chain(self.turn.iter())
            .chain(self.about.axis_turn().map(|turn| &turn.angle))
    }

    pub fn expressions_mut(&mut self) -> impl Iterator<Item = &mut Expression> {
        let angle = match &mut self.about {
            TurnCentre::Axis(turn) => Some(&mut turn.angle),
            TurnCentre::Origin | TurnCentre::Body => None,
        };
        self.offset
            .iter_mut()
            .chain(self.turn.iter_mut())
            .chain(angle)
    }

    pub fn parameters(&self) -> BTreeSet<ParameterId> {
        self.expressions()
            .flat_map(|expression| expression.parameters())
            .collect()
    }

    pub fn uses_parameter(&self, parameter: ParameterId) -> bool {
        self.expressions()
            .any(|expression| expression.uses(parameter))
    }

    pub fn features(&self) -> BTreeSet<FeatureId> {
        let mut used = BTreeSet::from([self.body]);
        used.extend(self.axis_datum());
        used.extend(self.axis_body());
        used.extend(self.axis_sketch());
        used
    }

    pub fn axis_datum(&self) -> Option<FeatureId> {
        self.about.axis().and_then(AxisReference::datum)
    }

    pub fn axis_body(&self) -> Option<FeatureId> {
        self.about.axis().and_then(AxisReference::body)
    }

    pub fn axis_sketch(&self) -> Option<FeatureId> {
        self.about.axis().and_then(AxisReference::sketch)
    }

    pub fn frames(&self) -> BTreeSet<FeatureId> {
        self.frame
            .into_iter()
            .chain(self.about.axis().and_then(AxisReference::frame))
            .collect()
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        self.about
            .axis()
            .map(AxisReference::origin_features)
            .unwrap_or_default()
    }

    pub fn heap_size(&self) -> usize {
        self.offset
            .iter()
            .chain(self.turn.iter())
            .map(Expression::heap_size)
            .sum::<usize>()
            + self.about.heap_size()
    }

    pub fn pivot(&self, body: &Solid, axis: Option<Ray>, frame: Option<&Plane>) -> Option<Pivot> {
        match self.about {
            TurnCentre::Origin => Some(Pivot::Point(frame.map_or(Point3::ZERO, Plane::origin))),
            TurnCentre::Body => Some(Pivot::Point(body.bounding_box()?.center())),
            TurnCentre::Axis(_) => axis.map(Pivot::Axis),
        }
    }

    pub fn placement(
        &self,
        parameters: &ParameterValues,
        pivot: Pivot,
        frame: Option<&Plane>,
    ) -> Option<RigidTransform> {
        placed(
            Placing {
                offset: &self.offset,
                turn: &self.turn,
                pivot,
                angle: self.about.axis_turn().map(|turn| &turn.angle),
                frame,
            },
            |expression, dimension, _| {
                expression
                    .evaluate_as(dimension, &|id| parameters.value(id))
                    .map_err(|_| ())
            },
            || (),
        )
        .ok()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BodyPlacement {
    pub offset: [Expression; 3],
    pub turn: [Expression; 3],
    pub frame: Option<FeatureId>,
}

impl Default for BodyPlacement {
    fn default() -> Self {
        Self {
            offset: std::array::from_fn(|_| Expression::Measure(0.0, Unit::Millimetre)),
            turn: std::array::from_fn(|_| Expression::Measure(0.0, Unit::Degree)),
            frame: None,
        }
    }
}

impl BodyPlacement {
    pub fn is_at_origin(&self) -> bool {
        self.frame.is_none() && self.is_unmoved()
    }

    pub fn is_unmoved(&self) -> bool {
        self.expressions().all(|expression| {
            matches!(expression, Expression::Number(value) | Expression::Measure(value, _) if *value == 0.0)
        })
    }

    pub fn expressions(&self) -> impl Iterator<Item = &Expression> {
        self.offset.iter().chain(self.turn.iter())
    }

    pub fn expressions_mut(&mut self) -> impl Iterator<Item = &mut Expression> {
        self.offset.iter_mut().chain(self.turn.iter_mut())
    }

    pub fn parameters(&self) -> BTreeSet<ParameterId> {
        self.expressions()
            .flat_map(|expression| expression.parameters())
            .collect()
    }

    pub fn uses_parameter(&self, parameter: ParameterId) -> bool {
        self.expressions()
            .any(|expression| expression.uses(parameter))
    }

    pub fn heap_size(&self) -> usize {
        self.expressions().map(Expression::heap_size).sum()
    }

    pub fn transform(&self, parameters: &ParameterValues) -> Option<RigidTransform> {
        placed(
            Placing::at_origin(&self.offset, &self.turn),
            |expression, dimension, _| {
                expression
                    .evaluate_as(dimension, &|id| parameters.value(id))
                    .map_err(|_| ())
            },
            || (),
        )
        .ok()
    }
}

struct Placing<'a> {
    offset: &'a [Expression; 3],
    turn: &'a [Expression; 3],
    pivot: Pivot,
    angle: Option<&'a Expression>,
    frame: Option<&'a Plane>,
}

impl<'a> Placing<'a> {
    fn at_origin(offset: &'a [Expression; 3], turn: &'a [Expression; 3]) -> Self {
        Self {
            offset,
            turn,
            pivot: Pivot::Point(Point3::ZERO),
            angle: None,
            frame: None,
        }
    }
}

pub(crate) struct Context<'a> {
    pub feature: &'a Feature,
    pub inputs: &'a Inputs<'a>,
}

impl Context<'_> {
    pub(crate) fn error(&self, reason: String, remedy: String) -> Failure {
        Failure::Error(Box::new(FeatureError {
            reason,
            remedy,
            fix: Some(FixTarget::Feature(self.feature.id())),
            constraints: Vec::new(),
            place: None,
        }))
    }

    pub(crate) fn value(
        &self,
        expression: &Expression,
        dimension: Dimension,
        what: &str,
    ) -> Result<f64, Failure> {
        expression
            .evaluate_as(dimension, &|id| self.inputs.parameters.value(id))
            .map_err(|error| {
                let (remedy, fix) = match &error {
                    EvalError::ParameterFailed { id, name } => (
                        format!("Fix {name} under Parameters, or edit the {what}."),
                        FixTarget::Parameter(*id),
                    ),
                    EvalError::WrongKind { .. } if dimension == Dimension::ANGLE => (
                        format!("Edit the {what} so it gives an angle, such as 90 deg."),
                        FixTarget::Feature(self.feature.id()),
                    ),
                    EvalError::WrongKind { .. } => (
                        format!("Edit the {what} so it gives a length, such as 10 mm."),
                        FixTarget::Feature(self.feature.id()),
                    ),
                    _ => (
                        format!("Edit the {what} or the parameters it uses."),
                        FixTarget::Feature(self.feature.id()),
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
}

fn transform(
    context: &Context<'_>,
    definition: &Move,
    pivot: Pivot,
    frame: Option<&Plane>,
) -> Result<RigidTransform, Failure> {
    placed(
        Placing {
            offset: &definition.offset,
            turn: &definition.turn,
            pivot,
            angle: definition.about.axis_turn().map(|turn| &turn.angle),
            frame,
        },
        |expression, dimension, what| context.value(expression, dimension, what),
        || unusable(context),
    )
}

pub(crate) fn placement_transform(
    feature: &Feature,
    inputs: &Inputs<'_>,
    placement: &BodyPlacement,
) -> Result<RigidTransform, Failure> {
    let context = Context { feature, inputs };
    let unusable = || {
        context.error(
            "The placement is too large or too small to place the body.".to_owned(),
            "Enter smaller distances and turns.".to_owned(),
        )
    };
    let local = placed(
        Placing::at_origin(&placement.offset, &placement.turn),
        |expression, dimension, what| context.value(expression, dimension, what),
        unusable,
    )?;
    let Some(frame) = placement.frame else {
        return Ok(local);
    };
    let frame = Resolver { feature, inputs }.frame(frame)?;
    let into_frame = RigidTransform::from_frame(&frame).ok_or_else(unusable)?;
    Ok(local.then(&into_frame))
}

fn placed<E>(
    Placing {
        offset,
        turn,
        pivot,
        angle,
        frame,
    }: Placing<'_>,
    value: impl Fn(&Expression, Dimension, &str) -> Result<f64, E>,
    unusable: impl Fn() -> E,
) -> Result<RigidTransform, E> {
    let mut placed = RigidTransform::IDENTITY;
    if let (Pivot::Axis(axis), Some(angle)) = (pivot, angle) {
        let angle = value(angle, Dimension::ANGLE, "turn about its axis")?.to_radians();
        placed = RigidTransform::rotation_about(axis.origin(), axis.direction(), angle)
            .ok_or_else(&unusable)?;
    }
    let centre = pivot.point();
    for axis in MoveAxis::ALL {
        let what = format!("turn about {}", axis.name());
        let angle = value(axis.of(turn), Dimension::ANGLE, &what)?.to_radians();
        let turned = RigidTransform::rotation_about(centre, axis.direction_in(frame), angle)
            .ok_or_else(&unusable)?;
        placed = placed.then(&turned);
    }
    let mut shift = Vector3::ZERO;
    for axis in MoveAxis::ALL {
        let what = format!("distance along {}", axis.name());
        shift += axis.direction_in(frame) * value(axis.of(offset), Dimension::LENGTH, &what)?;
    }
    let shifted = RigidTransform::translation(shift).ok_or_else(&unusable)?;
    Ok(placed.then(&shifted))
}

fn unusable(context: &Context<'_>) -> Failure {
    context.error(
        "The move is too large or too small to place the body.".to_owned(),
        "Enter smaller distances and turns.".to_owned(),
    )
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &Move,
    inputs: &Inputs<'_>,
    cancel: &CancelToken,
) -> Result<FeatureResult, Failure> {
    let context = Context { feature, inputs };
    let body_name = inputs
        .document
        .feature(definition.body)
        .map(|body| body.name.clone())
        .unwrap_or_default();
    let Some(solid) = inputs.body(definition.body) else {
        return Err(inputs.missing_body(definition.body));
    };
    let resolver = Resolver { feature, inputs };
    let axis = match definition.about.axis() {
        Some(reference) => Some(resolver.axis(reference)?),
        None => None,
    };
    let frame = match definition.frame {
        Some(frame) => Some(resolver.frame(frame)?),
        None => None,
    };
    let pivot = definition
        .pivot(solid, axis, frame.as_ref())
        .ok_or_else(|| {
            context.error(
                format!("The body of {body_name} has no size to find its centre from."),
                "Turn it about the origin instead.".to_owned(),
            )
        })?;
    let placement = transform(&context, definition, pivot, frame.as_ref())?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let moved = solid.transformed(&placement).map_err(|_| {
        context.error(
            format!("The move would take the body of {body_name} too far from the origin."),
            "Enter smaller distances.".to_owned(),
        )
    })?;
    let placed = if definition.copy {
        feature.id()
    } else {
        definition.body
    };
    Ok(FeatureResult::Solid(SolidResult::new(placed, moved)))
}
