mod bend;
mod path;
mod pipe;
#[cfg(test)]
mod pipe_tests;
#[cfg(test)]
mod shape_tests;
mod stations;
#[cfg(test)]
mod tests;

use std::f64::consts::PI;

use caditor_geometry::{
    Aabb2, Plane, Point2, Point3, RigidTransform, Similarity, Vector2, Vector3,
};
use thiserror::Error;

pub use self::pipe::{
    BendMiddle, BendMiddles, PipeCorner, PipeError, PipeSection, PipeShape, PipeStand, pipe,
    pipe_stand,
};
use self::{
    path::{Chain, Leg},
    stations::{
        FIT_TOLERANCE, FramesAt, ShapeAt, Stations, rotating, rotation_minimising, sliding,
        sliding_surface, winding,
    },
};
use super::{
    Axis2, LinearBound, SweepError,
    extrude::{Cap, Level, side_surface, span},
    lift,
    plan::{Plan, PlanCoedge, PlanFace, outward_sense},
    revolve::{Frame, Kind, arc_spline, ellipse_spline, prepare, surface_of},
    traversal_sense, traversal_tangent,
};
use crate::{
    bspline::BSpline,
    curve::{BSplineCurve, Circle, Curve},
    curve2::{BSplineCurve2, Curve2},
    error::GeometryError,
    interrupt::{self, Interrupted},
    interval::Interval,
    naming::{EdgeName, FaceName, FaceOrigin},
    profile::{Piece, Region},
    sense::Sense,
    surface::{PlaneSurface, Surface},
    tolerance::{LINEAR_RESOLUTION, MAX_SIZE, SamplingTolerance},
    topology::{CrossingCheck, Solid},
};

