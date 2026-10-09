use std::f64::consts::{PI, TAU};

use caditor_geometry::{Point2, Vector2};

use crate::{
    curve2::{Circle2, Curve2, Line2},
    error::GeometryError,
    interval::Interval,
    profile::{Piece, PieceId},
};

const SMOOTH_TURN: f64 = 1e-6;
const FULL_TURN_SLACK: f64 = 1e-9;
const SAMPLE_ANGLE: f64 = PI / 90.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Strand {
    Line {
        start: Point2,
        end: Point2,
    },
    Arc {
        center: Point2,
        radius: f64,
        from: f64,
        sweep: f64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OffsetFailure {
    Shrinks(usize),
    Apart(usize, usize),
    Folds(usize, usize),
}

impl OffsetFailure {
    pub fn strands(self) -> Vec<usize> {
        match self {
            Self::Shrinks(strand) => vec![strand],
            Self::Apart(before, after) | Self::Folds(before, after) => vec![before, after],
        }
    }
}

fn wrapped(angle: f64) -> f64 {
    (angle + PI).rem_euclid(TAU) - PI
}

fn direction_angle(direction: Vector2) -> f64 {
    direction.y.atan2(direction.x)
}

impl Strand {
    pub fn of_curve(curve: &Curve2, range: Interval, reversed: bool) -> Option<Self> {
        match curve {
            Curve2::Line(line) => {
                let (first, last) = (line.point(range.start()), line.point(range.end()));
                Some(if reversed {
                    Self::Line {
                        start: last,
                        end: first,
                    }
                } else {
                    Self::Line {
                        start: first,
                        end: last,
                    }
                })
            }
            Curve2::Circle(circle) => {
                let counter_clockwise = circle.is_counter_clockwise() != reversed;
                let begin = if reversed { range.end() } else { range.start() };
                let sweep = if counter_clockwise {
                    range.length()
                } else {
                    -range.length()
                };
                Some(Self::Arc {
                    center: circle.center(),
                    radius: circle.radius(),
                    from: direction_angle(curve.point(begin) - circle.center()),
                    sweep,
                })
            }
            Curve2::BSpline(_) => None,
        }
    }

    pub fn of_piece(piece: &Piece) -> Option<Self> {
        Self::of_curve(piece.curve(), piece.range(), piece.is_reversed())
    }

    pub fn is_line(&self) -> bool {
        matches!(self, Self::Line { .. })
    }

    pub fn is_full_circle(&self) -> bool {
        matches!(self, Self::Arc { sweep, .. } if sweep.abs() >= TAU - FULL_TURN_SLACK)
    }

    pub fn start(&self) -> Point2 {
        match *self {
            Self::Line { start, .. } => start,
            Self::Arc {
                center,
                radius,
                from,
                ..
            } => center + Vector2::from_angle(from) * radius,
        }
    }

    pub fn end(&self) -> Point2 {
        match *self {
            Self::Line { end, .. } => end,
            Self::Arc {
                center,
                radius,
                from,
                sweep,
            } => center + Vector2::from_angle(from + sweep) * radius,
        }
    }

    pub fn middle(&self) -> (Point2, Vector2) {
        match *self {
            Self::Line { start, end } => (start.midpoint(end), self.start_tangent()),
            Self::Arc {
                center,
                radius,
                from,
                sweep,
            } => {
                let angle = from + 0.5 * sweep;
                (
                    center + Vector2::from_angle(angle) * radius,
                    Vector2::from_angle(angle).perp() * sweep.signum(),
                )
            }
        }
    }

    pub fn start_tangent(&self) -> Vector2 {
        match *self {
            Self::Line { start, end } => (end - start).normalize_or_zero(),
            Self::Arc { from, sweep, .. } => Vector2::from_angle(from).perp() * sweep.signum(),
        }
    }

    pub fn end_tangent(&self) -> Vector2 {
        match *self {
            Self::Line { .. } => self.start_tangent(),
            Self::Arc { from, sweep, .. } => {
                Vector2::from_angle(from + sweep).perp() * sweep.signum()
            }
        }
    }

    #[must_use]
    pub fn reversed(&self) -> Self {
        match *self {
            Self::Line { start, end } => Self::Line {
                start: end,
                end: start,
            },
            Self::Arc {
                center,
                radius,
                from,
                sweep,
            } => Self::Arc {
                center,
                radius,
                from: from + sweep,
                sweep: -sweep,
            },
        }
    }

    pub fn offset(&self, by: f64) -> Option<Self> {
        match *self {
            Self::Line { start, end } => {
                let shift = self.start_tangent().perp() * by;
                Some(Self::Line {
                    start: start + shift,
                    end: end + shift,
                })
            }
            Self::Arc {
                center,
                radius,
                from,
                sweep,
            } => {
                let radius = radius - by * sweep.signum();
                (radius > 0.0).then_some(Self::Arc {
                    center,
                    radius,
                    from,
                    sweep,
                })
            }
        }
    }

    #[must_use]
    pub fn with_start(self, point: Point2) -> Self {
        match self {
            Self::Line { end, .. } => Self::Line { start: point, end },
            Self::Arc {
                center,
                radius,
                from,
                sweep,
            } => {
                let moved = from + wrapped(direction_angle(point - center) - from);
                Self::Arc {
                    center,
                    radius,
                    from: moved,
                    sweep: from + sweep - moved,
                }
            }
        }
    }

    #[must_use]
    pub fn with_end(self, point: Point2) -> Self {
        match self {
            Self::Line { start, .. } => Self::Line { start, end: point },
            Self::Arc {
                center,
                radius,
                from,
                sweep,
            } => {
                let end = from + sweep;
                let moved = end + wrapped(direction_angle(point - center) - end);
                Self::Arc {
                    center,
                    radius,
                    from,
                    sweep: moved - from,
                }
            }
        }
    }

    fn keeps_course_of(&self, original: &Self, tolerance: f64) -> bool {
        match (*self, *original) {
            (Self::Line { start, end }, Self::Line { .. }) => {
                (end - start).dot(original.start_tangent()) > tolerance
            }
            (Self::Arc { radius, sweep, .. }, Self::Arc { sweep: whole, .. }) => {
                radius > tolerance
                    && (whole.abs() >= TAU - FULL_TURN_SLACK
                        || sweep * whole.signum() * radius > tolerance)
            }
            _ => false,
        }
    }

    pub fn meeting(before: &Self, after: &Self, aim: Point2) -> Option<Point2> {
        let candidates = match (*before, *after) {
            (Self::Line { .. }, Self::Line { .. }) => {
                let (first, second) = (before.start_tangent(), after.start_tangent());
                let turn = first.perp_dot(second);
                if turn.abs() <= SMOOTH_TURN * SMOOTH_TURN {
                    return None;
                }
                let along = (after.start() - before.start()).perp_dot(second) / turn;
                vec![before.start() + first * along]
            }
            (Self::Line { .. }, Self::Arc { center, radius, .. }) => {
                line_circle(before.start(), before.start_tangent(), center, radius)
            }
            (Self::Arc { center, radius, .. }, Self::Line { .. }) => {
                line_circle(after.start(), after.start_tangent(), center, radius)
            }
            (
                Self::Arc { center, radius, .. },
                Self::Arc {
                    center: other_center,
                    radius: other_radius,
                    ..
                },
            ) => circle_circle(center, radius, other_center, other_radius),
        };
        candidates
            .into_iter()
            .filter(|point| point.is_finite())
            .min_by(|a, b| a.distance(aim).total_cmp(&b.distance(aim)))
    }

    pub fn curve(&self) -> Result<(Curve2, Interval, bool), GeometryError> {
        match *self {
            Self::Line { start, end } => {
                let line = Line2::through(start, end)?;
                let range =
                    Interval::new(0.0, start.distance(end)).ok_or(GeometryError::NonFinite)?;
                Ok((line.into(), range, false))
            }
            Self::Arc {
                center,
                radius,
                from,
                sweep,
            } => {
                let circle = Circle2::new(center, radius)?;
                let (low, high, reversed) = if sweep >= 0.0 {
                    (from, from + sweep, false)
                } else {
                    (from + sweep, from, true)
                };
                let range = Interval::new(low, high).ok_or(GeometryError::NonFinite)?;
                Ok((circle.into(), range, reversed))
            }
        }
    }

    pub fn piece(&self, id: PieceId) -> Result<Piece, GeometryError> {
        let (curve, range, reversed) = self.curve()?;
        Ok(Piece {
            id,
            curve,
            range,
            reversed,
        })
    }

    pub fn points(&self) -> Vec<Point2> {
        match *self {
            Self::Line { start, end } => vec![start, end],
            Self::Arc {
                center,
                radius,
                from,
                sweep,
            } => {
                let segments = (sweep.abs() / SAMPLE_ANGLE).ceil().max(1.0) as usize;
                (0..=segments)
                    .map(|index| {
                        let angle = from + sweep * index as f64 / segments as f64;
                        center + Vector2::from_angle(angle) * radius
                    })
                    .collect()
            }
        }
    }
}

fn line_circle(through: Point2, direction: Vector2, center: Point2, radius: f64) -> Vec<Point2> {
    let offset = through - center;
    let half = direction.dot(offset);
    let rest = offset.length_squared() - radius * radius;
    let discriminant = half * half - rest;
    if discriminant < -1e-12 * radius * radius {
        return Vec::new();
    }
    let root = discriminant.max(0.0).sqrt();
    vec![
        through + direction * (-half - root),
        through + direction * (-half + root),
    ]
}

fn circle_circle(first: Point2, radius: f64, second: Point2, other_radius: f64) -> Vec<Point2> {
    let between = second - first;
    let distance = between.length();
    if distance <= f64::EPSILON * (radius + other_radius) {
        return Vec::new();
    }
    let along =
        (radius * radius - other_radius * other_radius + distance * distance) / (2.0 * distance);
    let squared = radius * radius - along * along;
    if squared < -1e-12 * radius * radius {
        return Vec::new();
    }
    let across = squared.max(0.0).sqrt();
    let axis = between / distance;
    let foot = first + axis * along;
    vec![foot + axis.perp() * across, foot - axis.perp() * across]
}

pub(crate) fn is_smooth(before: &Strand, after: &Strand) -> bool {
    let (leaving, entering) = (before.end_tangent(), after.start_tangent());
    leaving.perp_dot(entering).abs() <= SMOOTH_TURN && leaving.dot(entering) > 0.0
}

pub(crate) fn offset_strands(
    strands: &[Strand],
    closed: bool,
    by: f64,
    aims: Option<&[Strand]>,
    tolerance: f64,
) -> Result<Vec<Strand>, OffsetFailure> {
    let untrimmed: Vec<Strand> = strands
        .iter()
        .enumerate()
        .map(|(index, strand)| strand.offset(by).ok_or(OffsetFailure::Shrinks(index)))
        .collect::<Result<_, _>>()?;
    let count = untrimmed.len();
    if by == 0.0 || count == 0 || (closed && count == 1) {
        return match untrimmed
            .iter()
            .position(|strand| matches!(strand, Strand::Arc { radius, .. } if *radius <= tolerance))
        {
            Some(index) => Err(OffsetFailure::Shrinks(index)),
            None => Ok(untrimmed),
        };
    }
    let mut trimmed = untrimmed.clone();
    let first_joint = usize::from(!closed);
    for joint in first_joint..count {
        let before = (joint + count - 1) % count;
        let (Some(original_before), Some(original_after)) =
            (strands.get(before), strands.get(joint))
        else {
            continue;
        };
        let (Some(moved_before), Some(moved_after)) = (untrimmed.get(before), untrimmed.get(joint))
        else {
            continue;
        };
        let leaving = original_before.end_tangent();
        let entering = original_after.start_tangent();
        let point = if leaving.perp_dot(entering).abs() <= SMOOTH_TURN {
            if leaving.dot(entering) < 0.0 {
                return Err(OffsetFailure::Folds(before, joint));
            }
            moved_before.end().midpoint(moved_after.start())
        } else {
            let aim = aims.and_then(|aims| aims.get(joint)).map_or_else(
                || moved_before.end().midpoint(moved_after.start()),
                Strand::start,
            );
            Strand::meeting(moved_before, moved_after, aim)
                .ok_or(OffsetFailure::Apart(before, joint))?
        };
        if let Some(strand) = trimmed.get_mut(before) {
            *strand = strand.with_end(point);
        }
        if let Some(strand) = trimmed.get_mut(joint) {
            *strand = strand.with_start(point);
        }
    }
    for (index, (strand, original)) in trimmed.iter().zip(strands).enumerate() {
        if !strand.keeps_course_of(original, tolerance) {
            return Err(OffsetFailure::Shrinks(index));
        }
    }
    Ok(trimmed)
}

pub(crate) fn polygon(strands: &[Strand]) -> Vec<Point2> {
    let mut points: Vec<Point2> = Vec::new();
    for strand in strands {
        let mut sampled = strand.points();
        if !points.is_empty() && !sampled.is_empty() {
            sampled.remove(0);
        }
        points.extend(sampled);
    }
    if points.len() > 1 && points.first() == points.last() {
        points.pop();
    }
    points
}

pub(crate) fn signed_area(polygon: &[Point2]) -> f64 {
    let following = polygon.iter().skip(1).chain(polygon.first());
    0.5 * polygon
        .iter()
        .zip(following)
        .map(|(a, b)| a.perp_dot(*b))
        .sum::<f64>()
}

struct Segment {
    polygon: usize,
    index: usize,
    count: usize,
    from: Point2,
    to: Point2,
}

impl Segment {
    fn low(&self) -> Point2 {
        self.from.min(self.to)
    }

    fn high(&self) -> Point2 {
        self.from.max(self.to)
    }

    fn adjacent(&self, other: &Self) -> bool {
        if self.polygon != other.polygon {
            return false;
        }
        let (low, high) = (self.index.min(other.index), self.index.max(other.index));
        high - low <= 1 || (low == 0 && high + 1 == self.count)
    }
}

fn orientation(a: Point2, b: Point2, c: Point2) -> f64 {
    (b - a).perp_dot(c - a)
}

fn properly_cross(first: &Segment, second: &Segment) -> bool {
    let (a, b, c, d) = (first.from, first.to, second.from, second.to);
    orientation(a, b, c) * orientation(a, b, d) < 0.0
        && orientation(c, d, a) * orientation(c, d, b) < 0.0
}

pub(crate) fn polygons_cross(polygons: &[Vec<Point2>]) -> bool {
    let mut segments: Vec<Segment> = polygons
        .iter()
        .enumerate()
        .flat_map(|(polygon, points)| {
            let count = points.len();
            let following = points.iter().skip(1).chain(points.first());
            points
                .iter()
                .zip(following)
                .enumerate()
                .map(move |(index, (from, to))| Segment {
                    polygon,
                    index,
                    count,
                    from: *from,
                    to: *to,
                })
        })
        .collect();
    segments.sort_by(|a, b| a.low().x.total_cmp(&b.low().x));
    for (position, first) in segments.iter().enumerate() {
        let (low, high) = (first.low(), first.high());
        for second in segments.iter().skip(position + 1) {
            if second.low().x > high.x {
                break;
            }
            let overlapping = second.low().y <= high.y && second.high().y >= low.y;
            if overlapping && !first.adjacent(second) && properly_cross(first, second) {
                return true;
            }
        }
    }
    false
}
