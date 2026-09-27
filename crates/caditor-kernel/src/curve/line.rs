use caditor_geometry::{Point3, Vector3};

use crate::{
    checks::{finite_point, unit},
    error::GeometryError,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Line {
    origin: Point3,
    direction: Vector3,
}

impl Line {
    pub fn new(origin: Point3, direction: Vector3) -> Result<Self, GeometryError> {
        Ok(Self {
            origin: finite_point(origin)?,
            direction: unit(direction)?,
        })
    }

    pub fn through(start: Point3, end: Point3) -> Result<Self, GeometryError> {
        Self::new(start, end - start)
    }

    pub fn origin(&self) -> Point3 {
        self.origin
    }

    pub fn direction(&self) -> Vector3 {
        self.direction
    }

    pub fn point(&self, parameter: f64) -> Point3 {
        self.origin + self.direction * parameter
    }

    #[must_use]
    pub fn reversed(&self) -> Self {
        Self {
            origin: self.origin,
            direction: -self.direction,
        }
    }

    pub fn parameter_of(&self, point: Point3) -> f64 {
        (point - self.origin).dot(self.direction)
    }
}
