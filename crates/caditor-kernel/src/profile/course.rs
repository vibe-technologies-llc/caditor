use std::f64::consts::TAU;

use caditor_geometry::{Point2, Vector2};

use crate::{
    curve2::{BSplineCurve2, Curve2},
    error::GeometryError,
    interval::Interval,
    profile::{
        Piece, PieceId,
        strand::{OffsetFailure, Strand, is_smooth_turn, offset_strands},
    },
    tolerance::LINEAR_RESOLUTION,
};

const SCAN_STEPS: usize = 256;
const BISECTION_STEPS: usize = 60;
const FIRST_FIT_SPANS: usize = 24;
const MAX_FIT_SPANS: usize = 2048;
const FIT_TOLERANCE: f64 = 1e-6;
const STRETCH_FLOOR: f64 = 0.05;
const OUTLINE_STEPS: usize = 128;
const FIT_DEGREE: usize = 3;
const FULL_TURN_SLACK: f64 = 1e-9;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Bend {
    curve: Curve2,
    range: Interval,
    reversed: bool,
}

impl Bend {
    fn travel(&self) -> f64 {
        if self.reversed { -1.0 } else { 1.0 }
    }

    fn parameter(&self, fraction: f64) -> f64 {
        self.range.at(if self.reversed {
            1.0 - fraction
        } else {
            fraction
        })
    }

    fn tangent(&self, fraction: f64) -> Vector2 {
        (self.curve.evaluate(self.parameter(fraction)).first * self.travel()).normalize_or_zero()
    }

    fn curvature(&self, fraction: f64) -> f64 {
        let at = self.curve.evaluate(self.parameter(fraction));
        let speed = at.first.length();
        self.travel() * at.first.perp_dot(at.second) / (speed * speed * speed)
    }

    fn offset_point(&self, fraction: f64, by: f64) -> Point2 {
        let at = self.curve.point(self.parameter(fraction));
        at + self.tangent(fraction).perp() * by
    }

    fn reach(&self) -> f64 {
        self.curve
            .bounding_box(self.range)
            .size()
            .length()
            .max(LINEAR_RESOLUTION)
    }

    fn is_full_turn(&self) -> bool {
        self.curve.period().is_some() && self.range.length() >= TAU - FULL_TURN_SLACK
    }

