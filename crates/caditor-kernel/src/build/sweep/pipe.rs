use caditor_geometry::{Aabb2, Plane, Point2, Point3, Vector3};
use thiserror::Error;

use super::{
    MOST_MITRE_TURN, PathSegment, PathSweepError, RELATIVE_TOLERANCE, SMOOTH_TURN,
    SweepOrientation,
    bend::{self, Rounding},
    degenerate,
    path::{self, Chain, Leg},
    sweep,
};
use crate::{
    curve::Curve,
    error::GeometryError,
    interval::Interval,
    naming::path_bend,
    profile::{Profile, ProfileCurve, ProfileError, Region},
    tolerance::{LINEAR_RESOLUTION, MAX_SIZE},
    topology::Solid,
};

const OUTER: u64 = 1;
const SQUARE_SIDES: u64 = 4;
const TRIANGLE_SIDES: u64 = 3;
const INRADIUS_PER_SIDE: f64 = 0.288_675_134_594_812_9;
const BEND_SAMPLES: u32 = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum PipeShape {
    #[default]
    Round,
    Square,
    Triangle,
}

impl PipeShape {
    pub const ALL: [Self; 3] = [Self::Round, Self::Square, Self::Triangle];

    fn outline(self, size: f64, first: u64) -> Vec<ProfileCurve> {
        let corners = match self {
            Self::Round => return vec![ProfileCurve::circle(first, Point2::ZERO, size / 2.0)],
            Self::Square => {
                let half = size / 2.0;
                vec![
                    Point2::new(-half, -half),
                    Point2::new(half, -half),
                    Point2::new(half, half),
                    Point2::new(-half, half),
                ]
            }
            Self::Triangle => {
                let inradius = size * INRADIUS_PER_SIDE;
                vec![
                    Point2::new(-size / 2.0, -inradius),
                    Point2::new(size / 2.0, -inradius),
                    Point2::new(0.0, 2.0 * inradius),
                ]
            }
        };
        let count = corners.len();
        corners
            .iter()
            .zip(corners.iter().cycle().skip(1))
            .zip(first..)
            .take(count)
            .map(|((start, end), entity)| ProfileCurve::line(entity, *start, *end))
            .collect()
    }

