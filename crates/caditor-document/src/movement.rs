use std::collections::BTreeSet;

use caditor_expression::{Dimension, Expression, ParameterId, Unit};
use caditor_geometry::{Aabb, Plane, Point3, Ray, RigidTransform, Vector3};
use caditor_kernel::Solid;

use crate::{
    body_pieces::{PieceKey, source_of_piece_key, source_piece_key},
    datum::{AxisReference, Resolver},
    document::{Feature, FeatureId},
    failure::{Cause, Expected},
    recompute::{CancelToken, Failure, FeatureError, FeatureResult, FixTarget, Inputs},
    solid::SolidResult,
    values::ParameterValues,
};

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
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

#[derive(Clone, Debug, Default, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
pub struct AxisTurn {
    pub axis: AxisReference,
    pub angle: Expression,
}

#[derive(Clone, Copy, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
pub struct Move {
    pub body: FeatureId,
    pub more_bodies: Vec<FeatureId>,
    pub offset: [Expression; 3],
    pub turn: [Expression; 3],
    pub copy: bool,
    pub about: TurnCentre,
    pub frame: Option<FeatureId>,
}

impl Move {
    #[must_use]
    pub fn with_more_bodies(mut self, bodies: Vec<FeatureId>) -> Self {
        self.more_bodies = bodies;
        self
    }

