use caditor_geometry::{Point2, Point3};

use crate::{
    coordinates::{Coordinates, angle_between},
    curve::Curve,
    curve2::Curve2,
    interrupt,
    intersect::{IntersectionError, inside_intervals},
    interval::Interval,
    parametric::Parametric,
    tolerance::LINEAR_RESOLUTION,
};

const TOLERANCE: f64 = LINEAR_RESOLUTION;
const ON_CURVE: f64 = 0.1 * LINEAR_RESOLUTION;
const TANGENT_SINE: f64 = 1e-7;
const OVERLAP_SINE: f64 = 1e-6;
const OVERLAP_CURVATURE: f64 = 1e-6;
const MIN_OVERLAP: f64 = 10.0 * LINEAR_RESOLUTION;
const LEAF_TURN: f64 = 0.3;
const TINY_LEAF: f64 = 1e2 * LINEAR_RESOLUTION;
const MAX_PAIRS: usize = 1 << 16;
const MAX_DEPTH: usize = 60;
const MAX_NEWTON: usize = 50;
const MAX_HALVINGS: usize = 24;
const CONNECTION_SAMPLES: [f64; 3] = [0.25, 0.5, 0.75];
const OVERLAP_SAMPLES: usize = 8;
const CLIP_SAMPLES: usize = 32;
const PARALLEL_CROSS: f64 = 1e-12;
const EXTENSION_BISECTIONS: usize = 60;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurveCurvePoint<P> {
    pub first: f64,
    pub second: f64,
    pub point: P,
    pub tangent: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurveCurveOverlap {
    pub first: Interval,
    pub second_start: f64,
    pub second_end: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CurveCurveIntersection<P> {
    pub points: Vec<CurveCurvePoint<P>>,
    pub overlaps: Vec<CurveCurveOverlap>,
}

impl<P> Default for CurveCurveIntersection<P> {
    fn default() -> Self {
        Self {
            points: Vec::new(),
            overlaps: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Candidate {
    Pair {
        first: f64,
        second: f64,
        touch: bool,
    },
    Along(Interval),
    End {
        first: f64,
        second: f64,
    },
}

impl Candidate {
    fn low(&self) -> f64 {
        match self {
            Self::Pair { first, .. } | Self::End { first, .. } => *first,
            Self::Along(range) => range.start(),
        }
    }

    fn high(&self) -> f64 {
        match self {
            Self::Pair { first, .. } | Self::End { first, .. } => *first,
            Self::Along(range) => range.end(),
        }
    }
}

pub(crate) trait Traceable: Parametric {
    fn range_bounds(&self, range: Interval) -> [Self::Point; 2];

    fn nearest(&self, point: Self::Point, range: Interval) -> f64;

    fn span_length(&self, range: Interval) -> f64;

    fn period_of(&self) -> Option<f64>;

    fn analytic(
        first: &Self,
        first_range: Interval,
        second: &Self,
        second_range: Interval,
    ) -> Option<Vec<Candidate>>;
}

impl Traceable for Curve {
    fn range_bounds(&self, range: Interval) -> [Point3; 2] {
        let bounds = self.piece_bounds(range);
        [bounds.min(), bounds.max()]
    }

    fn nearest(&self, point: Point3, range: Interval) -> f64 {
        self.closest_parameter(point, range)
    }

    fn span_length(&self, range: Interval) -> f64 {
        self.length(range)
    }

    fn period_of(&self) -> Option<f64> {
        self.period()
    }

    fn analytic(
        first: &Self,
        first_range: Interval,
        second: &Self,
        second_range: Interval,
    ) -> Option<Vec<Candidate>> {
        let (Curve::Line(a), Curve::Line(b)) = (first, second) else {
            return None;
        };
        let (da, db) = (a.direction(), b.direction());
        let offset = b.origin() - a.origin();
        let cross = da.cross(db);
        let denominator = cross.length_squared();
        if denominator <= PARALLEL_CROSS * PARALLEL_CROSS {
            let apart = offset - da * offset.dot(da);
            return Some(if apart.length() <= TOLERANCE {
                vec![Candidate::Along(first_range)]
            } else {
                Vec::new()
            });
        }
        let s = offset.cross(db).dot(cross) / denominator;
        let t = offset.cross(da).dot(cross) / denominator;
        let hit = first_range.contains(s)
            && second_range.contains(t)
            && a.point(s).distance(b.point(t)) <= TOLERANCE;
        Some(if hit {
            vec![Candidate::Pair {
                first: s,
                second: t,
                touch: false,
            }]
        } else {
            Vec::new()
        })
    }
}

fn pair_from_point(
    first: &Curve2,
    first_range: Interval,
    second: &Curve2,
    second_range: Interval,
    point: Point2,
    touch: bool,
) -> Option<Candidate> {
    let s = first.closest_parameter(point, first_range);
    let t = second.closest_parameter(point, second_range);
    let close =
        first.point(s).distance(point) <= TOLERANCE && second.point(t).distance(point) <= TOLERANCE;
    close.then_some(Candidate::Pair {
        first: s,
        second: t,
        touch,
    })
}

fn line_circle(
    origin: Point2,
    direction: Point2,
    center: Point2,
    radius: f64,
) -> Vec<(Point2, bool)> {
    let foot = (center - origin).dot(direction);
    let closest = origin + direction * foot;
    let apart = closest.distance(center);
    if (apart - radius).abs() <= TOLERANCE {
        vec![(
            center + (closest - center).normalize_or_zero() * radius,
            true,
        )]
    } else if apart < radius {
        let half = (radius * radius - apart * apart).sqrt();
        vec![
            (closest - direction * half, false),
            (closest + direction * half, false),
        ]
    } else {
        Vec::new()
    }
}

impl Traceable for Curve2 {
    fn range_bounds(&self, range: Interval) -> [Point2; 2] {
        let bounds = self.piece_bounds(range);
        [bounds.min(), bounds.max()]
    }

    fn nearest(&self, point: Point2, range: Interval) -> f64 {
        self.closest_parameter(point, range)
    }

    fn span_length(&self, range: Interval) -> f64 {
        self.length(range)
    }

    fn period_of(&self) -> Option<f64> {
        self.period()
    }

    fn analytic(
        first: &Self,
        first_range: Interval,
        second: &Self,
        second_range: Interval,
    ) -> Option<Vec<Candidate>> {
        let points: Vec<(Point2, bool)> = match (first, second) {
            (Curve2::Line(a), Curve2::Line(b)) => {
                let cross = a.direction().perp_dot(b.direction());
                let offset = b.origin() - a.origin();
                if cross.abs() <= PARALLEL_CROSS {
                    let apart = offset.perp_dot(a.direction()).abs();
                    return Some(if apart <= TOLERANCE {
                        vec![Candidate::Along(first_range)]
                    } else {
                        Vec::new()
                    });
                }
                let s = offset.perp_dot(b.direction()) / cross;
                vec![(a.point(s), false)]
            }
            (Curve2::Line(line), Curve2::Circle(circle))
            | (Curve2::Circle(circle), Curve2::Line(line)) => line_circle(
                line.origin(),
                line.direction(),
                circle.center(),
                circle.radius(),
            ),
            (Curve2::Circle(a), Curve2::Circle(b)) => {
                let apart = a.center().distance(b.center());
                let (ra, rb) = (a.radius(), b.radius());
                if apart <= TOLERANCE {
                    return Some(if (ra - rb).abs() <= TOLERANCE {
                        vec![Candidate::Along(first_range)]
                    } else {
                        Vec::new()
                    });
                }
                let axis = (b.center() - a.center()) / apart;
                let along = (apart * apart + ra * ra - rb * rb) / (2.0 * apart);
                let touching = (apart - (ra + rb)).abs() <= TOLERANCE
                    || (apart - (ra - rb).abs()).abs() <= TOLERANCE;
                if touching {
                    let sign = if along < 0.0 { -1.0 } else { 1.0 };
                    vec![(a.center() + axis * (ra * sign), true)]
                } else if along.abs() < ra {
                    let half = (ra * ra - along * along).sqrt();
                    let base = a.center() + axis * along;
                    vec![
                        (base + axis.perp() * half, false),
                        (base - axis.perp() * half, false),
                    ]
                } else {
                    Vec::new()
                }
            }
            _ => return None,
        };
        Some(
            points
                .into_iter()
                .filter_map(|(point, touch)| {
                    pair_from_point(first, first_range, second, second_range, point, touch)
                })
                .collect(),
        )
    }
}

pub fn intersect_curves(
    first: &Curve,
    first_range: Interval,
    second: &Curve,
    second_range: Interval,
) -> Result<CurveCurveIntersection<Point3>, IntersectionError> {
    intersect(first, first_range, second, second_range)
}

pub fn intersect_curves2(
    first: &Curve2,
    first_range: Interval,
    second: &Curve2,
    second_range: Interval,
) -> Result<CurveCurveIntersection<Point2>, IntersectionError> {
    intersect(first, first_range, second, second_range)
}

fn one_period(curve: &impl Traceable, range: Interval) -> Interval {
    match curve.period_of() {
        Some(period) if range.length() > period => {
            Interval::new(range.start(), range.start() + period).unwrap_or(range)
        }
        _ => range,
    }
}

struct Pair<'a, C> {
    first: &'a C,
    first_range: Interval,
    second: &'a C,
    second_range: Interval,
}

impl<C: Traceable> Pair<'_, C> {
    fn gap(&self, parameter: f64) -> f64 {
        let [point, _, _] = self.first.evaluate(parameter);
        let other = self.second.nearest(point, self.second_range);
        let [projected, _, _] = self.second.evaluate(other);
        point.distance_to(projected)
    }

    fn sine(&self, first: f64, second: f64) -> f64 {
        let [_, a, _] = self.first.evaluate(first);
        let [_, b, _] = self.second.evaluate(second);
        let angle = angle_between(a, b);
        angle.sin().abs()
    }

    fn curvature_gap(&self, first: f64) -> f64 {
        let [point, _, _] = self.first.evaluate(first);
        let second = self.second.nearest(point, self.second_range);
        let bend = |[_, tangent, curvature]: [C::Point; 3]| {
            let speed = tangent.dot(tangent);
            if speed <= f64::MIN_POSITIVE {
                return None;
            }
            let along = tangent * (tangent.dot(curvature) / speed);
            Some((curvature - along) * (1.0 / speed))
        };
        match (
            bend(self.first.evaluate(first)),
            bend(self.second.evaluate(second)),
        ) {
            (Some(a), Some(b)) => a.distance_to(b),
            _ => f64::INFINITY,
        }
    }
}

fn intersect<C: Traceable>(
    first: &C,
    first_range: Interval,
    second: &C,
    second_range: Interval,
) -> Result<CurveCurveIntersection<C::Point>, IntersectionError> {
    let pair = Pair {
        first,
        first_range: one_period(first, first_range),
        second,
        second_range: one_period(second, second_range),
    };
    let candidates = match C::analytic(first, pair.first_range, second, pair.second_range) {
        Some(candidates) => candidates,
        None => general(&pair)?,
    };
    Ok(finish(&pair, candidates))
}

fn turn<C: Traceable>(curve: &C, range: Interval) -> f64 {
    let [_, start, _] = curve.evaluate(range.start());
    let [_, middle, _] = curve.evaluate(range.middle());
    let [_, end, _] = curve.evaluate(range.end());
    angle_between(start, end)
        .max(angle_between(start, middle))
        .max(angle_between(middle, end))
}

fn halves(range: Interval) -> Option<(Interval, Interval)> {
    let middle = range.middle();
    if middle <= range.start() || middle >= range.end() {
        return None;
    }
    Some((
        Interval::new(range.start(), middle)?,
        Interval::new(middle, range.end())?,
    ))
}

fn general<C: Traceable>(pair: &Pair<C>) -> Result<Vec<Candidate>, IntersectionError> {
    let mut pending = vec![(pair.first_range, pair.second_range, 0usize)];
    let mut candidates = Vec::new();
    let mut visited = 0usize;
    while let Some((a, b, depth)) = pending.pop() {
        interrupt::check()?;
        visited += 1;
        if visited > MAX_PAIRS {
            return Err(IntersectionError::TooComplex(MAX_PAIRS));
        }
        let [a_min, a_max] = pair.first.range_bounds(a);
        let [b_min, b_max] = pair.second.range_bounds(b);
        let low = a_min.component_max(b_min);
        let high = a_max.component_min(b_max);
        if (high - low).min_component() < -TOLERANCE {
            continue;
        }
        let a_size = a_min.distance_to(a_max);
        let b_size = b_min.distance_to(b_max);
        let a_flat = a_size <= TINY_LEAF || turn(pair.first, a) <= LEAF_TURN;
        let b_flat = b_size <= TINY_LEAF || turn(pair.second, b) <= LEAF_TURN;
        if (a_flat && b_flat) || depth >= MAX_DEPTH {
            candidates.extend(solve_leaf(pair, a, b));
            continue;
        }
        let split_first = !a_flat && (b_flat || a_size >= b_size);
        let halves = if split_first { halves(a) } else { halves(b) };
        match halves {
            Some((low, high)) if split_first => {
                pending.push((high, b, depth + 1));
                pending.push((low, b, depth + 1));
            }
            Some((low, high)) => {
                pending.push((a, high, depth + 1));
                pending.push((a, low, depth + 1));
            }
            None => candidates.extend(solve_leaf(pair, a, b)),
        }
    }
    Ok(candidates)
}

fn solve_leaf<C: Traceable>(pair: &Pair<C>, a: Interval, b: Interval) -> Vec<Candidate> {
    let on_other = a.split(4).all(|parameter| {
        let [point, _, _] = pair.first.evaluate(parameter);
        let other = pair.second.nearest(point, b);
        let [projected, _, _] = pair.second.evaluate(other);
        point.distance_to(projected) <= ON_CURVE
    });
    if on_other {
        return vec![Candidate::Along(a)];
    }
    let mut found: Vec<(f64, f64, C::Point)> = Vec::new();
    for start in [a.middle(), a.start(), a.end()] {
        let [point, _, _] = pair.first.evaluate(start);
        let other = pair.second.nearest(point, b);
        let (s, t, gap) = newton(pair.first, a, pair.second, b, start, other);
        if gap > TOLERANCE {
            continue;
        }
        let [at, _, _] = pair.first.evaluate(s);
        if found
            .iter()
            .all(|(_, _, known)| known.distance_to(at) > TOLERANCE)
        {
            found.push((s, t, at));
        }
    }
    found
        .into_iter()
        .map(|(s, t, _)| Candidate::Pair {
            first: s,
            second: t,
            touch: pair.sine(s, t) <= TANGENT_SINE,
        })
        .collect()
}

fn newton<C: Traceable>(
    first: &C,
    first_range: Interval,
    second: &C,
    second_range: Interval,
    s: f64,
    t: f64,
) -> (f64, f64, f64) {
    let distance = |s: f64, t: f64| {
        let [a, _, _] = first.evaluate(s);
        let [b, _, _] = second.evaluate(t);
        a.distance_to(b)
    };
    let (mut s, mut t) = (s, t);
    let mut current = distance(s, t);
    for _ in 0..MAX_NEWTON {
        if current == 0.0 {
            break;
        }
        let [pa, da, sa] = first.evaluate(s);
        let [pb, db, sb] = second.evaluate(t);
        let difference = pa - pb;
        let gradient = (da.dot(difference), -db.dot(difference));
        let cross = -da.dot(db);
        let full = (
            da.dot(da) + sa.dot(difference),
            cross,
            db.dot(db) - sb.dot(difference),
        );
        let plain = (da.dot(da), cross, db.dot(db));
        let Some((ds, dt)) = solve_2x2(full, gradient).or_else(|| solve_2x2(plain, gradient))
        else {
            break;
        };
        let mut scale = 1.0;
        let mut accepted = None;
        for _ in 0..MAX_HALVINGS {
            let (ns, nt) = (
                first_range.clamp(s + ds * scale),
                second_range.clamp(t + dt * scale),
            );
            let value = distance(ns, nt);
            if value < current {
                accepted = Some((ns, nt, value));
                break;
            }
            scale *= 0.5;
        }
        let Some((ns, nt, value)) = accepted else {
            break;
        };
        let moved = (ns - s).abs() + (nt - t).abs();
        s = ns;
        t = nt;
        current = value;
        if moved <= 1e-15 * (1.0 + s.abs() + t.abs()) {
            break;
        }
    }
    (s, t, current)
}

fn solve_2x2((a, b, c): (f64, f64, f64), (g, h): (f64, f64)) -> Option<(f64, f64)> {
    let determinant = a * c - b * b;
    let scale = a.abs().max(c.abs()).max(b.abs());
    if a <= 0.0 || determinant <= 1e-14 * scale * scale || !determinant.is_finite() {
        return None;
    }
    let step = (
        -(c * g - b * h) / determinant,
        -(a * h - b * g) / determinant,
    );
    (step.0.is_finite() && step.1.is_finite()).then_some(step)
}

fn connected<C: Traceable>(pair: &Pair<C>, low: f64, high: f64) -> bool {
    high <= low
        || CONNECTION_SAMPLES
            .iter()
            .all(|fraction| pair.gap(low + (high - low) * fraction) <= TOLERANCE)
}

fn overlap_like<C: Traceable>(pair: &Pair<C>, low: f64, high: f64) -> bool {
    let Some(span) = Interval::new(low, high) else {
        return false;
    };
    let middle = span.middle();
    let [point, _, _] = pair.first.evaluate(middle);
    let other = pair.second.nearest(point, pair.second_range);
    span.split(OVERLAP_SAMPLES)
        .all(|parameter| pair.gap(parameter) <= TOLERANCE)
        && pair.sine(middle, other) <= OVERLAP_SINE
        && pair.curvature_gap(middle) <= OVERLAP_CURVATURE
}

fn extend<C: Traceable>(pair: &Pair<C>, inside: f64, limit: f64) -> f64 {
    if pair.gap(limit) <= TOLERANCE && connected(pair, inside, limit) {
        return limit;
    }
    let (mut inside, mut outside) = (inside, limit);
    for _ in 0..EXTENSION_BISECTIONS {
        let middle = 0.5 * (inside + outside);
        if middle == inside || middle == outside {
            break;
        }
        if pair.gap(middle) <= ON_CURVE {
            inside = middle;
        } else {
            outside = middle;
        }
    }
    inside
}

fn end_candidates<C: Traceable>(pair: &Pair<C>) -> Vec<Candidate> {
    let mut ends = Vec::new();
    for first in [pair.first_range.start(), pair.first_range.end()] {
        let [point, _, _] = pair.first.evaluate(first);
        let second = pair.second.nearest(point, pair.second_range);
        let [other, _, _] = pair.second.evaluate(second);
        if point.distance_to(other) <= TOLERANCE {
            ends.push(Candidate::End { first, second });
        }
    }
    for second in [pair.second_range.start(), pair.second_range.end()] {
        let [point, _, _] = pair.second.evaluate(second);
        let first = pair.first.nearest(point, pair.first_range);
        let [other, _, _] = pair.first.evaluate(first);
        if point.distance_to(other) <= TOLERANCE {
            ends.push(Candidate::End { first, second });
        }
    }
    ends
}

fn cluster_high(cluster: &[Candidate]) -> f64 {
    cluster.iter().map(Candidate::high).fold(f64::MIN, f64::max)
}

fn cluster_low(cluster: &[Candidate]) -> f64 {
    cluster.iter().map(Candidate::low).fold(f64::MAX, f64::min)
}

fn distinct<C: Traceable>(curve: &C, cluster: &[Candidate]) -> usize {
    let mut spots: Vec<C::Point> = Vec::new();
    let mut count = 0;
    for candidate in cluster {
        match candidate {
            Candidate::Along(_) => count += 1,
            Candidate::Pair { first, .. } => {
                let [point, _, _] = curve.evaluate(*first);
                if !spots
                    .iter()
                    .any(|spot| spot.distance_to(point) <= TOLERANCE)
                {
                    spots.push(point);
                    count += 1;
                }
            }
            Candidate::End { .. } => {}
        }
    }
    count
}

fn exact_ends<C: Traceable>(
    pair: &Pair<C>,
    cluster: &[Candidate],
    start: (f64, f64),
) -> (f64, f64) {
    cluster
        .iter()
        .fold(start, |(s, t), candidate| match candidate {
            Candidate::End { first, second } => {
                let first_end =
                    *first == pair.first_range.start() || *first == pair.first_range.end();
                let second_end =
                    *second == pair.second_range.start() || *second == pair.second_range.end();
                (
                    if first_end { *first } else { s },
                    if second_end { *second } else { t },
                )
            }
            _ => (s, t),
        })
}

fn finish<C: Traceable>(
    pair: &Pair<C>,
    mut candidates: Vec<Candidate>,
) -> CurveCurveIntersection<C::Point> {
    candidates.extend(end_candidates(pair));
    candidates.retain(|candidate| {
        candidate.low() >= pair.first_range.start() && candidate.high() <= pair.first_range.end()
    });
    candidates.sort_by(|a, b| a.low().total_cmp(&b.low()));
    let mut clusters: Vec<Vec<Candidate>> = Vec::new();
    for candidate in candidates {
        match clusters.last_mut() {
            Some(cluster)
                if cluster_high(cluster) >= candidate.low()
                    || connected(pair, cluster_high(cluster), candidate.low()) =>
            {
                cluster.push(candidate);
            }
            _ => clusters.push(vec![candidate]),
        }
    }
    let mut result = CurveCurveIntersection::default();
    for (index, cluster) in clusters.iter().enumerate() {
        let (low, high) = (cluster_low(cluster), cluster_high(cluster));
        let along = cluster
            .iter()
            .any(|candidate| matches!(candidate, Candidate::Along(_)));
        let genuine = distinct(pair.first, cluster);
        let span = Interval::new(low, high).map_or(0.0, |span| pair.first.span_length(span));
        let overlap =
            span > MIN_OVERLAP && (along || (genuine >= 2 && overlap_like(pair, low, high)));
        if overlap {
            let previous = index
                .checked_sub(1)
                .and_then(|before| clusters.get(before))
                .map_or(pair.first_range.start(), |cluster| cluster_high(cluster));
            let next = clusters
                .get(index + 1)
                .map_or(pair.first_range.end(), |cluster| cluster_low(cluster));
            let start = extend(pair, low, previous);
            let end = extend(pair, high, next);
            let Some(extent) = Interval::new(start, end) else {
                continue;
            };
            let ends: Vec<f64> = cluster
                .iter()
                .filter_map(|candidate| match candidate {
                    Candidate::End { first, .. } => Some(*first),
                    _ => None,
                })
                .collect();
            let snap = |parameter: f64| {
                let [point, _, _] = pair.first.evaluate(parameter);
                ends.iter()
                    .copied()
                    .find(|end| {
                        let [at, _, _] = pair.first.evaluate(*end);
                        at.distance_to(point) <= 2.0 * TOLERANCE
                    })
                    .unwrap_or(parameter)
            };
            for piece in inside_intervals(
                extent,
                CLIP_SAMPLES,
                &range_end_seeds(pair, extent),
                |parameter| pair.gap(parameter) <= TOLERANCE,
            ) {
                let Some(piece) = Interval::new(snap(piece.start()), snap(piece.end())) else {
                    continue;
                };
                let second_of = |parameter: f64| {
                    let [point, _, _] = pair.first.evaluate(parameter);
                    pair.second.nearest(point, pair.second_range)
                };
                if pair.first.span_length(piece) <= MIN_OVERLAP {
                    let [point, _, _] = pair.first.evaluate(piece.start());
                    result.points.push(CurveCurvePoint {
                        first: piece.start(),
                        second: second_of(piece.start()),
                        point,
                        tangent: true,
                    });
                } else {
                    result.overlaps.push(CurveCurveOverlap {
                        first: piece,
                        second_start: second_of(piece.start()),
                        second_end: second_of(piece.end()),
                    });
                }
            }
            continue;
        }
        let end = cluster.iter().find_map(|candidate| match candidate {
            Candidate::End { first, second } => Some((*first, *second)),
            _ => None,
        });
        let touch = cluster.iter().find_map(|candidate| match candidate {
            Candidate::Pair {
                first,
                second,
                touch: true,
            } => Some((*first, *second)),
            _ => None,
        });
        let any = cluster.iter().next().map(|candidate| match candidate {
            Candidate::Pair { first, second, .. } | Candidate::End { first, second } => {
                (*first, *second)
            }
            Candidate::Along(range) => {
                let [point, _, _] = pair.first.evaluate(range.middle());
                (
                    range.middle(),
                    pair.second.nearest(point, pair.second_range),
                )
            }
        });
        let Some((first, second)) = end.or(touch).or(any) else {
            continue;
        };
        let (first, second) = if end.is_some() {
            exact_ends(pair, cluster, (first, second))
        } else {
            (first, second)
        };
        let [point, _, _] = pair.first.evaluate(first);
        let tangent =
            genuine >= 2 || touch.is_some() || along || pair.sine(first, second) <= TANGENT_SINE;
        result.points.push(CurveCurvePoint {
            first,
            second,
            point,
            tangent,
        });
    }
    let overlaps = result.overlaps.clone();
    result.points.retain(|point| {
        !overlaps
            .iter()
            .any(|overlap| overlap.first.contains(point.first))
    });
    result.points.sort_by(|a, b| a.first.total_cmp(&b.first));
    if full_period(pair.first, pair.first_range)
        && let (Some(first), Some(last)) = (result.points.first(), result.points.last())
        && result.points.len() > 1
        && first.point.distance_to(last.point) <= TOLERANCE
    {
        result.points.pop();
    }
    result
}

fn range_end_seeds<C: Traceable>(pair: &Pair<C>, extent: Interval) -> Vec<f64> {
    if full_period(pair.second, pair.second_range) {
        return Vec::new();
    }
    let mut feet: Vec<f64> = [pair.second_range.start(), pair.second_range.end()]
        .into_iter()
        .map(|end| {
            let [point, _, _] = pair.second.evaluate(end);
            pair.first.nearest(point, extent)
        })
        .collect();
    feet.sort_by(f64::total_cmp);
    let betweens: Vec<f64> = feet
        .windows(2)
        .filter_map(|neighbours| match neighbours {
            [low, high] => Some(0.5 * (low + high)),
            _ => None,
        })
        .collect();
    feet.extend(betweens);
    feet
}

fn full_period<C: Traceable>(curve: &C, range: Interval) -> bool {
    curve
        .period_of()
        .is_some_and(|period| range.length() >= period * (1.0 - 1e-12))
}
