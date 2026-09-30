mod extrude;
pub(crate) mod plan;
mod reach;
mod revolve;
#[cfg(test)]
mod tests;

use std::f64::consts::TAU;

use caditor_geometry::{Plane, Point2, Vector2, Vector3};
use thiserror::Error;

pub use self::{
    extrude::extrude,
    reach::{Heights, NextFace, ReachError, heights, next_face},
    revolve::revolve,
};
use crate::{
    error::GeometryError,
    interrupt::Interrupted,
    profile::Piece,
    sense::Sense,
    tolerance::{ANGULAR_RESOLUTION, LINEAR_RESOLUTION, MAX_SIZE},
    topology::BuildError,
};

const FULL_TURN_SLACK: f64 = 1e-9;

pub(crate) fn lift(plane: &Plane, direction: Vector2) -> Vector3 {
    plane.x_axis() * direction.x + plane.y_axis() * direction.y
}

pub(crate) fn traversal_sense(piece: &Piece) -> Sense {
    if piece.is_reversed() {
        Sense::Reversed
    } else {
        Sense::Same
    }
}

pub(crate) fn traversal_tangent(piece: &Piece, parameter: f64) -> Vector2 {
    let tangent = piece.curve().evaluate(parameter).first;
    if piece.is_reversed() {
        -tangent
    } else {
        tangent
    }
}

