mod primitives;
#[cfg(test)]
mod tests;

use std::f64::consts::TAU;

use caditor_geometry::{Aabb2, Plane, Point2, RigidTransform2, Vector2};

pub use self::primitives::{Circle2, Ellipse2, Line2};
use crate::{
    bspline::BSpline,
    curve::{Circle, Curve, Ellipse, Line, circle_parameters},
    error::GeometryError,
    interval::{Domain, Interval},
    parametric::{self, Parametric},
    tolerance::SamplingTolerance,
};

pub type BSplineCurve2 = BSpline<Point2>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Curve2Derivatives {
    pub point: Point2,
    pub first: Vector2,
    pub second: Vector2,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Curve2Sample {
    pub parameter: f64,
    pub point: Point2,
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Curve2 {
    Line(Line2),
    Circle(Circle2),
    BSpline(BSplineCurve2),
    Ellipse(Ellipse2),
}

impl Curve2 {
    pub fn evaluate(&self, parameter: f64) -> Curve2Derivatives {
        let [point, first, second] = Parametric::evaluate(self, parameter);
        Curve2Derivatives {
            point,
            first,
            second,
        }
    }

    pub fn point(&self, parameter: f64) -> Point2 {
        self.evaluate(parameter).point
    }

    pub fn period(&self) -> Option<f64> {
        match self {
            Self::Circle(_) | Self::Ellipse(_) => Some(TAU),
            Self::Line(_) | Self::BSpline(_) => None,
        }
    }

    pub fn domain(&self) -> Domain {
        match self {
            Self::Line(_) => Domain::UNBOUNDED,
            Self::Circle(_) | Self::Ellipse(_) => Domain::from(Interval::FULL_TURN),
            Self::BSpline(spline) => Domain::from(spline.domain()),
        }
    }

    pub fn closest_parameter(&self, point: Point2, range: Interval) -> f64 {
        match self {
            Self::Line(line) => range.clamp(line.parameter_of(point)),
            Self::Circle(circle) => circle.closest_parameter(point, range),
            Self::BSpline(_) | Self::Ellipse(_) => {
                parametric::closest_parameter(self, point, range)
            }
        }
    }

    pub fn bounding_box(&self, range: Interval) -> Aabb2 {
        match self {
            Self::Line(line) => {
                Aabb2::from_point(line.point(range.start())).including(line.point(range.end()))
            }
            Self::Circle(circle) => circle.bounds(range),
            Self::Ellipse(ellipse) => ellipse.bounds(range),
            Self::BSpline(spline) => {
                Aabb2::from_points(spline.control_points_over(range).iter().copied())
                    .unwrap_or_else(|| Aabb2::from_point(spline.point(range.start())))
            }
        }
    }

    pub(crate) fn piece_bounds(&self, range: Interval) -> Aabb2 {
        let coarse = self.bounding_box(range);
        match self {
            Self::BSpline(spline) if spline.single_span(range) => {
                let [low, high] =
                    parametric::tight_bounds(self, range, [coarse.min(), coarse.max()]);
                Aabb2::from_point(low).including(high)
            }
            _ => coarse,
        }
    }

    pub fn sample(&self, range: Interval, tolerance: &SamplingTolerance) -> Vec<Curve2Sample> {
        let parameters = match self {
            Self::Line(_) => vec![range.start(), range.end()],
            Self::Circle(circle) => circle_parameters(circle.radius(), range, tolerance),
            Self::Ellipse(ellipse) => circle_parameters(
                ellipse.major_radius().max(ellipse.minor_radius()),
                range,
                tolerance,
            ),
            Self::BSpline(_) => parametric::adaptive_parameters(self, range, tolerance),
        };
        parameters
            .into_iter()
            .map(|parameter| Curve2Sample {
                parameter,
                point: self.point(parameter),
            })
            .collect()
    }

    pub fn length(&self, range: Interval) -> f64 {
        match self {
            Self::Line(_) => range.length(),
            Self::Circle(circle) => circle.radius() * range.length(),
            Self::BSpline(_) | Self::Ellipse(_) => parametric::length(self, range),
        }
    }

    pub fn length_up_to(&self, range: Interval, cap: f64) -> f64 {
        match self {
            Self::Line(_) | Self::Circle(_) => self.length(range).min(cap),
            Self::BSpline(_) | Self::Ellipse(_) => parametric::length_up_to(self, range, cap),
        }
    }

    pub fn is_longer_than(&self, range: Interval, bound: f64) -> bool {
        match self {
            Self::Line(_) | Self::Circle(_) => self.length(range) > bound,
            Self::BSpline(_) | Self::Ellipse(_) => parametric::longer_than(self, range, bound),
        }
    }

    pub fn reversal_pivot(&self) -> f64 {
        match self {
            Self::Line(_) | Self::Circle(_) | Self::Ellipse(_) => 0.0,
            Self::BSpline(spline) => spline.domain().start() + spline.domain().end(),
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
            Self::BSpline(spline) => Self::BSpline(spline.reversed()),
            Self::Ellipse(ellipse) => Self::Ellipse(ellipse.reversed()),
        }
    }

    pub fn transformed(&self, transform: &RigidTransform2) -> Result<Self, GeometryError> {
        Ok(match self {
            Self::Line(line) => Self::Line(Line2::new(
                transform.apply_point(line.origin()),
                transform.apply_vector(line.direction()),
            )?),
            Self::Circle(circle) => Self::Circle(Circle2::with_axes(
                transform.apply_point(circle.center()),
                circle.radius(),
                transform.apply_vector(circle.x_axis()),
                circle.is_counter_clockwise(),
            )?),
            Self::BSpline(spline) => {
                Self::BSpline(spline.map_points(|point| transform.apply_point(point))?)
            }
            Self::Ellipse(ellipse) => Self::Ellipse(Ellipse2::with_axes(
                transform.apply_point(ellipse.center()),
                ellipse.major_radius(),
                ellipse.minor_radius(),
                transform.apply_vector(ellipse.x_axis()),
                ellipse.is_counter_clockwise(),
            )?),
        })
    }

    pub fn on_plane(&self, plane: &Plane) -> Result<Curve, GeometryError> {
        let lift = |direction: Vector2| plane.x_axis() * direction.x + plane.y_axis() * direction.y;
        Ok(match self {
            Self::Line(line) => Curve::Line(Line::new(
                plane.to_world(line.origin()),
                lift(line.direction()),
            )?),
            Self::Circle(circle) => {
                let x_axis = lift(circle.x_axis());
                let normal = x_axis.cross(lift(circle.y_axis()));
                let frame = Plane::from_frame(plane.to_world(circle.center()), normal, x_axis)
                    .ok_or(GeometryError::ZeroDirection)?;
                Curve::Circle(Circle::new(frame, circle.radius())?)
            }
            Self::BSpline(spline) => {
                Curve::BSpline(spline.map_points(|point| plane.to_world(point))?)
            }
            Self::Ellipse(ellipse) => {
                let x_axis = lift(ellipse.x_axis());
                let normal = x_axis.cross(lift(ellipse.y_axis()));
                let frame = Plane::from_frame(plane.to_world(ellipse.center()), normal, x_axis)
                    .ok_or(GeometryError::ZeroDirection)?;
                Curve::Ellipse(Ellipse::new(
                    frame,
                    ellipse.major_radius(),
                    ellipse.minor_radius(),
                )?)
            }
        })
    }
}

impl Parametric for Curve2 {
    type Point = Point2;

    fn evaluate(&self, parameter: f64) -> [Point2; 3] {
        match self {
            Self::Line(line) => [line.point(parameter), line.direction(), Vector2::ZERO],
            Self::Circle(circle) => circle.evaluate(parameter),
            Self::BSpline(spline) => spline.derivatives(parameter),
            Self::Ellipse(ellipse) => ellipse.evaluate(parameter),
        }
    }

    fn seeds(&self, range: Interval) -> Vec<f64> {
        match self {
            Self::Line(_) => vec![range.start(), range.end()],
            Self::Circle(_) | Self::Ellipse(_) => crate::curve::conic_seeds(range),
            Self::BSpline(spline) => spline.seeds(range),
        }
    }

    fn nearby_runs(&self, point: Point2, range: Interval) -> Vec<Interval> {
        match self {
            Self::BSpline(spline) => spline.nearby_runs(point, range),
            _ => vec![range],
        }
    }
}

impl From<Line2> for Curve2 {
    fn from(line: Line2) -> Self {
        Self::Line(line)
    }
}

impl From<Circle2> for Curve2 {
    fn from(circle: Circle2) -> Self {
        Self::Circle(circle)
    }
}

impl From<Ellipse2> for Curve2 {
    fn from(ellipse: Ellipse2) -> Self {
        Self::Ellipse(ellipse)
    }
}

impl From<BSplineCurve2> for Curve2 {
    fn from(spline: BSplineCurve2) -> Self {
        Self::BSpline(spline)
    }
}
