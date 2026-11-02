use caditor_geometry::{Plane, Point2, Point3, RigidTransform, Rotation3, Vector2, Vector3};
use caditor_kernel::{
    ChainedPath, Curve, Curve2, EdgeId, EdgeNaming, EdgeReference, Interval, LINEAR_RESOLUTION,
    PathSegment, PathSweepError, ProfileCurve, ProfileError, ReferenceError, Solid, chained_path,
};
use caditor_sketch::Sketch;

use crate::{
    pattern::{CopyOrientation, along_curve},
    solid::profile_curves,
};

const TRANSPORT_STEPS: usize = 64;
const BISECTION_STEPS: usize = 60;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PathError {
    #[error("the sketch has no curve")]
    NoCurves,
    #[error("the curves do not form one chain")]
    NotOneChain,
    #[error("curve {entity} has no length")]
    NoLength { entity: u64 },
    #[error("a curve could not be read: {0}")]
    Curve(ProfileError),
}

struct Segment {
    entity: u64,
    curve: Curve2,
    range: Interval,
    ends: [Point2; 2],
    length: f64,
}

struct Piece {
    curve: Curve2,
    range: Interval,
    reversed: bool,
    length: f64,
}

pub(crate) struct CurvePath {
    pieces: Vec<Piece>,
    closed: bool,
    length: f64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Station {
    pub point: Point2,
    pub tangent: Vector2,
}

fn segment(curve: &ProfileCurve) -> Result<Segment, PathError> {
    let (path, range) = curve.curve().map_err(PathError::Curve)?;
    let length = path.length(range);
    if !length.is_finite() || length <= LINEAR_RESOLUTION {
        return Err(PathError::NoLength {
            entity: curve.entity,
        });
    }
    let ends = [
        path.evaluate(range.start()).point,
        path.evaluate(range.end()).point,
    ];
    Ok(Segment {
        entity: curve.entity,
        curve: path,
        range,
        ends,
        length,
    })
}

fn end_point(segments: &[Segment], end: usize) -> Option<Point2> {
    let [start, finish] = segments.get(end / 2)?.ends;
    Some(if end.is_multiple_of(2) { start } else { finish })
}

fn is_closed(segment: &Segment) -> bool {
    let [start, finish] = segment.ends;
    start.distance(finish) <= LINEAR_RESOLUTION
}

fn partners(segments: &[Segment]) -> Vec<Vec<usize>> {
    let ends = segments.len() * 2;
    (0..ends)
        .map(|end| {
            let point = end_point(segments, end);
            (0..ends)
                .filter(|other| {
                    *other / 2 != end / 2
                        && point
                            .zip(end_point(segments, *other))
                            .is_some_and(|(point, near)| point.distance(near) <= LINEAR_RESOLUTION)
                })
                .collect()
        })
        .collect()
}

fn first_end(segments: &[Segment], joined: &[Vec<usize>]) -> Result<(usize, bool), PathError> {
    let free: Vec<usize> = joined
        .iter()
        .enumerate()
        .filter(|(_, partners)| partners.is_empty())
        .map(|(end, _)| end)
        .collect();
    let order = |end: usize| {
        segments
            .get(end / 2)
            .map(|segment| (segment.entity, end % 2))
    };
    match free.as_slice() {
        [] => segments
            .iter()
            .enumerate()
            .min_by_key(|(_, segment)| segment.entity)
            .map(|(index, _)| (index * 2, true))
            .ok_or(PathError::NoCurves),
        [one, other] => Ok((
            if order(*one) <= order(*other) {
                *one
            } else {
                *other
            },
            false,
        )),
        _ => Err(PathError::NotOneChain),
    }
}

pub(crate) fn curve_path(sketch: &Sketch) -> Result<CurvePath, PathError> {
    let segments = profile_curves(sketch)
        .iter()
        .map(segment)
        .collect::<Result<Vec<_>, _>>()?;
    if let [only] = segments.as_slice() {
        let closed = is_closed(only);
        return Ok(CurvePath::new(
            vec![Piece {
                curve: only.curve.clone(),
                range: only.range,
                reversed: false,
                length: only.length,
            }],
            closed,
        ));
    }
    if segments.is_empty() {
        return Err(PathError::NoCurves);
    }
    if segments.iter().any(is_closed) {
        return Err(PathError::NotOneChain);
    }
    let joined = partners(&segments);
    if joined.iter().any(|partners| partners.len() > 1) {
        return Err(PathError::NotOneChain);
    }
    let (start, closed) = first_end(&segments, &joined)?;
    let mut pieces = Vec::with_capacity(segments.len());
    let mut at = start;
    loop {
        let segment = segments.get(at / 2).ok_or(PathError::NotOneChain)?;
        pieces.push(Piece {
            curve: segment.curve.clone(),
            range: segment.range,
            reversed: !at.is_multiple_of(2),
            length: segment.length,
        });
        let next = joined.get(at ^ 1).and_then(|partners| partners.first());
        match next {
            None if !closed => break,
            Some(next) if closed && *next == start => break,
            Some(next) if pieces.len() < segments.len() => at = *next,
            _ => return Err(PathError::NotOneChain),
        }
    }
    if pieces.len() != segments.len() {
        return Err(PathError::NotOneChain);
    }
    Ok(CurvePath::new(pieces, closed))
}

impl Piece {
    fn parameter_at(&self, distance: f64) -> f64 {
        let (from, to) = if self.reversed {
            (self.range.end(), self.range.start())
        } else {
            (self.range.start(), self.range.end())
        };
        let share = (distance / self.length).clamp(0.0, 1.0);
        if matches!(self.curve, Curve2::Line(_) | Curve2::Circle(_)) {
            return from + (to - from) * share;
        }
        let (mut low, mut high) = (0.0, 1.0);
        for _ in 0..BISECTION_STEPS {
            let middle = 0.5 * (low + high);
            let parameter = from + (to - from) * middle;
            let span = Interval::new(from.min(parameter), from.max(parameter));
            let reached = span.map_or(0.0, |span| self.curve.length(span));
            if reached < distance {
                low = middle;
            } else {
                high = middle;
            }
        }
        from + (to - from) * 0.5 * (low + high)
    }

