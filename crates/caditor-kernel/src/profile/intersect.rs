use std::f64::consts::TAU;

use caditor_geometry::{Aabb2, Point2};

use crate::{
    curve2::{Circle2, Curve2, Line2},
    interval::Interval,
    profile::{geometry::overlaps, source::Source},
};

const PARALLEL: f64 = 1e-14;
const LEAF_FRACTION: f64 = 1e-3;
const LEAF_TOLERANCES: f64 = 4.0;
const MAX_BOX_TESTS: usize = 100_000;
const MAX_LEAVES: usize = 2048;
const MAX_SOLVER_STEPS: usize = 80;
const MAX_DAMPING_TRIES: usize = 24;
const INITIAL_DAMPING: f64 = 1e-12;
const MIN_DAMPING: f64 = 1e-16;
const TANGENT_SPAN: f64 = 1e-2;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Hit {
    pub first: f64,
    pub second: f64,
    pub point: Point2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Overlapping;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Scale {
    pub tolerance: f64,
    pub size: f64,
}

impl Scale {
    fn leaf(&self) -> f64 {
        (self.size * LEAF_FRACTION).max(self.tolerance * LEAF_TOLERANCES)
    }
}

pub(crate) fn between(
    first: &Source,
    first_segments: &[Interval],
    second: &Source,
    second_segments: &[Interval],
    scale: Scale,
) -> Result<Vec<Hit>, Overlapping> {
    let tolerance = scale.tolerance;
    match (&first.curve, &second.curve) {
        (Curve2::Line(a), Curve2::Line(b)) => {
            Ok(line_line(a, first.range, b, second.range, tolerance))
        }
        (Curve2::Line(line), Curve2::Circle(circle)) => Ok(line_circle(
            line,
            first.range,
            circle,
            second.range,
            tolerance,
        )),
        (Curve2::Circle(circle), Curve2::Line(line)) => {
            Ok(
                line_circle(line, second.range, circle, first.range, tolerance)
                    .into_iter()
                    .map(swap)
                    .collect(),
            )
        }
        (Curve2::Circle(a), Curve2::Circle(b)) => {
            Ok(circle_circle(a, first.range, b, second.range, tolerance))
        }
        _ => {
            let mut hits = Vec::new();
            let mut budget = MAX_BOX_TESTS;
            for first_segment in first_segments {
                for second_segment in second_segments {
                    hits.extend(subdivide(
                        (first, *first_segment),
                        (second, *second_segment),
                        scale,
                        &mut budget,
                    )?);
                }
            }
            consolidate(first, second, hits, scale, false)
        }
    }
}

pub(crate) fn within(
    source: &Source,
    segments: &[Interval],
    scale: Scale,
) -> Result<Vec<Hit>, Overlapping> {
    let mut hits = Vec::new();
    let mut budget = MAX_BOX_TESTS;
    for (index, first) in segments.iter().enumerate() {
        for (offset, second) in segments.iter().enumerate().skip(index + 1) {
            let found = subdivide((source, *first), (source, *second), scale, &mut budget)?;
            let adjacent = offset == index + 1;
            let joint = source.point(first.end());
            hits.extend(found.into_iter().filter(|hit| {
                let at_joint = hit.point.distance(joint) <= LEAF_TOLERANCES * scale.tolerance
                    && source.length_between(hit.first, hit.second)
                        <= LEAF_TOLERANCES * scale.tolerance;
                !(adjacent && at_joint)
            }));
        }
    }
    let hits = consolidate(source, source, hits, scale, true)?;
    Ok(hits
        .into_iter()
        .filter(|hit| source.length_between(hit.first, hit.second) > scale.tolerance)
        .collect())
}

fn swap(hit: Hit) -> Hit {
    Hit {
        first: hit.second,
        second: hit.first,
        point: hit.point,
    }
}

fn line_parameter(range: Interval, parameter: f64, tolerance: f64) -> Option<f64> {
    (parameter >= range.start() - tolerance && parameter <= range.end() + tolerance)
        .then(|| range.clamp(parameter))
}

fn circle_parameter(
    circle: &Circle2,
    range: Interval,
    point: Point2,
    tolerance: f64,
) -> Option<f64> {
    let offset = point - circle.center();
    let angle = offset
        .dot(circle.y_axis())
        .atan2(offset.dot(circle.x_axis()));
    let slack = tolerance / circle.radius();
    let turns = ((range.start() - slack - angle) / TAU).ceil();
    let parameter = angle + turns * TAU;
    (parameter <= range.end() + slack).then(|| range.clamp(parameter))
}

fn line_line(
    a: &Line2,
    a_range: Interval,
    b: &Line2,
    b_range: Interval,
    tolerance: f64,
) -> Vec<Hit> {
    let cross = a.direction().perp_dot(b.direction());
    if cross.abs() <= PARALLEL {
        return Vec::new();
    }
    let offset = b.origin() - a.origin();
    let along_a = offset.perp_dot(b.direction()) / cross;
    let along_b = offset.perp_dot(a.direction()) / cross;
    match (
        line_parameter(a_range, along_a, tolerance),
        line_parameter(b_range, along_b, tolerance),
    ) {
        (Some(first), Some(second)) => vec![Hit {
            first,
            second,
            point: a.point(first),
        }],
        _ => Vec::new(),
    }
}

fn line_circle(
    line: &Line2,
    line_range: Interval,
    circle: &Circle2,
    circle_range: Interval,
    tolerance: f64,
) -> Vec<Hit> {
    let foot_parameter = line.parameter_of(circle.center());
    let foot = line.point(foot_parameter);
    let distance = foot.distance(circle.center());
    let radius = circle.radius();
    if distance > radius + tolerance {
        return Vec::new();
    }
    let parameters = if (distance - radius).abs() <= tolerance {
        vec![foot_parameter]
    } else {
        let half_chord = (radius * radius - distance * distance).max(0.0).sqrt();
        vec![foot_parameter - half_chord, foot_parameter + half_chord]
    };
    parameters
        .into_iter()
        .filter_map(|parameter| {
            let first = line_parameter(line_range, parameter, tolerance)?;
            let point = line.point(first);
            let second = circle_parameter(circle, circle_range, point, tolerance)?;
            Some(Hit {
                first,
                second,
                point,
            })
        })
        .collect()
}

fn circle_circle(
    a: &Circle2,
    a_range: Interval,
    b: &Circle2,
    b_range: Interval,
    tolerance: f64,
) -> Vec<Hit> {
    let between = b.center() - a.center();
    let distance = between.length();
    let (ra, rb) = (a.radius(), b.radius());
    if distance <= tolerance
        || distance > ra + rb + tolerance
        || distance < (ra - rb).abs() - tolerance
    {
        return Vec::new();
    }
    let toward = between / distance;
    let points = if (distance - (ra + rb)).abs() <= tolerance {
        vec![a.center() + toward * ra]
    } else if (distance - (ra - rb).abs()).abs() <= tolerance {
        let sign = if ra >= rb { 1.0 } else { -1.0 };
        vec![a.center() + toward * (ra * sign)]
    } else {
        let along = (distance * distance + ra * ra - rb * rb) / (2.0 * distance);
        let across = (ra * ra - along * along).max(0.0).sqrt();
        let base = a.center() + toward * along;
        vec![base - toward.perp() * across, base + toward.perp() * across]
    };
    points
        .into_iter()
        .filter_map(|point| {
            Some(Hit {
                first: circle_parameter(a, a_range, point, tolerance)?,
                second: circle_parameter(b, b_range, point, tolerance)?,
                point,
            })
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Span {
    range: Interval,
    start: Point2,
    end: Point2,
}

impl Span {
    fn of(source: &Source, range: Interval) -> Self {
        Self {
            range,
            start: source.point(range.start()),
            end: source.point(range.end()),
        }
    }

    fn bounds(&self) -> Aabb2 {
        Aabb2::from_point(self.start).including(self.end)
    }

    fn halves(&self, source: &Source) -> Option<[Self; 2]> {
        let middle = self.range.middle();
        if middle <= self.range.start() || middle >= self.range.end() {
            return None;
        }
        let point = source.point(middle);
        Some([
            Self {
                range: Interval::new(self.range.start(), middle)?,
                start: self.start,
                end: point,
            },
            Self {
                range: Interval::new(middle, self.range.end())?,
                start: point,
                end: self.end,
            },
        ])
    }
}

fn largest_side(bounds: &Aabb2) -> f64 {
    bounds.size().max_element()
}

fn subdivide(
    (first, first_segment): (&Source, Interval),
    (second, second_segment): (&Source, Interval),
    scale: Scale,
    budget: &mut usize,
) -> Result<Vec<Hit>, Overlapping> {
    let leaf = scale.leaf();
    let mut pending = vec![(
        Span::of(first, first_segment),
        Span::of(second, second_segment),
    )];
    let mut leaves = Vec::new();
    while let Some((a, b)) = pending.pop() {
        *budget = budget.checked_sub(1).ok_or(Overlapping)?;
        let (a_box, b_box) = (a.bounds(), b.bounds());
        if !overlaps(
            &a_box.expanded(scale.tolerance),
            &b_box.expanded(scale.tolerance),
        ) {
            continue;
        }
        let (a_size, b_size) = (largest_side(&a_box), largest_side(&b_box));
        let split_first = a_size >= b_size;
        let halves = if a_size <= leaf && b_size <= leaf {
            None
        } else if split_first {
            a.halves(first)
        } else {
            b.halves(second)
        };
        match halves {
            Some(halves) => {
                for half in halves {
                    pending.push(if split_first { (half, b) } else { (a, half) });
                }
            }
            None => {
                if leaves.len() >= MAX_LEAVES {
                    return Err(Overlapping);
                }
                leaves.push((a.range.middle(), b.range.middle()));
            }
        }
    }
    let mut hits: Vec<Hit> = Vec::new();
    for start in leaves {
        let near = first.point(start.0);
        if hits.iter().any(|hit| hit.point.distance(near) <= leaf) {
            continue;
        }
        let (s, t, distance) = converge(
            &first.curve,
            first_segment,
            &second.curve,
            second_segment,
            start,
        );
        if distance <= scale.tolerance {
            hits.push(Hit {
                first: s,
                second: t,
                point: (first.point(s) + second.point(t)) * 0.5,
            });
        }
    }
    Ok(hits)
}

fn converge(
    first: &Curve2,
    first_range: Interval,
    second: &Curve2,
    second_range: Interval,
    (mut s, mut t): (f64, f64),
) -> (f64, f64, f64) {
    let residual = |s: f64, t: f64| first.point(s) - second.point(t);
    let mut offset = residual(s, t);
    let mut norm = offset.length();
    let mut damping = INITIAL_DAMPING;
    for _ in 0..MAX_SOLVER_STEPS {
        if norm == 0.0 || !norm.is_finite() {
            break;
        }
        let along_first = first.evaluate(s).first;
        let along_second = second.evaluate(t).first;
        let (h11, h12, h22) = (
            along_first.dot(along_first),
            -along_first.dot(along_second),
            along_second.dot(along_second),
        );
        let (g1, g2) = (along_first.dot(offset), -along_second.dot(offset));
        let scale = h11 + h22;
        if !(scale.is_finite() && scale > 0.0) {
            break;
        }
        let mut improved = false;
        for _ in 0..MAX_DAMPING_TRIES {
            let (a11, a22) = (h11 + damping * scale, h22 + damping * scale);
            let determinant = a11 * a22 - h12 * h12;
            if determinant > 0.0 {
                let step_s = -(a22 * g1 - h12 * g2) / determinant;
                let step_t = -(a11 * g2 - h12 * g1) / determinant;
                let (next_s, next_t) = (
                    first_range.clamp(s + step_s),
                    second_range.clamp(t + step_t),
                );
                let next = residual(next_s, next_t);
                let next_norm = next.length();
                if next_norm < norm {
                    s = next_s;
                    t = next_t;
                    offset = next;
                    norm = next_norm;
                    damping = (damping * 0.1).max(MIN_DAMPING);
                    improved = true;
                    break;
                }
            }
            damping *= 10.0;
        }
        if !improved {
            break;
        }
    }
    (s, t, norm)
}

fn consolidate(
    first: &Source,
    second: &Source,
    mut hits: Vec<Hit>,
    scale: Scale,
    same: bool,
) -> Result<Vec<Hit>, Overlapping> {
    hits.sort_by(|a, b| a.first.total_cmp(&b.first));
    let mut distinct: Vec<Hit> = Vec::new();
    for hit in hits {
        let repeated = distinct.iter().any(|kept| {
            kept.point.distance(hit.point) <= scale.tolerance
                && first.length_between(kept.first, hit.first) <= LEAF_TOLERANCES * scale.tolerance
        });
        if !repeated {
            distinct.push(hit);
        }
    }
    let mut chains: Vec<Vec<Hit>> = Vec::new();
    for hit in distinct {
        let joined = chains
            .last()
            .and_then(|chain| chain.last())
            .is_some_and(|last| {
                let halfway = 0.5 * (last.first + hit.first);
                let middle = first.point(halfway);
                let span = Interval::new(last.second.min(hit.second), last.second.max(hit.second))
                    .unwrap_or(second.range);
                let nearest = second.curve.closest_parameter(middle, span);
                let trivial = same && span.contains(halfway);
                !trivial && second.point(nearest).distance(middle) <= scale.tolerance
            });
        match chains.last_mut() {
            Some(chain) if joined => chain.push(hit),
            _ => chains.push(vec![hit]),
        }
    }
    let mut result = Vec::with_capacity(chains.len());
    for chain in chains {
        let (Some(start), Some(end)) = (chain.first(), chain.last()) else {
            continue;
        };
        if first.length_between(start.first, end.first) > TANGENT_SPAN * scale.size {
            return Err(Overlapping);
        }
        if let Some(middle) = chain.get(chain.len() / 2) {
            result.push(*middle);
        }
    }
    Ok(result)
}
