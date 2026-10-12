mod groove;
#[cfg(test)]
mod tests;

use std::f64::consts::TAU;

use caditor_geometry::{Plane, Point2, Vector2};
use thiserror::Error;

pub use self::groove::{GROOVE_PARTS, SPANS_PER_TURN};
use crate::{
    boolean::{BooleanError, BooleanOperation, boolean},
    build::{AngularExtent, Axis2, SweepError, revolve},
    error::GeometryError,
    interrupt::{self, Interrupted},
    profile::{Profile, ProfileCurve, ProfileError, Selection},
    tolerance::{LINEAR_RESOLUTION, MAX_SIZE},
    topology::{BuildError, Solid},
};

pub const MAX_THREAD_TURNS: f64 = 2000.0;
pub const RING_ENTITY: u64 = 1 << 40;
const FAR_REACH: f64 = 0.05;
const RUN_OUT: f64 = 1.5;
const LEAST_LAND: f64 = 0.02;
const RING_SLACK: f64 = 10.0 * LINEAR_RESOLUTION;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThreadedFace {
    Shaft,
    Bore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Handedness {
    Right,
    Left,
}

impl Handedness {
    fn sign(self) -> f64 {
        match self {
            Self::Right => 1.0,
            Self::Left => -1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThreadEnd {
    RunsOut,
    Stops,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThreadForm {
    pub pitch: f64,
    pub flank_angle: f64,
    pub root_radius: f64,
    pub root_width: f64,
    pub crest_radius: f64,
}

impl ThreadForm {
    pub fn half_width_at(&self, radius: f64) -> f64 {
        0.5 * self.root_width + (radius - self.root_radius).abs() * self.flank_angle.tan()
    }

    fn values(&self) -> [f64; 5] {
        [
            self.pitch,
            self.flank_angle,
            self.root_radius,
            self.root_width,
            self.crest_radius,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThreadSpec {
    pub frame: Plane,
    pub face: ThreadedFace,
    pub face_radius: f64,
    pub form: ThreadForm,
    pub hand: Handedness,
    pub length: f64,
    pub start: ThreadEnd,
    pub end: ThreadEnd,
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum ThreadError {
    #[error("a thread dimension is not a finite number")]
    NonFinite,
    #[error("the pitch must be more than zero")]
    NoPitch,
    #[error("the flanks must lean between 0° and 90°")]
    FlankAngle,
    #[error("the groove must have a width at its root")]
    NoRootWidth,
    #[error("the groove is as wide as the pitch, so turns of it would overlap")]
    GrooveOverlaps,
    #[error("the root of the thread lies on the wrong side of the face")]
    RootOutsideFace,
    #[error("the crest of the thread lies on the wrong side of its root")]
    CrestBeyondRoot,
    #[error("the thread is {length} mm long, shorter than the {needed} mm its form needs")]
    TooShort { length: f64, needed: f64 },
    #[error("the thread would turn {turns} times, more than {MAX_THREAD_TURNS}")]
    TooManyTurns { turns: f64 },
    #[error("the thread reaches farther than {MAX_SIZE} mm")]
    TooLarge,
    #[error("the thread's geometry cannot be built: {0}")]
    Geometry(#[from] GeometryError),
    #[error("the thread's groove could not be assembled")]
    GrooveUnassembled,
    #[error("the thread's groove is not a valid solid: {0}")]
    Groove(BuildError),
    #[error("the face could not be brought to the thread's crest: {0}")]
    Ring(SweepError),
    #[error("the groove could not be cut into the body: {0}")]
    Cut(BooleanError),
    #[error(transparent)]
    Cancelled(#[from] Interrupted),
}

impl From<BuildError> for ThreadError {
    fn from(error: BuildError) -> Self {
        match error.interrupted() {
            Some(interrupted) => Self::Cancelled(interrupted),
            None => Self::Groove(error),
        }
    }
}

impl From<BooleanError> for ThreadError {
    fn from(error: BooleanError) -> Self {
        match error {
            BooleanError::Cancelled(interrupted) => Self::Cancelled(interrupted),
            error => Self::Cut(error),
        }
    }
}

impl From<SweepError> for ThreadError {
    fn from(error: SweepError) -> Self {
        match error {
            SweepError::Cancelled(interrupted) => Self::Cancelled(interrupted),
            error => Self::Ring(error),
        }
    }
}

impl From<ProfileError> for ThreadError {
    fn from(_: ProfileError) -> Self {
        Self::Ring(SweepError::NoRegions)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Groove {
    pub frame: Plane,
    pub hand: Handedness,
    pub pitch: f64,
    pub root_radius: f64,
    pub root_half_width: f64,
    pub far_radius: f64,
    pub far_half_width: f64,
    pub first_centre: f64,
    pub turns: f64,
    pub crossings: Vec<f64>,
}

impl Groove {
    fn of(spec: &ThreadSpec, far_radius: f64) -> Result<Self, ThreadError> {
        let form = spec.form;
        let far_half_width = form.half_width_at(far_radius);
        if 2.0 * far_half_width > (1.0 - LEAST_LAND) * form.pitch {
            return Err(ThreadError::GrooveOverlaps);
        }
        let first_centre = match spec.start {
            ThreadEnd::RunsOut => -RUN_OUT * form.pitch,
            ThreadEnd::Stops => far_half_width,
        };
        let last_centre = match spec.end {
            ThreadEnd::RunsOut => spec.length + RUN_OUT * form.pitch,
            ThreadEnd::Stops => spec.length - far_half_width,
        };
        let needed = 2.0 * far_half_width + form.pitch;
        if last_centre - first_centre < form.pitch || spec.length < LINEAR_RESOLUTION {
            return Err(ThreadError::TooShort {
                length: spec.length,
                needed,
            });
        }
        let turns = (last_centre - first_centre) / form.pitch;
        if turns > MAX_THREAD_TURNS {
            return Err(ThreadError::TooManyTurns { turns });
        }
        let turns_to = |along: f64| (along - first_centre) / form.pitch;
        let crossings = [
            (spec.start == ThreadEnd::RunsOut).then(|| turns_to(0.0)),
            (spec.end == ThreadEnd::RunsOut).then(|| turns_to(spec.length)),
        ]
        .into_iter()
        .flatten()
        .collect();
        Ok(Self {
            frame: spec.frame,
            hand: spec.hand,
            pitch: form.pitch,
            root_radius: form.root_radius,
            root_half_width: 0.5 * form.root_width,
            far_radius,
            far_half_width,
            first_centre,
            turns,
            crossings,
        })
    }

    pub fn sweep(&self) -> f64 {
        TAU * self.turns
    }
}

pub fn thread(solid: &Solid, spec: &ThreadSpec, feature: u64) -> Result<Solid, ThreadError> {
    let tools = thread_tools(spec, feature)?;
    let mut body = solid.clone();
    for tool in &tools {
        interrupt::check()?;
        body = boolean(&body, tool, BooleanOperation::Difference)?;
    }
    Ok(body)
}

pub fn thread_tools(spec: &ThreadSpec, feature: u64) -> Result<Vec<Solid>, ThreadError> {
    check(spec)?;
    let form = spec.form;
    let reach = FAR_REACH * form.pitch;
    let mut tools = Vec::new();
    let far_radius = match spec.face {
        ThreadedFace::Shaft if spec.face_radius > form.crest_radius + RING_SLACK => {
            tools.push(ring(
                spec,
                form.crest_radius,
                spec.face_radius + reach,
                feature,
            )?);
            form.crest_radius + reach
        }
        ThreadedFace::Shaft => spec.face_radius + reach,
        ThreadedFace::Bore if spec.face_radius < form.crest_radius - RING_SLACK => {
            tools.push(ring(
                spec,
                spec.face_radius - reach,
                form.crest_radius,
                feature,
            )?);
            form.crest_radius - reach
        }
        ThreadedFace::Bore => spec.face_radius - reach,
    };
    if far_radius <= LINEAR_RESOLUTION {
        return Err(ThreadError::RootOutsideFace);
    }
    let groove = Groove::of(spec, far_radius)?;
    tools.push(groove::tool(&groove, feature)?);
    Ok(tools)
}

fn check(spec: &ThreadSpec) -> Result<(), ThreadError> {
    let form = spec.form;
    let finite = form
        .values()
        .into_iter()
        .chain([spec.face_radius, spec.length])
        .all(f64::is_finite);
    if !finite {
        return Err(ThreadError::NonFinite);
    }
    if form.pitch <= LINEAR_RESOLUTION {
        return Err(ThreadError::NoPitch);
    }
    if !(form.flank_angle > 0.0 && form.flank_angle < 0.5 * std::f64::consts::PI) {
        return Err(ThreadError::FlankAngle);
    }
    if form.root_width <= LINEAR_RESOLUTION {
        return Err(ThreadError::NoRootWidth);
    }
    let reach = [form.root_radius, form.crest_radius, spec.face_radius]
        .into_iter()
        .fold(spec.length, f64::max)
        + spec.frame.origin().abs().max_element();
    if reach > MAX_SIZE {
        return Err(ThreadError::TooLarge);
    }
    let (root_inside, crest_beyond) = match spec.face {
        ThreadedFace::Shaft => (
            form.root_radius > LINEAR_RESOLUTION && form.root_radius < spec.face_radius,
            form.crest_radius > form.root_radius,
        ),
        ThreadedFace::Bore => (
            form.root_radius > spec.face_radius,
            form.crest_radius < form.root_radius && form.crest_radius > LINEAR_RESOLUTION,
        ),
    };
    if !crest_beyond {
        return Err(ThreadError::CrestBeyondRoot);
    }
    if !root_inside {
        return Err(ThreadError::RootOutsideFace);
    }
    Ok(())
}

fn ring(spec: &ThreadSpec, inner: f64, outer: f64, feature: u64) -> Result<Solid, ThreadError> {
    let pitch = spec.form.pitch;
    let from = match spec.start {
        ThreadEnd::RunsOut => -RUN_OUT * pitch,
        ThreadEnd::Stops => 0.0,
    };
    let to = match spec.end {
        ThreadEnd::RunsOut => spec.length + RUN_OUT * pitch,
        ThreadEnd::Stops => spec.length,
    };
    let frame = spec.frame;
    let radial = frame.x_axis();
    let axial = frame.normal();
    let plane = Plane::from_frame(frame.origin(), radial.cross(axial), radial)
        .ok_or(GeometryError::DegenerateSurface)?;
    let corner = |radius: f64, along: f64| Point2::new(radius, along);
    let outline = [
        corner(inner, from),
        corner(outer, from),
        corner(outer, to),
        corner(inner, to),
    ];
    let curves: Vec<ProfileCurve> = (0..outline.len())
        .filter_map(|index| {
            let start = *outline.get(index)?;
            let end = *outline.get((index + 1) % outline.len())?;
            Some(ProfileCurve::line(RING_ENTITY + index as u64, start, end))
        })
        .collect();
    let regions = Profile::new(&curves)?.select(&Selection::EvenDepth)?;
    let axis = Axis2::new(Point2::ZERO, Vector2::Y)?;
    Ok(revolve(
        &plane,
        &regions,
        axis,
        AngularExtent::full(),
        feature,
    )?)
}
