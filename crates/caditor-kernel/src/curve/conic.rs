use std::f64::consts::{PI, TAU};

use caditor_geometry::{Aabb, Plane, Point3};

use crate::{
    checks::{checked_frame, positive},
    error::GeometryError,
    interval::Interval,
};

const MAX_EXTREMA_PER_AXIS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Circle {
    frame: Plane,
    radius: f64,
}

impl Circle {
    pub fn new(frame: Plane, radius: f64) -> Result<Self, GeometryError> {
        Ok(Self {
            frame: checked_frame(frame)?,
            radius: positive(radius)?,
        })
    }

    pub fn frame(&self) -> &Plane {
        &self.frame
    }

    pub fn center(&self) -> Point3 {
        self.frame.origin()
    }

    pub fn radius(&self) -> f64 {
        self.radius
    }

    #[must_use]
    pub fn reversed(&self) -> Self {
        Self {
            frame: self.frame.flipped(),
            radius: self.radius,
        }
    }

    pub(crate) fn evaluate(&self, parameter: f64) -> [Point3; 3] {
        conic_derivatives(&self.frame, self.radius, self.radius, parameter)
    }

    pub(crate) fn closest_parameter(&self, point: Point3, range: Interval) -> f64 {
        let local = point - self.frame.origin();
        let (x, y) = (
            local.dot(self.frame.x_axis()),
            local.dot(self.frame.y_axis()),
        );
        if x.hypot(y) <= f64::MIN_POSITIVE {
            return range.start();
        }
        let angle = y.atan2(x);
        let unwrapped = range.start() + (angle - range.start()).rem_euclid(TAU);
        let [start, _, _] = self.evaluate(range.start());
        let [end, _, _] = self.evaluate(range.end());
        let mut best = if start.distance_squared(point) <= end.distance_squared(point) {
            (range.start(), start.distance_squared(point))
        } else {
            (range.end(), end.distance_squared(point))
        };
        if unwrapped <= range.end() {
            let [inside, _, _] = self.evaluate(unwrapped);
            if inside.distance_squared(point) < best.1 {
                best = (unwrapped, inside.distance_squared(point));
            }
        }
        best.0
    }

    pub(crate) fn bounds(&self, range: Interval) -> Aabb {
        conic_bounds(&self.frame, self.radius, self.radius, range)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ellipse {
    frame: Plane,
    major_radius: f64,
    minor_radius: f64,
}

impl Ellipse {
    pub fn new(frame: Plane, major_radius: f64, minor_radius: f64) -> Result<Self, GeometryError> {
        Ok(Self {
            frame: checked_frame(frame)?,
            major_radius: positive(major_radius)?,
            minor_radius: positive(minor_radius)?,
        })
    }

    pub fn frame(&self) -> &Plane {
        &self.frame
    }

    pub fn center(&self) -> Point3 {
        self.frame.origin()
    }

    pub fn major_radius(&self) -> f64 {
        self.major_radius
    }

    pub fn minor_radius(&self) -> f64 {
        self.minor_radius
    }

    #[must_use]
    pub fn reversed(&self) -> Self {
        Self {
            frame: self.frame.flipped(),
            ..*self
        }
    }

    pub(crate) fn evaluate(&self, parameter: f64) -> [Point3; 3] {
        conic_derivatives(&self.frame, self.major_radius, self.minor_radius, parameter)
    }

    pub(crate) fn bounds(&self, range: Interval) -> Aabb {
        conic_bounds(&self.frame, self.major_radius, self.minor_radius, range)
    }
}

fn conic_derivatives(frame: &Plane, along_x: f64, along_y: f64, parameter: f64) -> [Point3; 3] {
    let (sin, cos) = parameter.sin_cos();
    let x = frame.x_axis();
    let y = frame.y_axis();
    let radial = x * (along_x * cos) + y * (along_y * sin);
    let tangent = x * (-along_x * sin) + y * (along_y * cos);
    [frame.origin() + radial, tangent, -radial]
}

fn conic_bounds(frame: &Plane, along_x: f64, along_y: f64, range: Interval) -> Aabb {
    let point = |parameter: f64| {
        let [position, _, _] = conic_derivatives(frame, along_x, along_y, parameter);
        position
    };
    let mut bounds = Aabb::from_point(point(range.start())).including(point(range.end()));
    let x = frame.x_axis().to_array();
    let y = frame.y_axis().to_array();
    for (x_component, y_component) in x.into_iter().zip(y) {
        let extreme = (along_y * y_component).atan2(along_x * x_component);
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

pub(crate) fn conic_seeds(range: Interval) -> Vec<f64> {
    let pieces = (range.length() / (PI / 4.0)).ceil();
    let pieces = if pieces.is_finite() && pieces >= 1.0 {
        (pieces as usize).min(4096)
    } else {
        1
    };
    range.split(pieces).collect()
}
