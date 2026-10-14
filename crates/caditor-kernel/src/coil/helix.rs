use std::f64::consts::TAU;

use caditor_geometry::{Plane, Point2, Point3, Vector3};
use thiserror::Error;

use super::{
    AXIS_CLEARANCE, LEAST_COIL_TURNS, MAX_COIL_TAPER_DEGREES, MAX_COIL_TURNS, STRAIGHT_TAPER,
    Winding,
    faces::{Motion, pieces},
};
use crate::{
    build::PathSegment,
    curve::Curve,
    error::GeometryError,
    interval::Interval,
    thread::Handedness,
    tolerance::{LINEAR_RESOLUTION, MAX_SIZE},
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HelixSpec {
    pub frame: Plane,
    pub radius: f64,
    pub pitch: f64,
    pub turns: f64,
    pub taper: f64,
    pub hand: Handedness,
}

impl HelixSpec {
    fn values(&self) -> [f64; 4] {
        [self.radius, self.pitch, self.turns, self.taper]
    }

    pub fn height(&self) -> f64 {
        self.pitch * self.turns
    }

    pub fn end_radius(&self) -> f64 {
        self.radius + self.taper.tan() * self.height()
    }

    pub fn axis(&self) -> (Point3, Vector3) {
        (self.frame.origin(), self.frame.normal())
    }

    pub fn start(&self) -> Point3 {
        self.frame.origin() + self.frame.x_axis() * self.radius
    }
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum HelixError {
    #[error("a helix dimension is not a finite number")]
    NonFinite,
    #[error("the helix's radius must be more than zero")]
    NoRadius,
    #[error("the pitch must be more than zero")]
    NoPitch,
    #[error("the helix must make at least {LEAST_COIL_TURNS} of a turn, not {turns}")]
    TooFewTurns { turns: f64 },
    #[error("the helix would turn {turns} times, more than {MAX_COIL_TURNS}")]
    TooManyTurns { turns: f64 },
    #[error("the taper must be less than {MAX_COIL_TAPER_DEGREES}°")]
    TaperTooSteep,
    #[error("the taper draws the helix in to its axis")]
    ReachesAxis,
    #[error("the helix reaches farther than {MAX_SIZE} mm")]
    TooLarge,
    #[error("the helix's geometry cannot be built: {0}")]
    Geometry(#[from] GeometryError),
}

#[derive(Debug, Clone, PartialEq)]
pub struct HelixTurn {
    curve: Curve,
    range: Interval,
    axis: (Point3, Vector3),
}

impl HelixTurn {
    pub fn curve(&self) -> &Curve {
        &self.curve
    }

    pub fn range(&self) -> Interval {
        self.range
    }

    pub fn path_segment(&self, key: u128) -> PathSegment {
        let (origin, direction) = self.axis;
        PathSegment::new(self.curve.clone(), self.range, key).winding_about(origin, direction)
    }
}

fn checked(spec: &HelixSpec) -> Result<f64, HelixError> {
    if !spec.values().into_iter().all(f64::is_finite) {
        return Err(HelixError::NonFinite);
    }
    if spec.radius <= LINEAR_RESOLUTION {
        return Err(HelixError::NoRadius);
    }
    if spec.pitch <= LINEAR_RESOLUTION {
        return Err(HelixError::NoPitch);
    }
    if spec.turns < LEAST_COIL_TURNS {
        return Err(HelixError::TooFewTurns { turns: spec.turns });
    }
    if spec.turns > MAX_COIL_TURNS {
        return Err(HelixError::TooManyTurns { turns: spec.turns });
    }
    if spec.taper.abs() >= MAX_COIL_TAPER_DEGREES.to_radians() {
        return Err(HelixError::TaperTooSteep);
    }
    let reach = spec.radius.max(spec.end_radius())
        + spec.height()
        + spec.frame.origin().abs().max_element();
    if reach > MAX_SIZE {
        return Err(HelixError::TooLarge);
    }
    if spec.end_radius() <= AXIS_CLEARANCE {
        return Err(HelixError::ReachesAxis);
    }
    Ok(if spec.taper.abs() < STRAIGHT_TAPER {
        0.0
    } else {
        spec.taper
    })
}

pub fn helix(spec: &HelixSpec) -> Result<Vec<HelixTurn>, HelixError> {
    let taper = checked(spec)?;
    let motion = Motion {
        winding: Winding {
            frame: spec.frame,
            hand: spec.hand,
            pitch: spec.pitch,
            slope: spec.pitch * taper.tan() / TAU,
        },
        centre: Point2::ZERO,
    };
    let at = Point2::new(spec.radius, 0.0);
    pieces(spec.turns)
        .into_iter()
        .map(|piece| {
            let curve = motion.helix(at, piece)?;
            let range = Interval::new(0.0, piece.spans() as f64).ok_or(GeometryError::NonFinite)?;
            Ok(HelixTurn {
                curve,
                range,
                axis: spec.axis(),
            })
        })
        .collect()
}