fn curves(entities: &[u64]) -> String {
    let names: Vec<String> = entities.iter().map(u64::to_string).collect();
    match names.as_slice() {
        [] => "no curves".to_owned(),
        [single] => format!("curve {single}"),
        [rest @ .., last] => format!("curves {} and {last}", rest.join(", ")),
    }
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum SweepError {
    #[error("no region is chosen to sweep")]
    NoRegions,
    #[error("an extent or angle is not a finite number")]
    NonFinite,
    #[error("the extrusion has no length")]
    ZeroLength,
    #[error("the extrusion reaches farther than {MAX_SIZE} mm")]
    TooLong,
    #[error("an end plane of the extrusion runs along its direction")]
    EndAlongDirection,
    #[error("the end planes of the extrusion meet or cross within the profile")]
    EndsCross,
    #[error("the revolution has no angle")]
    ZeroAngle,
    #[error("the revolution turns more than once")]
    BeyondFullTurn,
    #[error("the revolution axis has no direction")]
    DegenerateAxis,
    #[error("{} {} the revolution axis", curves(.entities), if .entities.len() == 1 { "crosses" } else { "cross" })]
    CrossesAxis { entities: Vec<u64> },
    #[error("the profile lies on both sides of the revolution axis: {} on one side, {} on the other", curves(.left), curves(.right))]
    BothSidesOfAxis { left: Vec<u64>, right: Vec<u64> },
    #[error("the profile lies on the revolution axis")]
    OnAxis,
    #[error("the swept solid could not be assembled from the profile")]
    Unassembled,
    #[error("the swept geometry cannot be built: {0}")]
    Geometry(#[from] GeometryError),
    #[error("the swept solid is not valid: {0}")]
    Invalid(BuildError),
    #[error(transparent)]
    Cancelled(#[from] Interrupted),
}

impl From<BuildError> for SweepError {
    fn from(error: BuildError) -> Self {
        match error.interrupted() {
            Some(interrupted) => Self::Cancelled(interrupted),
            None => Self::Invalid(error),
        }
    }
}

impl SweepError {
    pub fn entities(&self) -> Vec<u64> {
        match self {
            Self::CrossesAxis { entities } => entities.clone(),
            Self::BothSidesOfAxis { left, right } => left.iter().chain(right).copied().collect(),
            Self::NoRegions
            | Self::NonFinite
            | Self::ZeroLength
            | Self::TooLong
            | Self::EndAlongDirection
            | Self::EndsCross
            | Self::ZeroAngle
            | Self::BeyondFullTurn
            | Self::DegenerateAxis
            | Self::OnAxis
            | Self::Unassembled
            | Self::Geometry(_)
            | Self::Invalid(_)
            | Self::Cancelled(_) => Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LinearBound {
    Offset(f64),
    Plane(Plane),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinearExtent {
    start: LinearBound,
    end: LinearBound,
}

impl LinearExtent {
    pub fn new(start: f64, end: f64) -> Result<Self, SweepError> {
        if !start.is_finite() || !end.is_finite() {
            return Err(SweepError::NonFinite);
        }
        if (end - start).abs() <= LINEAR_RESOLUTION {
            return Err(SweepError::ZeroLength);
        }
        if start.abs().max(end.abs()) > MAX_SIZE {
            return Err(SweepError::TooLong);
        }
        Ok(Self {
            start: LinearBound::Offset(start),
            end: LinearBound::Offset(end),
        })
    }

    pub fn one_side(distance: f64) -> Result<Self, SweepError> {
        Self::new(0.0, distance)
    }

    pub fn two_sided(forward: f64, backward: f64) -> Result<Self, SweepError> {
        Self::new(-backward, forward)
    }

    pub fn symmetric(total: f64) -> Result<Self, SweepError> {
        Self::new(-0.5 * total, 0.5 * total)
    }

    pub fn between(start: LinearBound, end: LinearBound) -> Result<Self, SweepError> {
        match (start, end) {
            (LinearBound::Offset(start), LinearBound::Offset(end)) => Self::new(start, end),
            _ => {
                for bound in [start, end] {
                    match bound {
                        LinearBound::Offset(offset) if !offset.is_finite() => {
                            return Err(SweepError::NonFinite);
                        }
                        LinearBound::Offset(offset) if offset.abs() > MAX_SIZE => {
                            return Err(SweepError::TooLong);
                        }
                        LinearBound::Plane(plane)
                            if !plane.origin().is_finite() || !plane.normal().is_finite() =>
                        {
                            return Err(SweepError::NonFinite);
                        }
                        LinearBound::Offset(_) | LinearBound::Plane(_) => {}
                    }
                }
                Ok(Self { start, end })
            }
        }
    }

    pub fn start(&self) -> LinearBound {
        self.start
    }

    pub fn end(&self) -> LinearBound {
        self.end
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AngularExtent {
    start: f64,
    end: f64,
}

impl AngularExtent {
    pub fn new(start: f64, end: f64) -> Result<Self, SweepError> {
        if !start.is_finite() || !end.is_finite() {
            return Err(SweepError::NonFinite);
        }
        let sweep = (end - start).abs();
        if sweep <= ANGULAR_RESOLUTION {
            return Err(SweepError::ZeroAngle);
        }
        if sweep > TAU + FULL_TURN_SLACK {
            return Err(SweepError::BeyondFullTurn);
        }
        Ok(Self { start, end })
    }

    pub fn full() -> Self {
        Self {
            start: 0.0,
            end: TAU,
        }
    }

    pub fn one_side(angle: f64) -> Result<Self, SweepError> {
        Self::new(0.0, angle)
    }

    pub fn symmetric(total: f64) -> Result<Self, SweepError> {
        Self::new(-0.5 * total, 0.5 * total)
    }

    pub fn start(&self) -> f64 {
        self.start
    }

    pub fn end(&self) -> f64 {
        self.end
    }

    pub fn sweep(&self) -> f64 {
        (self.end - self.start).abs()
    }

    pub fn is_full(&self) -> bool {
        self.sweep() >= TAU - FULL_TURN_SLACK
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Axis2 {
    origin: Point2,
    direction: Vector2,
}

impl Axis2 {
    pub fn new(origin: Point2, direction: Vector2) -> Result<Self, SweepError> {
        if !origin.is_finite() || !direction.is_finite() {
            return Err(SweepError::NonFinite);
        }
        let direction = direction
            .try_normalize()
            .ok_or(SweepError::DegenerateAxis)?;
        Ok(Self { origin, direction })
    }

    pub fn through(start: Point2, end: Point2) -> Result<Self, SweepError> {
        Self::new(start, end - start)
    }

    pub fn origin(&self) -> Point2 {
        self.origin
    }

    pub fn direction(&self) -> Vector2 {
        self.direction
    }

    pub(crate) fn signed_distance(&self, point: Point2) -> f64 {
        self.direction.perp_dot(point - self.origin)
    }

    pub(crate) fn along(&self, point: Point2) -> f64 {
        self.direction.dot(point - self.origin)
    }

    #[must_use]
    pub(crate) fn reversed(&self) -> Self {
        Self {
            origin: self.origin,
            direction: -self.direction,
        }
    }
}
