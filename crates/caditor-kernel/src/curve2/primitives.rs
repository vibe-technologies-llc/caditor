use std::f64::consts::{PI, TAU};

use caditor_geometry::{Aabb2, Point2, Vector2};

use crate::{
    checks::{finite_point2, size, unit2},
    error::GeometryError,
    interval::Interval,
};

const MAX_EXTREMA_PER_AXIS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Line2 {
    origin: Point2,
    direction: Vector2,
}

impl Line2 {
    pub fn new(origin: Point2, direction: Vector2) -> Result<Self, GeometryError> {
        Ok(Self {
            origin: finite_point2(origin)?,
            direction: unit2(direction)?,
        })
    }

    pub fn through(start: Point2, end: Point2) -> Result<Self, GeometryError> {
        Self::new(start, end - start)
    }

    pub fn origin(&self) -> Point2 {
        self.origin
    }

    pub fn direction(&self) -> Vector2 {
        self.direction
    }

    pub fn point(&self, parameter: f64) -> Point2 {
        self.origin + self.direction * parameter
    }

    pub fn parameter_of(&self, point: Point2) -> f64 {
        (point - self.origin).dot(self.direction)
    }

    #[must_use]
    pub fn reversed(&self) -> Self {
        Self {
            origin: self.origin,
            direction: -self.direction,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Circle2 {
    center: Point2,
    radius: f64,
    x_axis: Vector2,
    y_axis: Vector2,
}

impl Circle2 {
    pub fn new(center: Point2, radius: f64) -> Result<Self, GeometryError> {
        Self::with_axes(center, radius, Vector2::X, true)
    }

    pub fn with_axes(
        center: Point2,
        radius: f64,
        x_axis: Vector2,
        counter_clockwise: bool,
    ) -> Result<Self, GeometryError> {
        let x_axis = unit2(x_axis)?;
        let y_axis = if counter_clockwise {
            x_axis.perp()
        } else {
            -x_axis.perp()
        };
        Ok(Self {
            center: finite_point2(center)?,
            radius: size(radius)?,
            x_axis,
            y_axis,
        })
    }

    pub fn center(&self) -> Point2 {
        self.center
    }

    pub fn radius(&self) -> f64 {
        self.radius
    }

    pub fn x_axis(&self) -> Vector2 {
        self.x_axis
    }

    pub fn y_axis(&self) -> Vector2 {
        self.y_axis
    }

    pub fn is_counter_clockwise(&self) -> bool {
        self.x_axis.perp_dot(self.y_axis) > 0.0
    }

    #[must_use]
    pub fn reversed(&self) -> Self {
        Self {
            y_axis: -self.y_axis,
            ..*self
        }
    }

    pub(crate) fn evaluate(&self, parameter: f64) -> [Point2; 3] {
        let (sin, cos) = parameter.sin_cos();
        let radial = (self.x_axis * cos + self.y_axis * sin) * self.radius;
        let tangent = (self.y_axis * cos - self.x_axis * sin) * self.radius;
        [self.center + radial, tangent, -radial]
    }

    pub(crate) fn closest_parameter(&self, point: Point2, range: Interval) -> f64 {
        let local = point - self.center;
        let (x, y) = (local.dot(self.x_axis), local.dot(self.y_axis));
        if x.hypot(y) <= f64::MIN_POSITIVE {
            return range.start();
        }
        let unwrapped = range.start() + (y.atan2(x) - range.start()).rem_euclid(TAU);
        let distance = |parameter: f64| {
            let [position, _, _] = self.evaluate(parameter);
            position.distance_squared(point)
        };
        let mut best = if distance(range.start()) <= distance(range.end()) {
            range.start()
        } else {
            range.end()
        };
        if unwrapped <= range.end() && distance(unwrapped) < distance(best) {
            best = unwrapped;
        }
        best
    }

    pub(crate) fn bounds(&self, range: Interval) -> Aabb2 {
        let point = |parameter: f64| {
            let [position, _, _] = self.evaluate(parameter);
            position
        };
        let mut bounds = Aabb2::from_point(point(range.start())).including(point(range.end()));
        let components = [
            (self.x_axis.x, self.y_axis.x),
            (self.x_axis.y, self.y_axis.y),
        ];
        for (x_component, y_component) in components {
            let extreme = y_component.atan2(x_component);
            let first = ((range.start() - extreme) / PI).ceil();
            for step in 0..MAX_EXTREMA_PER_AXIS {
                let parameter = extreme + (first + step as f64) * PI;
                if parameter > range.end() {
                    break;
                }
                bounds = bounds.including(point(parameter));
            }
        }
        bounds
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ellipse2 {
    center: Point2,
    major_radius: f64,
    minor_radius: f64,
    x_axis: Vector2,
    y_axis: Vector2,
}

impl Ellipse2 {
    pub fn new(center: Point2, major: Vector2, minor_radius: f64) -> Result<Self, GeometryError> {
        Self::with_axes(center, major.length(), minor_radius, major, true)
    }

    pub fn with_axes(
        center: Point2,
        major_radius: f64,
        minor_radius: f64,
        x_axis: Vector2,
        counter_clockwise: bool,
    ) -> Result<Self, GeometryError> {
        let x_axis = unit2(x_axis)?;
        let y_axis = if counter_clockwise {
            x_axis.perp()
        } else {
            -x_axis.perp()
        };
        Ok(Self {
            center: finite_point2(center)?,
            major_radius: size(major_radius)?,
            minor_radius: size(minor_radius)?,
            x_axis,
            y_axis,
        })
    }

    pub fn center(&self) -> Point2 {
        self.center
    }

    pub fn major_radius(&self) -> f64 {
        self.major_radius
    }

    pub fn minor_radius(&self) -> f64 {
        self.minor_radius
    }

    pub fn x_axis(&self) -> Vector2 {
        self.x_axis
    }

    pub fn y_axis(&self) -> Vector2 {
        self.y_axis
    }

    pub fn is_counter_clockwise(&self) -> bool {
        self.x_axis.perp_dot(self.y_axis) > 0.0
    }

    #[must_use]
    pub fn reversed(&self) -> Self {
        Self {
            y_axis: -self.y_axis,
            ..*self
        }
    }

    pub fn parameter_of(&self, point: Point2) -> f64 {
        let local = point - self.center;
        let along = local.dot(self.x_axis) / self.major_radius;
        let across = local.dot(self.y_axis) / self.minor_radius;
        across.atan2(along)
    }

    pub(crate) fn evaluate(&self, parameter: f64) -> [Point2; 3] {
        let (sin, cos) = parameter.sin_cos();
        let radial =
            self.x_axis * (self.major_radius * cos) + self.y_axis * (self.minor_radius * sin);
        let tangent =
            self.y_axis * (self.minor_radius * cos) - self.x_axis * (self.major_radius * sin);
        [self.center + radial, tangent, -radial]
    }

    pub(crate) fn extreme_along(&self, direction: Vector2) -> Option<f64> {
        let along = Vector2::new(
            self.major_radius * direction.dot(self.x_axis),
            self.minor_radius * direction.dot(self.y_axis),
        );
        (along != Vector2::ZERO).then(|| along.y.atan2(along.x))
    }

    pub(crate) fn bounds(&self, range: Interval) -> Aabb2 {
        let point = |parameter: f64| {
            let [position, _, _] = self.evaluate(parameter);
            position
        };
        let mut bounds = Aabb2::from_point(point(range.start())).including(point(range.end()));
        for direction in [Vector2::X, Vector2::Y] {
            let Some(extreme) = self.extreme_along(direction) else {
                continue;
            };
            let first = ((range.start() - extreme) / PI).ceil();
            for step in 0..MAX_EXTREMA_PER_AXIS {
                let parameter = extreme + (first + step as f64) * PI;
                if parameter > range.end() {
                    break;
                }
                bounds = bounds.including(point(parameter));
            }
        }
        bounds
    }
}
