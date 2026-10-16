use std::collections::BTreeSet;

use caditor_expression::{Dimension, Expression, ParameterId, format_number};
use caditor_geometry::{Plane, Point3, Similarity};
use caditor_kernel::{GeometryError, TransformError};

use crate::{
    datum::Resolver,
    document::{Feature, FeatureId},
    failure::Cause,
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
    pub frame: Option<FeatureId>,
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
    fn error(&self, cause: ScaleCause) -> Failure {
        Failure::from(
            FeatureError::new(cause).fixing(FixTarget::Feature(self.resolver.feature.id())),
        )
    }

    fn factor(&self, definition: &Scale) -> Result<f64, Failure> {
        let factor = self
            .resolver
            .value(&definition.factor, "scale factor", Dimension::NONE)?;
        if factor <= 0.0 {
            return Err(self.error(ScaleCause::FactorNotPositive { factor }));
        }
        if !(MIN_SCALE_FACTOR..=MAX_SCALE_FACTOR).contains(&factor) {
            return Err(self.error(ScaleCause::FactorRange { factor }));
        }
        Ok(factor)
    }

    fn center(&self, definition: &Scale) -> Result<Point3, Failure> {
        let frame = match definition.frame {
            Some(frame) => Some(self.resolver.frame(frame)?),
            None => None,
        };
        let mut center = frame.as_ref().map_or(Point3::ZERO, Plane::origin);
        for axis in MoveAxis::ALL {
            let what = format!("centre {}", axis.name());
            let value =
                self.resolver
                    .value(axis.of(&definition.center), &what, Dimension::LENGTH)?;
            center += axis.direction_in(frame.as_ref()) * value;
        }
        Ok(center)
    }

    fn failure(&self, error: &TransformError, factor: f64) -> Failure {
        let body = self.body_name.clone();
        match error {
            TransformError::Cancelled(_) => Failure::Cancelled,
            TransformError::Geometry(GeometryError::BelowResolution(_)) => {
                self.error(ScaleCause::BelowResolution { body, factor })
            }
            TransformError::Geometry(_) => self.error(ScaleCause::TooFar { body, factor }),
            error => {
                log::warn!("{} could not be built: {error}", self.resolver.feature.name);
                self.error(ScaleCause::Unscalable { body, factor })
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ScaleCause {
    FactorNotPositive { factor: f64 },
    FactorRange { factor: f64 },
    BelowResolution { body: String, factor: f64 },
    TooFar { body: String, factor: f64 },
    Unscalable { body: String, factor: f64 },
    CentreUnusable,
}

impl ScaleCause {
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::FactorNotPositive { factor } => format!(
                "The scale factor must be more than zero, and {} is not.",
                format_number(*factor)
            ),
            Self::FactorRange { factor } => format!(
                "The scale factor {} is outside what caditor can model, from {} to {}.",
                format_number(*factor),
                format_number(MIN_SCALE_FACTOR),
                format_number(MAX_SCALE_FACTOR)
            ),
            Self::BelowResolution { body, factor } => format!(
                "Scaling the body of {body} by {} would shrink an edge below the smallest length \
                 caditor tells apart, a millionth of a millimetre.",
                format_number(*factor)
            ),
            Self::TooFar { body, factor } => format!(
                "Scaling the body of {body} by {} would reach farther than a kilometre from the \
                 origin, the largest size caditor models.",
                format_number(*factor)
            ),
            Self::Unscalable { body, factor } => format!(
                "The body of {body} could not be scaled by {}.",
                format_number(*factor)
            ),
            Self::CentreUnusable => "The centre of the scaling is not a usable point.".to_owned(),
        }
    }

    pub(crate) fn remedy(&self) -> String {
        match self {
            Self::FactorNotPositive { .. } => {
                "Enter a factor above zero, such as 2 to double the size or 0.5 to halve it."
                    .to_owned()
            }
            Self::FactorRange { .. } | Self::Unscalable { .. } => {
                "Enter a factor nearer 1.".to_owned()
            }
            Self::BelowResolution { .. } => "Enter a larger factor.".to_owned(),
            Self::TooFar { .. } => {
                "Enter a smaller factor, or a centre nearer the body.".to_owned()
            }
            Self::CentreUnusable => "Enter smaller coordinates for the centre.".to_owned(),
        }
    }
}

impl From<ScaleCause> for Cause {
    fn from(cause: ScaleCause) -> Self {
        Self::Scale(cause)
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
        .body_label(definition.body)
        .unwrap_or_default();
    let context = Context {
        resolver: Resolver { feature, inputs },
        body_name,
    };
    let factor = context.factor(definition)?;
    let center = context.center(definition)?;
    let scaling = Similarity::scaling(center, factor)
        .ok_or_else(|| context.error(ScaleCause::CentreUnusable))?;
    let Some(solid) = inputs.body(definition.body) else {
        return Err(inputs.missing_body(definition.body));
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