    fn station(&self, distance: f64) -> Option<Station> {
        let derivatives = self.curve.evaluate(self.parameter_at(distance));
        let tangent = derivatives.first.try_normalize()?;
        Some(Station {
            point: derivatives.point,
            tangent: if self.reversed { -tangent } else { tangent },
        })
    }
}

impl CurvePath {
    fn new(pieces: Vec<Piece>, closed: bool) -> Self {
        let length = pieces.iter().map(|piece| piece.length).sum();
        Self {
            pieces,
            closed,
            length,
        }
    }

    pub(crate) fn length(&self) -> f64 {
        self.length
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.closed
    }

    fn forward(&self, distance: f64) -> Option<Station> {
        let mut left = distance.clamp(0.0, self.length);
        let last = self.pieces.len().checked_sub(1)?;
        for (index, piece) in self.pieces.iter().enumerate() {
            if left <= piece.length || index == last {
                return piece.station(left);
            }
            left -= piece.length;
        }
        None
    }

    pub(crate) fn station(&self, distance: f64, reversed: bool) -> Option<Station> {
        if !reversed {
            return self.forward(distance);
        }
        let back = if self.closed {
            (self.length - distance).rem_euclid(self.length)
        } else {
            self.length - distance
        };
        self.forward(back).map(|station| Station {
            point: station.point,
            tangent: -station.tangent,
        })
    }
}

pub fn is_curve_path(sketch: &Sketch) -> bool {
    curve_path(sketch).is_ok()
}

pub struct PatternPath {
    walk: Walk,
}

enum Walk {
    Flat { path: CurvePath, plane: Plane },
    Spatial(EdgeWalk),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CopyStop {
    pub placement: RigidTransform,
    pub point: Point3,
    pub tangent: Vector3,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum EdgePathError {
    Lost {
        edge: usize,
        cause: ReferenceError<EdgeId>,
    },
    Chain(PathSweepError),
    Unfollowable,
}

pub(crate) fn edge_segments(
    solid: &Solid,
    edges: &[EdgeReference],
) -> Result<Vec<(PathSegment, EdgeId)>, EdgePathError> {
    let naming = EdgeNaming::new(solid);
    edges
        .iter()
        .enumerate()
        .map(|(index, reference)| {
            let lost = |cause| EdgePathError::Lost { edge: index, cause };
            let id = reference.resolve_in(&naming).map_err(lost)?;
            let edge = solid
                .edge(id)
                .ok_or_else(|| lost(ReferenceError::Missing))?;
            Ok((
                PathSegment::new(
                    edge.curve().clone(),
                    edge.interval(),
                    reference.name().digest(),
                ),
                id,
            ))
        })
        .collect()
}

pub(crate) fn edge_walk(segments: &[PathSegment]) -> Result<PatternPath, EdgePathError> {
    let chain = chained_path(segments).map_err(EdgePathError::Chain)?;
    let walk = EdgeWalk::new(&chain).ok_or(EdgePathError::Unfollowable)?;
    Ok(PatternPath {
        walk: Walk::Spatial(walk),
    })
}

impl PatternPath {
    pub fn of(sketch: &Sketch) -> Option<Self> {
        Some(Self::flat(curve_path(sketch).ok()?, sketch.plane()))
    }

    pub fn along_edges(solid: &Solid, edges: &[EdgeReference]) -> Option<Self> {
        let segments: Vec<PathSegment> = edge_segments(solid, edges)
            .ok()?
            .into_iter()
            .map(|(segment, _)| segment)
            .collect();
        edge_walk(&segments).ok()
    }

    pub(crate) fn flat(path: CurvePath, plane: Plane) -> Self {
        Self {
            walk: Walk::Flat { path, plane },
        }
    }

    pub fn length(&self) -> f64 {
        match &self.walk {
            Walk::Flat { path, .. } => path.length(),
            Walk::Spatial(walk) => walk.forward.length,
        }
    }

    pub fn is_closed(&self) -> bool {
        match &self.walk {
            Walk::Flat { path, .. } => path.is_closed(),
            Walk::Spatial(walk) => walk.closed,
        }
    }

    pub fn copy_at(
        &self,
        distance: f64,
        reversed: bool,
        orientation: CopyOrientation,
    ) -> Option<CopyStop> {
        match &self.walk {
            Walk::Flat { path, plane } => {
                let start = path.station(0.0, reversed)?;
                let station = path.station(distance, reversed)?;
                let tangent = station.tangent;
                Some(CopyStop {
                    placement: along_curve(plane, start, station, orientation)?,
                    point: plane.to_world(station.point),
                    tangent: plane.x_axis() * tangent.x + plane.y_axis() * tangent.y,
                })
            }
            Walk::Spatial(walk) => walk.copy_at(distance, reversed, orientation),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Frame {
    point: Point3,
    tangent: Vector3,
    across: Vector3,
}

impl Frame {
    fn first(point: Point3, tangent: Vector3) -> Option<Self> {
        let reference = if tangent.x.abs() < 0.9 {
            Vector3::X
        } else {
            Vector3::Y
        };
        Some(Self {
            point,
            tangent,
            across: tangent.cross(reference).try_normalize()?,
        })
    }

    fn carried_to(&self, point: Point3, tangent: Vector3) -> Option<Self> {
        let step = point - self.point;
        let reach = step.length_squared();
        let (across, along) = if reach > 0.0 {
            (
                self.across - step * (2.0 * step.dot(self.across) / reach),
                self.tangent - step * (2.0 * step.dot(self.tangent) / reach),
            )
        } else {
            (self.across, self.tangent)
        };
        let second = tangent - along;
        let second_reach = second.length_squared();
        let turned = if second_reach > 0.0 {
            across - second * (2.0 * second.dot(across) / second_reach)
        } else {
            across
        };
        Some(Self {
            point,
            tangent,
            across: (turned - tangent * tangent.dot(turned)).try_normalize()?,
        })
    }

    fn turned_to(&self, point: Point3, tangent: Vector3) -> Option<Self> {
        let across = Rotation3::from_rotation_arc(self.tangent, tangent) * self.across;
        Some(Self {
            point,
            tangent,
            across: (across - tangent * tangent.dot(across)).try_normalize()?,
        })
    }

    fn rotation(&self) -> Option<Rotation3> {
        let normal = self.tangent.cross(self.across);
        let frame = Plane::from_frame(Point3::ZERO, normal, self.tangent)?;
        RigidTransform::from_frame(&frame).map(|placed| placed.rotation())
    }
}

struct Sample {
    parameter: f64,
    frame: Frame,
}

struct Leg {
    curve: Curve,
    range: Interval,
    length: f64,
    samples: Vec<Sample>,
}

impl Leg {
    fn parameter_at(&self, distance: f64) -> f64 {
        let (from, to) = (self.range.start(), self.range.end());
        let share = (distance / self.length).clamp(0.0, 1.0);
        if matches!(self.curve, Curve::Line(_) | Curve::Circle(_)) {
            return from + (to - from) * share;
        }
        let (mut low, mut high) = (0.0, 1.0);
        for _ in 0..BISECTION_STEPS {
            let middle = 0.5 * (low + high);
            let reached = Interval::new(from, from + (to - from) * middle)
                .map_or(0.0, |span| self.curve.length(span));
            if reached < distance {
                low = middle;
            } else {
                high = middle;
            }
        }
        from + (to - from) * 0.5 * (low + high)
    }

    fn frame_at(&self, distance: f64) -> Option<Frame> {
        let parameter = self.parameter_at(distance);
        let derivatives = self.curve.evaluate(parameter);
        let tangent = derivatives.first.try_normalize()?;
        let before = self
            .samples
            .iter()
            .rev()
            .find(|sample| sample.parameter <= parameter)
            .or_else(|| self.samples.first())?;
        before.frame.carried_to(derivatives.point, tangent)
    }
}

struct Spatial {
    legs: Vec<Leg>,
    length: f64,
}

impl Spatial {
    fn new(legs: impl Iterator<Item = (Curve, Interval)>) -> Option<Self> {
        let mut built = Vec::new();
        let mut carried: Option<Frame> = None;
        for (curve, range) in legs {
            let length = curve.length(range);
            if !length.is_finite() || length <= LINEAR_RESOLUTION {
                return None;
            }
            let mut samples = Vec::with_capacity(TRANSPORT_STEPS + 1);
            for step in 0..=TRANSPORT_STEPS {
                let share = step as f64 / TRANSPORT_STEPS as f64;
                let parameter = range.start() + (range.end() - range.start()) * share;
                let derivatives = curve.evaluate(parameter);
                let tangent = derivatives.first.try_normalize()?;
                let frame = match (carried, step) {
                    (Some(previous), 0) => previous.turned_to(derivatives.point, tangent)?,
                    (Some(previous), _) => previous.carried_to(derivatives.point, tangent)?,
                    (None, _) => Frame::first(derivatives.point, tangent)?,
                };
                carried = Some(frame);
                samples.push(Sample { parameter, frame });
            }
            built.push(Leg {
                curve,
                range,
                length,
                samples,
            });
        }
        let length = built.iter().map(|leg| leg.length).sum();
        Some(Self {
            legs: built,
            length,
        })
    }

    fn frame_at(&self, distance: f64) -> Option<Frame> {
        let mut left = distance.clamp(0.0, self.length);
        let last = self.legs.len().checked_sub(1)?;
        for (index, leg) in self.legs.iter().enumerate() {
            if left <= leg.length || index == last {
                return leg.frame_at(left);
            }
            left -= leg.length;
        }
        None
    }
}

struct EdgeWalk {
    forward: Spatial,
    backward: Spatial,
    closed: bool,
}

impl EdgeWalk {
    fn new(chain: &ChainedPath) -> Option<Self> {
        let forward = Spatial::new(chain.legs.iter().map(|leg| (leg.curve.clone(), leg.range)))?;
        let backward = Spatial::new(
            chain
                .legs
                .iter()
                .rev()
                .map(|leg| (leg.curve.reversed(), leg.curve.reversed_range(leg.range))),
        )?;
        Some(Self {
            forward,
            backward,
            closed: chain.closed,
        })
    }

    fn copy_at(
        &self,
        distance: f64,
        reversed: bool,
        orientation: CopyOrientation,
    ) -> Option<CopyStop> {
        let walk = if reversed {
            &self.backward
        } else {
            &self.forward
        };
        let start = walk.frame_at(0.0)?;
        let station = walk.frame_at(distance)?;
        let placement = match orientation {
            CopyOrientation::Kept => RigidTransform::translation(station.point - start.point)?,
            CopyOrientation::Following => {
                let rotation = station.rotation()? * start.rotation()?.conjugate();
                RigidTransform::new(rotation, station.point - rotation * start.point)?
            }
        };
        Some(CopyStop {
            placement,
            point: station.point,
            tangent: station.tangent,
        })
    }
}