const RELATIVE_TOLERANCE: f64 = 1e-7;
const LEAST_RISE: f64 = 1e-2;
const FOLD_MARGIN: f64 = 1e-3;
const SMOOTH_TURN: f64 = 1e-6;
const MOST_MITRE_TURN: f64 = PI * 0.95;
const MERIDIAN_SLACK: f64 = 0.1 * LINEAR_RESOLUTION;
const STATIONS_PER_SPAN: usize = 4;
const LEAST_STATIONS: usize = 8;
const STATION_TURN: f64 = PI / 8.0;
const RISE_SAMPLES_PER_SPAN: usize = 16;
const CLOSING_SLACK: f64 = 1e-4;
const TILT_SLACK: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindingAxis {
    pub origin: Point3,
    pub direction: Vector3,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PathSegment {
    curve: Curve,
    range: Interval,
    key: u128,
    winding: Option<WindingAxis>,
}

impl PathSegment {
    pub fn new(curve: Curve, range: Interval, key: u128) -> Self {
        Self {
            curve,
            range,
            key,
            winding: None,
        }
    }

    #[must_use]
    pub fn winding_about(self, origin: Point3, direction: Vector3) -> Self {
        Self {
            winding: Some(WindingAxis { origin, direction }),
            ..self
        }
    }

    pub fn winding(&self) -> Option<WindingAxis> {
        self.winding
    }

    pub fn curve(&self) -> &Curve {
        &self.curve
    }

    pub fn range(&self) -> Interval {
        self.range
    }

    pub fn key(&self) -> u128 {
        self.key
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum SweepOrientation {
    #[default]
    Square,
    Parallel,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SweepShape {
    pub twist: f64,
    pub scale: f64,
}

impl SweepShape {
    pub const NONE: Self = Self {
        twist: 0.0,
        scale: 1.0,
    };

    pub fn is_none(&self) -> bool {
        self.twist == 0.0 && self.scale == 1.0
    }

    fn usable(&self) -> bool {
        self.twist.is_finite() && Similarity::scaling(Point3::ZERO, self.scale).is_some()
    }
}

impl Default for SweepShape {
    fn default() -> Self {
        Self::NONE
    }
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum PathSweepError {
    #[error("no path is chosen to sweep along")]
    NoPath,
    #[error("path curve {segment} is too short to sweep along")]
    SegmentTooShort { segment: usize },
    #[error("the path branches where path curves {segments:?} meet")]
    Branches { segments: Vec<usize> },
    #[error("path curves {segments:?} do not join the rest of the path")]
    Apart { segments: Vec<usize> },
    #[error("path curve {segment} cannot be swept along")]
    UnsupportedPath { segment: usize },
    #[error("the path reaches farther than {MAX_SIZE} mm")]
    TooLong,
    #[error("the path leaves the profile along the profile's plane")]
    StartsAlongProfile,
    #[error("path curve {segment} turns to run along the profile's plane")]
    TurnsAlongProfile { segment: usize },
    #[error("the path turns a sharp corner between path curves {segments:?}")]
    Corner { segments: [usize; 2] },
    #[error("the corner between path curves {segments:?} is too sharp for the profile")]
    CornerTooSharp { segments: [usize; 2] },
    #[error("the profile is too large for the bend of path curve {segment}")]
    ProfileTooLarge { segment: usize },
    #[error("the closed path never crosses the profile's plane")]
    ClosedPathMissesProfile,
    #[error("the closed path brings the profile back turned")]
    ClosedPathTwists { turned: Option<f64> },
    #[error("a sweep along a closed path cannot change the profile's size")]
    ClosedPathScales,
    #[error("the twist or end scale is out of range")]
    UnusableShape,
    #[error(
        "the path turns a sharp corner between path curves {segments:?}, which a twisted or \
         scaled sweep cannot mitre"
    )]
    ShapedCorner { segments: [usize; 2] },
    #[error("path curve {segment} bends too intricately to sweep along")]
    TooIntricate { segment: usize },
    #[error("the swept solid runs into itself")]
    CrossesItself,
    #[error(transparent)]
    Sweep(SweepError),
    #[error(transparent)]
    Cancelled(Interrupted),
}

impl From<SweepError> for PathSweepError {
    fn from(error: SweepError) -> Self {
        match error {
            SweepError::Cancelled(interrupted) => Self::Cancelled(interrupted),
            other => Self::Sweep(other),
        }
    }
}

impl From<Interrupted> for PathSweepError {
    fn from(interrupted: Interrupted) -> Self {
        Self::Cancelled(interrupted)
    }
}

impl From<GeometryError> for PathSweepError {
    fn from(error: GeometryError) -> Self {
        Self::Sweep(SweepError::Geometry(error))
    }
}

impl PathSweepError {
    pub fn segments(&self) -> Vec<usize> {
        match self {
            Self::SegmentTooShort { segment }
            | Self::UnsupportedPath { segment }
            | Self::TurnsAlongProfile { segment }
            | Self::ProfileTooLarge { segment }
            | Self::TooIntricate { segment } => vec![*segment],
            Self::Branches { segments } | Self::Apart { segments } => segments.clone(),
            Self::Corner { segments }
            | Self::CornerTooSharp { segments }
            | Self::ShapedCorner { segments } => {
                let mut named = segments.to_vec();
                named.dedup();
                named
            }
            Self::NoPath
            | Self::TooLong
            | Self::StartsAlongProfile
            | Self::ClosedPathMissesProfile
            | Self::ClosedPathTwists { .. }
            | Self::ClosedPathScales
            | Self::UnusableShape
            | Self::CrossesItself
            | Self::Sweep(_)
            | Self::Cancelled(_) => Vec::new(),
        }
    }
}

fn degenerate() -> PathSweepError {
    GeometryError::ZeroDirection.into()
}

struct Outline {
    centre: Point3,
    samples: Vec<Point2>,
    control_points: Vec<Point3>,
    tolerance: f64,
}

impl Outline {
    fn of(plane: &Plane, regions: &[Region]) -> Result<Self, PathSweepError> {
        let bounds = regions
            .iter()
            .filter_map(Region::bounds)
            .reduce(Aabb2::union)
            .ok_or(SweepError::NoRegions)?;
        let size = bounds.size().length();
        let sampling = SamplingTolerance::for_extent(size);
        let mut samples = Vec::new();
        let mut control_points = Vec::new();
        for piece in regions.iter().flat_map(Region::pieces) {
            samples.extend(
                piece
                    .curve()
                    .sample(piece.range(), &sampling)
                    .into_iter()
                    .map(|sample| sample.point),
            );
            control_points.extend_from_slice(piece_spline(plane, piece)?.control_points());
        }
        Ok(Self {
            centre: plane.to_world(bounds.center()),
            samples,
            control_points,
            tolerance: (size * RELATIVE_TOLERANCE).max(LINEAR_RESOLUTION),
        })
    }

    fn placed(&self, plane: &Plane, placement: &Similarity) -> impl Iterator<Item = Point3> {
        self.samples
            .iter()
            .map(move |sample| placement.apply_point(plane.to_world(*sample)))
    }
}

fn piece_spline(plane: &Plane, piece: &Piece) -> Result<BSplineCurve, PathSweepError> {
    let range = piece.range();
    let flat: BSplineCurve2 = match piece.curve() {
        Curve2::Line(line) => BSpline::new(
            1,
            vec![range.start(), range.start(), range.end(), range.end()],
            vec![line.point(range.start()), line.point(range.end())],
        )?,
        Curve2::Circle(circle) => arc_spline(circle, range)?,
        Curve2::Ellipse(ellipse) => ellipse_spline(ellipse, range)?,
        Curve2::BSpline(spline) => spline
            .restricted(range)
            .ok_or(PathSweepError::Sweep(SweepError::Unassembled))?,
    };
    Ok(flat.map_points(|point| plane.to_world(point))?)
}

#[derive(Debug, Clone, PartialEq)]
enum Motion {
    Straight {
        direction: Vector3,
        length: f64,
    },
    Turn {
        origin: Point3,
        axis: Vector3,
        angle: f64,
    },
    Slide {
        path: BSplineCurve,
        from: Point3,
    },
    Fitted(Stations),
}

#[derive(Debug, Clone, PartialEq)]
struct Stage {
    leg: Leg,
    start: RigidTransform,
    motion: Motion,
}

impl Stage {
    fn placement_at(&self, fraction: f64) -> Result<Similarity, PathSweepError> {
        if let Motion::Fitted(stations) = &self.motion {
            let frame = if fraction <= 0.0 {
                stations.first()
            } else if fraction >= 1.0 {
                stations.last()
            } else {
                stations.middle()
            };
            return frame.ok_or_else(degenerate);
        }
        self.rigid_at(fraction).map(Similarity::from)
    }

    fn rigid_at(&self, fraction: f64) -> Result<RigidTransform, PathSweepError> {
        let moved = match &self.motion {
            Motion::Straight { direction, length } => {
                RigidTransform::translation(*direction * (length * fraction))
            }
            Motion::Turn {
                origin,
                axis,
                angle,
            } => RigidTransform::rotation_about(*origin, *axis, angle * fraction),
            Motion::Slide { path, from } => {
                RigidTransform::translation(path.point(path.domain().at(fraction)) - *from)
            }
            Motion::Fitted(stations) => return Ok(stations.finish()),
        }
        .ok_or_else(degenerate)?;
        Ok(self.start.then(&moved))
    }

    fn end(&self) -> Result<Similarity, PathSweepError> {
        self.placement_at(1.0)
    }

    fn finish(&self) -> Result<RigidTransform, PathSweepError> {
        self.rigid_at(1.0)
    }

    fn segment(&self) -> usize {
        self.leg.segment
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Section {
    Placed(Similarity),
    Mitre {
        incoming: Plane,
        direction: Vector3,
        mitre: Plane,
    },
}

fn mitre_cap(incoming: &Plane, direction: Vector3, mitre: &Plane) -> Result<Cap, SweepError> {
    let level = Level::along(incoming, direction, LinearBound::Plane(*mitre))?;
    Cap::new(incoming, direction, level)
}

impl Section {
    fn point(&self, plane: &Plane, point: Point2) -> Result<Point3, PathSweepError> {
        Ok(match self {
            Self::Placed(placement) => placement.apply_point(plane.to_world(point)),
            Self::Mitre {
                incoming,
                direction,
                mitre,
            } => mitre_cap(incoming, *direction, mitre)?.point(incoming, point),
        })
    }

    fn edge(&self, plane: &Plane, piece: &Piece) -> Result<(Curve, Interval), PathSweepError> {
        Ok(match self {
            Self::Placed(placement) => match placement.as_rigid() {
                Some(rigid) => (
                    piece.curve().on_plane(&plane.transformed(&rigid))?,
                    piece.range(),
                ),
                None => {
                    let (curve, stretch) = piece.curve().on_plane(plane)?.mapped(placement)?;
                    let range = stretch.interval(piece.range()).ok_or_else(degenerate)?;
                    (curve, range)
                }
            },
            Self::Mitre {
                incoming,
                direction,
                mitre,
            } => mitre_cap(incoming, *direction, mitre)?.edge(incoming, piece)?,
        })
    }

    fn level_on(&self, placed: &Plane, direction: Vector3) -> Result<Option<Level>, SweepError> {
        match self {
            Self::Placed(_) => Ok(None),
            Self::Mitre { mitre, .. } => {
                Level::along(placed, direction, LinearBound::Plane(*mitre)).map(Some)
            }
        }
    }
}

struct Course {
    stages: Vec<Stage>,
    sections: Vec<Section>,
    closed: bool,
    facing: f64,
    orientation: SweepOrientation,
    tolerance: f64,
    bends_freely: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Shaping {
    centre: Point3,
    axis: Vector3,
    shape: SweepShape,
    total: f64,
}

impl Shaping {
    fn at(&self, travelled: f64) -> Result<Similarity, PathSweepError> {
        let fraction = (travelled / self.total).clamp(0.0, 1.0);
        let grown = Similarity::scaling(self.centre, 1.0 + (self.shape.scale - 1.0) * fraction)
            .ok_or(PathSweepError::UnusableShape)?;
        let turned =
            RigidTransform::rotation_about(self.centre, self.axis, self.shape.twist * fraction)
                .ok_or(PathSweepError::UnusableShape)?;
        Ok(grown.then(&Similarity::from(turned)))
    }

    fn stations_for(&self, length: f64) -> usize {
        let share = self.shape.twist.abs() * length / self.total;
        (share / STATION_TURN).ceil() as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Closing {
    Exact,
    Corrected,
}

fn travelled(leg: &Leg, parameter: f64) -> f64 {
    Interval::new(leg.range.start(), parameter.min(leg.range.end()))
        .map_or(0.0, |part| leg.curve.length(part))
}

fn turned_by(plane: &Plane, axis: Vector3, end: &Similarity) -> Option<f64> {
    let normal = end.apply_direction(plane.normal());
    if normal.dot(plane.normal()) < 1.0 - TILT_SLACK {
        return None;
    }
    let x_axis = end.apply_direction(plane.x_axis());
    let angle = x_axis.dot(plane.y_axis()).atan2(x_axis.dot(plane.x_axis()));
    Some(angle * axis.dot(plane.normal()).signum())
}

pub fn sweep(
    plane: &Plane,
    regions: &[Region],
    path: &[PathSegment],
    orientation: SweepOrientation,
    feature: u64,
) -> Result<Solid, PathSweepError> {
    sweep_shaped(
        plane,
        regions,
        path,
        (orientation, SweepShape::NONE),
        feature,
    )
}

pub fn sweep_shaped(
    plane: &Plane,
    regions: &[Region],
    path: &[PathSegment],
    (orientation, shape): (SweepOrientation, SweepShape),
    feature: u64,
) -> Result<Solid, PathSweepError> {
    if regions.is_empty() {
        return Err(SweepError::NoRegions.into());
    }
    if !shape.usable() {
        return Err(PathSweepError::UnusableShape);
    }
    let outline = Outline::of(plane, regions)?;
    let reach = path
        .iter()
        .map(|segment| segment.curve.bounding_box(segment.range).diagonal())
        .fold(0.0, f64::max);
    let tolerance = outline.tolerance.max(reach * RELATIVE_TOLERANCE);
    let chain = path::chain(path, plane, outline.centre, tolerance)?;
    if chain.closed && shape.scale != 1.0 {
        return Err(PathSweepError::ClosedPathScales);
    }
    let planned = Course::plan(
        (plane, regions, &outline),
        &chain,
        (orientation, shape),
        Closing::Exact,
    );
    let course = match planned {
        Err(PathSweepError::ClosedPathTwists {
            turned: Some(turned),
        }) if turned.abs() <= CLOSING_SLACK => {
            let corrected = SweepShape {
                twist: shape.twist - turned,
                ..shape
            };
            Course::plan(
                (plane, regions, &outline),
                &chain,
                (orientation, corrected),
                Closing::Corrected,
            )?
        }
        other => other?,
    };
    let solid = course.build(plane, regions, feature)?;
    if course.may_cross_itself()
        && let CrossingCheck::Crossing(_) = solid.find_crossing()?
    {
        return Err(PathSweepError::CrossesItself);
    }
    Ok(solid)
}

fn bend_check(
    plane: &Plane,
    outline: &Outline,
    curve: &Curve,
    (parameter, placement): (f64, &Similarity),
    facing: f64,
    segment: usize,
) -> Result<(), PathSweepError> {
    let derivatives = curve.evaluate(parameter);
    let tangent = derivatives.first.try_normalize().ok_or_else(degenerate)?;
    let bend = derivatives.curvature().unwrap_or(Vector3::ZERO);
    let normal = placement.apply_direction(plane.normal());
    for point in outline.placed(plane, placement) {
        let reach = point - derivatives.point;
        let stretch =
            (1.0 - bend.dot(reach)) * tangent.dot(normal) + bend.dot(normal) * tangent.dot(reach);
        if !stretch.is_finite() || facing * stretch <= FOLD_MARGIN {
            return Err(PathSweepError::ProfileTooLarge { segment });
        }
    }
    Ok(())
}

fn spans_of(path: &Curve) -> usize {
    match path {
        Curve::BSpline(spline) => spline.breakpoints().len().max(2) - 1,
        Curve::Line(_) | Curve::Circle(_) | Curve::Ellipse(_) | Curve::Intersection(_) => 1,
    }
}

fn fold_check(
    plane: &Plane,
    outline: &Outline,
    path: &Curve,
    stations: &Stations,
    facing: f64,
    segment: usize,
) -> Result<(), PathSweepError> {
    let steps = stations
        .parameters()
        .windows(2)
        .zip(stations.frames().windows(2));
    for (parameters, frames) in steps {
        interrupt::check()?;
        let ([from, to], [here, there]) = (parameters, frames) else {
            continue;
        };
        let travel = path.point(*to).distance(path.point(*from));
        let normal = here.apply_direction(plane.normal());
        for (start, end) in outline
            .placed(plane, here)
            .zip(outline.placed(plane, there))
        {
            let advance = facing * (end - start).dot(normal);
            if !advance.is_finite() || advance <= FOLD_MARGIN * travel {
                return Err(PathSweepError::ProfileTooLarge { segment });
            }
        }
    }
    Ok(())
}

fn rise_check(
    curve: &Curve,
    range: Interval,
    normal: Vector3,
    facing: f64,
    segment: usize,
) -> Result<(), PathSweepError> {
    let spans = match curve {
        Curve::BSpline(spline) => spline.breakpoints().len().max(2) - 1,
        Curve::Line(_) | Curve::Circle(_) | Curve::Ellipse(_) | Curve::Intersection(_) => 4,
    };
    for parameter in range.split(spans * RISE_SAMPLES_PER_SPAN) {
        let tangent = curve
            .evaluate(parameter)
            .first
            .try_normalize()
            .ok_or_else(degenerate)?;
        let rise = facing * tangent.dot(normal);
        if !rise.is_finite() || rise < LEAST_RISE {
            return Err(PathSweepError::TurnsAlongProfile { segment });
        }
    }
    Ok(())
}

fn meridian(placed: &Plane, outline: &Outline, origin: Point3, axis: Vector3) -> bool {
    let reach = outline
        .placed(placed, &Similarity::IDENTITY)
        .map(|point| point.distance(origin))
        .fold(0.0, f64::max);
    placed.normal().dot(axis).abs() * reach + placed.signed_distance(origin).abs() <= MERIDIAN_SLACK
}

impl Course {
    fn plan(
        (plane, regions, outline): (&Plane, &[Region], &Outline),
        chain: &Chain,
        (orientation, shape): (SweepOrientation, SweepShape),
        closing: Closing,
    ) -> Result<Self, PathSweepError> {
        let first = chain.legs.first().ok_or(PathSweepError::NoPath)?;
        let rise = first
            .start_tangent()
            .ok_or(PathSweepError::UnsupportedPath {
                segment: first.segment,
            })?
            .dot(plane.normal());
        if rise.abs() < LEAST_RISE {
            return Err(PathSweepError::StartsAlongProfile);
        }
        let facing = rise.signum();
        let axis = plane.normal() * facing;
        let total: f64 = chain
            .legs
            .iter()
            .map(|leg| leg.curve.length(leg.range))
            .sum();
        let shaping = (!shape.is_none()).then(|| Shaping {
            centre: plane.to_world(plane.to_local(first.start_point())),
            axis,
            shape,
            total,
        });
        let mut placement = RigidTransform::IDENTITY;
        let mut before = 0.0;
        let mut bends_freely = false;
        let mut sections = vec![Section::Placed(Similarity::IDENTITY)];
        let mut stages: Vec<Stage> = Vec::with_capacity(chain.legs.len());
        let count = chain.legs.len();
        for (index, leg) in chain.legs.iter().enumerate() {
            interrupt::check()?;
            let carried = (plane, outline, leg, &placement);
            bends_freely |= bends_freely_along(carried);
            let motion = match &shaping {
                Some(shaping) => shaped_motion(carried, (orientation, facing), (shaping, before))?,
                None => motion_of(carried, orientation, facing)?,
            };
            before += leg.curve.length(leg.range);
            let stage = Stage {
                leg: leg.clone(),
                start: placement,
                motion,
            };
            let end = stage.end()?;
            let finish = stage.finish()?;
            let next = if index + 1 < count {
                chain.legs.get(index + 1)
            } else if chain.closed {
                chain.legs.first()
            } else {
                None
            };
            match next {
                None => sections.push(Section::Placed(end)),
                Some(next) => {
                    let incoming = leg.end_tangent().ok_or_else(degenerate)?;
                    let outgoing = next.start_tangent().ok_or_else(degenerate)?;
                    let turn = incoming.angle_between(outgoing);
                    let segments = [leg.segment, next.segment];
                    let winds_on = leg.winding.is_some() && leg.winding == next.winding;
                    let between_lines =
                        matches!((&leg.curve, &next.curve), (Curve::Line(_), Curve::Line(_)));
                    if turn <= SMOOTH_TURN || orientation == SweepOrientation::Parallel || winds_on
                    {
                        sections.push(Section::Placed(end));
                        placement = finish;
                    } else if let (Motion::Straight { direction, .. }, Curve::Line(_)) =
                        (&stage.motion, &next.curve)
                    {
                        if turn > MOST_MITRE_TURN {
                            return Err(PathSweepError::CornerTooSharp { segments });
                        }
                        let corner = leg.end_point();
                        let mitre = Plane::new(corner, *direction + outgoing)
                            .ok_or(PathSweepError::CornerTooSharp { segments })?;
                        sections.push(Section::Mitre {
                            incoming: plane.transformed(&stage.start),
                            direction: *direction,
                            mitre,
                        });
                        let swing =
                            RigidTransform::rotation_about(corner, direction.cross(outgoing), turn)
                                .ok_or(PathSweepError::CornerTooSharp { segments })?;
                        placement = finish.then(&swing);
                    } else if shaping.is_some() && between_lines {
                        return Err(PathSweepError::ShapedCorner { segments });
                    } else {
                        return Err(PathSweepError::Corner { segments });
                    }
                }
            }
            stages.push(stage);
        }
        if chain.closed {
            match sections.pop() {
                Some(Section::Placed(end)) => {
                    let slack = match closing {
                        Closing::Exact => FIT_TOLERANCE,
                        Closing::Corrected => {
                            let reach = outline
                                .placed(plane, &Similarity::IDENTITY)
                                .map(|point| point.distance(outline.centre))
                                .fold(0.0, f64::max);
                            FIT_TOLERANCE + 2.0 * CLOSING_SLACK * reach
                        }
                    };
                    let twisted = outline
                        .placed(plane, &end)
                        .zip(outline.placed(plane, &Similarity::IDENTITY))
                        .any(|(back, start)| back.distance(start) > slack);
                    if twisted {
                        let turned = match closing {
                            Closing::Exact => turned_by(plane, axis, &end),
                            Closing::Corrected => None,
                        };
                        return Err(PathSweepError::ClosedPathTwists { turned });
                    }
                    if let Some(Stage {
                        motion: Motion::Fitted(stations),
                        ..
                    }) = stages.last_mut()
                    {
                        stations.closing_on(Similarity::IDENTITY);
                    }
                }
                Some(Section::Mitre { .. }) | None => {
                    let last = chain.legs.last().map_or(0, |leg| leg.segment);
                    return Err(PathSweepError::Corner {
                        segments: [last, first.segment],
                    });
                }
            }
        }
        let course = Self {
            stages,
            sections,
            closed: chain.closed,
            facing,
            orientation,
            tolerance: outline.tolerance,
            bends_freely,
        };
        course.check_mitres(plane, regions)?;
        Ok(course)
    }

    fn section(&self, joint: usize) -> Result<&Section, PathSweepError> {
        self.sections
            .get(joint % self.sections.len().max(1))
            .ok_or_else(degenerate)
    }

    fn check_mitres(&self, plane: &Plane, regions: &[Region]) -> Result<(), PathSweepError> {
        for (index, stage) in self.stages.iter().enumerate() {
            let Motion::Straight { direction, length } = stage.motion else {
                continue;
            };
            let placed = plane.transformed(&stage.start);
            let start = self.section(index)?.level_on(&placed, direction)?;
            let end = self.section(index + 1)?.level_on(&placed, direction)?;
            if start.is_none() && end.is_none() {
                continue;
            }
            let neighbour = if end.is_some() {
                self.stages.get((index + 1) % self.stages.len())
            } else {
                self.stages
                    .get((index + self.stages.len() - 1) % self.stages.len())
            }
            .map_or(stage.segment(), Stage::segment);
            let gap = end
                .unwrap_or(Level::flat(length))
                .minus(start.unwrap_or(Level::flat(0.0)));
            let (least, _) = span(&gap, regions).ok_or(SweepError::NoRegions)?;
            if !least.is_finite() || least <= self.tolerance {
                return Err(PathSweepError::CornerTooSharp {
                    segments: [stage.segment(), neighbour],
                });
            }
        }
        Ok(())
    }

    fn may_cross_itself(&self) -> bool {
        self.orientation == SweepOrientation::Square && (self.stages.len() > 1 || self.bends_freely)
    }

    fn whole_turn(&self) -> bool {
        self.closed
            && matches!(
                self.stages.as_slice(),
                [Stage {
                    motion: Motion::Turn { .. },
                    ..
                }]
            )
    }

    fn build(
        &self,
        plane: &Plane,
        regions: &[Region],
        feature: u64,
    ) -> Result<Solid, PathSweepError> {
        let mut plan = Plan::default();
        let end = self
            .stages
            .last()
            .map(Stage::end)
            .transpose()?
            .unwrap_or(Similarity::IDENTITY);
        for region in regions {
            interrupt::check()?;
            plan.label(region.entities());
            let caps = Caps {
                start: FaceName::start_cap(feature, region.key()),
                end: FaceName::end_cap(feature, region.key()),
            };
            let mut start_loops = Vec::new();
            let mut end_loops = Vec::new();
            for profile_loop in region.loops() {
                let (start_loop, end_loop) =
                    self.sweep_loop(&mut plan, plane, profile_loop.pieces(), &caps, feature)?;
                start_loops.push(start_loop);
                end_loops.push(end_loop);
            }
            if self.closed {
                continue;
            }
            let start_plane = Plane::from_frame(
                plane.origin(),
                plane.normal() * -self.facing,
                plane.x_axis(),
            )
            .ok_or_else(degenerate)?;
            let placed_end = plane.mapped(&end);
            let end_plane = Plane::from_frame(
                placed_end.origin(),
                placed_end.normal() * self.facing,
                placed_end.x_axis(),
            )
            .ok_or_else(degenerate)?;
            plan.face(PlanFace {
                surface: PlaneSurface::new(start_plane)?.into(),
                sense: Sense::Same,
                name: caps.start,
                origin: Some(FaceOrigin::StartCap { feature }),
                loops: start_loops,
            });
            plan.face(PlanFace {
                surface: PlaneSurface::new(end_plane)?.into(),
                sense: Sense::Same,
                name: caps.end,
                origin: Some(FaceOrigin::EndCap { feature }),
                loops: end_loops,
            });
        }
        Ok(plan.build().map_err(SweepError::from)?)
    }

    fn sweep_loop(
        &self,
        plan: &mut Plan,
        plane: &Plane,
        pieces: &[Piece],
        caps: &Caps,
        feature: u64,
    ) -> Result<(Vec<PlanCoedge>, Vec<PlanCoedge>), PathSweepError> {
        let count = pieces.len();
        let piece_at = |index: usize| pieces.get(index % count.max(1)).ok_or_else(degenerate);
        let stage_at = |index: usize| {
            self.stages
                .get(index % self.stages.len().max(1))
                .ok_or_else(degenerate)
        };
        let side = |stage: usize, piece: usize| -> Result<FaceName, PathSweepError> {
            let leg = &stage_at(stage)?.leg;
            Ok(FaceName::swept(
                feature,
                piece_at(piece)?.id(),
                leg.key,
                leg.part,
            ))
        };
        let joints = self.sections.len();
        let legs = self.stages.len();
        let mut vertices: Vec<Vec<usize>> = Vec::with_capacity(joints);
        for joint in 0..joints {
            let section = self.section(joint)?;
            let mut row = Vec::with_capacity(count);
            for piece in pieces {
                row.push(plan.vertex(section.point(plane, piece.start())?));
            }
            vertices.push(row);
        }
        let vertex = |joint: usize, index: usize| -> Result<usize, PathSweepError> {
            vertices
                .get(joint % joints)
                .and_then(|row| row.get(index % count.max(1)))
                .copied()
                .ok_or_else(degenerate)
        };
        let whole_turn = self.whole_turn();
        let mut sections: Vec<Vec<Option<usize>>> = Vec::with_capacity(joints);
        for joint in 0..joints {
            interrupt::check()?;
            let section = self.section(joint)?;
            let mut row = Vec::with_capacity(count);
            for (index, piece) in pieces.iter().enumerate() {
                if whole_turn && self.flat_in_turn(plane, piece)? {
                    row.push(None);
                    continue;
                }
                let name = if !self.closed && joint == 0 {
                    EdgeName::between(side(0, index)?, caps.start)
                } else if !self.closed && joint == legs {
                    EdgeName::between(side(legs - 1, index)?, caps.end)
                } else if legs == 1 {
                    EdgeName::seam(side(0, index)?)
                } else {
                    EdgeName::between(side(joint + legs - 1, index)?, side(joint, index)?)
                };
                let (curve, range) = section.edge(plane, piece)?;
                let (from, to) = (vertex(joint, index)?, vertex(joint, index + 1)?);
                let ends = if piece.is_reversed() {
                    (to, from)
                } else {
                    (from, to)
                };
                row.push(Some(plan.edge(curve, range, ends, name)));
            }
            sections.push(row);
        }
        let section_edge = |joint: usize, index: usize| -> Result<usize, PathSweepError> {
            sections
                .get(joint % joints)
                .and_then(|row| row.get(index))
                .copied()
                .flatten()
                .ok_or_else(degenerate)
        };
        for (number, stage) in self.stages.iter().enumerate() {
            interrupt::check()?;
            let mut alongs = Vec::with_capacity(count);
            for index in 0..count {
                let name =
                    EdgeName::between(side(number, index + count - 1)?, side(number, index)?);
                let from = vertex(number, index)?;
                let to = vertex(number + 1, index)?;
                alongs.push(Self::along_edge(plan, stage, (from, to), name)?);
            }
            let along = |index: usize| {
                alongs
                    .get(index % count.max(1))
                    .copied()
                    .ok_or_else(degenerate)
            };
            for (index, piece) in pieces.iter().enumerate() {
                let surface = self.surface_of(plane, stage, piece)?;
                let middle = piece.range().middle();
                let halfway = stage.placement_at(0.5)?;
                let probe = halfway.apply_point(plane.to_world(piece.curve().point(middle)));
                let outward = halfway.apply_direction(
                    lift(plane, traversal_tangent(piece, middle)).cross(plane.normal()),
                );
                let sense = outward_sense(&surface, probe, outward, None);
                let name = side(number, index)?;
                let origin = Some(FaceOrigin::Side {
                    feature,
                    entity: piece.entity(),
                });
                if whole_turn && self.flat_in_turn(plane, piece)? {
                    let rim = |at: usize| -> Result<(usize, Point3), PathSweepError> {
                        let point = plan.point(vertex(0, at)?).ok_or_else(degenerate)?;
                        Ok((along(at)?, point))
                    };
                    let loops =
                        self.disc_loops((&surface, sense), [rim(index)?, rim(index + 1)?])?;
                    plan.face(PlanFace {
                        surface,
                        sense,
                        name,
                        origin,
                        loops,
                    });
                    continue;
                }
                let travel = traversal_sense(piece);
                let mut coedges = vec![
                    PlanCoedge::new(section_edge(number, index)?, travel),
                    PlanCoedge::new(along(index + 1)?, Sense::Same),
                    PlanCoedge::new(section_edge(number + 1, index)?, travel.reversed()),
                    PlanCoedge::new(along(index)?, Sense::Reversed),
                ];
                if self.facing < 0.0 {
                    coedges = turned_over(coedges);
                }
                plan.face(PlanFace {
                    surface,
                    sense,
                    name,
                    origin,
                    loops: vec![coedges],
                });
            }
        }
        let mut start_loop = Vec::new();
        let mut end_loop = Vec::new();
        if !self.closed {
            for (index, piece) in pieces.iter().enumerate() {
                let travel = traversal_sense(piece);
                start_loop.push(PlanCoedge::new(section_edge(0, index)?, travel));
                end_loop.push(PlanCoedge::new(section_edge(legs, index)?, travel));
            }
            if self.facing > 0.0 {
                start_loop = turned_over(start_loop);
            } else {
                end_loop = turned_over(end_loop);
            }
        }
        Ok((start_loop, end_loop))
    }

    fn flat_in_turn(&self, plane: &Plane, piece: &Piece) -> Result<bool, PathSweepError> {
        match self.stages.as_slice() {
            [stage] => match &stage.motion {
                Motion::Turn {
                    origin,
                    axis,
                    angle,
                } => {
                    let frame = turn_frame(plane, stage, (*origin, *axis), *angle, self.tolerance)?;
                    Ok(prepare(&frame, piece)?.kind == Kind::Flat)
                }
                Motion::Straight { .. } | Motion::Slide { .. } | Motion::Fitted(_) => Ok(false),
            },
            _ => Ok(false),
        }
    }

    fn disc_loops(
        &self,
        (surface, sense): (&Surface, Sense),
        rims: [(usize, Point3); 2],
    ) -> Result<Vec<Vec<PlanCoedge>>, PathSweepError> {
        let Some(Stage {
            motion: Motion::Turn { origin, axis, .. },
            ..
        }) = self.stages.first()
        else {
            return Err(degenerate());
        };
        let radius = |point: Point3| {
            let offset = point - *origin;
            (offset - *axis * offset.dot(*axis)).length()
        };
        let [(first, first_point), (second, second_point)] = rims;
        let (outer, inner) = if radius(first_point) >= radius(second_point) {
            (first, second)
        } else {
            (second, first)
        };
        let normal = surface.normal(0.0, 0.0).unwrap_or(*axis);
        let outward = if sense.is_same() { normal } else { -normal };
        let around = Sense::from_sign(outward.dot(*axis));
        Ok(vec![
            vec![PlanCoedge::new(outer, around)],
            vec![PlanCoedge::new(inner, around.reversed())],
        ])
    }

    fn along_edge(
        plan: &mut Plan,
        stage: &Stage,
        (from, to): (usize, usize),
        name: EdgeName,
    ) -> Result<usize, PathSweepError> {
        let start = plan.point(from).ok_or_else(degenerate)?;
        Ok(match &stage.motion {
            Motion::Straight { .. } => plan.line(from, to, name)?,
            Motion::Turn {
                origin,
                axis,
                angle,
            } => {
                let centre = *origin + *axis * (start - *origin).dot(*axis);
                let radial = start - centre;
                let frame = Plane::from_frame(
                    centre,
                    *axis,
                    radial.try_normalize().ok_or_else(degenerate)?,
                )
                .ok_or_else(degenerate)?;
                let circle = Circle::new(frame, radial.length())?;
                let interval = Interval::new(0.0, *angle).ok_or_else(degenerate)?;
                plan.edge(circle.into(), interval, (from, to), name)
            }
            Motion::Slide { path, from: origin } => {
                let shift = start - *origin;
                let moved = path.map_points(|point| point + shift)?;
                let interval = moved.domain();
                plan.edge(Curve::BSpline(moved), interval, (from, to), name)
            }
            Motion::Fitted(stations) => {
                let unplaced = stations
                    .first()
                    .ok_or_else(degenerate)?
                    .inverse()
                    .apply_point(start);
                let trajectory = stations.trajectory(unplaced)?;
                let interval = trajectory.domain();
                plan.edge(Curve::BSpline(trajectory), interval, (from, to), name)
            }
        })
    }

    fn surface_of(
        &self,
        plane: &Plane,
        stage: &Stage,
        piece: &Piece,
    ) -> Result<Surface, PathSweepError> {
        let placed = plane.transformed(&stage.start);
        Ok(match &stage.motion {
            Motion::Straight { direction, .. } => side_surface(&placed, *direction, piece)?.0,
            Motion::Turn {
                origin,
                axis,
                angle,
            } => {
                let frame = turn_frame(plane, stage, (*origin, *axis), *angle, self.tolerance)?;
                surface_of(&frame, &prepare(&frame, piece)?)?
            }
            Motion::Slide { path, from } => {
                let profile = piece_spline(&placed, piece)?;
                Surface::from(sliding_surface(&profile, path, *from)?)
            }
            Motion::Fitted(stations) => {
                Surface::from(stations.surface(&piece_spline(plane, piece)?)?)
            }
        })
    }
}

struct Caps {
    start: FaceName,
    end: FaceName,
}

fn turned_over(coedges: Vec<PlanCoedge>) -> Vec<PlanCoedge> {
    coedges
        .into_iter()
        .rev()
        .map(|coedge| PlanCoedge {
            sense: coedge.sense.reversed(),
            ..coedge
        })
        .collect()
}

fn turn_frame(
    plane: &Plane,
    stage: &Stage,
    (origin, axis): (Point3, Vector3),
    angle: f64,
    tolerance: f64,
) -> Result<Frame, PathSweepError> {
    let placed = plane.transformed(&stage.start);
    let along = Vector2::new(axis.dot(placed.x_axis()), axis.dot(placed.y_axis()));
    let line = Axis2::new(placed.to_local(origin), along)?;
    let toward = placed.to_local(stage.leg.start_point());
    if line.signed_distance(toward) >= 0.0 {
        Ok(Frame::turning(
            placed,
            line,
            (origin, axis),
            RigidTransform::IDENTITY,
            angle,
            tolerance,
        )?)
    } else {
        let low = RigidTransform::rotation_about(origin, axis, angle).ok_or_else(degenerate)?;
        Ok(Frame::turning(
            placed,
            line.reversed(),
            (origin, -axis),
            low,
            angle,
            tolerance,
        )?)
    }
}

type Carried<'a> = (&'a Plane, &'a Outline, &'a Leg, &'a RigidTransform);

fn bends_freely_along((plane, outline, leg, placement): Carried<'_>) -> bool {
    match &leg.curve {
        Curve::Line(_) => false,
        Curve::Circle(circle) => !meridian(
            &plane.transformed(placement),
            outline,
            circle.center(),
            circle.frame().normal(),
        ),
        Curve::Ellipse(_) | Curve::BSpline(_) | Curve::Intersection(_) => true,
    }
}

fn motion_of(
    (plane, outline, leg, placement): Carried<'_>,
    orientation: SweepOrientation,
    facing: f64,
) -> Result<Motion, PathSweepError> {
    let segment = leg.segment;
    let starting = Similarity::from(*placement);
    let start = (leg.range.start(), &starting);
    match (&leg.curve, orientation) {
        (Curve::Line(_), _) => {
            let (from, to) = (leg.start_point(), leg.end_point());
            let direction = (to - from).try_normalize().ok_or_else(degenerate)?;
            if orientation == SweepOrientation::Parallel {
                rise_check(&leg.curve, leg.range, plane.normal(), facing, segment)?;
            } else {
                bend_check(plane, outline, &leg.curve, start, facing, segment)?;
            }
            Ok(Motion::Straight {
                direction,
                length: from.distance(to),
            })
        }
        (_, SweepOrientation::Parallel) => {
            rise_check(&leg.curve, leg.range, plane.normal(), facing, segment)?;
            Ok(Motion::Slide {
                path: leg.spline()?,
                from: leg.start_point(),
            })
        }
        (Curve::Circle(circle), SweepOrientation::Square) => {
            bend_check(plane, outline, &leg.curve, start, facing, segment)?;
            let origin = circle.center();
            let axis = circle.frame().normal();
            let angle = leg.range.length();
            let placed = plane.transformed(placement);
            if meridian(&placed, outline, origin, axis) {
                return Ok(Motion::Turn {
                    origin,
                    axis,
                    angle,
                });
            }
            let from = leg.range.start();
            let frames =
                |parameters: &[f64]| rotating((origin, axis), from, parameters, *placement);
            let initial = ((angle / STATION_TURN).ceil() as usize).max(4);
            let stations = Stations::fit(
                leg.range,
                initial,
                (&frames, None),
                &outline.control_points,
                segment,
            )?;
            Ok(Motion::Fitted(stations))
        }
        (_, SweepOrientation::Square) if let Some(axis) = leg.winding => {
            let path = Curve::BSpline(leg.spline()?);
            let frames = |parameters: &[f64]| winding(&path, axis, parameters, *placement);
            let initial = (spans_of(&path) * STATIONS_PER_SPAN).max(LEAST_STATIONS);
            let stations = Stations::fit(
                leg.range,
                initial,
                (&frames, None),
                &outline.control_points,
                segment,
            )?;
            fold_check(plane, outline, &path, &stations, facing, segment)?;
            Ok(Motion::Fitted(stations))
        }
        (_, SweepOrientation::Square) => {
            let path = Curve::BSpline(leg.spline()?);
            let frames = |parameters: &[f64]| rotation_minimising(&path, parameters, *placement);
            let initial = (spans_of(&path) * STATIONS_PER_SPAN).max(LEAST_STATIONS);
            let stations = Stations::fit(
                leg.range,
                initial,
                (&frames, None),
                &outline.control_points,
                segment,
            )?;
            for (parameter, frame) in stations.parameters().iter().zip(stations.frames()) {
                bend_check(plane, outline, &path, (*parameter, frame), facing, segment)?;
            }
            Ok(Motion::Fitted(stations))
        }
    }
}

fn shaped_motion(
    (plane, outline, leg, placement): Carried<'_>,
    (orientation, facing): (SweepOrientation, f64),
    (shaping, before): (&Shaping, f64),
) -> Result<Motion, PathSweepError> {
    let segment = leg.segment;
    let shape = |parameter: f64| shaping.at(before + travelled(leg, parameter));
    let path = Curve::BSpline(leg.spline()?);
    let turning = match &leg.curve {
        Curve::Circle(_) => (leg.range.length() / STATION_TURN).ceil() as usize,
        Curve::Line(_) | Curve::Ellipse(_) | Curve::BSpline(_) | Curve::Intersection(_) => 0,
    };
    let initial = (spans_of(&path) * STATIONS_PER_SPAN)
        .max(LEAST_STATIONS)
        .max(turning)
        .max(shaping.stations_for(leg.curve.length(leg.range)));
    let fit = |frames: &FramesAt<'_>| {
        let shape: &ShapeAt<'_> = &shape;
        Stations::fit(
            leg.range,
            initial,
            (frames, Some(shape)),
            &outline.control_points,
            segment,
        )
    };
    let bent = |curve: &Curve, stations: &Stations| {
        stations
            .parameters()
            .iter()
            .zip(stations.frames())
            .try_for_each(|(parameter, frame)| {
                bend_check(plane, outline, curve, (*parameter, frame), facing, segment)
            })
    };
    let stations = match (&leg.curve, orientation) {
        (_, SweepOrientation::Parallel) => {
            rise_check(&leg.curve, leg.range, plane.normal(), facing, segment)?;
            fit(&|parameters: &[f64]| sliding(&path, parameters, *placement))?
        }
        (Curve::Line(_), SweepOrientation::Square) => {
            let starting = Similarity::from(*placement);
            bend_check(
                plane,
                outline,
                &leg.curve,
                (leg.range.start(), &starting),
                facing,
                segment,
            )?;
            fit(&|parameters: &[f64]| sliding(&path, parameters, *placement))?
        }
        (Curve::Circle(circle), SweepOrientation::Square) => {
            let turning = (circle.center(), circle.frame().normal());
            let from = leg.range.start();
            let stations =
                fit(&|parameters: &[f64]| rotating(turning, from, parameters, *placement))?;
            bent(&leg.curve, &stations)?;
            stations
        }
        (_, SweepOrientation::Square) if let Some(axis) = leg.winding => {
            let stations = fit(&|parameters: &[f64]| winding(&path, axis, parameters, *placement))?;
            fold_check(plane, outline, &path, &stations, facing, segment)?;
            stations
        }
        (_, SweepOrientation::Square) => {
            let stations =
                fit(&|parameters: &[f64]| rotation_minimising(&path, parameters, *placement))?;
            bent(&path, &stations)?;
            stations
        }
    };
    Ok(Motion::Fitted(stations))
}
