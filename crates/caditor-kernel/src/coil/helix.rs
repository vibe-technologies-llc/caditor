use std::f64::consts::TAU;

use caditor_geometry::{Plane, Point2, Point3, RigidTransform, Vector3};
use thiserror::Error;

use super::{
    AXIS_CLEARANCE, LEAST_COIL_TURNS, MAX_COIL_TAPER_DEGREES, MAX_COIL_TURNS, STRAIGHT_TAPER,
    Winding,
    faces::{Motion, pieces_through},
    pitch::{ClosedEndLaw, END_CHANGE_TURNS, PitchLaw},
};
use crate::{
    build::PathSegment,
    curve::Curve,
    error::GeometryError,
    interval::Interval,
    thread::Handedness,
    tolerance::{LINEAR_RESOLUTION, MAX_SIZE},
};

const ROUNDING_SLACK: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HelixEnds {
    pub turns: f64,
    pub pitch: f64,
    pub change: f64,
}

impl HelixEnds {
    pub fn with_default_change(turns: f64, pitch: f64) -> Self {
        Self {
            turns,
            pitch,
            change: END_CHANGE_TURNS,
        }
    }

    pub fn turns_taken(&self) -> f64 {
        2.0 * (self.turns + self.change)
    }

    fn turns_at_end_pitch(&self) -> f64 {
        2.0 * self.turns + self.change
    }