    pub fn curves(self) -> u64 {
        match self {
            Self::Round => 1,
            Self::Square => SQUARE_SIDES,
            Self::Triangle => TRIANGLE_SIDES,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PipeSection {
    pub shape: PipeShape,
    pub size: f64,
    pub wall: Option<f64>,
}

impl PipeSection {
    fn inner_size(&self, wall: f64) -> f64 {
        match self.shape {
            PipeShape::Round | PipeShape::Square => self.size - 2.0 * wall,
            PipeShape::Triangle => self.size - wall / INRADIUS_PER_SIDE,
        }
    }

    fn regions(&self) -> Result<Vec<Region>, PipeError> {
        if !self.size.is_finite() || self.size <= LINEAR_RESOLUTION || self.size > MAX_SIZE {
            return Err(PipeError::InvalidSize);
        }
        let mut curves = self.shape.outline(self.size, OUTER);
        if let Some(wall) = self.wall {
            if !wall.is_finite() || wall <= LINEAR_RESOLUTION {
                return Err(PipeError::InvalidWall);
            }
            let inner = self.inner_size(wall);
            if inner <= LINEAR_RESOLUTION {
                return Err(PipeError::WallTooThick);
            }
            curves.extend(self.shape.outline(inner, OUTER + self.shape.curves()));
        }
        let profile = Profile::new(&curves).map_err(PipeError::Section)?;
        Ok(profile
            .regions()
            .iter()
            .filter(|region| region.depth() == 0)
            .cloned()
            .collect())
    }
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum PipeError {
    #[error("the pipe's size must be a length above zero and within reach")]
    InvalidSize,
    #[error("the pipe's wall thickness must be a length above zero")]
    InvalidWall,
    #[error("the pipe's wall is as thick as its section allows or thicker")]
    WallTooThick,
    #[error("the bend radius must be a length of zero or more within reach")]
    InvalidBend,
    #[error("rounding the corner between path curves {segments:?} needs more than their length")]
    BendTooLong { segments: [usize; 2] },
    #[error("the bend between path curves {segments:?} is tighter than the pipe's section")]
    BendTooTight { segments: [usize; 2] },
    #[error("the pipe's section could not be made: {0}")]
    Section(ProfileError),
    #[error(transparent)]
    Path(PathSweepError),
}

impl From<PathSweepError> for PipeError {
    fn from(error: PathSweepError) -> Self {
        Self::Path(error)
    }
}

impl From<GeometryError> for PipeError {
    fn from(error: GeometryError) -> Self {
        Self::Path(error.into())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Source {
    Segment(usize),
    Bend([usize; 2]),
}

#[derive(Debug, Clone, PartialEq)]
struct Bend {
    segment: PathSegment,
    segments: [usize; 2],
}

#[derive(Debug, Clone, PartialEq)]
struct Rounded {
    segments: Vec<PathSegment>,
    sources: Vec<Source>,
}

impl Rounded {
    fn original(&self, index: usize) -> usize {
        match self.sources.get(index) {
            Some(Source::Segment(segment) | Source::Bend([segment, _])) => *segment,
            None => index,
        }
    }

    fn remapped(&self, error: PathSweepError) -> PipeError {
        let one = |segment: usize| self.original(segment);
        let pair = |[first, second]: [usize; 2]| [one(first), one(second)];
        let many = |segments: Vec<usize>| {
            let mut named: Vec<usize> = segments.into_iter().map(one).collect();
            named.sort_unstable();
            named.dedup();
            named
        };
        PipeError::Path(match error {
            PathSweepError::ProfileTooLarge { segment } => match self.sources.get(segment) {
                Some(Source::Bend(segments)) => {
                    return PipeError::BendTooTight {
                        segments: *segments,
                    };
                }
                _ => PathSweepError::ProfileTooLarge {
                    segment: one(segment),
                },
            },
            PathSweepError::SegmentTooShort { segment } => PathSweepError::SegmentTooShort {
                segment: one(segment),
            },
            PathSweepError::UnsupportedPath { segment } => PathSweepError::UnsupportedPath {
                segment: one(segment),
            },
            PathSweepError::TurnsAlongProfile { segment } => PathSweepError::TurnsAlongProfile {
                segment: one(segment),
            },
            PathSweepError::TooIntricate { segment } => PathSweepError::TooIntricate {
                segment: one(segment),
            },
            PathSweepError::Branches { segments } => PathSweepError::Branches {
                segments: many(segments),
            },
            PathSweepError::Apart { segments } => PathSweepError::Apart {
                segments: many(segments),
            },
            PathSweepError::Corner { segments } => PathSweepError::Corner {
                segments: pair(segments),
            },
            PathSweepError::CornerTooSharp { segments } => PathSweepError::CornerTooSharp {
                segments: pair(segments),
            },
            PathSweepError::ShapedCorner { segments } => PathSweepError::ShapedCorner {
                segments: pair(segments),
            },
            other => other,
        })
    }
}

fn is_line(leg: &Leg) -> bool {
    matches!(leg.curve, Curve::Line(_))
}

struct Corners {
    bends: Vec<Option<Bend>>,
    kept: Vec<[f64; 2]>,
}

fn overrun(leg: &Leg, [start, end]: [f64; 2]) -> f64 {
    Interval::new(end, start).map_or(0.0, |overlap| leg.curve.length(overlap))
}

fn corners(chain: &Chain, radius: f64) -> Result<Corners, PipeError> {
    let legs = &chain.legs;
    let count = legs.len();
    let mut bends = vec![None; count];
    let mut kept: Vec<[f64; 2]> = legs
        .iter()
        .map(|leg| [leg.range.start(), leg.range.end()])
        .collect();
    if radius <= 0.0 {
        return Ok(Corners { bends, kept });
    }
    let joints = if chain.closed && count > 1 {
        count
    } else {
        count.saturating_sub(1)
    };
    for index in 0..joints {
        let next = (index + 1) % count;
        let (Some(leg), Some(after)) = (legs.get(index), legs.get(next)) else {
            continue;
        };
        let incoming = leg.end_tangent().ok_or_else(degenerate)?;
        let outgoing = after.start_tangent().ok_or_else(degenerate)?;
        let turn = incoming.angle_between(outgoing);
        if turn <= SMOOTH_TURN {
            continue;
        }
        let segments = [leg.segment, after.segment];
        if turn > MOST_MITRE_TURN {
            return Err(PathSweepError::CornerTooSharp { segments }.into());
        }
        let Rounding {
            incoming_end,
            outgoing_start,
            segment,
        } = bend::rounding(leg, after, radius, path_bend(leg.key, after.key))?;
        if let Some([_, end]) = kept.get_mut(index) {
            *end = incoming_end;
        }
        if let Some([start, _]) = kept.get_mut(next) {
            *start = outgoing_start;
        }
        for (trimmed, held) in [(index, leg), (next, after)] {
            let overran = kept
                .get(trimmed)
                .is_some_and(|ends| overrun(held, *ends) > LINEAR_RESOLUTION);
            if overran {
                return Err(PipeError::BendTooLong { segments });
            }
        }
        if let Some(slot) = bends.get_mut(index) {
            *slot = Some(Bend { segment, segments });
        }
    }
    Ok(Corners { bends, kept })
}

fn rounded(chain: &Chain, radius: f64) -> Result<Rounded, PipeError> {
    let Corners { bends, kept } = corners(chain, radius)?;
    let mut segments = Vec::with_capacity(chain.legs.len() * 2);
    let mut sources = Vec::with_capacity(chain.legs.len() * 2);
    for ((leg, [start, end]), bend) in chain.legs.iter().zip(kept).zip(bends) {
        let left = Interval::new(start, end).filter(|left| {
            *left == leg.range || leg.curve.is_longer_than(*left, LINEAR_RESOLUTION)
        });
        if let Some(left) = left {
            segments.push(leg.segment_over(left));
            sources.push(Source::Segment(leg.segment));
        }
        if let Some(Bend {
            segment,
            segments: joined,
        }) = bend
        {
            segments.push(segment);
            sources.push(Source::Bend(joined));
        }
    }
    Ok(Rounded { segments, sources })
}

fn across(tangent: Vector3, up: Vector3) -> Option<Vector3> {
    [up, Vector3::Z, Vector3::X, Vector3::Y]
        .into_iter()
        .filter_map(|candidate| candidate.reject_from(tangent).try_normalize())
        .find(|candidate| candidate.length_squared() > 0.5)
}

fn section_plane(start: &PathSegment, up: Vector3) -> Result<Plane, PipeError> {
    let parameter = start.range().start();
    let derivatives = start.curve().evaluate(parameter);
    let tangent = derivatives.first.try_normalize().ok_or_else(degenerate)?;
    let upward = across(tangent, up).ok_or_else(degenerate)?;
    Plane::from_frame(derivatives.point, tangent, upward.cross(tangent))
        .ok_or_else(|| degenerate().into())
}

fn section_tolerance(regions: &[Region]) -> f64 {
    regions
        .iter()
        .filter_map(Region::bounds)
        .reduce(Aabb2::union)
        .map_or(LINEAR_RESOLUTION, |bounds| {
            (bounds.size().length() * RELATIVE_TOLERANCE).max(LINEAR_RESOLUTION)
        })
}

fn path_reach(path: &[PathSegment]) -> f64 {
    path.iter()
        .map(|segment| segment.curve().bounding_box(segment.range()).diagonal())
        .fold(0.0, f64::max)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BendMiddle {
    pub radius: f64,
    pub point: Point3,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BendMiddles {
    AlongBisector,
    Sampled(Vec<BendMiddle>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PipeCorner {
    pub point: Point3,
    pub incoming: Vector3,
    pub outgoing: Vector3,
    pub middles: BendMiddles,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PipeStand {
    pub section: Plane,
    pub corner: Option<PipeCorner>,
}

fn bend_middle(leg: &Leg, after: &Leg, radius: f64) -> Option<BendMiddle> {
    let made = bend::rounding(leg, after, radius, path_bend(leg.key, after.key)).ok()?;
    Some(BendMiddle {
        radius,
        point: made.segment.curve().point(made.segment.range().middle()),
    })
}

fn sampled_middles(leg: &Leg, after: &Leg, turn: f64, bend: f64) -> Vec<BendMiddle> {
    let corner = leg.end_point();
    let room = corner
        .distance(leg.start_point())
        .min(corner.distance(after.end_point()));
    let widest = room / (turn / 2.0).tan();
    let mut radii: Vec<f64> = (1..=BEND_SAMPLES)
        .map(|index| widest * f64::from(index) / f64::from(BEND_SAMPLES))
        .collect();
    if bend.is_finite() && bend > 0.0 && bend <= MAX_SIZE {
        radii.push(bend);
    }
    radii.sort_by(f64::total_cmp);
    let mut middles = vec![BendMiddle {
        radius: 0.0,
        point: corner,
    }];
    middles.extend(
        radii
            .into_iter()
            .filter(|radius| radius.is_finite() && *radius > 0.0)
            .filter_map(|radius| bend_middle(leg, after, radius)),
    );
    middles
}

fn first_corner(chain: &Chain, bend: f64) -> Option<PipeCorner> {
    let legs = &chain.legs;
    let count = legs.len();
    let joints = if chain.closed && count > 1 {
        count
    } else {
        count.saturating_sub(1)
    };
    (0..joints).find_map(|index| {
        let leg = legs.get(index)?;
        let after = legs.get((index + 1) % count)?;
        let incoming = leg.end_tangent()?;
        let outgoing = after.start_tangent()?;
        let turn = incoming.angle_between(outgoing);
        if turn <= SMOOTH_TURN || turn > MOST_MITRE_TURN {
            return None;
        }
        let middles = if is_line(leg) && is_line(after) {
            BendMiddles::AlongBisector
        } else {
            BendMiddles::Sampled(sampled_middles(leg, after, turn, bend))
        };
        Some(PipeCorner {
            point: leg.end_point(),
            incoming,
            outgoing,
            middles,
        })
    })
}

pub fn pipe_stand(path: &[PathSegment], bend: f64, up: Vector3) -> Result<PipeStand, PipeError> {
    let tolerance = (path_reach(path) * RELATIVE_TOLERANCE).max(LINEAR_RESOLUTION);
    let chain = path::ordered(path, tolerance)?;
    let rounded = rounded(&chain, bend).or_else(|_| rounded(&chain, 0.0))?;
    let start = rounded.segments.first().ok_or(PathSweepError::NoPath)?;
    Ok(PipeStand {
        section: section_plane(start, up)?,
        corner: first_corner(&chain, bend),
    })
}

pub fn pipe(
    path: &[PathSegment],
    section: &PipeSection,
    bend: f64,
    up: Vector3,
    feature: u64,
) -> Result<Solid, PipeError> {
    if !bend.is_finite() || bend < 0.0 || bend > MAX_SIZE {
        return Err(PipeError::InvalidBend);
    }
    let regions = section.regions()?;
    let tolerance = section_tolerance(&regions).max(path_reach(path) * RELATIVE_TOLERANCE);
    let chain = path::ordered(path, tolerance)?;
    let rounded = rounded(&chain, bend)?;
    let start = rounded.segments.first().ok_or(PathSweepError::NoPath)?;
    let plane = section_plane(start, up)?;
    sweep(
        &plane,
        &regions,
        &rounded.segments,
        SweepOrientation::Square,
        feature,
    )
    .map_err(|error| rounded.remapped(error))
}
