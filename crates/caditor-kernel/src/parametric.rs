use crate::{
    coordinates::{Coordinates, angle_between, distance_to_segment},
    interrupt::{self, Interrupted},
    interval::Interval,
    numeric::{Taylor, integrate, minimize_near},
    tolerance::SamplingTolerance,
};

const MAX_SAMPLES: usize = 1 << 16;
const MAX_DEPTH: usize = 32;
const CLOSEST_SEED_REFINEMENT: usize = 4;
const LENGTH_SEED_REFINEMENT: usize = 2;
const BOUND_SAMPLES: usize = 4;
const BOUND_SAFETY: f64 = 2.0;
const SEEDS_PER_POLL: usize = 1024;
const MAX_MERGED_SAMPLES: usize = 64;
const CORNER_PROBE: f64 = 1e-3;
const CORNER_SHARE: f64 = 0.01;

pub(crate) trait Parametric {
    type Point: Coordinates;

    fn evaluate(&self, parameter: f64) -> [Self::Point; 3];

    fn seeds(&self, range: Interval) -> Vec<f64>;

    fn nearby_runs(&self, _point: Self::Point, range: Interval) -> Vec<Interval> {
        vec![range]
    }
}

pub(crate) fn refined_seeds(curve: &impl Parametric, range: Interval, factor: usize) -> Vec<f64> {
    let seeds = curve.seeds(range);
    let mut refined = vec![range.start()];
    for pair in seeds.windows(2) {
        if let [start, end] = pair
            && let Some(piece) = Interval::new(*start, *end)
        {
            refined.extend(piece.split(factor).skip(1));
        }
    }
    refined.dedup();
    refined
}

pub(crate) fn seed_runs(
    curve: &impl Parametric,
    runs: Vec<Interval>,
    factor: usize,
) -> Vec<Vec<f64>> {
    runs.into_iter()
        .map(|run| refined_seeds(curve, run, factor))
        .collect()
}

pub(crate) fn closest_parameter<C: Parametric>(curve: &C, point: C::Point, range: Interval) -> f64 {
    closest_parameter_near(curve, point, range, None)
}

pub(crate) fn closest_parameter_near<C: Parametric>(
    curve: &C,
    point: C::Point,
    range: Interval,
    hint: Option<f64>,
) -> f64 {
    closest_parameter_among(curve, point, range, hint, || {
        curve.nearby_runs(point, range)
    })
}

pub(crate) fn closest_parameter_among<C: Parametric>(
    curve: &C,
    point: C::Point,
    range: Interval,
    hint: Option<f64>,
    runs: impl FnOnce() -> Vec<Interval>,
) -> f64 {
    let samples = || seed_runs(curve, runs(), CLOSEST_SEED_REFINEMENT);
    let objective = |parameter: f64| {
        let parameter = range.clamp(parameter);
        let [position, first, second] = curve.evaluate(parameter);
        let offset = position - point;
        Taylor {
            value: offset.dot(offset),
            slope: 2.0 * first.dot(offset),
            curvature: 2.0 * (first.dot(first) + second.dot(offset)),
        }
    };
    range.clamp(minimize_near(samples, objective, range, hint).unwrap_or(range.start()))
}

pub(crate) fn length<C: Parametric>(curve: &C, range: Interval) -> f64 {
    let breaks = refined_seeds(curve, range, LENGTH_SEED_REFINEMENT);
    integrate(&breaks, |parameter| {
        let [_, first, _] = curve.evaluate(parameter);
        first.norm()
    })
}

pub(crate) fn length_up_to<C: Parametric>(curve: &C, range: Interval, cap: f64) -> f64 {
    let breaks = refined_seeds(curve, range, LENGTH_SEED_REFINEMENT);
    let mut travelled = 0.0;
    for piece in breaks.windows(2) {
        travelled += integrate(piece, |parameter| {
            let [_, first, _] = curve.evaluate(parameter);
            first.norm()
        });
        if travelled >= cap {
            return cap;
        }
    }
    travelled
}

pub(crate) fn polyline_length_up_to<C: Parametric>(
    curve: &C,
    range: Interval,
    cap: f64,
) -> Result<f64, Interrupted> {
    let point = |parameter: f64| {
        let [point, _, _] = curve.evaluate(parameter);
        point
    };
    let mut travelled = 0.0;
    let mut previous = point(range.start());
    for (index, parameter) in curve.seeds(range).into_iter().skip(1).enumerate() {
        if index % SEEDS_PER_POLL == 0 {
            interrupt::check()?;
        }
        let next = point(parameter);
        travelled += previous.distance_to(next);
        if travelled >= cap {
            return Ok(cap);
        }
        previous = next;
    }
    Ok(travelled)
}

pub(crate) fn longer_than<C: Parametric>(curve: &C, range: Interval, bound: f64) -> bool {
    let point = |parameter: f64| {
        let [point, _, _] = curve.evaluate(parameter);
        point
    };
    let start = point(range.start());
    if start.distance_to(point(range.end())) > bound {
        return true;
    }
    let mut travelled = 0.0;
    let mut previous = start;
    for parameter in curve.seeds(range).into_iter().skip(1) {
        let next = point(parameter);
        travelled += previous.distance_to(next);
        if travelled > bound {
            return true;
        }
        previous = next;
    }
    length(curve, range) > bound
}

