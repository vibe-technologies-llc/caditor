use caditor_geometry::{Point2, Vector2};
use caditor_kernel::{Curve2, Interval, LINEAR_RESOLUTION, ProfileCurve, ProfileError};
use caditor_sketch::Sketch;

use crate::solid::profile_curves;

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
