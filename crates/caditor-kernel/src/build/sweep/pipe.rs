use caditor_geometry::{Aabb2, Plane, Point2, Point3, Vector3};
use thiserror::Error;

use super::{
    MOST_MITRE_TURN, PathSegment, PathSweepError, RELATIVE_TOLERANCE, SMOOTH_TURN,
    SweepOrientation, degenerate,
    path::{self, Chain, Leg},
    sweep,
};
use crate::{
    curve::{Circle, Curve},
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
    corner: Point3,
    incoming: Vector3,
    outgoing: Vector3,
    turn: f64,
    cut: f64,
    key: u128,
    segments: [usize; 2],
}

impl Bend {
    fn arc(&self, radius: f64) -> Result<PathSegment, PipeError> {
        let inward = (self.outgoing - self.incoming)
            .try_normalize()
            .ok_or_else(degenerate)?;
        let centre = self.corner + inward * (radius / (self.turn / 2.0).cos());
        let normal = self
            .incoming
            .cross(self.outgoing)
            .try_normalize()
            .ok_or_else(degenerate)?;
        let frame = Plane::from_frame(centre, normal, self.incoming.cross(normal))
            .ok_or_else(degenerate)?;
        let range = Interval::new(0.0, self.turn).ok_or_else(degenerate)?;
        Ok(PathSegment::new(
            Curve::Circle(Circle::new(frame, radius)?),
            range,
            self.key,
        ))
    }
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
            other => other,
        })
    }
}

fn is_line(leg: &Leg) -> bool {
    matches!(leg.curve, Curve::Line(_))
}

struct Corners {
    bends: Vec<Option<Bend>>,
    trims: Vec<[f64; 2]>,
}

fn corners(chain: &Chain, radius: f64) -> Result<Corners, PipeError> {
    let legs = &chain.legs;
    let count = legs.len();
    let mut bends = vec![None; count];
    let mut trims = vec![[0.0; 2]; count];
    if radius <= 0.0 {
        return Ok(Corners { bends, trims });
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
        if !is_line(leg) || !is_line(after) {
            continue;
        }
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
        let cut = radius * (turn / 2.0).tan();
        if let Some([_, end]) = trims.get_mut(index) {
            *end = cut;
        }
        if let Some([start, _]) = trims.get_mut(next) {
            *start = cut;
        }
        for (trimmed, held) in [(index, leg), (next, after)] {
            let used = trims.get(trimmed).map_or(0.0, |[start, end]| start + end);
            if used > held.range.length() + LINEAR_RESOLUTION {
                return Err(PipeError::BendTooLong { segments });
            }
        }
        if let Some(slot) = bends.get_mut(index) {
            *slot = Some(Bend {
                corner: leg.end_point(),
                incoming,
                outgoing,
                turn,
                cut,
                key: path_bend(leg.key, after.key),
                segments,
            });
        }
    }
    Ok(Corners { bends, trims })
}

fn rounded(chain: &Chain, radius: f64) -> Result<Rounded, PipeError> {
    let Corners { bends, trims } = corners(chain, radius)?;
    let mut segments = Vec::with_capacity(chain.legs.len() * 2);
    let mut sources = Vec::with_capacity(chain.legs.len() * 2);
    for ((leg, [before, after]), bend) in chain.legs.iter().zip(trims).zip(&bends) {
        let range = leg.range;
        if before == 0.0 && after == 0.0 {
            segments.push(PathSegment::new(leg.curve.clone(), range, leg.key));
            sources.push(Source::Segment(leg.segment));
        } else if range.length() - before - after > LINEAR_RESOLUTION {
            let kept = Interval::new(range.start() + before, range.end() - after)
                .ok_or_else(degenerate)?;
            segments.push(PathSegment::new(leg.curve.clone(), kept, leg.key));
            sources.push(Source::Segment(leg.segment));
        }
        if let Some(bend) = bend {
            segments.push(bend.arc(radius)?);
            sources.push(Source::Bend(bend.segments));
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
    let reach = path
        .iter()
        .map(|segment| segment.curve().bounding_box(segment.range()).diagonal())
        .fold(0.0, f64::max);
    let tolerance = section_tolerance(&regions).max(reach * RELATIVE_TOLERANCE);
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
