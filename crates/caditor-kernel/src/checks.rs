use caditor_geometry::{Plane, Point2, Point3, Vector2, Vector3};

use crate::error::GeometryError;

pub(crate) fn finite_point(point: Point3) -> Result<Point3, GeometryError> {
    if point.is_finite() {
        Ok(point)
    } else {
        Err(GeometryError::NonFinite)
    }
}

pub(crate) fn finite_point2(point: Point2) -> Result<Point2, GeometryError> {
    if point.is_finite() {
        Ok(point)
    } else {
        Err(GeometryError::NonFinite)
    }
}

pub(crate) fn unit(direction: Vector3) -> Result<Vector3, GeometryError> {
    if !direction.is_finite() {
        return Err(GeometryError::NonFinite);
    }
    direction
        .try_normalize()
        .ok_or(GeometryError::ZeroDirection)
}

pub(crate) fn unit2(direction: Vector2) -> Result<Vector2, GeometryError> {
    if !direction.is_finite() {
        return Err(GeometryError::NonFinite);
    }
    direction
        .try_normalize()
        .ok_or(GeometryError::ZeroDirection)
}

pub(crate) fn positive(value: f64) -> Result<f64, GeometryError> {
    if !value.is_finite() {
        Err(GeometryError::NonFinite)
    } else if value <= 0.0 {
        Err(GeometryError::NonPositive(value))
    } else {
        Ok(value)
    }
}

pub(crate) fn checked_frame(frame: Plane) -> Result<Plane, GeometryError> {
    let finite =
        frame.origin().is_finite() && frame.normal().is_finite() && frame.x_axis().is_finite();
    if !finite {
        return Err(GeometryError::NonFinite);
    }
    Plane::from_frame(frame.origin(), frame.normal(), frame.x_axis())
        .ok_or(GeometryError::ZeroDirection)
}