    fn law(&self) -> ClosedEndLaw {
        ClosedEndLaw {
            pitch: self.pitch,
            turns: self.turns,
            change: self.change,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HelixPitch {
    pub ends: Option<HelixEnds>,
    pub final_pitch: Option<f64>,
}

impl HelixPitch {
    pub fn turns_taken(&self) -> f64 {
        self.ends.map_or(0.0, |ends| ends.turns_taken())
    }

    fn at_end_pitch(&self) -> (f64, f64) {
        self.ends
            .map_or((0.0, 0.0), |ends| (ends.turns_at_end_pitch(), ends.pitch))
    }

    fn mean(&self, pitch: f64) -> f64 {
        self.final_pitch.map_or(pitch, |last| 0.5 * (pitch + last))
    }

    pub fn height(&self, pitch: f64, turns: f64) -> f64 {
        let (at_end, end_pitch) = self.at_end_pitch();
        at_end * end_pitch + (turns - at_end) * self.mean(pitch)
    }

    pub fn pitch_for(&self, height: f64, turns: f64) -> f64 {
        let (at_end, end_pitch) = self.at_end_pitch();
        let mean = (height - at_end * end_pitch) / (turns - at_end);
        self.final_pitch.map_or(mean, |last| 2.0 * mean - last)
    }

    pub fn turns_for(&self, height: f64, pitch: f64) -> f64 {
        let (at_end, end_pitch) = self.at_end_pitch();
        (height - at_end * end_pitch) / self.mean(pitch) + at_end
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HelixSpec {
    pub frame: Plane,
    pub radius: f64,
    pub pitch: f64,
    pub turns: f64,
    pub taper: f64,
    pub hand: Handedness,
    pub start_angle: f64,
    pub ends: Option<HelixEnds>,
    pub final_pitch: Option<f64>,
}

impl HelixSpec {
    fn values(&self) -> [f64; 9] {
        let (end_turns, end_pitch, change) = self.ends.map_or((0.0, self.pitch, 0.0), |ends| {
            (ends.turns, ends.pitch, ends.change)
        });
        [
            self.radius,
            self.pitch,
            self.turns,
            self.taper,
            self.start_angle,
            end_turns,
            end_pitch,
            change,
            self.last_pitch(),
        ]
    }

    fn last_pitch(&self) -> f64 {
        self.final_pitch.unwrap_or(self.pitch)
    }

    pub fn plan(&self) -> HelixPitch {
        HelixPitch {
            ends: self.ends,
            final_pitch: self.final_pitch,
        }
    }

    fn law(&self) -> PitchLaw {
        let body = (self.pitch, self.last_pitch());
        match (self.ends, self.final_pitch) {
            (Some(ends), _) => PitchLaw::closed_ends(body, ends.law(), self.turns),
            (None, Some(last)) => PitchLaw::progressive(self.pitch, last, self.turns),
            (None, None) => PitchLaw::even(self.pitch),
        }
    }

    pub fn body_start(&self) -> f64 {
        self.ends
            .map_or(0.0, |ends| TAU * (ends.turns + ends.change))
    }

    pub fn start_frame(&self) -> Option<Plane> {
        let (origin, direction) = self.axis();
        let turn = RigidTransform::rotation_about(origin, direction, self.start_angle)?;
        Some(self.frame.transformed(&turn))
    }

    pub fn point_at(&self, angle: f64) -> Option<Point3> {
        let frame = self.start_frame()?;
        let around =
            frame.x_axis() * angle.cos() + frame.y_axis() * (self.hand.sign() * angle.sin());
        let rise = self.law().rise(angle);
        let radius = self.radius + self.taper.tan() * rise;
        Some(frame.origin() + around * radius + frame.normal() * rise)
    }

    pub fn height(&self) -> f64 {
        self.plan().height(self.pitch, self.turns)
    }

    pub fn end_radius(&self) -> f64 {
        self.radius + self.taper.tan() * self.height()
    }

    pub fn axis(&self) -> (Point3, Vector3) {
        (self.frame.origin(), self.frame.normal())
    }

    pub fn start(&self) -> Option<Point3> {
        self.point_at(0.0)
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
    #[error("a closed end must make no turns or at least {LEAST_COIL_TURNS}, not {turns}")]
    EndTurns { turns: f64 },
    #[error(
        "the closed ends and their changes of pitch take {taken} turns, leaving none or at least \
         {LEAST_COIL_TURNS} for the rest, but the helix makes {turns}"
    )]
    EndsTooLong { taken: f64, turns: f64 },
    #[error("a change of pitch must take at least {LEAST_COIL_TURNS} of a turn, not {turns}")]
    EndChange { turns: f64 },
    #[error(
        "the closed ends and their changes of pitch take all {turns} turns, leaving none for the \
         pitch to change over from {from} to {to}"
    )]
    ProgressionWithoutBody { from: f64, to: f64, turns: f64 },
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
    pub(super) fn new(curve: Curve, range: Interval, axis: (Point3, Vector3)) -> Self {
        Self { curve, range, axis }
    }

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
    if spec
        .final_pitch
        .is_some_and(|last| last <= LINEAR_RESOLUTION)
    {
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
    if let Some(ends) = spec.ends {
        checked_ends(&ends, spec)?;
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

fn checked_ends(ends: &HelixEnds, spec: &HelixSpec) -> Result<(), HelixError> {
    let turns = spec.turns;
    if ends.pitch <= LINEAR_RESOLUTION {
        return Err(HelixError::NoPitch);
    }
    if ends.turns < 0.0 || (ends.turns > 0.0 && ends.turns < LEAST_COIL_TURNS) {
        return Err(HelixError::EndTurns { turns: ends.turns });
    }
    if ends.change < LEAST_COIL_TURNS {
        return Err(HelixError::EndChange { turns: ends.change });
    }
    let taken = ends.turns_taken();
    let rest = turns - taken;
    if rest < -ROUNDING_SLACK || (rest > ROUNDING_SLACK && rest < LEAST_COIL_TURNS) {
        return Err(HelixError::EndsTooLong { taken, turns });
    }
    let last = spec.last_pitch();
    if rest <= ROUNDING_SLACK && (last - spec.pitch).abs() > LINEAR_RESOLUTION {
        return Err(HelixError::ProgressionWithoutBody {
            from: spec.pitch,
            to: last,
            turns,
        });
    }
    Ok(())
}

pub fn helix(spec: &HelixSpec) -> Result<Vec<HelixTurn>, HelixError> {
    let taper = checked(spec)?;
    let frame = spec.start_frame().ok_or(GeometryError::ZeroDirection)?;
    let law = spec.law();
    let motion = Motion {
        winding: Winding {
            frame,
            hand: spec.hand,
            law,
            spread: taper.tan(),
        },
        centre: Point2::ZERO,
    };
    let at = Point2::new(spec.radius, 0.0);
    pieces_through(&law.breaks(spec.turns))
        .into_iter()
        .map(|piece| {
            let curve = motion.helix(at, piece)?;
            let range = Interval::new(0.0, piece.spans() as f64).ok_or(GeometryError::NonFinite)?;
            Ok(HelixTurn::new(curve, range, spec.axis()))
        })
        .collect()
}
