mod faces;
mod helix;
#[cfg(test)]
mod tests;

use std::f64::consts::{FRAC_1_SQRT_2, TAU};

use caditor_geometry::{Plane, Point2};
use thiserror::Error;

pub use self::helix::{HelixError, HelixSpec, HelixTurn, helix};
use crate::{
    error::GeometryError,
    interrupt::Interrupted,
    thread::Handedness,
    tolerance::{LINEAR_RESOLUTION, MAX_SIZE},
    topology::{BuildError, Solid},
};

pub const MAX_COIL_TURNS: f64 = 500.0;
pub const LEAST_COIL_TURNS: f64 = 0.05;
pub const MAX_COIL_TAPER_DEGREES: f64 = 80.0;
pub const COIL_PARTS: CoilParts = CoilParts {
    start: 5,
    end: 6,
    per_turn: 8,
};
const LEAST_GAP: f64 = 0.01;
const AXIS_CLEARANCE: f64 = 10.0 * LINEAR_RESOLUTION;
const STRAIGHT_TAPER: f64 = 1e-12;
const SECTION_PIECES: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoilShape {
    Round,
    Square,
}

impl CoilShape {
    pub const ALL: [Self; 2] = [Self::Round, Self::Square];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoilPlacement {
    Inside,
    Centred,
    Outside,
}

impl CoilPlacement {
    pub const ALL: [Self; 3] = [Self::Inside, Self::Centred, Self::Outside];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoilFace {
    Bottom,
    Outside,
    Top,
    Inside,
    Start,
    End,
}

impl CoilFace {
    const SIDES: [Self; SECTION_PIECES] = [Self::Bottom, Self::Outside, Self::Top, Self::Inside];