    pub fn bodies(&self) -> impl Iterator<Item = FeatureId> + '_ {
        std::iter::once(self.body).chain(self.more_bodies.iter().copied())
    }

    pub fn moves_several_bodies(&self) -> bool {
        !self.more_bodies.is_empty()
    }

    pub fn makes_a_body_of_each_copy(&self) -> bool {
        self.copy && !self.more_bodies.is_empty()
    }

    pub fn changed_bodies(&self) -> Vec<FeatureId> {
        if self.copy {
            Vec::new()
        } else {
            self.bodies().collect()
        }
    }

    pub fn copy_key(source: FeatureId) -> PieceKey {
        source_piece_key(source)
    }

    pub fn copy_source(&self, key: PieceKey) -> Option<FeatureId> {
        source_of_piece_key(self.bodies(), key)
    }

    pub fn moved_bodies(&self, feature: FeatureId) -> Vec<FeatureId> {
        if !self.copy {
            self.bodies().collect()
        } else if self.makes_a_body_of_each_copy() {
            self.bodies()
                .filter_map(|source| feature.piece(Self::copy_key(source)))
                .collect()
        } else {
            vec![feature]
        }
    }

    pub(crate) fn settle(&mut self) {
        let mut seen = BTreeSet::from([self.body]);

        self.more_bodies.retain(|more| seen.insert(*more));
    }

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

        used.extend(self.more_bodies.iter().copied());
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
        self.more_bodies.len() * size_of::<FeatureId>()
            + self
                .offset
                .iter()
                .chain(self.turn.iter())
                .map(Expression::heap_size)
                .sum::<usize>()
            + self.about.heap_size()
    }

    pub fn pivot(&self, body: &Solid, axis: Option<Ray>, frame: Option<&Plane>) -> Option<Pivot> {
        self.pivot_of([body], axis, frame)
    }

    pub fn pivot_of<'a>(
        &self,
        bodies: impl IntoIterator<Item = &'a Solid>,
        axis: Option<Ray>,
        frame: Option<&Plane>,
    ) -> Option<Pivot> {
        match self.about {
            TurnCentre::Origin => Some(Pivot::Point(frame.map_or(Point3::ZERO, Plane::origin))),
            TurnCentre::Body => {
                let bounds = bodies
                    .into_iter()
                    .filter_map(Solid::bounding_box)
                    .reduce(Aabb::union)?;
                Some(Pivot::Point(bounds.center()))
            }
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

#[derive(Clone, Debug, PartialEq)]
pub struct BodyPlacement {
    pub offset: [Expression; 3],
    pub turn: [Expression; 3],
    pub frame: Option<FeatureId>,
    pub scale: Expression,
}

impl Default for BodyPlacement {
    fn default() -> Self {
        Self {
            offset: std::array::from_fn(|_| Expression::Measure(0.0, Unit::Millimetre)),
            turn: std::array::from_fn(|_| Expression::Measure(0.0, Unit::Degree)),
            frame: None,
            scale: Expression::Number(1.0),
        }
    }
}

impl BodyPlacement {
    pub fn is_at_origin(&self) -> bool {
        self.frame.is_none() && self.is_unmoved() && self.is_unscaled()
    }

    pub fn is_unmoved(&self) -> bool {
        self.offset.iter().chain(self.turn.iter()).all(|expression| {
            matches!(expression, Expression::Number(value) | Expression::Measure(value, _) if *value == 0.0)
        })
    }

    pub fn is_unscaled(&self) -> bool {
        matches!(self.scale, Expression::Number(value) if value == 1.0)
    }

    pub fn expressions(&self) -> impl Iterator<Item = &Expression> {
        self.offset
            .iter()
            .chain(self.turn.iter())
            .chain(std::iter::once(&self.scale))
    }

    pub fn expressions_mut(&mut self) -> impl Iterator<Item = &mut Expression> {
        self.offset
            .iter_mut()
            .chain(self.turn.iter_mut())
            .chain(std::iter::once(&mut self.scale))
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
    pub(crate) fn error(&self, cause: impl Into<Cause>) -> Failure {
        Failure::from(FeatureError::new(cause).fixing(FixTarget::Feature(self.feature.id())))
    }

    pub(crate) fn value(
        &self,
        expression: &Expression,
        dimension: Dimension,
        what: &str,
    ) -> Result<f64, Failure> {
        let expected = if dimension == Dimension::ANGLE {
            Expected::Angle(90.0)
        } else {
            Expected::Length(10.0)
        };
        expression
            .evaluate_as(dimension, &|id| self.inputs.parameters.value(id))
            .map_err(|error| {
                Failure::from(FeatureError::unevaluable(
                    self.feature.id(),
                    what,
                    expected,
                    error,
                ))
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
    let unusable = || context.error(MoveCause::PlacementUnusable);

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
    context.error(MoveCause::MoveUnusable)
}

fn copy_owner(
    context: &Context<'_>,
    definition: &Move,
    source: FeatureId,
) -> Result<FeatureId, Failure> {
    let feature = context.feature;
    if !definition.copy {
        Ok(source)
    } else if definition.makes_a_body_of_each_copy() {
        feature.id().piece(Move::copy_key(source)).ok_or_else(|| {
            context.error(MoveCause::CopiesTooMany {
                movement: feature.name.clone(),
            })
        })
    } else {
        Ok(feature.id())
    }
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &Move,
    inputs: &Inputs<'_>,
    cancel: &CancelToken,
) -> Result<FeatureResult, Failure> {
    let context = Context { feature, inputs };
    let mut sources = Vec::new();

    for source in definition.bodies() {
        let Some(solid) = inputs.body(source) else {
            return Err(inputs.missing_body(source));
        };

        sources.push((source, solid));
    }

    let resolver = Resolver { feature, inputs };
    let axis = match definition.about.axis() {
        Some(reference) => Some(resolver.axis(reference)?),
        None => None,
    };
    let frame = match definition.frame {
        Some(frame) => Some(resolver.frame(frame)?),
        None => None,
    };

    let pivot = match definition.about {
        TurnCentre::Origin => Pivot::Point(frame.as_ref().map_or(Point3::ZERO, Plane::origin)),
        TurnCentre::Axis(_) => Pivot::Axis(axis.ok_or_else(|| {
            context.error(MoveCause::NoSize {
                body: inputs
                    .document
                    .body_label(definition.body)
                    .unwrap_or_default(),
            })
        })?),
        TurnCentre::Body => {
            let mut combined: Option<Aabb> = None;

            for (source, solid) in &sources {
                let bounds = solid.bounding_box().ok_or_else(|| {
                    context.error(MoveCause::NoSize {
                        body: inputs.document.body_label(*source).unwrap_or_default(),
                    })
                })?;

                combined = Some(match combined {
                    Some(known) => known.union(bounds),
                    None => bounds,
                });
            }

            let bounds = combined.ok_or_else(|| {
                context.error(MoveCause::NoSize {
                    body: inputs
                        .document
                        .body_label(definition.body)
                        .unwrap_or_default(),
                })
            })?;
            Pivot::Point(bounds.center())
        }
    };
    let placement = transform(&context, definition, pivot, frame.as_ref())?;

    let mut owners = BTreeSet::new();
    let mut results = Vec::with_capacity(sources.len());

    for (source, solid) in sources {
        if cancel.is_cancelled() {
            return Err(Failure::Cancelled);
        }

        let owner = copy_owner(&context, definition, source)?;

        if !owners.insert(owner) {
            return Err(context.error(MoveCause::CopiesAlike {
                movement: feature.name.clone(),
            }));
        }

        let moved = solid.transformed(&placement).map_err(|_| {
            context.error(MoveCause::TooFar {
                body: inputs.document.body_label(source).unwrap_or_default(),
            })
        })?;

        results.push(SolidResult::new(owner, moved));
    }

    let mut results = results.into_iter();

    let first = results.next().ok_or_else(|| unusable(&context))?;
    Ok(FeatureResult::Solid(first.with_others(results)))
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum MoveCause {
    PlacementUnusable,
    MoveUnusable,
    NoSize { body: String },
    TooFar { body: String },
    CopiesTooMany { movement: String },
    CopiesAlike { movement: String },
}

impl MoveCause {
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::PlacementUnusable => {
                "The placement is too large or too small to place the body.".to_owned()
            }
            Self::MoveUnusable => {
                "The move is too large or too small to place the body.".to_owned()
            }
            Self::NoSize { body } => {
                format!("The body of {body} has no size to find its centre from.")
            }
            Self::TooFar { body } => {
                format!("The move would take the body of {body} too far from the origin.")
            }
            Self::CopiesTooMany { movement } => {
                format!("The copies of {movement} cannot each become a body in a model this large.")
            }
            Self::CopiesAlike { movement } => format!(
                "Two of the bodies {movement} copies cannot be told apart, so their copies cannot \
                 each become a body."
            ),
        }
    }

    pub(crate) fn remedy(&self) -> String {
        match self {
            Self::PlacementUnusable | Self::MoveUnusable => {
                "Enter smaller distances and turns.".to_owned()
            }
            Self::NoSize { .. } => "Turn it about the origin instead.".to_owned(),
            Self::TooFar { .. } => "Enter smaller distances.".to_owned(),
            Self::CopiesTooMany { .. } | Self::CopiesAlike { .. } => {
                "Copy these bodies in separate copies, or move them in place.".to_owned()
            }
        }
    }
}

impl From<MoveCause> for Cause {
    fn from(cause: MoveCause) -> Self {
        Self::Move(cause)
    }
}
