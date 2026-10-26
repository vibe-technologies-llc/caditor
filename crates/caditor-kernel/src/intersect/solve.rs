use caditor_geometry::{Point2, Point3, Vector2, Vector3};

use crate::{
    intersect::linear::damped_least_squares,
    surface::{Surface, SurfaceDerivatives},
    tolerance::LINEAR_RESOLUTION,
};

pub(crate) const CONTACT_ACCEPT: f64 = 0.1 * LINEAR_RESOLUTION;
const MAX_CONTACT_ITERATIONS: usize = 60;
const MAX_HALVINGS: usize = 20;
const TINY_SPEED: f64 = 1e-150;
const STEP_EPSILON: f64 = 1e-15;
const NOISE_FLOOR: f64 = 1e-13;
const STAGNANT_SCALE: f64 = 1.0 / 64.0;
const STAGNANT_GAIN: f64 = 1e-3;
const MAX_BRACKET_ITERATIONS: usize = 200;
const MAX_MINIMUM_ITERATIONS: usize = 100;
const SHRINK: f64 = 0.381_966_011_250_105_1;
const MINIMUM_PRECISION: f64 = 1e-10;
const MINIMUM_FLOOR: f64 = 1e-14;
const COEFFICIENT_FLOOR: f64 = 1e-15;
const PARALLEL_SINE: f64 = 1e-6;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Contact {
    pub uv: [Point2; 2],
    pub point: Point3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Coordinate {
    FirstU,
    FirstV,
    SecondU,
    SecondV,
}

impl Coordinate {
    fn read(self, state: [f64; 4]) -> f64 {
        let [u1, v1, u2, v2] = state;
        match self {
            Self::FirstU => u1,
            Self::FirstV => v1,
            Self::SecondU => u2,
            Self::SecondV => v2,
        }
    }

    fn unit(self) -> [f64; 4] {
        match self {
            Self::FirstU => [1.0, 0.0, 0.0, 0.0],
            Self::FirstV => [0.0, 1.0, 0.0, 0.0],
            Self::SecondU => [0.0, 0.0, 1.0, 0.0],
            Self::SecondV => [0.0, 0.0, 0.0, 1.0],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Constraint {
    Free,
    Plane { point: Point3, normal: Vector3 },
    Parameter { coordinate: Coordinate, value: f64 },
}

struct System {
    rows: [[f64; 4]; 4],
    values: [f64; 4],
    first: SurfaceDerivatives,
    second: SurfaceDerivatives,
}

impl System {
    fn merit(&self) -> f64 {
        self.values.iter().map(|value| value * value).sum()
    }

    fn gap(&self) -> f64 {
        self.first.point.distance(self.second.point)
    }

    fn constraint_error(&self) -> f64 {
        let [_, _, _, constraint] = self.values;
        constraint.abs()
    }
}

fn clamp_parameter(surface: &Surface, value: f64, along_u: bool) -> f64 {
    match (along_u, surface.u_period(), surface.v_period()) {
        (true, Some(_), _) | (false, _, Some(_)) => value,
        (true, None, _) => surface.u_domain().clamp(value),
        (false, _, None) => surface.v_domain().clamp(value),
    }
}

fn clamp_state(surfaces: [&Surface; 2], state: [f64; 4]) -> [f64; 4] {
    let [first, second] = surfaces;
    let [u1, v1, u2, v2] = state;
    [
        clamp_parameter(first, u1, true),
        clamp_parameter(first, v1, false),
        clamp_parameter(second, u2, true),
        clamp_parameter(second, v2, false),
    ]
}

fn nearest_turn(value: f64, reference: f64, period: Option<f64>) -> f64 {
    match period {
        Some(period) if period > 0.0 && value.is_finite() && reference.is_finite() => {
            value - ((value - reference) / period).round() * period
        }
        _ => value,
    }
}

fn near_start(
    surfaces: [&Surface; 2],
    state: [f64; 4],
    start: [f64; 4],
    constraint: Constraint,
) -> [f64; 4] {
    let [first, second] = surfaces;
    let periods = [
        first.u_period(),
        first.v_period(),
        second.u_period(),
        second.v_period(),
    ];
    let held = match constraint {
        Constraint::Parameter { coordinate, .. } => Some(coordinate),
        Constraint::Free | Constraint::Plane { .. } => None,
    };
    let mut wrapped = state;
    for (((value, reference), period), coordinate) in
        wrapped.iter_mut().zip(start).zip(periods).zip([
            Coordinate::FirstU,
            Coordinate::FirstV,
            Coordinate::SecondU,
            Coordinate::SecondV,
        ])
    {
        if held != Some(coordinate) {
            *value = nearest_turn(*value, reference, period);
        }
    }
    wrapped
}

fn speed_of(
    coordinate: Coordinate,
    first: &SurfaceDerivatives,
    second: &SurfaceDerivatives,
) -> f64 {
    let speed = match coordinate {
        Coordinate::FirstU => first.du.length(),
        Coordinate::FirstV => first.dv.length(),
        Coordinate::SecondU => second.du.length(),
        Coordinate::SecondV => second.dv.length(),
    };
    if speed.is_finite() && speed > TINY_SPEED {
        speed
    } else {
        1.0
    }
}

fn system(surfaces: [&Surface; 2], state: [f64; 4], constraint: Constraint) -> System {
    let [a, b] = surfaces;
    let [u1, v1, u2, v2] = state;
    let first = a.evaluate(u1, v1);
    let second = b.evaluate(u2, v2);
    let gap = first.point - second.point;
    let (constraint_row, constraint_value) = match constraint {
        Constraint::Free => ([0.0; 4], 0.0),
        Constraint::Plane { point, normal } => {
            let middle = first.point.lerp(second.point, 0.5);
            (
                [
                    0.5 * first.du.dot(normal),
                    0.5 * first.dv.dot(normal),
                    0.5 * second.du.dot(normal),
                    0.5 * second.dv.dot(normal),
                ],
                (middle - point).dot(normal),
            )
        }
        Constraint::Parameter { coordinate, value } => {
            let speed = speed_of(coordinate, &first, &second);
            (
                coordinate.unit().map(|entry| entry * speed),
                (coordinate.read(state) - value) * speed,
            )
        }
    };
    let rows = [
        [first.du.x, first.dv.x, -second.du.x, -second.dv.x],
        [first.du.y, first.dv.y, -second.du.y, -second.dv.y],
        [first.du.z, first.dv.z, -second.du.z, -second.dv.z],
        constraint_row,
    ];
    System {
        rows,
        values: [gap.x, gap.y, gap.z, constraint_value],
        first,
        second,
    }
}

fn minimal_norm_step(rows: &[[f64; 4]; 4], values: &[f64; 4]) -> Option<[f64; 4]> {
    let rows: Vec<Vec<f64>> = rows.iter().map(|row| row.to_vec()).collect();
    match damped_least_squares(&rows, values)?.as_slice() {
        [a, b, c, d] => Some([*a, *b, *c, *d]),
        _ => None,
    }
}

fn add(state: [f64; 4], step: [f64; 4], scale: f64) -> [f64; 4] {
    let [a, b, c, d] = state;
    let [e, f, g, h] = step;
    [a + e * scale, b + f * scale, c + g * scale, d + h * scale]
}

fn state_of(uv: [Point2; 2]) -> [f64; 4] {
    let [first, second] = uv;
    [first.x, first.y, second.x, second.y]
}

fn uv_of(state: [f64; 4]) -> [Point2; 2] {
    let [u1, v1, u2, v2] = state;
    [Point2::new(u1, v1), Point2::new(u2, v2)]
}

fn converged(current: &System) -> bool {
    let scale = 1.0 + current.first.point.abs().max_element();
    current.gap() <= NOISE_FLOOR * scale && current.constraint_error() <= NOISE_FLOOR * scale
}

pub(crate) fn refine_contact(
    surfaces: [&Surface; 2],
    start: [Point2; 2],
    constraint: Constraint,
) -> Option<Contact> {
    let mut state = clamp_state(surfaces, state_of(start));
    if !state.iter().all(|value| value.is_finite()) {
        return None;
    }
    let origin = state;
    let mut current = system(surfaces, state, constraint);
    for _ in 0..MAX_CONTACT_ITERATIONS {
        if converged(&current) {
            break;
        }
        let Some(step) = minimal_norm_step(&current.rows, &current.values) else {
            break;
        };
        let mut scale = 1.0;
        let mut accepted = None;
        for _ in 0..MAX_HALVINGS {
            let candidate = near_start(
                surfaces,
                clamp_state(surfaces, add(state, step, scale)),
                origin,
                constraint,
            );
            let next = system(surfaces, candidate, constraint);
            if next.merit().is_finite() && next.merit() < current.merit() {
                accepted = Some((candidate, next));
                break;
            }
            scale *= 0.5;
        }
        let Some((next_state, next)) = accepted else {
            break;
        };
        let stagnant =
            scale < STAGNANT_SCALE && next.merit() > (1.0 - STAGNANT_GAIN) * current.merit();
        let moved = next_state
            .iter()
            .zip(state)
            .map(|(after, before)| (after - before).abs() / (1.0 + before.abs()))
            .fold(0.0, f64::max);
        state = next_state;
        current = next;
        if moved <= STEP_EPSILON || stagnant {
            break;
        }
    }
    let accepted = current.gap() <= CONTACT_ACCEPT && current.constraint_error() <= CONTACT_ACCEPT;
    accepted.then(|| Contact {
        uv: uv_of(state),
        point: current.first.point.lerp(current.second.point, 0.5),
    })
}

fn carried(
    surfaces: [&Surface; 2],
    start: [Point2; 2],
    carrier_first: bool,
    point: Point3,
    normal: Vector3,
) -> Option<Contact> {
    let [first, second] = surfaces;
    let [first_uv, second_uv] = start;
    let (carrier, other, mut uv, hint) = if carrier_first {
        (first, second, first_uv, second_uv)
    } else {
        (second, first, second_uv, first_uv)
    };
    let clamp = |uv: Point2| {
        Point2::new(
            clamp_parameter(carrier, uv.x, true),
            clamp_parameter(carrier, uv.y, false),
        )
    };
    let residual = |uv: Point2, hint: Point2| {
        let derivatives = carrier.evaluate(uv.x, uv.y);
        let other_uv = other.project(derivatives.point, Some(hint));
        let foot = other.point_at(other_uv);
        let other_normal = other.normal(other_uv.x, other_uv.y)?;
        Some((
            derivatives,
            other_uv,
            (derivatives.point - foot).dot(other_normal),
            (derivatives.point - point).dot(normal),
            other_normal,
        ))
    };
    uv = clamp(uv);
    let (mut derivatives, mut other_uv, mut g1, mut g2, mut other_normal) = residual(uv, hint)?;
    for _ in 0..MAX_CONTACT_ITERATIONS {
        let merit = g1 * g1 + g2 * g2;
        let scale = 1.0 + derivatives.point.abs().max_element();
        if merit.sqrt() <= NOISE_FLOOR * scale {
            break;
        }
        let (a, b) = (
            derivatives.du.dot(other_normal),
            derivatives.dv.dot(other_normal),
        );
        let (c, d) = (derivatives.du.dot(normal), derivatives.dv.dot(normal));
        let determinant = a * d - b * c;
        if determinant.abs() <= f64::MIN_POSITIVE || !determinant.is_finite() {
            break;
        }
        let step = Vector2::new(
            -(d * g1 - b * g2) / determinant,
            -(a * g2 - c * g1) / determinant,
        );
        let mut factor = 1.0;
        let mut accepted = None;
        for _ in 0..MAX_HALVINGS {
            let candidate = clamp(uv + step * factor);
            if let Some(next) = residual(candidate, other_uv)
                && next.2 * next.2 + next.3 * next.3 < merit
            {
                accepted = Some((candidate, next));
                break;
            }
            factor *= 0.5;
        }
        let Some((next_uv, next)) = accepted else {
            break;
        };
        uv = next_uv;
        (derivatives, other_uv, g1, g2, other_normal) = next;
    }
    let gap = derivatives.point.distance(other.point_at(other_uv));
    if gap > CONTACT_ACCEPT || g2.abs() > CONTACT_ACCEPT {
        return None;
    }
    Some(Contact {
        uv: if carrier_first {
            [uv, other_uv]
        } else {
            [other_uv, uv]
        },
        point: derivatives.point,
    })
}

pub(crate) fn refine_on_plane(
    surfaces: [&Surface; 2],
    start: [Point2; 2],
    point: Point3,
    normal: Vector3,
) -> Option<Contact> {
    refine_contact(surfaces, start, Constraint::Plane { point, normal })
        .or_else(|| carried_on_plane(surfaces, start, point, normal))
}

pub(crate) fn carried_on_plane(
    surfaces: [&Surface; 2],
    start: [Point2; 2],
    point: Point3,
    normal: Vector3,
) -> Option<Contact> {
    carried(surfaces, start, true, point, normal)
        .or_else(|| carried(surfaces, start, false, point, normal))
}

pub(crate) fn closest_approach(surfaces: [&Surface; 2], start: [Point2; 2]) -> ([Point2; 2], f64) {
    let mut state = clamp_state(surfaces, state_of(start));
    let origin = state;
    let mut current = system(surfaces, state, Constraint::Free);
    for _ in 0..MAX_CONTACT_ITERATIONS {
        if converged(&current) {
            break;
        }
        let Some(step) = minimal_norm_step(&current.rows, &current.values) else {
            break;
        };
        let mut scale = 1.0;
        let mut accepted = None;
        for _ in 0..MAX_HALVINGS {
            let candidate = near_start(
                surfaces,
                clamp_state(surfaces, add(state, step, scale)),
                origin,
                Constraint::Free,
            );
            let next = system(surfaces, candidate, Constraint::Free);
            if next.merit().is_finite() && next.merit() < current.merit() {
                accepted = Some((candidate, next));
                break;
            }
            scale *= 0.5;
        }
        let Some((next_state, next)) = accepted else {
            break;
        };
        let stagnant =
            scale < STAGNANT_SCALE && next.merit() > (1.0 - STAGNANT_GAIN) * current.merit();
        state = next_state;
        current = next;
        if stagnant {
            break;
        }
    }
    (uv_of(state), current.gap())
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Touch {
    pub point: Point3,
    pub gap: f64,
}

struct Parallel {
    rows: [[f64; 4]; 4],
    values: [f64; 4],
    first: SurfaceDerivatives,
    second: SurfaceDerivatives,
}

impl Parallel {
    fn merit(&self) -> f64 {
        self.values.iter().map(|value| value * value).sum()
    }

    fn touch(&self) -> Touch {
        Touch {
            point: self.first.point.lerp(self.second.point, 0.5),
            gap: self.first.point.distance(self.second.point),
        }
    }
}

fn parallel_system(surfaces: [&Surface; 2], state: [f64; 4]) -> Parallel {
    let [a, b] = surfaces;
    let [u1, v1, u2, v2] = state;
    let first = a.evaluate(u1, v1);
    let second = b.evaluate(u2, v2);
    let gap = first.point - second.point;
    let normal = second.du.cross(second.dv);
    let normal_u = second.duu.cross(second.dv) + second.du.cross(second.duv);
    let normal_v = second.duv.cross(second.dv) + second.du.cross(second.dvv);
    let rows = [
        [
            first.du.dot(second.du),
            first.dv.dot(second.du),
            gap.dot(second.duu) - second.du.dot(second.du),
            gap.dot(second.duv) - second.dv.dot(second.du),
        ],
        [
            first.du.dot(second.dv),
            first.dv.dot(second.dv),
            gap.dot(second.duv) - second.du.dot(second.dv),
            gap.dot(second.dvv) - second.dv.dot(second.dv),
        ],
        [
            normal.dot(first.duu),
            normal.dot(first.duv),
            normal_u.dot(first.du),
            normal_v.dot(first.du),
        ],
        [
            normal.dot(first.duv),
            normal.dot(first.dvv),
            normal_u.dot(first.dv),
            normal_v.dot(first.dv),
        ],
    ];
    let values = [
        gap.dot(second.du),
        gap.dot(second.dv),
        normal.dot(first.du),
        normal.dot(first.dv),
    ];
    Parallel {
        rows,
        values,
        first,
        second,
    }
}

pub(crate) fn parallel_normals(surfaces: [&Surface; 2], start: [Point2; 2]) -> Option<Touch> {
    let mut state = clamp_state(surfaces, state_of(start));
    if !state.iter().all(|value| value.is_finite()) {
        return None;
    }
    let origin = state;
    let mut current = parallel_system(surfaces, state);
    for _ in 0..MAX_CONTACT_ITERATIONS {
        let Some(step) = minimal_norm_step(&current.rows, &current.values) else {
            break;
        };
        let mut scale = 1.0;
        let mut accepted = None;
        for _ in 0..MAX_HALVINGS {
            let candidate = near_start(
                surfaces,
                clamp_state(surfaces, add(state, step, scale)),
                origin,
                Constraint::Free,
            );
            let next = parallel_system(surfaces, candidate);
            if next.merit().is_finite() && next.merit() < current.merit() {
                accepted = Some((candidate, next));
                break;
            }
            scale *= 0.5;
        }
        let Some((next_state, next)) = accepted else {
            break;
        };
        let moved = next_state
            .iter()
            .zip(state)
            .map(|(after, before)| (after - before).abs() / (1.0 + before.abs()))
            .fold(0.0, f64::max);
        state = next_state;
        current = next;
        if moved <= STEP_EPSILON {
            break;
        }
    }
    let [first_normal, second_normal] = [
        current.first.du.cross(current.first.dv).try_normalize()?,
        current.second.du.cross(current.second.dv).try_normalize()?,
    ];
    let along = (current.first.point - current.second.point)
        .reject_from_normalized(second_normal)
        .length();
    let parallel = first_normal.cross(second_normal).length() <= PARALLEL_SINE;
    (parallel && along <= LINEAR_RESOLUTION).then(|| current.touch())
}

pub(crate) fn contact_normals(surfaces: [&Surface; 2], contact: &Contact) -> Option<[Vector3; 2]> {
    let [a, b] = surfaces;
    let [first, second] = contact.uv;
    Some([a.normal(first.x, first.y)?, b.normal(second.x, second.y)?])
}

pub(crate) fn contact_direction(
    surfaces: [&Surface; 2],
    contact: &Contact,
) -> Option<(Vector3, f64)> {
    let [first, second] = contact_normals(surfaces, contact)?;
    let across = first.cross(second);
    let sine = across.length();
    across.try_normalize().map(|direction| (direction, sine))
}

pub(crate) fn uv_direction(derivatives: &SurfaceDerivatives, direction: Vector3) -> Vector2 {
    let (a, b, c) = (
        derivatives.du.dot(derivatives.du),
        derivatives.du.dot(derivatives.dv),
        derivatives.dv.dot(derivatives.dv),
    );
    let (first, second) = (derivatives.du.dot(direction), derivatives.dv.dot(direction));
    let determinant = a * c - b * b;
    if determinant > 1e-14 * (a * c).max(f64::MIN_POSITIVE) && determinant.is_finite() {
        Vector2::new(
            (c * first - b * second) / determinant,
            (a * second - b * first) / determinant,
        )
    } else if a > TINY_SPEED && a >= c {
        Vector2::new(first / a, 0.0)
    } else if c > TINY_SPEED {
        Vector2::new(0.0, second / c)
    } else {
        Vector2::ZERO
    }
}

pub(crate) fn bracket_root(
    function: impl Fn(f64) -> f64,
    mut low: f64,
    mut high: f64,
    mut low_value: f64,
    mut high_value: f64,
) -> f64 {
    for iteration in 0..MAX_BRACKET_ITERATIONS {
        if low_value == 0.0 {
            return low;
        }
        if high_value == 0.0 {
            return high;
        }
        let width = high - low;
        if width.abs() <= 4.0 * f64::EPSILON * (low.abs() + high.abs()) + f64::MIN_POSITIVE {
            break;
        }
        let secant = high - high_value * width / (high_value - low_value);
        let lower = low.min(high);
        let upper = low.max(high);
        let next = if iteration % 2 == 0 && secant > lower && secant < upper {
            secant
        } else {
            low + 0.5 * width
        };
        if next == low || next == high {
            break;
        }
        let value = function(next);
        if !value.is_finite() {
            break;
        }
        if (value < 0.0) == (low_value < 0.0) {
            low = next;
            low_value = value;
        } else {
            high = next;
            high_value = value;
        }
    }
    if low_value.abs() <= high_value.abs() {
        low
    } else {
        high
    }
}

#[derive(Debug, Clone, Copy)]
struct Sample {
    at: f64,
    value: f64,
}

impl Sample {
    fn below(self, other: Self) -> bool {
        self.value <= other.value
    }
}

fn parabolic_step(best: Sample, second: Sample, third: Sample) -> (f64, f64) {
    let towards_second = (best.at - second.at) * (best.value - third.value);
    let towards_third = (best.at - third.at) * (best.value - second.value);
    let numerator = (best.at - third.at) * towards_third - (best.at - second.at) * towards_second;
    let denominator = 2.0 * (towards_third - towards_second);
    if denominator > 0.0 {
        (-numerator, denominator)
    } else {
        (numerator, -denominator)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Bracket {
    pub lower: f64,
    pub upper: f64,
    pub at: f64,
    pub value: f64,
}

pub(crate) fn minimize_bracket(
    function: impl Fn(f64) -> f64,
    low: f64,
    high: f64,
    settled: impl Fn(Bracket) -> bool,
) -> (f64, f64) {
    let (mut lower, mut upper) = (low.min(high), low.max(high));
    let sample = |at: f64| Sample {
        at,
        value: function(at),
    };
    let ends = [sample(lower), sample(upper)];
    let mut best = sample(lower + SHRINK * (upper - lower));
    let (mut second, mut third) = (best, best);
    let (mut step, mut previous) = (0.0_f64, 0.0_f64);

    for _ in 0..MAX_MINIMUM_ITERATIONS {
        let middle = 0.5 * (lower + upper);
        let tolerance = MINIMUM_PRECISION * best.at.abs() + MINIMUM_FLOOR;
        let bracket = Bracket {
            lower,
            upper,
            at: best.at,
            value: best.value,
        };
        if (best.at - middle).abs() <= 2.0 * tolerance - 0.5 * (upper - lower) || settled(bracket) {
            break;
        }

        let earlier = previous;
        previous = step;
        let (numerator, denominator) = parabolic_step(best, second, third);
        let parabolic = earlier.abs() > tolerance
            && numerator.abs() < (0.5 * denominator * earlier).abs()
            && numerator > denominator * (lower - best.at)
            && numerator < denominator * (upper - best.at);
        if parabolic {
            step = numerator / denominator;
            let landing = best.at + step;
            if landing - lower < 2.0 * tolerance || upper - landing < 2.0 * tolerance {
                step = tolerance.copysign(middle - best.at);
            }
        } else {
            previous = if best.at >= middle {
                lower - best.at
            } else {
                upper - best.at
            };
            step = SHRINK * previous;
        }

        let reach = if step.abs() >= tolerance {
            step
        } else {
            tolerance.copysign(step)
        };
        let next = sample(best.at + reach);
        if next.below(best) {
            if next.at >= best.at {
                lower = best.at;
            } else {
                upper = best.at;
            }
            (third, second, best) = (second, best, next);
        } else {
            if next.at < best.at {
                lower = next.at;
            } else {
                upper = next.at;
            }
            if next.below(second) || second.at == best.at {
                (third, second) = (second, next);
            } else if next.below(third) || third.at == best.at || third.at == second.at {
                third = next;
            }
        }
    }

    ends.into_iter()
        .chain([best])
        .filter(|sample| !sample.value.is_nan())
        .min_by(|x, y| x.value.total_cmp(&y.value))
        .map_or((lower, f64::INFINITY), |sample| (sample.at, sample.value))
}

pub(crate) fn evaluate_polynomial(coefficients: &[f64], at: f64) -> f64 {
    coefficients
        .iter()
        .rev()
        .fold(0.0, |sum, coefficient| sum * at + coefficient)
}

fn derivative(coefficients: &[f64]) -> Vec<f64> {
    coefficients
        .iter()
        .enumerate()
        .skip(1)
        .map(|(power, coefficient)| coefficient * power as f64)
        .collect()
}

fn trimmed(coefficients: &[f64]) -> Vec<f64> {
    let largest = coefficients.iter().fold(0.0f64, |largest, coefficient| {
        largest.max(coefficient.abs())
    });
    let mut kept = coefficients.to_vec();
    while kept
        .last()
        .is_some_and(|coefficient| coefficient.abs() <= COEFFICIENT_FLOOR * largest)
    {
        kept.pop();
    }
    kept
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct PolynomialRoots {
    pub roots: Vec<f64>,
    pub extrema: Vec<f64>,
}

pub(crate) fn polynomial_roots(coefficients: &[f64], low: f64, high: f64) -> PolynomialRoots {
    let coefficients = trimmed(coefficients);
    PolynomialRoots {
        roots: roots_between(&coefficients, low, high),
        extrema: roots_between(&trimmed(&derivative(&coefficients)), low, high),
    }
}

fn roots_between(coefficients: &[f64], low: f64, high: f64) -> Vec<f64> {
    match coefficients {
        [] | [_] => Vec::new(),
        [constant, slope] => {
            let root = -constant / slope;
            if root >= low && root <= high {
                vec![root]
            } else {
                Vec::new()
            }
        }
        _ => {
            let mut knots = vec![low];
            knots.extend(roots_between(
                &trimmed(&derivative(coefficients)),
                low,
                high,
            ));
            knots.push(high);
            let value = |at: f64| evaluate_polynomial(coefficients, at);
            let mut roots: Vec<f64> = Vec::new();
            for pair in knots.windows(2) {
                let [start, end] = pair else {
                    continue;
                };
                let (start_value, end_value) = (value(*start), value(*end));
                if start_value == 0.0 {
                    roots.push(*start);
                } else if (start_value < 0.0) != (end_value < 0.0) && end_value != 0.0 {
                    roots.push(bracket_root(value, *start, *end, start_value, end_value));
                }
            }
            if value(high) == 0.0 {
                roots.push(high);
            }
            roots.dedup();
            roots
        }
    }
}

#[cfg(test)]
mod tests {
    use caditor_geometry::Plane;

    use super::*;
    use crate::surface::{Cylinder, PlaneSurface, Sphere};

    #[test]
    fn polynomial_roots_are_isolated_through_derivatives() {
        let quartic = [24.0, -50.0, 35.0, -10.0, 1.0];
        let found = polynomial_roots(&quartic, -10.0, 10.0);
        let expected = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(found.roots.len(), 4);
        for (root, exact) in found.roots.iter().zip(expected) {
            assert!((root - exact).abs() < 1e-12);
        }
        assert_eq!(found.extrema.len(), 3);
        let double = polynomial_roots(&[1.0, -2.0, 1.0], -5.0, 5.0);
        assert!(double.roots.len() <= 1);
        assert!((double.extrema[0] - 1.0).abs() < 1e-15);
        assert!(
            polynomial_roots(&[1.0, 0.0, 1.0], -5.0, 5.0)
                .roots
                .is_empty()
        );
    }

    #[test]
    fn brackets_and_minima_converge() {
        let root = bracket_root(|x| x * x - 2.0, 0.0, 2.0, -2.0, 2.0);
        assert!((root - 2f64.sqrt()).abs() < 1e-14);
        let (at, value) = minimize_bracket(|x| (x - 0.3).powi(2) + 1.0, -1.0, 2.0, |_| false);
        assert!((at - 0.3).abs() < 1e-7);
        assert!((value - 1.0).abs() < 1e-14);
    }

    #[test]
    fn contacts_refine_onto_both_surfaces() {
        let plane: Surface = PlaneSurface::new(Plane::XY).unwrap().into();
        let sphere: Surface = Sphere::new(Plane::XY, 2.0).unwrap().into();
        let contact = refine_contact(
            [&plane, &sphere],
            [Point2::new(1.5, 0.4), Point2::new(0.3, 0.2)],
            Constraint::Free,
        )
        .unwrap();
        assert!(contact.point.z.abs() < 1e-12);
        assert!((contact.point.length() - 2.0).abs() < 1e-12);
        let cylinder: Surface = Cylinder::new(Plane::XY, 1.0).unwrap().into();
        let fixed = refine_contact(
            [&plane, &cylinder],
            [Point2::new(0.9, 0.1), Point2::new(0.2, 0.0)],
            Constraint::Parameter {
                coordinate: Coordinate::SecondU,
                value: 0.5,
            },
        )
        .unwrap();
        assert!((fixed.uv[1].x - 0.5).abs() < 1e-12);
        assert!((fixed.point.x - 0.5f64.cos()).abs() < 1e-12);
        let (_, gap) = closest_approach(
            [
                &plane,
                &Sphere::new(Plane::XY.flipped(), 1.0).unwrap().into(),
            ],
            [Point2::new(0.1, 0.1), Point2::new(0.0, 0.1)],
        );
        assert!(gap < 1e-12);
    }
}
