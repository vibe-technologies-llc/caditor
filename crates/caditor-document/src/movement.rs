use std::collections::BTreeSet;

use caditor_expression::{Dimension, EvalError, Expression, ParameterId};
use caditor_geometry::{Point3, RigidTransform, Vector3};

use crate::{
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct Move {
    pub body: FeatureId,
    pub offset: [Expression; 3],
    pub turn: [Expression; 3],
    pub copy: bool,
}

impl Move {
    pub fn expressions(&self) -> impl Iterator<Item = &Expression> {
        self.offset.iter().chain(self.turn.iter())
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
        BTreeSet::from([self.body])
    }

    pub fn heap_size(&self) -> usize {
        self.expressions().map(Expression::heap_size).sum()
    }

    pub fn placement(&self, parameters: &ParameterValues) -> Option<RigidTransform> {
        placed(
            self,
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

struct Context<'a> {
    feature: &'a Feature,
    inputs: &'a Inputs<'a>,
}

impl Context<'_> {
    fn error(&self, reason: String, remedy: String) -> Failure {
        Failure::Error(Box::new(FeatureError {
            reason,
            remedy,
            fix: Some(FixTarget::Feature(self.feature.id())),
            constraints: Vec::new(),
            place: None,
        }))
    }

    fn value(
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

fn transform(context: &Context<'_>, definition: &Move) -> Result<RigidTransform, Failure> {
    placed(
        definition,
        |expression, dimension, what| context.value(expression, dimension, what),
        || unusable(context),
    )
}

fn placed<E>(
    definition: &Move,
    value: impl Fn(&Expression, Dimension, &str) -> Result<f64, E>,
    unusable: impl Fn() -> E,
) -> Result<RigidTransform, E> {
    let mut placed = RigidTransform::IDENTITY;
    for axis in MoveAxis::ALL {
        let what = format!("turn about {}", axis.name());
        let angle = value(axis.of(&definition.turn), Dimension::ANGLE, &what)?.to_radians();
        let turned = RigidTransform::rotation_about(Point3::ZERO, axis.direction(), angle)
            .ok_or_else(&unusable)?;
        placed = placed.then(&turned);
    }
    let mut offset = Vector3::ZERO;
    for axis in MoveAxis::ALL {
        let what = format!("distance along {}", axis.name());
        offset += axis.direction() * value(axis.of(&definition.offset), Dimension::LENGTH, &what)?;
    }
    let shifted = RigidTransform::translation(offset).ok_or_else(&unusable)?;
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
    let placement = transform(&context, definition)?;
    let body_name = inputs
        .document
        .feature(definition.body)
        .map(|body| body.name.clone())
        .unwrap_or_default();
    let Some(solid) = inputs.body(definition.body) else {
        return Err(inputs.missing_body(definition.body));
    };
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
