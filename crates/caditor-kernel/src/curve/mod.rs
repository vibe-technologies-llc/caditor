mod conic;
mod intersection;
mod line;
#[cfg(test)]
mod tests;

use std::f64::consts::TAU;

use caditor_geometry::{Aabb, Point3, RigidTransform, Vector3};

pub(crate) use self::conic::conic_seeds;
pub use self::{
    conic::{Circle, Ellipse},
    intersection::{IntersectionCurve, IntersectionNode},
    line::Line,
};
use crate::{
    bspline::BSpline,
    error::GeometryError,
    interval::{Domain, Interval},
    parametric::{self, Parametric},
    tolerance::SamplingTolerance,
};

const MAX_CIRCLE_SEGMENTS: usize = 1 << 16;

pub type BSplineCurve = BSpline<Point3>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurveDerivatives {
    pub point: Point3,
    pub first: Vector3,
    pub second: Vector3,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurveSample {
    pub parameter: f64,
    pub point: Point3,
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Curve {
    Line(Line),
    Circle(Circle),
    Ellipse(Ellipse),
    BSpline(BSplineCurve),
    Intersection(IntersectionCurve),
}

impl Curve {
    pub fn heap_size(&self) -> usize {
        match self {
            Self::Line(_) | Self::Circle(_) | Self::Ellipse(_) => 0,
            Self::BSpline(spline) => spline.heap_size(),
            Self::Intersection(curve) => curve.heap_size(),
        }
    }

    pub fn evaluate(&self, parameter: f64) -> CurveDerivatives {
        let [point, first, second] = Parametric::evaluate(self, parameter);
        CurveDerivatives {
            point,
            first,
            second,
        }
    }

    pub fn point(&self, parameter: f64) -> Point3 {
        self.evaluate(parameter).point
    }

    pub fn period(&self) -> Option<f64> {
        match self {
            Self::Circle(_) | Self::Ellipse(_) => Some(TAU),
            Self::Line(_) | Self::BSpline(_) => None,
            Self::Intersection(curve) => curve.period(),
        }
    }

    pub fn domain(&self) -> Domain {
        match self {
            Self::Line(_) => Domain::UNBOUNDED,
            Self::Circle(_) | Self::Ellipse(_) => Domain::from(Interval::FULL_TURN),
            Self::BSpline(spline) => Domain::from(spline.domain()),
            Self::Intersection(curve) => Domain::from(curve.domain()),
        }
    }

    pub fn closest_parameter(&self, point: Point3, range: Interval) -> f64 {
        match self {
            Self::Line(line) => range.clamp(line.parameter_of(point)),
            Self::Circle(circle) => circle.closest_parameter(point, range),
            Self::Ellipse(_) | Self::BSpline(_) | Self::Intersection(_) => {
                parametric::closest_parameter(self, point, range)
            }
        }
    }

    pub fn bounding_box(&self, range: Interval) -> Aabb {
        match self {
            Self::Line(line) => {
                Aabb::from_point(line.point(range.start())).including(line.point(range.end()))
            }
            Self::Circle(circle) => circle.bounds(range),
            Self::Ellipse(ellipse) => ellipse.bounds(range),
            Self::BSpline(spline) => {
                Aabb::from_points(spline.control_points_over(range).iter().copied())
                    .unwrap_or_else(|| Aabb::from_point(spline.point(range.start())))
            }
            Self::Intersection(curve) => curve.bounds(range),
        }
    }

    pub(crate) fn piece_bounds(&self, range: Interval) -> Aabb {
        let coarse = self.bounding_box(range);
        match self {
            Self::BSpline(spline) if spline.single_span(range) => {
                let [low, high] =
                    parametric::tight_bounds(self, range, [coarse.min(), coarse.max()]);
                Aabb::from_point(low).including(high)
            }
            _ => coarse,
        }
    }

    pub fn sample(&self, range: Interval, tolerance: &SamplingTolerance) -> Vec<CurveSample> {
        let parameters = match self {
            Self::Line(_) => vec![range.start(), range.end()],
            Self::Circle(circle) => circle_parameters(circle.radius(), range, tolerance),
            Self::Ellipse(_) | Self::BSpline(_) | Self::Intersection(_) => {
                parametric::adaptive_parameters(self, range, tolerance)
            }
        };
        parameters
            .into_iter()
            .map(|parameter| CurveSample {
                parameter,
                point: self.point(parameter),
            })
            .collect()
    }

    pub fn length(&self, range: Interval) -> f64 {
        match self {
            Self::Line(_) => range.length(),
            Self::Circle(circle) => circle.radius() * range.length(),
            Self::Ellipse(_) | Self::BSpline(_) | Self::Intersection(_) => {
                parametric::length(self, range)
            }
        }
    }

    pub fn reversal_pivot(&self) -> f64 {
        match self {
            Self::Line(_) | Self::Circle(_) | Self::Ellipse(_) => 0.0,
            Self::BSpline(spline) => spline.domain().start() + spline.domain().end(),
            Self::Intersection(curve) => curve.reversal_pivot(),
        }
    }

    pub fn reversed_parameter(&self, parameter: f64) -> f64 {
        self.reversal_pivot() - parameter
    }

    pub fn reversed_range(&self, range: Interval) -> Interval {
        range.mirrored(self.reversal_pivot())
    }

    #[must_use]
    pub fn reversed(&self) -> Self {
        match self {
            Self::Line(line) => Self::Line(line.reversed()),
            Self::Circle(circle) => Self::Circle(circle.reversed()),
            Self::Ellipse(ellipse) => Self::Ellipse(ellipse.reversed()),
            Self::BSpline(spline) => Self::BSpline(spline.reversed()),
            Self::Intersection(curve) => Self::Intersection(curve.reversed()),
        }
    }

    pub fn transformed(&self, transform: &RigidTransform) -> Result<Self, GeometryError> {
        Ok(match self {
            Self::Line(line) => Self::Line(Line::new(
                transform.apply_point(line.origin()),
                transform.apply_vector(line.direction()),
            )?),
            Self::Circle(circle) => Self::Circle(Circle::new(
                circle.frame().transformed(transform),
                circle.radius(),
            )?),
            Self::Ellipse(ellipse) => Self::Ellipse(Ellipse::new(
                ellipse.frame().transformed(transform),
                ellipse.major_radius(),
                ellipse.minor_radius(),
            )?),
            Self::BSpline(spline) => {
                Self::BSpline(spline.map_points(|point| transform.apply_point(point))?)
            }
            Self::Intersection(curve) => Self::Intersection(curve.transformed(transform)?),
        })
    }
}

impl Parametric for Curve {
    type Point = Point3;

    fn evaluate(&self, parameter: f64) -> [Point3; 3] {
        match self {
            Self::Line(line) => [line.point(parameter), line.direction(), Vector3::ZERO],
            Self::Circle(circle) => circle.evaluate(parameter),
            Self::Ellipse(ellipse) => ellipse.evaluate(parameter),
            Self::BSpline(spline) => spline.derivatives(parameter),
            Self::Intersection(curve) => curve.evaluate(parameter),
        }
    }

    fn seeds(&self, range: Interval) -> Vec<f64> {
        match self {
            Self::Line(_) => vec![range.start(), range.end()],
            Self::Circle(_) | Self::Ellipse(_) => conic::conic_seeds(range),
            Self::BSpline(spline) => spline.seeds(range),
            Self::Intersection(curve) => curve.seeds(range),
        }
    }
}

impl From<Line> for Curve {
    fn from(line: Line) -> Self {
        Self::Line(line)
    }
}

impl From<Circle> for Curve {
    fn from(circle: Circle) -> Self {
        Self::Circle(circle)
    }
}

impl From<Ellipse> for Curve {
    fn from(ellipse: Ellipse) -> Self {
        Self::Ellipse(ellipse)
    }
}

impl From<IntersectionCurve> for Curve {
    fn from(curve: IntersectionCurve) -> Self {
        Self::Intersection(curve)
    }
}

impl From<BSplineCurve> for Curve {
    fn from(spline: BSplineCurve) -> Self {
        Self::BSpline(spline)
    }
}

pub(crate) fn circle_parameters(
    radius: f64,
    range: Interval,
    tolerance: &SamplingTolerance,
) -> Vec<f64> {
    let step = tolerance.step_on_radius(radius);
    let segments = (radius * range.length() / step).ceil();
    let segments = if segments.is_finite() && segments >= 1.0 {
        (segments as usize).min(MAX_CIRCLE_SEGMENTS)
    } else {
        1
    };
    range.split(segments).collect()
}