    pub fn of_entity(entity: u64) -> Option<Self> {
        let parts = COIL_PARTS;
        if entity == parts.start {
            return Some(Self::Start);
        }
        if entity == parts.end {
            return Some(Self::End);
        }
        let side = usize::try_from(entity % parts.per_turn)
            .ok()?
            .checked_sub(1)?;
        Self::SIDES.get(side).copied()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoilParts {
    pub start: u64,
    pub end: u64,
    pub per_turn: u64,
}

impl CoilParts {
    pub fn side(self, turn: usize, piece: usize) -> u64 {
        (piece as u64 + 1).wrapping_add(self.per_turn.wrapping_mul(turn as u64))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoilSpec {
    pub frame: Plane,
    pub radius: f64,
    pub shape: CoilShape,
    pub size: f64,
    pub placement: CoilPlacement,
    pub pitch: f64,
    pub turns: f64,
    pub taper: f64,
    pub hand: Handedness,
}

impl CoilSpec {
    fn values(&self) -> [f64; 5] {
        [self.radius, self.size, self.pitch, self.turns, self.taper]
    }

    pub fn height(&self) -> f64 {
        self.pitch * self.turns
    }

    pub fn section_area(&self) -> f64 {
        match self.shape {
            CoilShape::Round => std::f64::consts::PI * 0.25 * self.size * self.size,
            CoilShape::Square => self.size * self.size,
        }
    }

    pub fn centre_radius(&self) -> f64 {
        let half = 0.5 * self.size;
        match self.placement {
            CoilPlacement::Inside => self.radius - half,
            CoilPlacement::Centred => self.radius,
            CoilPlacement::Outside => self.radius + half,
        }
    }

    pub fn radial_rise(&self) -> f64 {
        self.taper.tan() * self.height()
    }

    pub fn volume(&self) -> f64 {
        let centre = self.centre_radius();
        self.section_area() * TAU * self.turns * (centre + 0.5 * self.radial_rise())
    }
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum CoilError {
    #[error("a coil dimension is not a finite number")]
    NonFinite,
    #[error("the coil's radius must be more than zero")]
    NoRadius,
    #[error("the coil's section must have a size")]
    NoSection,
    #[error("the pitch must be more than zero")]
    NoPitch,
    #[error("the coil must make at least {LEAST_COIL_TURNS} of a turn, not {turns}")]
    TooFewTurns { turns: f64 },
    #[error("the coil would turn {turns} times, more than {MAX_COIL_TURNS}")]
    TooManyTurns { turns: f64 },
    #[error("the taper must be less than {MAX_COIL_TAPER_DEGREES}°")]
    TaperTooSteep,
    #[error("the turns would touch: the pitch {pitch} mm must be more than {least} mm")]
    TurnsOverlap { pitch: f64, least: f64 },
    #[error("the section reaches the coil's axis")]
    ReachesAxis,
    #[error("the coil reaches farther than {MAX_SIZE} mm")]
    TooLarge,
    #[error("the coil's geometry cannot be built: {0}")]
    Geometry(#[from] GeometryError),
    #[error("the coil could not be assembled")]
    Unassembled,
    #[error("the coil is not a valid solid: {0}")]
    Invalid(BuildError),
    #[error(transparent)]
    Cancelled(#[from] Interrupted),
}

impl From<BuildError> for CoilError {
    fn from(error: BuildError) -> Self {
        match error.interrupted() {
            Some(interrupted) => Self::Cancelled(interrupted),
            None => Self::Invalid(error),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct SectionPiece {
    points: Vec<Point2>,
    weights: Vec<f64>,
}

impl SectionPiece {
    fn line(from: Point2, to: Point2) -> Self {
        Self {
            points: vec![from, to],
            weights: vec![1.0, 1.0],
        }
    }

    fn quarter(from: Point2, corner: Point2, to: Point2) -> Self {
        Self {
            points: vec![from, corner, to],
            weights: vec![1.0, FRAC_1_SQRT_2, 1.0],
        }
    }

    fn is_arc(&self) -> bool {
        self.points.len() > 2
    }

    fn start(&self) -> Point2 {
        self.points.first().copied().unwrap_or(Point2::ZERO)
    }

    fn end(&self) -> Point2 {
        self.points.last().copied().unwrap_or(Point2::ZERO)
    }

    fn is_upright(&self) -> bool {
        !self.is_arc() && (self.start().x - self.end().x).abs() <= LINEAR_RESOLUTION
    }

    fn middle(&self) -> Point2 {
        let (sum, total) = self.points.iter().zip(&self.weights).enumerate().fold(
            (Point2::ZERO, 0.0),
            |(sum, total), (index, (point, weight))| {
                let basis = if self.is_arc() {
                    [0.25, 0.5, 0.25].get(index).copied().unwrap_or(0.0)
                } else {
                    0.5
                };
                (sum + *point * (basis * weight), total + basis * weight)
            },
        );
        sum / total
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Winding {
    frame: Plane,
    hand: Handedness,
    pitch: f64,
    slope: f64,
}

#[derive(Debug, Clone, PartialEq)]
struct Layout {
    winding: Winding,
    turns: f64,
    centre: Point2,
    section: [SectionPiece; SECTION_PIECES],
}

impl Layout {
    fn sweep(&self) -> f64 {
        TAU * self.turns
    }
}

fn section(shape: CoilShape, centre: Point2, half: f64) -> [SectionPiece; SECTION_PIECES] {
    let at = |r: f64, z: f64| centre + Point2::new(r, z) * half;
    match shape {
        CoilShape::Square => {
            let corners = [at(-1.0, -1.0), at(1.0, -1.0), at(1.0, 1.0), at(-1.0, 1.0)];
            let [first, second, third, fourth] = corners;
            [
                SectionPiece::line(first, second),
                SectionPiece::line(second, third),
                SectionPiece::line(third, fourth),
                SectionPiece::line(fourth, first),
            ]
        }
        CoilShape::Round => [
            SectionPiece::quarter(at(0.0, -1.0), at(1.0, -1.0), at(1.0, 0.0)),
            SectionPiece::quarter(at(1.0, 0.0), at(1.0, 1.0), at(0.0, 1.0)),
            SectionPiece::quarter(at(0.0, 1.0), at(-1.0, 1.0), at(-1.0, 0.0)),
            SectionPiece::quarter(at(-1.0, 0.0), at(-1.0, -1.0), at(0.0, -1.0)),
        ],
    }
}

fn least_pitch(spec: &CoilSpec) -> f64 {
    let gap = (LEAST_GAP * spec.size).max(AXIS_CLEARANCE);
    let reach = spec.size + gap;
    let slant = spec.taper.tan();
    match spec.shape {
        CoilShape::Square => reach / slant.abs().max(1.0),
        CoilShape::Round => reach / (1.0 + slant * slant).sqrt(),
    }
}

fn turns_apart(spec: &CoilSpec) -> bool {
    let reach = spec.size + (LEAST_GAP * spec.size).max(AXIS_CLEARANCE);
    let across = spec.pitch * spec.taper.tan();
    match spec.shape {
        CoilShape::Square => spec.pitch.max(across.abs()) > reach,
        CoilShape::Round => spec.pitch.hypot(across) > reach,
    }
}

fn layout(spec: &CoilSpec) -> Result<Layout, CoilError> {
    if !spec.values().into_iter().all(f64::is_finite) {
        return Err(CoilError::NonFinite);
    }
    if spec.radius <= LINEAR_RESOLUTION {
        return Err(CoilError::NoRadius);
    }
    if spec.size <= LINEAR_RESOLUTION {
        return Err(CoilError::NoSection);
    }
    if spec.pitch <= LINEAR_RESOLUTION {
        return Err(CoilError::NoPitch);
    }
    if spec.turns < LEAST_COIL_TURNS {
        return Err(CoilError::TooFewTurns { turns: spec.turns });
    }
    if spec.turns > MAX_COIL_TURNS {
        return Err(CoilError::TooManyTurns { turns: spec.turns });
    }
    if spec.taper.abs() >= MAX_COIL_TAPER_DEGREES.to_radians() {
        return Err(CoilError::TaperTooSteep);
    }
    let reach = spec.radius
        + spec.size
        + spec.radial_rise().abs()
        + spec.height()
        + spec.frame.origin().abs().max_element();
    if reach > MAX_SIZE {
        return Err(CoilError::TooLarge);
    }
    let half = 0.5 * spec.size;
    let centre = spec.centre_radius();
    let innermost = centre - half + spec.radial_rise().min(0.0);
    if innermost <= AXIS_CLEARANCE {
        return Err(CoilError::ReachesAxis);
    }
    if !turns_apart(spec) {
        return Err(CoilError::TurnsOverlap {
            pitch: spec.pitch,
            least: least_pitch(spec),
        });
    }
    let taper = if spec.taper.abs() < STRAIGHT_TAPER {
        0.0
    } else {
        spec.taper
    };
    let centre = Point2::new(centre, half);
    Ok(Layout {
        winding: Winding {
            frame: spec.frame,
            hand: spec.hand,
            pitch: spec.pitch,
            slope: spec.pitch * taper.tan() / TAU,
        },
        turns: spec.turns,
        centre,
        section: section(spec.shape, centre, half),
    })
}

pub fn coil(spec: &CoilSpec, feature: u64) -> Result<Solid, CoilError> {
    let layout = layout(spec)?;
    faces::build(&layout, feature)
}
