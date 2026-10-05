use std::collections::BTreeSet;

use caditor_expression::{Dimension, Expression, ParameterId, format_number};
use caditor_geometry::{Point3, Similarity};
use caditor_kernel::{GeometryError, TransformError};

use crate::{
    datum::Resolver,
    document::{Feature, FeatureId},
    movement::MoveAxis,
    recompute::{CancelToken, Failure, FeatureError, FeatureResult, FixTarget, Inputs},
    solid::SolidResult,
};

pub const MIN_SCALE_FACTOR: f64 = 1e-6;
pub const MAX_SCALE_FACTOR: f64 = 1e6;

#[derive(Debug, Clone, PartialEq)]
pub struct Scale {
    pub body: FeatureId,
    pub factor: Expression,
    pub center: [Expression; 3],
}

impl Scale {
    pub fn expressions(&self) -> impl Iterator<Item = &Expression> {
        std::iter::once(&self.factor).chain(self.center.iter())
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
}

struct Context<'a> {
    resolver: Resolver<'a>,
    body_name: String,
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

    fn factor(&self, definition: &Scale) -> Result<f64, Failure> {
        let factor = self
            .resolver
            .value(&definition.factor, "scale factor", Dimension::NONE)?;
        if factor <= 0.0 {
            return Err(self.error(
                format!(
                    "The scale factor must be more than zero, and {} is not.",
                    format_number(factor)
                ),
                "Enter a factor above zero, such as 2 to double the size or 0.5 to halve it.",
            ));
        }
        if !(MIN_SCALE_FACTOR..=MAX_SCALE_FACTOR).contains(&factor) {
            return Err(self.error(
                format!(
                    "The scale factor {} is outside what caditor can model, from {} to {}.",
                    format_number(factor),
                    format_number(MIN_SCALE_FACTOR),
                    format_number(MAX_SCALE_FACTOR)
                ),
                "Enter a factor nearer 1.",
            ));
        }
        Ok(factor)
    }

    fn center(&self, definition: &Scale) -> Result<Point3, Failure> {
        let mut center = Point3::ZERO;
        for axis in MoveAxis::ALL {
            let what = format!("centre {}", axis.name());
            let value =
                self.resolver
                    .value(axis.of(&definition.center), &what, Dimension::LENGTH)?;
            *axis.of_mut(center.as_mut()) = value;
        }
        Ok(center)
    }

    fn failure(&self, error: &TransformError, factor: f64) -> Failure {
        let body = &self.body_name;
        match error {
            TransformError::Cancelled(_) => Failure::Cancelled,
            TransformError::Geometry(GeometryError::BelowResolution(_)) => self.error(
                format!(
                    "Scaling the body of {body} by {} would shrink an edge below the \
                     smallest length caditor tells apart, a millionth of a millimetre.",
                    format_number(factor)
                ),
                "Enter a larger factor.",
            ),
            TransformError::Geometry(_) => self.error(
                format!(
                    "Scaling the body of {body} by {} would reach farther than a kilometre from \
                     the origin, the largest size caditor models.",
                    format_number(factor)
                ),
                "Enter a smaller factor, or a centre nearer the body.",
            ),
            error => {
                log::warn!("{} could not be built: {error}", self.resolver.feature.name);
                self.error(
                    format!(
                        "The body of {body} could not be scaled by {}.",
                        format_number(factor)
                    ),
                    "Enter a factor nearer 1.",
                )
            }
        }
    }
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &Scale,
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
        body_name,
    };
    let factor = context.factor(definition)?;
    let center = context.center(definition)?;
    let scaling = Similarity::scaling(center, factor).ok_or_else(|| {
        context.error(
            "The centre of the scaling is not a usable point.".to_owned(),
            "Enter smaller coordinates for the centre.",
        )
    })?;
    let Some(solid) = inputs.body(definition.body) else {
        return Err(Failure::Error(Box::new(FeatureError {
            reason: format!("The body made by {} has no shape.", context.body_name),
            remedy: format!("Fix {} first.", context.body_name),
            fix: Some(FixTarget::Feature(definition.body)),
            constraints: Vec::new(),
            place: None,
        })));
    };
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let scaled = solid
        .mapped(&scaling)
        .map_err(|error| context.failure(&error, factor))?;
    Ok(FeatureResult::Solid(SolidResult::new(
        definition.body,
        scaled,
    )))
}