pub(crate) fn adaptive_parameters<C: Parametric>(
    curve: &C,
    range: Interval,
    tolerance: &SamplingTolerance,
) -> Vec<f64> {
    let seeds = curve.seeds(range);
    let mut parameters = vec![range.start()];
    for pair in seeds.windows(2) {
        let [start, end] = pair else {
            continue;
        };
        let mut pending = vec![(*start, *end, 0)];
        while let Some((low, high, depth)) = pending.pop() {
            let budget_left = parameters.len() + pending.len() < MAX_SAMPLES;
            if depth < MAX_DEPTH && budget_left && needs_split(curve, low, high, tolerance) {
                let middle = 0.5 * (low + high);
                pending.push((middle, high, depth + 1));
                pending.push((low, middle, depth + 1));
            } else {
                parameters.push(high);
            }
        }
    }
    parameters.dedup();
    if parameters.len() < 2 {
        parameters.push(range.end());
    }
    parameters
}

pub(crate) fn merged_parameters<C: Parametric>(
    curve: &C,
    parameters: &[f64],
    tolerance: &SamplingTolerance,
) -> Vec<f64> {
    let samples: Vec<[C::Point; 2]> = parameters
        .iter()
        .map(|parameter| {
            let [point, tangent, _] = curve.evaluate(*parameter);
            [point, tangent]
        })
        .collect();
    let middles: Vec<C::Point> = parameters
        .windows(2)
        .filter_map(|pair| match pair {
            [low, high] => Some(curve.evaluate(0.5 * (low + high))[0]),
            _ => None,
        })
        .collect();
    let covers = |anchor: usize, reach: usize| {
        let (Some([start, start_tangent]), Some([end, end_tangent])) =
            (samples.get(anchor), samples.get(reach))
        else {
            return false;
        };
        let within =
            |point: &C::Point| distance_to_segment(*point, *start, *end) <= tolerance.chord();
        let turns_little =
            |tangent: &C::Point| angle_between(*start_tangent, *tangent) <= tolerance.angle();
        turns_little(end_tangent)
            && samples
                .get(anchor + 1..reach)
                .unwrap_or_default()
                .iter()
                .all(|[point, tangent]| within(point) && turns_little(tangent))
            && middles
                .get(anchor..reach)
                .unwrap_or_default()
                .iter()
                .all(within)
    };
    let corners: Vec<bool> = (0..parameters.len())
        .map(|index| {
            let (Some(before), Some(at), Some(after)) = (
                index
                    .checked_sub(1)
                    .and_then(|before| parameters.get(before)),
                parameters.get(index),
                parameters.get(index + 1),
            ) else {
                return true;
            };
            let probe = CORNER_PROBE * (at - before).min(after - at);
            let [_, left, _] = curve.evaluate(at - probe);
            let [_, right, _] = curve.evaluate(at + probe);
            let turn = angle_between(left, right);
            turn.is_nan() || turn > CORNER_SHARE * tolerance.angle()
        })
        .collect();
    let is_corner = |index: usize| corners.get(index).copied().unwrap_or(true);
    let Some(first) = parameters.first() else {
        return Vec::new();
    };
    let last = parameters.len() - 1;
    let mut merged = vec![*first];
    let mut anchor = 0;
    while anchor < last {
        let mut reach = anchor + 1;
        while reach < last
            && !is_corner(reach)
            && reach - anchor < MAX_MERGED_SAMPLES
            && covers(anchor, reach + 1)
        {
            reach += 1;
        }
        merged.extend(parameters.get(reach));
        anchor = reach;
    }
    merged
}

fn needs_split<C: Parametric>(
    curve: &C,
    low: f64,
    high: f64,
    tolerance: &SamplingTolerance,
) -> bool {
    let middle = 0.5 * (low + high);
    if middle <= low || middle >= high {
        return false;
    }
    let [start, start_tangent, _] = curve.evaluate(low);
    let [end, end_tangent, _] = curve.evaluate(high);
    let [center, center_tangent, _] = curve.evaluate(middle);
    let span = start.distance_to(center) + center.distance_to(end);
    if span <= tolerance.chord() {
        return false;
    }
    let deviation = distance_to_segment(center, start, end);
    let turning = angle_between(start_tangent, end_tangent)
        .max(angle_between(start_tangent, center_tangent))
        .max(angle_between(center_tangent, end_tangent));
    deviation > tolerance.chord() || turning > tolerance.angle()
}

pub(crate) fn tight_bounds<C: Parametric>(
    curve: &C,
    range: Interval,
    [coarse_min, coarse_max]: [C::Point; 2],
) -> [C::Point; 2] {
    let step = range.length() / BOUND_SAMPLES as f64;
    let mut low = curve.evaluate(range.start())[0];
    let mut high = low;
    let mut bend: f64 = 0.0;
    for parameter in range.split(BOUND_SAMPLES) {
        let [point, _, second] = curve.evaluate(parameter);
        low = low.component_min(point);
        high = high.component_max(point);
        bend = bend.max(second.norm());
    }
    let margin = C::Point::splat(BOUND_SAFETY * bend * step * step / 8.0);
    let min = (low - margin).component_max(coarse_min);
    let max = (high + margin).component_min(coarse_max);
    if (max - min).min_component() < 0.0 {
        [coarse_min, coarse_max]
    } else {
        [min, max]
    }
}