    fn keeps_going(&self, by: f64) -> bool {
        (0..=SCAN_STEPS).all(|step| {
            let fraction = step as f64 / SCAN_STEPS as f64;
            1.0 - by * self.curvature(fraction) > STRETCH_FLOOR
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Cut {
    fraction: f64,
    point: Point2,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Course {
    Plain(Strand),
    Bent { bend: Bend, by: f64 },
    Fitted(Vec<Point2>),
}

fn signed_gap(carrier: &Strand, point: Point2) -> f64 {
    match *carrier {
        Strand::Line { start, .. } => (point - start).dot(carrier.start_tangent().perp()),
        Strand::Arc { center, radius, .. } => point.distance(center) - radius,
    }
}

fn thinned(points: Vec<Point2>, gap: f64) -> Vec<Point2> {
    let last_index = points.len().saturating_sub(1);
    let mut kept: Vec<Point2> = Vec::with_capacity(points.len());
    for (index, point) in points.into_iter().enumerate() {
        match kept.last() {
            Some(previous) if previous.distance(point) < gap => {
                if index == last_index && kept.len() > 1 {
                    kept.pop();
                    kept.push(point);
                }
            }
            _ => kept.push(point),
        }
    }
    kept
}

fn interpolated(points: &[Point2]) -> Option<BSplineCurve2> {
    let degree = FIT_DEGREE.min(points.len().checked_sub(1)?);
    BSplineCurve2::interpolating(degree, points).ok()
}

fn outline(bend: &Bend, by: f64, from: &Cut, to: &Cut, spans: usize) -> Vec<Point2> {
    let along = |index: usize| {
        let share = index as f64 / spans as f64;
        from.fraction + (to.fraction - from.fraction) * share
    };
    std::iter::once(from.point)
        .chain((0..=spans).map(|index| bend.offset_point(along(index), by)))
        .chain(std::iter::once(to.point))
        .collect()
}

fn strays(bend: &Bend, by: f64, from: &Cut, to: &Cut, spans: usize, fitted: &BSplineCurve2) -> f64 {
    let domain = fitted.domain();
    let curve = Curve2::from(fitted.clone());
    (0..spans)
        .map(|index| {
            let share = (index as f64 + 0.5) / spans as f64;
            let fraction = from.fraction + (to.fraction - from.fraction) * share;
            let exact = bend.offset_point(fraction, by);
            let near = curve.closest_parameter(exact, domain);
            curve.point(near).distance(exact)
        })
        .fold(0.0, f64::max)
}

fn fit(bend: &Bend, by: f64, from: &Cut, to: &Cut, closed: bool) -> Vec<Point2> {
    let tolerance = FIT_TOLERANCE * bend.reach();
    let mut spans = FIRST_FIT_SPANS;
    loop {
        let mut points = thinned(outline(bend, by, from, to, spans), tolerance);
        if let (true, Some(first)) = (closed, points.first().copied())
            && let Some(last) = points.last_mut()
        {
            *last = first;
        }
        let within = interpolated(&points)
            .is_some_and(|fitted| strays(bend, by, from, to, spans, &fitted) <= tolerance);
        if within || spans >= MAX_FIT_SPANS {
            return points;
        }
        spans *= 2;
    }
}

fn whole(bend: &Bend, by: f64) -> (Cut, Cut) {
    (
        Cut {
            fraction: 0.0,
            point: bend.offset_point(0.0, by),
        },
        Cut {
            fraction: 1.0,
            point: bend.offset_point(1.0, by),
        },
    )
}

fn bisected(bend: &Bend, by: f64, carrier: &Strand, low: f64, high: f64) -> Cut {
    let (mut low, mut high) = (low, high);
    let low_gap = signed_gap(carrier, bend.offset_point(low, by));
    for _ in 0..BISECTION_STEPS {
        let middle = 0.5 * (low + high);
        let gap = signed_gap(carrier, bend.offset_point(middle, by));
        if gap * low_gap > 0.0 {
            low = middle;
        } else {
            high = middle;
        }
    }
    let fraction = 0.5 * (low + high);
    Cut {
        fraction,
        point: bend.offset_point(fraction, by),
    }
}

fn bent_meeting(bend: &Bend, by: f64, at_end: bool, carrier: &Strand, aim: Point2) -> Option<Cut> {
    let steps: Vec<(f64, f64)> = (0..=SCAN_STEPS)
        .map(|step| {
            let fraction = step as f64 / SCAN_STEPS as f64;
            (
                fraction,
                signed_gap(carrier, bend.offset_point(fraction, by)),
            )
        })
        .collect();
    let mut candidates: Vec<Cut> = steps
        .windows(2)
        .filter_map(|pair| match pair {
            [(low, low_gap), (high, high_gap)] if low_gap * high_gap <= 0.0 => {
                Some(bisected(bend, by, carrier, *low, *high))
            }
            _ => None,
        })
        .collect();
    let (end_fraction, direction) = if at_end {
        (1.0, bend.tangent(1.0))
    } else {
        (0.0, -bend.tangent(0.0))
    };
    let origin = bend.offset_point(end_fraction, by);
    candidates.extend(
        carrier
            .ray_crossings(origin, direction)
            .into_iter()
            .filter(|point| (*point - origin).dot(direction) >= 0.0)
            .map(|point| Cut {
                fraction: end_fraction,
                point,
            }),
    );
    candidates
        .into_iter()
        .filter(|cut| cut.point.is_finite())
        .min_by(|a, b| a.point.distance(aim).total_cmp(&b.point.distance(aim)))
}

impl Course {
    pub fn of_curve(curve: &Curve2, range: Interval, reversed: bool) -> Self {
        match Strand::of_curve(curve, range, reversed) {
            Some(strand) => Self::Plain(strand),
            None => Self::Bent {
                bend: Bend {
                    curve: curve.clone(),
                    range,
                    reversed,
                },
                by: 0.0,
            },
        }
    }

    pub fn of_piece(piece: &Piece) -> Self {
        Self::of_curve(piece.curve(), piece.range(), piece.is_reversed())
    }

    pub fn is_full_circle(&self) -> bool {
        match self {
            Self::Plain(strand) => strand.is_full_circle(),
            Self::Bent { bend, .. } => bend.is_full_turn(),
            Self::Fitted(_) => false,
        }
    }

    pub fn start(&self) -> Point2 {
        match self {
            Self::Plain(strand) => strand.start(),
            Self::Bent { bend, by } => bend.offset_point(0.0, *by),
            Self::Fitted(points) => points.first().copied().unwrap_or(Point2::ZERO),
        }
    }

    pub fn end(&self) -> Point2 {
        match self {
            Self::Plain(strand) => strand.end(),
            Self::Bent { bend, by } => bend.offset_point(1.0, *by),
            Self::Fitted(points) => points.last().copied().unwrap_or(Point2::ZERO),
        }
    }

    pub fn start_tangent(&self) -> Vector2 {
        match self {
            Self::Plain(strand) => strand.start_tangent(),
            Self::Bent { bend, .. } => bend.tangent(0.0),
            Self::Fitted(points) => match points.as_slice() {
                [first, second, ..] => (*second - *first).normalize_or_zero(),
                _ => Vector2::ZERO,
            },
        }
    }

    pub fn end_tangent(&self) -> Vector2 {
        match self {
            Self::Plain(strand) => strand.end_tangent(),
            Self::Bent { bend, .. } => bend.tangent(1.0),
            Self::Fitted(points) => match points.as_slice() {
                [.., before, last] => (*last - *before).normalize_or_zero(),
                _ => Vector2::ZERO,
            },
        }
    }

    #[must_use]
    pub fn reversed(&self) -> Self {
        match self {
            Self::Plain(strand) => Self::Plain(strand.reversed()),
            Self::Bent { bend, by } => Self::Bent {
                bend: Bend {
                    reversed: !bend.reversed,
                    ..bend.clone()
                },
                by: -*by,
            },
            Self::Fitted(points) => Self::Fitted(points.iter().rev().copied().collect()),
        }
    }

    fn offset(&self, by: f64) -> Option<Self> {
        match self {
            Self::Plain(strand) => strand.offset(by).map(Self::Plain),
            Self::Bent { bend, by: before } => {
                let total = before + by;
                bend.keeps_going(total).then(|| Self::Bent {
                    bend: bend.clone(),
                    by: total,
                })
            }
            Self::Fitted(_) => None,
        }
    }

    #[must_use]
    fn with_start(self, point: Point2) -> Self {
        match self {
            Self::Plain(strand) => Self::Plain(strand.with_start(point)),
            other => other,
        }
    }

    #[must_use]
    fn with_end(self, point: Point2) -> Self {
        match self {
            Self::Plain(strand) => Self::Plain(strand.with_end(point)),
            other => other,
        }
    }

    fn points_in_outline(&self) -> Vec<Point2> {
        match self {
            Self::Plain(strand) => strand.points(),
            Self::Bent { bend, by } => (0..=OUTLINE_STEPS)
                .map(|step| bend.offset_point(step as f64 / OUTLINE_STEPS as f64, *by))
                .collect(),
            Self::Fitted(points) => points.clone(),
        }
    }

    pub fn piece(&self, id: PieceId) -> Result<Piece, GeometryError> {
        match self {
            Self::Plain(strand) => strand.piece(id),
            Self::Bent { bend, by } if *by == 0.0 => Ok(Piece {
                id,
                curve: bend.curve.clone(),
                range: bend.range,
                reversed: bend.reversed,
            }),
            Self::Bent { bend, by } => {
                let (from, to) = whole(bend, *by);
                spline_piece(id, &fit(bend, *by, &from, &to, bend.is_full_turn()))
            }
            Self::Fitted(points) => spline_piece(id, points),
        }
    }
}

fn spline_piece(id: PieceId, points: &[Point2]) -> Result<Piece, GeometryError> {
    let fitted = interpolated(points).ok_or(GeometryError::TooFewControlPoints {
        degree: 1,
        points: points.len(),
    })?;
    Ok(Piece {
        id,
        range: fitted.domain(),
        curve: fitted.into(),
        reversed: false,
    })
}

pub(crate) fn outline_of(courses: &[Course]) -> Vec<Point2> {
    let mut points: Vec<Point2> = Vec::new();
    for course in courses {
        let mut sampled = course.points_in_outline();
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

fn finished(
    trimmed: Vec<Course>,
    cuts: &[[Option<Cut>; 2]],
    originals: &[Course],
    tolerance: f64,
) -> Result<Vec<Course>, OffsetFailure> {
    let mut finished = Vec::with_capacity(trimmed.len());
    for (index, course) in trimmed.into_iter().enumerate() {
        let [start, end] = cuts.get(index).copied().unwrap_or([None, None]);
        let course = match course {
            Course::Bent { bend, by } if start.is_some() || end.is_some() => {
                let (natural_start, natural_end) = whole(&bend, by);
                let (from, to) = (start.unwrap_or(natural_start), end.unwrap_or(natural_end));
                if to.fraction <= from.fraction {
                    return Err(OffsetFailure::Shrinks(index));
                }
                Course::Fitted(fit(&bend, by, &from, &to, false))
            }
            Course::Plain(strand) => {
                let kept = match originals.get(index) {
                    Some(Course::Plain(original)) => strand.keeps_course_of(original, tolerance),
                    _ => true,
                };
                if !kept {
                    return Err(OffsetFailure::Shrinks(index));
                }
                Course::Plain(strand)
            }
            other => other,
        };
        finished.push(course);
    }
    Ok(finished)
}

fn join_sharp(
    before: usize,
    joint: usize,
    moved_before: &Course,
    moved_after: &Course,
    aim: Point2,
    trimmed: &mut [Course],
    cuts: &mut [[Option<Cut>; 2]],
) -> Result<(), OffsetFailure> {
    let apart = OffsetFailure::Apart(before, joint);
    let (point, before_cut, after_cut) = match (moved_before, moved_after) {
        (Course::Plain(first), Course::Plain(second)) => (
            Strand::meeting(first, second, aim).ok_or(apart)?,
            None,
            None,
        ),
        (Course::Bent { bend, by }, Course::Plain(carrier)) => {
            let cut = bent_meeting(bend, *by, true, carrier, aim).ok_or(apart)?;
            (cut.point, Some(cut), None)
        }
        (Course::Plain(carrier), Course::Bent { bend, by }) => {
            let cut = bent_meeting(bend, *by, false, carrier, aim).ok_or(apart)?;
            (cut.point, None, Some(cut))
        }
        _ => return Err(apart),
    };
    set_joint(before, joint, point, trimmed);
    if let Some(slot) = cuts.get_mut(before) {
        slot[1] = before_cut.or(slot[1]);
    }
    if let Some(slot) = cuts.get_mut(joint) {
        slot[0] = after_cut.or(slot[0]);
    }
    Ok(())
}

fn set_joint(before: usize, joint: usize, point: Point2, trimmed: &mut [Course]) {
    if let Some(course) = trimmed.get_mut(before) {
        *course = course.clone().with_end(point);
    }
    if let Some(course) = trimmed.get_mut(joint) {
        *course = course.clone().with_start(point);
    }
}

pub(crate) fn offset_courses(
    courses: &[Course],
    closed: bool,
    by: f64,
    tolerance: f64,
) -> Result<Vec<Course>, OffsetFailure> {
    let strands: Vec<Strand> = courses
        .iter()
        .filter_map(|course| match course {
            Course::Plain(strand) => Some(*strand),
            Course::Bent { .. } | Course::Fitted(_) => None,
        })
        .collect();
    if strands.len() == courses.len() {
        return Ok(offset_strands(&strands, closed, by, None, tolerance)?
            .into_iter()
            .map(Course::Plain)
            .collect());
    }
    let untrimmed: Vec<Course> = courses
        .iter()
        .enumerate()
        .map(|(index, course)| course.offset(by).ok_or(OffsetFailure::Shrinks(index)))
        .collect::<Result<_, _>>()?;
    let count = untrimmed.len();
    if by == 0.0 || count == 0 || (closed && count == 1) {
        if let Some(index) = untrimmed.iter().position(
            |course| matches!(course, Course::Plain(Strand::Arc { radius, .. }) if *radius <= tolerance),
        ) {
            return Err(OffsetFailure::Shrinks(index));
        }
        return Ok(untrimmed
            .into_iter()
            .map(|course| match course {
                Course::Bent { bend, by } if by != 0.0 => {
                    let (from, to) = whole(&bend, by);
                    Course::Fitted(fit(&bend, by, &from, &to, bend.is_full_turn()))
                }
                other => other,
            })
            .collect());
    }
    let mut trimmed = untrimmed.clone();
    let mut cuts: Vec<[Option<Cut>; 2]> = vec![[None, None]; count];
    for joint in usize::from(!closed)..count {
        let before = (joint + count - 1) % count;
        let (Some(original_before), Some(original_after)) =
            (courses.get(before), courses.get(joint))
        else {
            continue;
        };
        let (Some(moved_before), Some(moved_after)) = (untrimmed.get(before), untrimmed.get(joint))
        else {
            continue;
        };
        let neutral = moved_before.end().midpoint(moved_after.start());
        let leaving = original_before.end_tangent();
        let entering = original_after.start_tangent();
        if is_smooth_turn(leaving, entering) {
            if leaving.dot(entering) < 0.0 {
                return Err(OffsetFailure::Folds(before, joint));
            }
            set_joint(before, joint, neutral, &mut trimmed);
        } else {
            join_sharp(
                before,
                joint,
                moved_before,
                moved_after,
                neutral,
                &mut trimmed,
                &mut cuts,
            )?;
        }
    }
    finished(trimmed, &cuts, courses, tolerance)
}
