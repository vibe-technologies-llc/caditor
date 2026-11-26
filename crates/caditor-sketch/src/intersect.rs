use std::f64::consts::TAU;

use caditor_geometry::{Point2, Vector2, larger};

use crate::{
    curve::{ArcGeometry, BSpline, EllipseGeometry, direction_angle},
    solve::tally,
};

const PARALLEL_TOLERANCE: f64 = 1e-12;
const SPLINE_SAMPLES_PER_POINT: usize = 16;
const MIN_SPLINE_SAMPLES: usize = 256;
const BISECTION_STEPS: usize = 100;
const NEWTON_STEPS: usize = 30;
const NEWTON_SETTLED: f64 = 1e-6;
const CLOSEST_REFINEMENTS: usize = 16;
const TOUCH_REFINEMENTS: usize = 32;
const ELLIPSE_SAMPLES: usize = 256;
const MIN_OVERLAP_INTERVALS: usize = 2;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Shape {
    Segment { start: Point2, end: Point2 },
    Circle { center: Point2, radius: f64 },
    Arc(ArcGeometry),
    Spline(BSpline),
    Ellipse(EllipseGeometry),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Carrier {
    Line { through: Point2, direction: Vector2 },
    Circle { center: Point2, radius: f64 },
}

impl Carrier {
    pub(crate) fn closest(&self, to: Point2) -> Point2 {
        match *self {
            Self::Line { through, direction } => {
                through + direction * (to - through).dot(direction)
            }
            Self::Circle { center, radius } => closest_on_circle(center, radius, to),
        }
    }
}

impl Shape {
    pub(crate) fn ends(&self) -> Option<(Point2, Point2)> {
        match self {
            Self::Segment { start, end } => Some((*start, *end)),
            Self::Circle { .. } => None,
            Self::Arc(arc) => Some((arc.point_at(arc.start_angle), arc.point_at(arc.end_angle()))),
            Self::Spline(spline) => Some((spline.point_at(0.0), spline.point_at(1.0))),
            Self::Ellipse(ellipse) if ellipse.is_full() => None,
            Self::Ellipse(ellipse) => Some((
                ellipse.point_at(ellipse.start),
                ellipse.point_at(ellipse.end()),
            )),
        }
    }

    pub(crate) fn extent(&self) -> f64 {
        let reach = |point: Point2| point.x.abs().max(point.y.abs());
        match self {
            Self::Segment { start, end } => reach(*start).max(reach(*end)),
            Self::Circle { center, radius } => reach(*center) + radius,
            Self::Arc(arc) => reach(arc.center) + arc.radius,
            Self::Spline(spline) => spline
                .control_points()
                .iter()
                .map(|point| reach(*point))
                .fold(0.0, larger),
            Self::Ellipse(ellipse) => {
                reach(ellipse.center) + ellipse.major_radius().max(ellipse.minor_radius)
            }
        }
    }

    pub(crate) fn closest(&self, to: Point2) -> Point2 {
        match self {
            Self::Segment { start, end } => closest_on_segment(*start, *end, to),
            Self::Circle { center, radius } => closest_on_circle(*center, *radius, to),
            Self::Arc(arc) => {
                let on_circle = closest_on_circle(arc.center, arc.radius, to);

                if on_arc(arc, on_circle, 0.0) {
                    return on_circle;
                }

                let (start, end) = (arc.point_at(arc.start_angle), arc.point_at(arc.end_angle()));
                if start.distance(to) <= end.distance(to) {
                    start
                } else {
                    end
                }
            }
            Self::Spline(spline) => closest_on_spline(spline, to),
            Self::Ellipse(ellipse) => ellipse.closest_point(to),
        }
    }
}

pub(crate) fn crossings(carrier: Carrier, other: &Shape, tolerance: f64) -> Vec<Point2> {
    match (carrier, other) {
        (Carrier::Line { through, direction }, Shape::Segment { start, end }) => {
            line_segment(through, direction, *start, *end, tolerance)
                .into_iter()
                .collect()
        }
        (Carrier::Line { through, direction }, Shape::Circle { center, radius }) => {
            line_circle(through, direction, *center, *radius, tolerance)
        }
        (Carrier::Line { through, direction }, Shape::Arc(arc)) => {
            line_circle(through, direction, arc.center, arc.radius, tolerance)
                .into_iter()
                .filter(|point| on_arc(arc, *point, tolerance))
                .collect()
        }
        (Carrier::Line { through, direction }, Shape::Spline(spline)) => {
            let Some(across) = direction.try_normalize() else {
                return Vec::new();
            };
            spline_roots(spline, |point| across.perp_dot(point - through), tolerance)
        }
        (Carrier::Circle { center, radius }, Shape::Segment { start, end }) => {
            let edge = *end - *start;
            let Some(along) = edge.try_normalize() else {
                return Vec::new();
            };
            let length = edge.length();
            line_circle(*start, along, center, radius, tolerance)
                .into_iter()
                .filter(|point| {
                    let reach = (*point - *start).dot(along);
                    reach >= -tolerance && reach <= length + tolerance
                })
                .collect()
        }
        (
            Carrier::Circle { center, radius },
            Shape::Circle {
                center: other,
                radius: other_radius,
            },
        ) => circle_circle(center, radius, *other, *other_radius, tolerance),
        (Carrier::Circle { center, radius }, Shape::Arc(arc)) => {
            circle_circle(center, radius, arc.center, arc.radius, tolerance)
                .into_iter()
                .filter(|point| on_arc(arc, *point, tolerance))
                .collect()
        }
        (Carrier::Circle { center, radius }, Shape::Spline(spline)) => {
            spline_roots(spline, |point| point.distance(center) - radius, tolerance)
        }
        (Carrier::Line { through, direction }, Shape::Ellipse(ellipse)) => {
            let Some(across) = direction.try_normalize() else {
                return Vec::new();
            };
            ellipse_roots(ellipse, |point| across.perp_dot(point - through), tolerance)
        }
        (Carrier::Circle { center, radius }, Shape::Ellipse(ellipse)) => {
            ellipse_roots(ellipse, |point| point.distance(center) - radius, tolerance)
        }
    }
}

pub(crate) fn ellipse_level(ellipse: &EllipseGeometry, point: Point2) -> f64 {
    let axis = ellipse.axis();
    let offset = point - ellipse.center;

    let along = offset.dot(axis) / ellipse.major_radius().max(f64::MIN_POSITIVE);
    let across = offset.dot(axis.perp()) / ellipse.minor_radius.abs().max(f64::MIN_POSITIVE);
    (along.hypot(across) - 1.0) * ellipse.minor_radius.min(ellipse.major_radius())
}

pub(crate) fn on_ellipse_sweep(ellipse: &EllipseGeometry, point: Point2, tolerance: f64) -> bool {
    if ellipse.is_full() {
        return true;
    }

    let reach = ellipse.major_radius().max(ellipse.minor_radius);
    let slack = if reach > 0.0 { tolerance / reach } else { 0.0 };
    let offset = (ellipse.parameter_of(point) - ellipse.start).rem_euclid(TAU);
    offset <= ellipse.sweep + slack || offset >= TAU - slack
}

pub(crate) fn ellipse_ellipse(
    along: &EllipseGeometry,
    other: &EllipseGeometry,
    tolerance: f64,
) -> Vec<Point2> {
    ellipse_roots(along, |point| ellipse_level(other, point), tolerance)
}

fn ellipse_roots(
    ellipse: &EllipseGeometry,
    signed: impl Fn(Point2) -> f64,
    tolerance: f64,
) -> Vec<Point2> {
    let point_at = |parameter: f64| ellipse.point_at(parameter);
    let found = roots_along(
        &point_at,
        (ellipse.start, ellipse.end()),
        ELLIPSE_SAMPLES,
        signed,
        tolerance,
    );
    let mut kept: Vec<Point2> = Vec::with_capacity(found.roots.len());

    for point in found.roots.into_iter().map(point_at) {
        if kept.iter().all(|known| known.distance(point) > tolerance) {
            kept.push(point);
        }
    }
    kept
}

pub(crate) fn line_crossings(
    carrier: Carrier,
    through: Point2,
    direction: Vector2,
    tolerance: f64,
) -> Vec<Point2> {
    match carrier {
        Carrier::Line {
            through: own,
            direction: own_direction,
        } => {
            let denominator = own_direction.perp_dot(direction);

            if denominator.abs() <= PARALLEL_TOLERANCE {
                return Vec::new();
            }

            let along = (through - own).perp_dot(direction) / denominator;
            vec![own + own_direction * along]
        }
        Carrier::Circle { center, radius } => {
            line_circle(through, direction, center, radius, tolerance)
        }
    }
}

pub(crate) fn on_arc(arc: &ArcGeometry, point: Point2, tolerance: f64) -> bool {
    let offset = (direction_angle(point - arc.center) - arc.start_angle).rem_euclid(TAU);
    let slack = if arc.radius > 0.0 {
        tolerance / arc.radius
    } else {
        0.0
    };
    offset <= arc.sweep + slack || offset >= TAU - slack
}

fn line_segment(
    through: Point2,
    direction: Vector2,
    start: Point2,
    end: Point2,
    tolerance: f64,
) -> Option<Point2> {
    let edge = end - start;
    let length = edge.length();
    let denominator = direction.perp_dot(edge);

    if length <= 0.0 || denominator.abs() <= PARALLEL_TOLERANCE * length {
        return None;
    }

    let along = direction.perp_dot(through - start) / denominator;
    let slack = tolerance / length;
    (along >= -slack && along <= 1.0 + slack).then(|| start + edge * along.clamp(0.0, 1.0))
}

fn line_circle(
    through: Point2,
    direction: Vector2,
    center: Point2,
    radius: f64,
    tolerance: f64,
) -> Vec<Point2> {
    let foot = through + direction * (center - through).dot(direction);
    let height = foot.distance(center);

    if height > radius + tolerance {
        return Vec::new();
    }
    if height >= radius - tolerance {
        return vec![foot];
    }

    let half = (radius * radius - height * height).max(0.0).sqrt();
    vec![foot - direction * half, foot + direction * half]
}

fn circle_circle(
    center: Point2,
    radius: f64,
    other: Point2,
    other_radius: f64,
    tolerance: f64,
) -> Vec<Point2> {
    let between = other - center;
    let distance = between.length();

    if distance <= tolerance
        || distance > radius + other_radius + tolerance
        || distance < (radius - other_radius).abs() - tolerance
    {
        return Vec::new();
    }

    let toward = between / distance;
    let along =
        (distance * distance + radius * radius - other_radius * other_radius) / (2.0 * distance);

    let base = center + toward * along;
    let height = (radius * radius - along * along).max(0.0).sqrt();

    if height <= tolerance {
        return vec![base];
    }

    let across = toward.perp() * height;
    vec![base + across, base - across]
}

pub(crate) fn spline_touching(
    spline: &BSpline,
    closest_on_other: impl Fn(Point2) -> Point2,
    tolerance: f64,
) -> Option<Point2> {
    let samples = spline_samples(spline);
    let gap_from = |own: Point2| {
        let other = closest_on_other(own);
        (own, other, own.distance(other))
    };

    let (mut own, mut other, rough) = (0..=samples)
        .map(|index| gap_from(spline.point_at(index as f64 / samples as f64)))
        .min_by(|a, b| a.2.total_cmp(&b.2))?;
    let mut gap = rough;

    for _ in 0..TOUCH_REFINEMENTS {
        if gap <= tolerance * NEWTON_SETTLED {
            break;
        }

        let (closer, beside, nearer) = gap_from(closest_on_spline(spline, other));

        if nearer >= gap {
            break;
        }
        (own, other, gap) = (closer, beside, nearer);
    }
    (gap <= tolerance).then(|| own.midpoint(other))
}

fn closest_on_spline(spline: &BSpline, to: Point2) -> Point2 {
    spline.point_at(closest_spline_parameter(spline, to))
}

pub(crate) fn closest_spline_parameter(spline: &BSpline, to: Point2) -> f64 {
    let samples = spline_samples(spline);
    let step = 1.0 / samples as f64;

    tally::add(samples);

    let rough = (0..samples)
        .map(|index| {
            let (from, until) = (index as f64 * step, (index + 1) as f64 * step);

            let (start, end) = (spline.point_at(from), spline.point_at(until));

            let along = end - start;
            let fraction = if along.length_squared() > 0.0 {
                ((to - start).dot(along) / along.length_squared()).clamp(0.0, 1.0)
            } else {
                0.0
            };
            from + fraction * step
        })
        .min_by(|a, b| {
            spline
                .point_at(*a)
                .distance(to)
                .total_cmp(&spline.point_at(*b).distance(to))
        })
        .unwrap_or(0.0);
    settled_closest(spline, to, rough)
}

fn settled_closest(spline: &BSpline, to: Point2, rough: f64) -> f64 {
    let mut parameter = rough;

    for _ in 0..CLOSEST_REFINEMENTS {
        let [tangent, bend] = spline.derivatives(parameter);
        let offset = spline.point_at(parameter) - to;

        let slope = tangent.dot(tangent) + offset.dot(bend);

        if !(slope > 0.0 && slope.is_finite()) {
            break;
        }

        let next = (parameter - offset.dot(tangent) / slope).clamp(0.0, 1.0);

        if (next - parameter).abs() <= f64::EPSILON {
            break;
        }
        parameter = next;
    }

    let (refined, first) = (spline.point_at(parameter), spline.point_at(rough));
    if refined.distance(to) <= first.distance(to) {
        parameter
    } else {
        rough
    }
}

fn spline_samples(spline: &BSpline) -> usize {
    (spline.control_points().len() * SPLINE_SAMPLES_PER_POINT).max(MIN_SPLINE_SAMPLES)
}

pub(crate) struct SplineMeeting {
    pub(crate) points: Vec<Point2>,
    overlaps: Vec<Overlap>,
}

impl SplineMeeting {
    pub(crate) fn bounded_by(
        &self,
        spline: &BSpline,
        ends: [Point2; 2],
        within: impl Fn(Point2) -> bool,
        tolerance: f64,
    ) -> Vec<Point2> {
        let ends_on_overlaps = ends
            .into_iter()
            .filter(|end| self.overlaps_at(spline, *end, tolerance));
        self.points
            .iter()
            .copied()
            .filter(|point| within(*point))
            .chain(ends_on_overlaps)
            .collect()
    }

    fn overlaps_at(&self, spline: &BSpline, point: Point2, tolerance: f64) -> bool {
        let parameter = closest_spline_parameter(spline, point);
        spline.point_at(parameter).distance(point) <= tolerance
            && self
                .overlaps
                .iter()
                .any(|overlap| (overlap.from..=overlap.to).contains(&parameter))
    }
}

pub(crate) fn spline_meeting(
    spline: &BSpline,
    signed: impl Fn(Point2) -> f64,
    tolerance: f64,
) -> SplineMeeting {
    let point_at = |parameter: f64| spline.point_at(parameter);
    let found = roots_along(
        &point_at,
        (0.0, 1.0),
        spline_samples(spline),
        signed,
        tolerance,
    );
    SplineMeeting {
        points: found.roots.into_iter().map(point_at).collect(),
        overlaps: found.overlaps,
    }
}

pub(crate) fn spline_roots(
    spline: &BSpline,
    signed: impl Fn(Point2) -> f64,
    tolerance: f64,
) -> Vec<Point2> {
    spline_meeting(spline, signed, tolerance).points
}

struct Meeting {
    roots: Vec<f64>,
    overlaps: Vec<Overlap>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Overlap {
    from: f64,
    to: f64,
    bracket_from: f64,
    bracket_to: f64,
}

impl Overlap {
    fn swallows(&self, parameter: f64) -> bool {
        (self.bracket_from..=self.bracket_to).contains(&parameter)
    }

    fn parted_ends(&self) -> impl Iterator<Item = f64> {
        let parts_before = self.bracket_from < self.from;
        let parts_after = self.to < self.bracket_to;
        [(parts_before, self.from), (parts_after, self.to)]
            .into_iter()
            .filter_map(|(parts, parameter)| parts.then_some(parameter))
    }
}

fn roots_along(
    point_at: &impl Fn(f64) -> Point2,
    (first, last): (f64, f64),
    samples: usize,
    signed: impl Fn(Point2) -> f64,
    tolerance: f64,
) -> Meeting {
    let value = |parameter: f64| signed(point_at(parameter));

    tally::add(samples);

    let sampled: Vec<(f64, f64)> = (0..=samples)
        .map(|index| {
            let parameter = first + (last - first) * index as f64 / samples as f64;
            let at = value(parameter);
            let touching_an_end = (index == 0 || index == samples) && at.abs() <= tolerance;
            (parameter, if touching_an_end { 0.0 } else { at })
        })
        .collect();

    let overlaps = overlaps_in(&sampled, &value, tolerance);
    let mut roots = Vec::new();

    for pair in sampled.windows(2) {
        let &[(low, at_low), (high, at_high)] = pair else {
            continue;
        };
        if at_low == 0.0 {
            roots.push(low);
        } else if at_low.signum() != at_high.signum() && at_high != 0.0 {
            roots.push(bisect(&value, low, high, at_low));
        }
    }
    if sampled.last().is_some_and(|(_, at_last)| *at_last == 0.0) {
        roots.push(last);
    }
    for triple in sampled.windows(3) {
        let &[(before, at_before), (_, at_middle), (after, at_after)] = triple else {
            continue;
        };
        let sense = at_middle.signum();

        let dips = at_middle != 0.0
            && at_before.signum() == sense
            && at_after.signum() == sense
            && at_middle.abs() <= at_before.abs()
            && at_middle.abs() < at_after.abs();
        if dips {
            roots.extend(roots_in_dip(&value, (before, after), sense, tolerance));
        }
    }
    roots.retain(|root| !overlaps.iter().any(|overlap| overlap.swallows(*root)));
    roots.extend(
        overlaps
            .iter()
            .flat_map(|overlap| [overlap.from, overlap.to]),
    );
    roots.sort_by(f64::total_cmp);
    Meeting { roots, overlaps }
}

fn overlaps_in(
    sampled: &[(f64, f64)],
    value: &impl Fn(f64) -> f64,
    tolerance: f64,
) -> Vec<Overlap> {
    let near = |at: f64| at.abs() <= tolerance;
    let coincident: Vec<bool> = sampled
        .windows(2)
        .map(|pair| match pair {
            &[(low, at_low), (high, at_high)] => {
                near(at_low) && near(at_high) && near(value((low + high) / 2.0))
            }
            _ => false,
        })
        .collect();
    let mut overlaps = Vec::new();
    let mut run_start = None;

    for (index, lies_on) in coincident.iter().copied().chain([false]).enumerate() {
        match (lies_on, run_start) {
            (true, None) => run_start = Some(index),
            (false, Some(start)) => {
                run_start = None;
                if index - start >= MIN_OVERLAP_INTERVALS {
                    overlaps.extend(overlap_between(sampled, value, tolerance, start, index));
                }
            }
            _ => {}
        }
    }
    overlaps
}

fn overlap_between(
    sampled: &[(f64, f64)],
    value: &impl Fn(f64) -> f64,
    tolerance: f64,
    first_sample: usize,
    last_sample: usize,
) -> Option<Overlap> {
    let (inside_first, _) = *sampled.get(first_sample)?;
    let (inside_last, _) = *sampled.get(last_sample)?;
    let before = first_sample
        .checked_sub(1)
        .and_then(|index| sampled.get(index));
    let after = sampled.get(last_sample + 1);

    let from = before.map_or(inside_first, |&(outside, _)| {
        overlap_edge(value, inside_first, outside, tolerance)
    });
    let to = after.map_or(inside_last, |&(outside, _)| {
        overlap_edge(value, inside_last, outside, tolerance)
    });
    Some(Overlap {
        from,
        to,
        bracket_from: before.map_or(inside_first, |&(parameter, _)| parameter),
        bracket_to: after.map_or(inside_last, |&(parameter, _)| parameter),
    })
}

fn overlap_edge(value: &impl Fn(f64) -> f64, inside: f64, outside: f64, tolerance: f64) -> f64 {
    let (mut inside, mut outside) = (inside, outside);

    for _ in 0..BISECTION_STEPS {
        let middle = (inside + outside) / 2.0;

        if middle == inside || middle == outside {
            break;
        }
        if value(middle).abs() <= tolerance {
            inside = middle;
        } else {
            outside = middle;
        }
    }
    inside
}

fn roots_in_dip(
    value: &impl Fn(f64) -> f64,
    (before, after): (f64, f64),
    sense: f64,
    tolerance: f64,
) -> Vec<f64> {
    let deepest = deepest_in(&|parameter| sense * value(parameter), before, after);
    let at_deepest = sense * value(deepest);
    if at_deepest < 0.0 {
        vec![
            bisect(value, before, deepest, sense),
            bisect(value, deepest, after, -sense),
        ]
    } else if at_deepest <= tolerance {
        vec![deepest]
    } else {
        Vec::new()
    }
}

fn deepest_in(value: &impl Fn(f64) -> f64, low: f64, high: f64) -> f64 {
    let ratio = (5f64.sqrt() - 1.0) / 2.0;
    let (mut low, mut high) = (low, high);

    let mut inner = high - ratio * (high - low);
    let mut outer = low + ratio * (high - low);

    let (mut at_inner, mut at_outer) = (value(inner), value(outer));

    for _ in 0..BISECTION_STEPS {
        if at_inner <= 0.0 {
            return inner;
        }
        if at_outer <= 0.0 {
            return outer;
        }
        if at_inner < at_outer {
            high = outer;
            outer = inner;
            at_outer = at_inner;
            inner = high - ratio * (high - low);
            at_inner = value(inner);
        } else {
            low = inner;
            inner = outer;
            at_inner = at_outer;
            outer = low + ratio * (high - low);
            at_outer = value(outer);
        }
    }
    if at_inner < at_outer { inner } else { outer }
}

fn sampled_spline(spline: &BSpline) -> Vec<(f64, Point2)> {
    let samples = spline_samples(spline);

    tally::add(samples);
    (0..=samples)
        .map(|index| {
            let parameter = index as f64 / samples as f64;
            (parameter, spline.point_at(parameter))
        })
        .collect()
}

#[derive(Clone, Copy)]
struct Foot {
    parameter: f64,
    squared: f64,
}

struct ChordMeetings {
    starts: Vec<(f64, f64)>,
    seeds_on_second: Vec<Option<f64>>,
    seeds_on_first: Vec<Option<f64>>,
}

fn nearest_feet(feet: Vec<Option<Foot>>) -> Vec<Option<f64>> {
    feet.into_iter()
        .map(|foot| foot.map(|foot| foot.parameter))
        .collect()
}

fn keep_nearer(feet: &mut [Option<Foot>], sample: usize, foot: Foot) {
    if let Some(nearest) = feet.get_mut(sample)
        && nearest.is_none_or(|known| foot.squared < known.squared)
    {
        *nearest = Some(foot);
    }
}

fn foot_on_chord(
    ((from, start), (until, end)): ((f64, Point2), (f64, Point2)),
    to: Point2,
) -> Foot {
    let along = end - start;
    let fraction = if along.length_squared() > 0.0 {
        ((to - start).dot(along) / along.length_squared()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    Foot {
        parameter: from + (until - from) * fraction,
        squared: (start + along * fraction).distance_squared(to),
    }
}

fn chord_meetings(
    along_first: &[(f64, Point2)],
    along_second: &[(f64, Point2)],
    tolerance: f64,
) -> ChordMeetings {
    let mut starts = Vec::new();
    let mut feet_on_second: Vec<Option<Foot>> = vec![None; along_first.len()];
    let mut feet_on_first: Vec<Option<Foot>> = vec![None; along_second.len()];

    tally::add(along_first.len() * along_second.len());

    for (index, first_pair) in along_first.windows(2).enumerate() {
        let &[(s0, a0), (s1, a1)] = first_pair else {
            continue;
        };

        let (low, high) = (a0.min(a1), a0.max(a1));
        for (other, second_pair) in along_second.windows(2).enumerate() {
            let &[(t0, b0), (t1, b1)] = second_pair else {
                continue;
            };

            let overlaps =
                b0.min(b1).cmple(high + tolerance).all() && b0.max(b1).cmpge(low - tolerance).all();

            if !overlaps {
                continue;
            }

            for (sample, point) in [(index, a0), (index + 1, a1)] {
                keep_nearer(
                    &mut feet_on_second,
                    sample,
                    foot_on_chord(((t0, b0), (t1, b1)), point),
                );
            }
            for (sample, point) in [(other, b0), (other + 1, b1)] {
                keep_nearer(
                    &mut feet_on_first,
                    sample,
                    foot_on_chord(((s0, a0), (s1, a1)), point),
                );
            }

            let Some((along, across)) = segment_fractions(a0, a1, b0, b1) else {
                continue;
            };
            let slack = 1e-9;

            if !(-slack..=1.0 + slack).contains(&along) || !(-slack..=1.0 + slack).contains(&across)
            {
                continue;
            }
            starts.push((
                (s0 + (s1 - s0) * along).clamp(0.0, 1.0),
                (t0 + (t1 - t0) * across).clamp(0.0, 1.0),
            ));
        }
    }
    ChordMeetings {
        starts,
        seeds_on_second: nearest_feet(feet_on_second),
        seeds_on_first: nearest_feet(feet_on_first),
    }
}

fn spline_overlaps(
    (first, along_first): (&BSpline, &[(f64, Point2)]),
    second: &BSpline,
    seeds: &[Option<f64>],
    tolerance: f64,
) -> Vec<Overlap> {
    let gap = |parameter: f64, seed: Option<f64>| {
        seed.map_or(f64::INFINITY, |seed| {
            let point = first.point_at(parameter);

            tally::add(CLOSEST_REFINEMENTS);
            second
                .point_at(settled_closest(second, point, seed))
                .distance(point)
        })
    };
    let value = |parameter: f64| gap(parameter, seed_near(along_first, seeds, parameter));
    let sampled: Vec<(f64, f64)> = along_first
        .iter()
        .zip(seeds)
        .map(|(&(parameter, _), &seed)| (parameter, gap(parameter, seed)))
        .collect();

    overlaps_in(&sampled, &value, tolerance)
}

fn seed_near(along: &[(f64, Point2)], seeds: &[Option<f64>], parameter: f64) -> Option<f64> {
    let seed_of = |sample: usize| seeds.get(sample).copied().flatten();
    let below = along
        .partition_point(|(sampled, _)| *sampled <= parameter)
        .saturating_sub(1);
    seed_of(below).or_else(|| seed_of(below + 1))
}

pub(crate) fn spline_spline(first: &BSpline, second: &BSpline, tolerance: f64) -> Vec<Point2> {
    let (along_first, along_second) = (sampled_spline(first), sampled_spline(second));

    let meetings = chord_meetings(&along_first, &along_second, tolerance);
    let overlaps = spline_overlaps(
        (first, &along_first),
        second,
        &meetings.seeds_on_second,
        tolerance,
    );
    let cutter_overlaps = spline_overlaps(
        (second, &along_second),
        first,
        &meetings.seeds_on_first,
        tolerance,
    );

    let cutter_ends = [second.point_at(0.0), second.point_at(1.0)];
    let at_a_cutter_end = |point: Point2| {
        cutter_ends
            .into_iter()
            .find(|end| end.distance(point) <= tolerance)
            .unwrap_or(point)
    };
    let mut found: Vec<Point2> = overlaps
        .iter()
        .flat_map(Overlap::parted_ends)
        .map(|parameter| at_a_cutter_end(first.point_at(parameter)))
        .collect();

    let [cutter_start, cutter_end] = cutter_ends;

    let cutter_is_open = cutter_start.distance(cutter_end) > tolerance;
    let cutter_run_ends = cutter_overlaps.iter().flat_map(|overlap| {
        if cutter_is_open {
            vec![overlap.from, overlap.to]
        } else {
            overlap.parted_ends().collect()
        }
    });

    for parameter in cutter_run_ends {
        let point = second.point_at(parameter);
        let Some(seed) = seed_near(&along_second, &meetings.seeds_on_first, parameter) else {
            continue;
        };

        tally::add(CLOSEST_REFINEMENTS + found.len());

        let on_first = settled_closest(first, point, seed);
        let new = !overlaps.iter().any(|overlap| overlap.swallows(on_first))
            && found.iter().all(|known| known.distance(point) > tolerance);
        if new {
            found.push(point);
        }
    }

    for start in meetings.starts {
        let swallowed = overlaps.iter().any(|overlap| overlap.swallows(start.0))
            || cutter_overlaps
                .iter()
                .any(|overlap| overlap.swallows(start.1));

        if swallowed {
            continue;
        }

        let Some(point) = refined_crossing(first, second, start, tolerance) else {
            continue;
        };

        tally::add(found.len());
        if found.iter().all(|known| known.distance(point) > tolerance) {
            found.push(point);
        }
    }
    found
}

fn segment_fractions(a0: Point2, a1: Point2, b0: Point2, b1: Point2) -> Option<(f64, f64)> {
    let (first, second) = (a1 - a0, b1 - b0);

    let denominator = first.perp_dot(second);

    if denominator.abs() <= PARALLEL_TOLERANCE * first.length() * second.length() {
        return None;
    }

    let offset = b0 - a0;
    Some((
        offset.perp_dot(second) / denominator,
        offset.perp_dot(first) / denominator,
    ))
}

fn refined_crossing(
    first: &BSpline,
    second: &BSpline,
    (mut s, mut t): (f64, f64),
    tolerance: f64,
) -> Option<Point2> {
    for _ in 0..NEWTON_STEPS {
        let gap = first.point_at(s) - second.point_at(t);

        tally::add(1);
        if gap.length() <= tolerance * NEWTON_SETTLED {
            break;
        }

        let ([along_first, _], [along_second, _]) = (first.derivatives(s), second.derivatives(t));

        let determinant = along_first.perp_dot(-along_second);

        if determinant.abs() <= f64::EPSILON {
            return None;
        }

        let step_s = gap.perp_dot(-along_second) / determinant;
        let step_t = along_first.perp_dot(gap) / determinant;

        s = (s - step_s).clamp(0.0, 1.0);
        t = (t - step_t).clamp(0.0, 1.0);
    }

    let (on_first, on_second) = (first.point_at(s), second.point_at(t));
    (on_first.distance(on_second) <= tolerance).then(|| on_first.midpoint(on_second))
}

fn bisect(value: &impl Fn(f64) -> f64, low: f64, high: f64, at_low: f64) -> f64 {
    let (mut low, mut high, mut at_low) = (low, high, at_low);

    for _ in 0..BISECTION_STEPS {
        let middle = (low + high) / 2.0;
        let at_middle = value(middle);

        if at_middle == 0.0 {
            return middle;
        }
        if at_middle.signum() == at_low.signum() {
            low = middle;
            at_low = at_middle;
        } else {
            high = middle;
        }
    }
    (low + high) / 2.0
}

pub(crate) fn closest_on_segment(start: Point2, end: Point2, to: Point2) -> Point2 {
    let edge = end - start;
    let length_squared = edge.length_squared();

    if length_squared <= 0.0 {
        return start;
    }
    start + edge * ((to - start).dot(edge) / length_squared).clamp(0.0, 1.0)
}

fn closest_on_circle(center: Point2, radius: f64, to: Point2) -> Point2 {
    let direction = (to - center).try_normalize().unwrap_or(Vector2::X);
    center + direction * radius
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOLERANCE: f64 = 1e-9;

    fn horizontal_through(y: f64) -> Carrier {
        Carrier::Line {
            through: Point2::new(0.0, y),
            direction: Vector2::X,
        }
    }

    #[test]
    fn a_line_meets_a_segment_only_within_it() {
        let segment = Shape::Segment {
            start: Point2::new(5.0, -5.0),
            end: Point2::new(5.0, 5.0),
        };

        assert_eq!(
            crossings(horizontal_through(2.0), &segment, TOLERANCE),
            vec![Point2::new(5.0, 2.0)]
        );
        assert!(crossings(horizontal_through(7.0), &segment, TOLERANCE).is_empty());
        assert_eq!(
            crossings(horizontal_through(5.0), &segment, TOLERANCE),
            vec![Point2::new(5.0, 5.0)]
        );
    }

    #[test]
    fn a_line_crosses_a_circle_twice_and_touches_it_once() {
        let circle = Shape::Circle {
            center: Point2::ZERO,
            radius: 5.0,
        };
        let crossed = crossings(horizontal_through(3.0), &circle, TOLERANCE);

        assert_eq!(crossed.len(), 2);
        assert!(
            crossed
                .iter()
                .any(|point| point.distance(Point2::new(-4.0, 3.0)) < 1e-12)
        );
        assert!(
            crossed
                .iter()
                .any(|point| point.distance(Point2::new(4.0, 3.0)) < 1e-12)
        );
        assert_eq!(
            crossings(horizontal_through(5.0), &circle, TOLERANCE),
            vec![Point2::new(0.0, 5.0)]
        );
    }

    #[test]
    fn an_arc_keeps_only_the_crossings_within_its_sweep() {
        let arc = Shape::Arc(ArcGeometry::from_points(
            Point2::ZERO,
            Point2::new(5.0, 0.0),
            Point2::new(-5.0, 0.0),
        ));

        assert_eq!(crossings(horizontal_through(3.0), &arc, TOLERANCE).len(), 2);
        assert!(crossings(horizontal_through(-3.0), &arc, TOLERANCE).is_empty());
    }

    #[test]
    fn circles_cross_where_expected() {
        let carrier = Carrier::Circle {
            center: Point2::ZERO,
            radius: 5.0,
        };
        let other = Shape::Circle {
            center: Point2::new(8.0, 0.0),
            radius: 5.0,
        };

        let crossed = crossings(carrier, &other, TOLERANCE);

        assert_eq!(crossed.len(), 2);
        assert!(
            crossed
                .iter()
                .any(|point| point.distance(Point2::new(4.0, 3.0)) < 1e-12)
        );
        assert!(
            crossed
                .iter()
                .any(|point| point.distance(Point2::new(4.0, -3.0)) < 1e-12)
        );
    }

    fn lopsided_bump() -> (Shape, Point2) {
        let Some(spline) = BSpline::clamped(vec![
            Point2::new(0.0, 0.0),
            Point2::new(3.0, 6.0),
            Point2::new(17.0, 2.0),
            Point2::new(20.0, 0.0),
        ]) else {
            panic!("a spline");
        };
        let peak = (0..=1_000_000)
            .map(|index| spline.point_at(f64::from(index) / 1e6))
            .max_by(|a, b| a.y.total_cmp(&b.y))
            .unwrap();
        (Shape::Spline(spline), peak)
    }

    #[test]
    fn a_spline_grazing_a_line_between_samples_is_crossed_twice_or_touched() {
        let (bump, peak) = lopsided_bump();
        let below = peak.y - 1e-7;

        let crossed = crossings(horizontal_through(below), &bump, TOLERANCE);

        assert_eq!(crossed.len(), 2, "{crossed:?}");
        assert!(crossed[0].x < peak.x && crossed[1].x > peak.x);

        for point in &crossed {
            assert!((point.y - below).abs() < 1e-12);
        }

        let touched = crossings(horizontal_through(peak.y + 1e-10), &bump, 1e-9);

        assert_eq!(touched.len(), 1, "{touched:?}");
        assert!(touched[0].distance(peak) < 1e-3);

        assert!(crossings(horizontal_through(peak.y + 1e-3), &bump, TOLERANCE).is_empty());
    }

    #[test]
    fn two_splines_cross_where_both_pass() {
        let Some(rising) = BSpline::clamped(vec![
            Point2::new(0.0, 0.0),
            Point2::new(5.0, 2.0),
            Point2::new(10.0, 10.0),
        ]) else {
            panic!("a spline");
        };
        let Some(wave) = BSpline::clamped(vec![
            Point2::new(0.0, 6.0),
            Point2::new(4.0, 0.0),
            Point2::new(7.0, 8.0),
            Point2::new(10.0, 1.0),
        ]) else {
            panic!("a spline");
        };

        let crossed = spline_spline(&rising, &wave, TOLERANCE);

        assert!(!crossed.is_empty());

        for point in &crossed {
            let on = |spline: &BSpline| {
                (0..=20_000)
                    .map(|index| {
                        spline
                            .point_at(f64::from(index) / 20_000.0)
                            .distance(*point)
                    })
                    .fold(f64::INFINITY, f64::min)
            };

            assert!(on(&rising) < 1e-3 && on(&wave) < 1e-3, "{point}");
        }

        let apart =
            BSpline::clamped(vec![Point2::new(0.0, 20.0), Point2::new(10.0, 30.0)]).unwrap();

        assert!(spline_spline(&rising, &apart, TOLERANCE).is_empty());
    }

    #[test]
    fn a_spline_lying_along_a_line_meets_it_only_at_the_ends_of_the_overlap() {
        let Some(straight) = BSpline::clamped(vec![
            Point2::new(2.0, 1.0),
            Point2::new(5.0, 1.0),
            Point2::new(9.0, 1.0),
        ]) else {
            panic!("a spline");
        };
        let crossed = crossings(horizontal_through(1.0), &Shape::Spline(straight), TOLERANCE);

        assert_eq!(crossed, vec![Point2::new(2.0, 1.0), Point2::new(9.0, 1.0)]);
    }

    #[test]
    fn a_spline_crossing_a_line_is_found_to_full_precision() {
        let Some(spline) = BSpline::clamped(vec![
            Point2::new(0.0, -5.0),
            Point2::new(3.0, 0.0),
            Point2::new(6.0, 5.0),
        ]) else {
            panic!("a spline");
        };
        let crossed = crossings(horizontal_through(1.0), &Shape::Spline(spline), TOLERANCE);

        assert_eq!(crossed.len(), 1);
        assert!((crossed[0].y - 1.0).abs() < 1e-12);
    }
}
