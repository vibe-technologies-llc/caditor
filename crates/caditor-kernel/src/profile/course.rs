use std::f64::consts::TAU;

use caditor_geometry::{Point2, Vector2};

use crate::{
    curve2::{BSplineCurve2, Curve2},
    error::GeometryError,
    interval::Interval,
    profile::{
        Piece, PieceId,
        grading::graded,
        strand::{OffsetFailure, Strand, is_smooth_turn, offset_strands},
    },
    tolerance::LINEAR_RESOLUTION,
};

const SCAN_STEPS: usize = 256;
const BISECTION_STEPS: usize = 60;
const FIRST_FIT_SPANS: usize = 24;
const FIT_TOLERANCE: f64 = 1e-6;
const STRETCH_FLOOR: f64 = 0.05;
const OUTLINE_STEPS: usize = 128;
const FIT_DEGREE: usize = 3;
const FULL_TURN_SLACK: f64 = 1e-9;
const FIT_CEILING: f64 = 1e-7;
const NEWTON_STEPS: usize = 40;
const DIFFERENCE_STEP: f64 = 1e-6;
const CROSSING_CONVERGENCE: f64 = 1e-9;
const RAY_SLACK: f64 = 1e-9;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Bend {
    curve: Curve2,
    range: Interval,
    reversed: bool,
    reach: f64,
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

    pub fn tangent(&self, fraction: f64) -> Vector2 {
        let at = self.parameter(fraction.clamp(0.0, 1.0));
        (self.curve.evaluate(at).first * self.travel()).normalize_or_zero()
    }

    fn curvature(&self, fraction: f64) -> f64 {
        let at = self.curve.evaluate(self.parameter(fraction));
        let speed = at.first.length();
        self.travel() * at.first.perp_dot(at.second) / (speed * speed * speed)
    }

    pub fn offset_point(&self, fraction: f64, by: f64) -> Point2 {
        let inside = fraction.clamp(0.0, 1.0);
        let at = self.curve.point(self.parameter(inside));
        let moved = at + self.tangent(inside).perp() * by;
        moved + self.tangent(inside) * ((fraction - inside) * self.reach)
    }

    pub fn reach(&self) -> f64 {
        self.reach
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
pub(crate) struct Trim {
    bend: Bend,
    by: f64,
    from: Cut,
    to: Cut,
    closed: bool,
}

impl Trim {
    fn whole(bend: &Bend, by: f64) -> Self {
        let (from, to) = whole(bend, by);
        Self {
            bend: bend.clone(),
            by,
            from,
            to,
            closed: bend.is_full_turn(),
        }
    }

    fn between(bend: &Bend, by: f64, from: Cut, to: Cut) -> Self {
        Self {
            bend: bend.clone(),
            by,
            from,
            to,
            closed: false,
        }
    }

    fn middle_fraction(&self) -> f64 {
        0.5 * (self.from.fraction + self.to.fraction)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Course {
    Plain(Strand),
    Bent { bend: Bend, by: f64 },
    Trimmed(Trim),
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
    (0..=spans)
        .map(|index| match index {
            0 => from.point,
            last if last == spans => to.point,
            _ => {
                let share = index as f64 / spans as f64;
                bend.offset_point(from.fraction + (to.fraction - from.fraction) * share, by)
            }
        })
        .collect()
}

fn fit(bend: &Bend, by: f64, from: &Cut, to: &Cut, closed: bool) -> Vec<Point2> {
    let tolerance = (FIT_TOLERANCE * bend.reach()).min(FIT_CEILING);
    let points_at = |nodes: &[f64]| -> Vec<Point2> {
        let mut points: Vec<Point2> = nodes
            .iter()
            .map(|fraction| bend.offset_point(*fraction, by))
            .collect();
        if let (true, Some(first)) = (closed, points.first().copied())
            && let Some(last) = points.last_mut()
        {
            *last = first;
        }
        points
    };
    let deviations = |nodes: &[f64]| -> Option<Vec<f64>> {
        let fitted = interpolated(&points_at(nodes))?;
        let domain = fitted.domain();
        let curve = Curve2::from(fitted);
        Some(
            nodes
                .windows(2)
                .map(|pair| {
                    let middle = pair.iter().sum::<f64>() * 0.5;
                    let exact = bend.offset_point(middle, by);
                    curve
                        .point(curve.closest_parameter(exact, domain))
                        .distance(exact)
                })
                .collect(),
        )
    };
    let nodes = graded(
        (from.fraction, to.fraction),
        &[0.0, 1.0],
        FIRST_FIT_SPANS,
        deviations,
        tolerance,
    );
    thinned(points_at(&nodes), tolerance)
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
            .map(|point| {
                let distance = (point - origin).dot(direction) / bend.reach();
                Cut {
                    fraction: if at_end {
                        end_fraction + distance
                    } else {
                        end_fraction - distance
                    },
                    point,
                }
            }),
    );
    candidates
        .into_iter()
        .filter(|cut| cut.point.is_finite())
        .min_by(|a, b| a.point.distance(aim).total_cmp(&b.point.distance(aim)))
}

fn segments_cross(first: (Point2, Point2), second: (Point2, Point2)) -> Option<(f64, f64)> {
    let along = first.1 - first.0;
    let across = second.1 - second.0;
    let denominator = along.perp_dot(across);
    if denominator == 0.0 {
        return None;
    }
    let gap = second.0 - first.0;
    let on_first = gap.perp_dot(across) / denominator;
    let on_second = gap.perp_dot(along) / denominator;
    ((0.0..=1.0).contains(&on_first) && (0.0..=1.0).contains(&on_second))
        .then_some((on_first, on_second))
}

fn slope(bend: &Bend, fraction: f64, by: f64) -> Vector2 {
    let low = (fraction - DIFFERENCE_STEP).max(0.0);
    let high = (fraction + DIFFERENCE_STEP).min(1.0);
    (bend.offset_point(high, by) - bend.offset_point(low, by)) / (high - low)
}

fn crossing_refined(
    first: (&Bend, f64),
    second: (&Bend, f64),
    guess: (f64, f64),
) -> Option<(f64, f64)> {
    let (mut on_first, mut on_second) = guess;
    let scale = first.0.reach().max(second.0.reach());
    for _ in 0..NEWTON_STEPS {
        let gap =
            first.0.offset_point(on_first, first.1) - second.0.offset_point(on_second, second.1);
        if gap.length() <= CROSSING_CONVERGENCE * scale {
            return Some((on_first, on_second));
        }
        let along = slope(first.0, on_first, first.1);
        let across = -slope(second.0, on_second, second.1);
        let determinant = along.perp_dot(across);
        if determinant == 0.0 {
            return None;
        }
        let rhs = -gap;
        on_first = (on_first + rhs.perp_dot(across) / determinant).clamp(0.0, 1.0);
        on_second = (on_second + along.perp_dot(rhs) / determinant).clamp(0.0, 1.0);
    }
    None
}

fn bent_crossing(before: (&Bend, f64), after: (&Bend, f64), aim: Point2) -> Option<(Cut, Cut)> {
    let points = |bend: &Bend, by: f64| -> Vec<Point2> {
        (0..=SCAN_STEPS)
            .map(|step| bend.offset_point(step as f64 / SCAN_STEPS as f64, by))
            .collect()
    };
    let (first, second) = (points(before.0, before.1), points(after.0, after.1));
    let fraction = |step: usize, share: f64| (step as f64 + share) / SCAN_STEPS as f64;
    let mut found: Vec<(Cut, Cut)> = Vec::new();
    for (index, pair) in first.windows(2).enumerate() {
        let &[a, b] = pair else { continue };
        for (other, pair) in second.windows(2).enumerate() {
            let &[c, d] = pair else { continue };
            let Some((on_first, on_second)) = segments_cross((a, b), (c, d)) else {
                continue;
            };
            let guess = (fraction(index, on_first), fraction(other, on_second));
            let Some((on_first, on_second)) = crossing_refined(before, after, guess) else {
                continue;
            };
            let point = before
                .0
                .offset_point(on_first, before.1)
                .midpoint(after.0.offset_point(on_second, after.1));
            found.push((
                Cut {
                    fraction: on_first,
                    point,
                },
                Cut {
                    fraction: on_second,
                    point,
                },
            ));
        }
    }
    let origin_before = before.0.offset_point(1.0, before.1);
    let origin_after = after.0.offset_point(0.0, after.1);
    let ahead = before.0.tangent(1.0);
    let behind = -after.0.tangent(0.0);
    let denominator = ahead.perp_dot(behind);
    if denominator != 0.0 {
        let gap = origin_after - origin_before;
        let reach_before = gap.perp_dot(behind) / denominator;
        let reach_after = gap.perp_dot(ahead) / denominator;
        let scale = before.0.reach().max(after.0.reach());
        if reach_before >= -RAY_SLACK * scale && reach_after >= -RAY_SLACK * scale {
            let point = origin_before + ahead * reach_before;
            found.push((
                Cut {
                    fraction: 1.0 + reach_before.max(0.0) / before.0.reach(),
                    point,
                },
                Cut {
                    fraction: -reach_after.max(0.0) / after.0.reach(),
                    point,
                },
            ));
        }
    }
    found
        .into_iter()
        .filter(|(cut, _)| cut.point.is_finite())
        .min_by(|(a, _), (b, _)| a.point.distance(aim).total_cmp(&b.point.distance(aim)))
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
                    reach: curve
                        .bounding_box(range)
                        .size()
                        .length()
                        .max(LINEAR_RESOLUTION),
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
            Self::Trimmed(_) => false,
        }
    }

    pub fn start(&self) -> Point2 {
        match self {
            Self::Plain(strand) => strand.start(),
            Self::Bent { bend, by } => bend.offset_point(0.0, *by),
            Self::Trimmed(trim) => trim.from.point,
        }
    }

    pub fn end(&self) -> Point2 {
        match self {
            Self::Plain(strand) => strand.end(),
            Self::Bent { bend, by } => bend.offset_point(1.0, *by),
            Self::Trimmed(trim) => trim.to.point,
        }
    }

    pub fn start_tangent(&self) -> Vector2 {
        match self {
            Self::Plain(strand) => strand.start_tangent(),
            Self::Bent { bend, .. } => bend.tangent(0.0),
            Self::Trimmed(trim) => trim.bend.tangent(trim.from.fraction),
        }
    }

    pub fn end_tangent(&self) -> Vector2 {
        match self {
            Self::Plain(strand) => strand.end_tangent(),
            Self::Bent { bend, .. } => bend.tangent(1.0),
            Self::Trimmed(trim) => trim.bend.tangent(trim.to.fraction),
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
            Self::Trimmed(trim) => Self::Trimmed(Trim {
                bend: Bend {
                    reversed: !trim.bend.reversed,
                    ..trim.bend.clone()
                },
                by: -trim.by,
                from: Cut {
                    fraction: 1.0 - trim.to.fraction,
                    point: trim.to.point,
                },
                to: Cut {
                    fraction: 1.0 - trim.from.fraction,
                    point: trim.from.point,
                },
                closed: trim.closed,
            }),
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
            Self::Trimmed(_) => None,
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
            Self::Trimmed(trim) => {
                outline(&trim.bend, trim.by, &trim.from, &trim.to, OUTLINE_STEPS)
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

    pub fn curve(&self) -> Result<(Curve2, Interval, bool), GeometryError> {
        match self {
            Self::Plain(strand) => strand.curve(),
            Self::Bent { bend, by } if *by == 0.0 => {
                Ok((bend.curve.clone(), bend.range, bend.reversed))
            }
            Self::Bent { bend, by } => {
                let (from, to) = whole(bend, *by);
                spline_curve(&fit(bend, *by, &from, &to, bend.is_full_turn()))
            }
            Self::Trimmed(trim) => {
                spline_curve(&fit(&trim.bend, trim.by, &trim.from, &trim.to, trim.closed))
            }
        }
    }

    pub fn overrun(&self) -> [f64; 2] {
        match self {
            Self::Trimmed(trim) => [
                (-trim.from.fraction).max(0.0),
                (trim.to.fraction - 1.0).max(0.0),
            ],
            Self::Plain(_) | Self::Bent { .. } => [0.0; 2],
        }
    }

    pub fn is_line(&self) -> bool {
        matches!(self, Self::Plain(strand) if strand.is_line())
    }

    pub fn middle(&self) -> (Point2, Vector2) {
        match self {
            Self::Plain(strand) => strand.middle(),
            Self::Bent { bend, by } => (bend.offset_point(0.5, *by), bend.tangent(0.5)),
            Self::Trimmed(trim) => {
                let fraction = trim.middle_fraction();
                (
                    trim.bend.offset_point(fraction, trim.by),
                    trim.bend.tangent(fraction),
                )
            }
        }
    }
}

fn spline_curve(points: &[Point2]) -> Result<(Curve2, Interval, bool), GeometryError> {
    let fitted = interpolated(points).ok_or(GeometryError::TooFewControlPoints {
        degree: 1,
        points: points.len(),
    })?;
    let range = fitted.domain();
    Ok((fitted.into(), range, false))
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
                Course::Trimmed(Trim::between(&bend, by, from, to))
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
        (
            Course::Bent { bend, by },
            Course::Bent {
                bend: next,
                by: next_by,
            },
        ) => {
            let (leaving, entering) =
                bent_crossing((bend, *by), (next, *next_by), aim).ok_or(apart)?;
            (leaving.point, Some(leaving), Some(entering))
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
    aims: Option<&[Course]>,
    tolerance: f64,
) -> Result<Vec<Course>, OffsetFailure> {
    let strands: Vec<Strand> = courses
        .iter()
        .filter_map(|course| match course {
            Course::Plain(strand) => Some(*strand),
            Course::Bent { .. } | Course::Trimmed(_) => None,
        })
        .collect();
    if strands.len() == courses.len() {
        let aim_strands: Option<Vec<Strand>> = aims.and_then(|aims| {
            aims.iter()
                .map(|course| match course {
                    Course::Plain(strand) => Some(*strand),
                    Course::Bent { .. } | Course::Trimmed(_) => None,
                })
                .collect()
        });
        return Ok(
            offset_strands(&strands, closed, by, aim_strands.as_deref(), tolerance)?
                .into_iter()
                .map(Course::Plain)
                .collect(),
        );
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
                Course::Bent { bend, by } if by != 0.0 => Course::Trimmed(Trim::whole(&bend, by)),
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
        let aim = aims
            .and_then(|aims| aims.get(joint))
            .map_or(neutral, Course::start);
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
                aim,
                &mut trimmed,
                &mut cuts,
            )?;
        }
    }
    finished(trimmed, &cuts, courses, tolerance)
}
