use std::{
    cell::Cell,
    f64::consts::{PI, TAU},
};

use caditor_geometry::{Aabb2, Plane, Point2, Point3};

use crate::{
    coordinates::angle_between,
    curve::Curve,
    interrupt,
    intersect::{
        IntersectionError, SurfacePatch, boxes_overlap, guided_intervals,
        solve::{bracket_root, minimize_bracket, polynomial_roots, uv_direction},
    },
    interval::Interval,
    surface::{Surface, refine_projection},
    tolerance::{LINEAR_RESOLUTION, parallel},
};

const TOLERANCE: f64 = LINEAR_RESOLUTION;
const ON_SURFACE: f64 = 0.1 * LINEAR_RESOLUTION;
const CONSTANT_VARIATION: f64 = 0.01 * LINEAR_RESOLUTION;
const TANGENT_SINE: f64 = 1e-7;
const OVERLAP_SINE: f64 = 1e-6;
const FOOT_SINE: f64 = 1e-6;
const OVERLAP_CURVATURE: f64 = 1e-6;
const MIN_OVERLAP: f64 = 10.0 * LINEAR_RESOLUTION;
const LEAF_TURN: f64 = 0.25;
const LEAF_CURVATURE: f64 = 0.25;
const TINY_LEAF: f64 = 1e3 * LINEAR_RESOLUTION;
const MAX_PIECES: usize = 1 << 14;
const MAX_DEPTH: usize = 48;
const LEAF_SAMPLES: usize = 5;
const PRECHECK_SAMPLES: usize = 16;
const CONNECTION_SAMPLES: [f64; 3] = [0.25, 0.5, 0.75];
const OVERLAP_SAMPLES: usize = 8;
const CLIP_SAMPLES: usize = 32;
const POLISH_ITERATIONS: usize = 4;
const MAX_PERIODIC_HITS: usize = 8;
const EXTENSION_BISECTIONS: usize = 60;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurveSurfacePoint {
    pub parameter: f64,
    pub point: Point3,
    pub uv: Point2,
    pub tangent: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurveSurfaceOverlap {
    pub range: Interval,
    pub start_uv: Point2,
    pub end_uv: Point2,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CurveSurfaceIntersection {
    pub points: Vec<CurveSurfacePoint>,
    pub overlaps: Vec<CurveSurfaceOverlap>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Candidate {
    Root(f64),
    Touch(f64),
    Along(Interval),
    End(f64),
}

impl Candidate {
    fn low(&self) -> f64 {
        match self {
            Self::Root(at) | Self::Touch(at) | Self::End(at) => *at,
            Self::Along(range) => range.start(),
        }
    }

    fn high(&self) -> f64 {
        match self {
            Self::Root(at) | Self::Touch(at) | Self::End(at) => *at,
            Self::Along(range) => range.end(),
        }
    }
}

pub(crate) struct Probe<'a> {
    pub curve: &'a Curve,
    pub surface: &'a Surface,
    pub hint: Cell<Option<Point2>>,
    pub local: Cell<bool>,
}

impl Probe<'_> {
    fn new<'a>(curve: &'a Curve, surface: &'a Surface) -> Probe<'a> {
        Probe {
            curve,
            surface,
            hint: Cell::new(None),
            local: Cell::new(false),
        }
    }

    fn project(&self, point: Point3) -> Point2 {
        let swept = self.local.get()
            && matches!(self.surface, Surface::Extrusion(_) | Surface::Revolution(_));
        let uv = match self.hint.get().filter(|_| swept) {
            Some(hint) => {
                let local = refine_projection(self.surface, point, hint);
                if local.is_finite() && self.is_foot(point, local) {
                    local
                } else {
                    self.surface.project(point, None)
                }
            }
            None => self.surface.project(point, None),
        };
        if swept {
            self.hint.set(Some(uv));
        }
        uv
    }

    fn is_foot(&self, point: Point3, uv: Point2) -> bool {
        let offset = point - self.surface.point_at(uv);
        if offset.length() <= TOLERANCE {
            return true;
        }
        match (self.surface.normal(uv.x, uv.y), offset.try_normalize()) {
            (Some(normal), Some(direction)) => direction.cross(normal).length() <= FOOT_SINE,
            _ => false,
        }
    }

    fn forget(&self) {
        self.hint.set(None);
    }

    fn offset(&self, parameter: f64) -> (f64, f64) {
        let point = self.curve.point(parameter);
        let uv = self.project(point);
        let offset = point - self.surface.point_at(uv);
        let distance = offset.length();
        let signed = match self.surface.normal(uv.x, uv.y) {
            Some(normal) => offset.dot(normal),
            None => distance,
        };
        (signed, distance)
    }

    pub(crate) fn signed(&self, parameter: f64) -> f64 {
        self.offset(parameter).0
    }

    pub(crate) fn distance(&self, parameter: f64) -> f64 {
        self.offset(parameter).1
    }

    fn sine(&self, parameter: f64) -> f64 {
        let derivatives = self.curve.evaluate(parameter);
        let uv = self.surface.project(derivatives.point, None);
        match (
            self.surface.normal(uv.x, uv.y),
            derivatives.first.try_normalize(),
        ) {
            (Some(normal), Some(tangent)) => tangent.dot(normal).abs(),
            _ => 1.0,
        }
    }

    fn relative_curvature(&self, parameter: f64) -> f64 {
        let derivatives = self.curve.evaluate(parameter);
        let uv = self.surface.project(derivatives.point, None);
        let surface = self.surface.evaluate(uv.x, uv.y);
        let Some(normal) = self.surface.normal(uv.x, uv.y) else {
            return f64::INFINITY;
        };
        let speed = derivatives.first.length_squared();
        if speed <= f64::MIN_POSITIVE {
            return f64::INFINITY;
        }
        let along = uv_direction(&surface, derivatives.first);
        let second_form = (surface.duu * (along.x * along.x)
            + surface.duv * (2.0 * along.x * along.y)
            + surface.dvv * (along.y * along.y))
            .dot(normal);
        ((derivatives.second.dot(normal) - second_form) / speed).abs()
    }

    fn surface_curvature(&self, point: Point3) -> f64 {
        let uv = self.surface.project(point, None);
        let derivatives = self.surface.evaluate(uv.x, uv.y);
        let Some(normal) = self.surface.normal(uv.x, uv.y) else {
            return 0.0;
        };
        let (su, sv) = (derivatives.du.length(), derivatives.dv.length());
        let ratio = |value: f64, scale: f64| {
            if scale > f64::MIN_POSITIVE {
                value.abs() / scale
            } else {
                0.0
            }
        };
        ratio(derivatives.duu.dot(normal), su * su)
            .max(ratio(derivatives.dvv.dot(normal), sv * sv))
            .max(ratio(derivatives.duv.dot(normal), su * sv))
    }

    fn polish(&self, parameter: f64, range: Interval) -> f64 {
        let mut best = (parameter, self.signed(parameter).abs());
        let mut at = parameter;
        for _ in 0..POLISH_ITERATIONS {
            let derivatives = self.curve.evaluate(at);
            let uv = self.surface.project(derivatives.point, None);
            let Some(normal) = self.surface.normal(uv.x, uv.y) else {
                break;
            };
            let slope = derivatives.first.dot(normal);
            let value = (derivatives.point - self.surface.point_at(uv)).dot(normal);
            if slope.abs() <= f64::MIN_POSITIVE {
                break;
            }
            at = range.clamp(at - value / slope);
            let error = self.signed(at).abs();
            if error < best.1 {
                best = (at, error);
            } else {
                break;
            }
        }
        best.0
    }
}

pub fn intersect_curve_surface(
    curve: &Curve,
    range: Interval,
    surface: &Surface,
    bounds: Option<Aabb2>,
) -> Result<CurveSurfaceIntersection, IntersectionError> {
    let patch = bounds
        .map(|bounds| SurfacePatch::new(surface, bounds))
        .transpose()?;
    let range = match curve.period() {
        Some(period) if range.length() > period => {
            Interval::new(range.start(), range.start() + period).unwrap_or(range)
        }
        _ => range,
    };
    let probe = Probe::new(curve, surface);
    let candidates = if lies_on_own_surface(curve, surface) {
        vec![Candidate::Along(range)]
    } else {
        match analytic(&probe, range) {
            Some(candidates) => candidates,
            None => general(&probe, range, patch.as_ref())?,
        }
    };
    Ok(finish(&probe, range, patch.as_ref(), candidates))
}

fn lies_on_own_surface(curve: &Curve, surface: &Surface) -> bool {
    match curve {
        Curve::Intersection(intersection) => intersection
            .surfaces()
            .iter()
            .any(|own| own.same_surface(surface).is_some()),
        _ => false,
    }
}

fn periodic_in(angle: f64, range: Interval) -> Vec<f64> {
    let first = angle + ((range.start() - angle) / TAU).ceil() * TAU;
    (0..MAX_PERIODIC_HITS)
        .map(|turn| first + turn as f64 * TAU)
        .take_while(|value| *value <= range.end())
        .filter(|value| value.is_finite())
        .collect()
}

fn implicit(probe: &Probe, range: Interval, coefficients: &[f64]) -> Vec<Candidate> {
    let found = polynomial_roots(coefficients, range.start(), range.end());
    let mut candidates: Vec<Candidate> = found
        .roots
        .into_iter()
        .map(|root| probe.polish(root, range))
        .filter(|root| probe.distance(*root) <= TOLERANCE)
        .map(Candidate::Root)
        .collect();
    candidates.extend(
        found
            .extrema
            .into_iter()
            .filter(|extremum| probe.distance(*extremum) <= TOLERANCE)
            .map(Candidate::Touch),
    );
    candidates
}

fn trigonometric(
    probe: &Probe,
    range: Interval,
    [constant, cosine, sine]: [f64; 3],
    scale: f64,
) -> Vec<Candidate> {
    let amplitude = cosine.hypot(sine);
    if amplitude / scale <= CONSTANT_VARIATION {
        return if probe.distance(range.middle()) <= TOLERANCE {
            vec![Candidate::Along(range)]
        } else {
            Vec::new()
        };
    }
    let phase = sine.atan2(cosine);
    let mut candidates: Vec<Candidate> = [phase, phase + PI]
        .into_iter()
        .flat_map(|extremum| periodic_in(extremum, range))
        .filter(|extremum| probe.distance(*extremum) <= TOLERANCE)
        .map(Candidate::Touch)
        .collect();
    let ratio = -constant / amplitude;
    if ratio.abs() <= 1.0 {
        let spread = ratio.acos();
        candidates.extend(
            [phase + spread, phase - spread]
                .into_iter()
                .flat_map(|root| periodic_in(root, range))
                .map(|root| probe.polish(root, range))
                .filter(|root| probe.distance(*root) <= TOLERANCE)
                .map(Candidate::Root),
        );
    }
    candidates
}

fn coaxial(frame: &Plane, origin: Point3, axis: caditor_geometry::Vector3) -> bool {
    let offset = frame.origin() - origin;
    let off_axis = offset - axis * offset.dot(axis);
    parallel(frame.normal(), axis) && off_axis.length() <= TOLERANCE
}

fn analytic(probe: &Probe, range: Interval) -> Option<Vec<Candidate>> {
    match (probe.curve, probe.surface) {
        (Curve::Line(line), Surface::Plane(plane)) => {
            let frame = plane.frame();
            let rate = line.direction().dot(frame.normal());
            let height = frame.signed_distance(line.origin());
            Some(
                if rate.abs() * range.length().max(1.0) <= CONSTANT_VARIATION {
                    if probe.distance(range.middle()) <= TOLERANCE {
                        vec![Candidate::Along(range)]
                    } else {
                        Vec::new()
                    }
                } else {
                    let root = -height / rate;
                    if range.contains(root) {
                        vec![Candidate::Root(root)]
                    } else {
                        Vec::new()
                    }
                },
            )
        }
        (Curve::Line(line), Surface::Sphere(sphere)) => {
            let offset = line.origin() - sphere.center();
            let direction = line.direction();
            Some(implicit(
                probe,
                range,
                &[
                    offset.length_squared() - sphere.radius() * sphere.radius(),
                    2.0 * offset.dot(direction),
                    1.0,
                ],
            ))
        }
        (Curve::Line(line), Surface::Cylinder(cylinder)) => {
            let axis = cylinder.frame().normal();
            let across = |vector: caditor_geometry::Vector3| vector - axis * vector.dot(axis);
            let offset = across(line.origin() - cylinder.frame().origin());
            let direction = across(line.direction());
            if direction.length_squared() <= 1e-24 {
                return Some(if probe.distance(range.middle()) <= TOLERANCE {
                    vec![Candidate::Along(range)]
                } else {
                    Vec::new()
                });
            }
            Some(implicit(
                probe,
                range,
                &[
                    offset.length_squared() - cylinder.radius() * cylinder.radius(),
                    2.0 * offset.dot(direction),
                    direction.length_squared(),
                ],
            ))
        }
        (Curve::Line(line), Surface::Cone(cone)) => {
            let axis = cone.opening_direction();
            let cosine = cone.half_angle().cos();
            let cos2 = cosine * cosine;
            let offset = line.origin() - cone.apex();
            let direction = line.direction();
            let (oa, da) = (offset.dot(axis), direction.dot(axis));
            let coefficients = [
                oa * oa - cos2 * offset.length_squared(),
                2.0 * (oa * da - cos2 * offset.dot(direction)),
                da * da - cos2,
            ];
            let scale = 1.0 + offset.length_squared();
            if coefficients
                .iter()
                .all(|value| value.abs() <= 1e-15 * scale)
            {
                return Some(if probe.distance(range.middle()) <= TOLERANCE {
                    vec![Candidate::Along(range)]
                } else {
                    Vec::new()
                });
            }
            Some(implicit(probe, range, &coefficients))
        }
        (Curve::Line(line), Surface::Torus(torus)) => {
            let frame = torus.frame();
            let (major, minor) = (torus.major_radius(), torus.minor_radius());
            let offset = line.origin() - frame.origin();
            let direction = line.direction();
            let normal = frame.normal();
            let b = 2.0 * offset.dot(direction);
            let c = offset.length_squared() + major * major - minor * minor;
            let (on, dn) = (offset.dot(normal), direction.dot(normal));
            let four = 4.0 * major * major;
            let reach = major + minor + TOLERANCE;
            let middle = -0.5 * b;
            let closest = offset.length_squared() - middle * middle;
            if reach * reach < closest {
                return Some(Vec::new());
            }
            let half = (reach * reach - closest).max(0.0).sqrt();
            let (low, high) = (middle - half, middle + half);
            if high < range.start() || low > range.end() {
                return Some(Vec::new());
            }
            let near = Interval::new(range.clamp(low), range.clamp(high))?;
            Some(implicit(
                probe,
                near,
                &[
                    c * c - four * (offset.length_squared() - on * on),
                    2.0 * b * c - four * (b - 2.0 * on * dn),
                    b * b + 2.0 * c - four * (1.0 - dn * dn),
                    2.0 * b,
                    1.0,
                ],
            ))
        }
        (Curve::Circle(circle), Surface::Plane(plane)) => {
            let frame = plane.frame();
            let own = circle.frame();
            let radius = circle.radius();
            Some(trigonometric(
                probe,
                range,
                [
                    frame.signed_distance(own.origin()),
                    radius * own.x_axis().dot(frame.normal()),
                    radius * own.y_axis().dot(frame.normal()),
                ],
                1.0,
            ))
        }
        (Curve::Ellipse(ellipse), Surface::Plane(plane)) => {
            let frame = plane.frame();
            let own = ellipse.frame();
            Some(trigonometric(
                probe,
                range,
                [
                    frame.signed_distance(own.origin()),
                    ellipse.major_radius() * own.x_axis().dot(frame.normal()),
                    ellipse.minor_radius() * own.y_axis().dot(frame.normal()),
                ],
                1.0,
            ))
        }
        (Curve::Circle(circle), Surface::Sphere(sphere)) => {
            let own = circle.frame();
            let radius = circle.radius();
            let offset = own.origin() - sphere.center();
            Some(trigonometric(
                probe,
                range,
                [
                    offset.length_squared() + radius * radius - sphere.radius() * sphere.radius(),
                    2.0 * radius * offset.dot(own.x_axis()),
                    2.0 * radius * offset.dot(own.y_axis()),
                ],
                2.0 * sphere.radius(),
            ))
        }
        (Curve::Circle(circle), Surface::Cylinder(_) | Surface::Cone(_) | Surface::Torus(_)) => {
            let (origin, axis) = match probe.surface {
                Surface::Cylinder(cylinder) => {
                    (cylinder.frame().origin(), cylinder.frame().normal())
                }
                Surface::Cone(cone) => (cone.frame().origin(), cone.frame().normal()),
                Surface::Torus(torus) => (torus.frame().origin(), torus.frame().normal()),
                _ => return None,
            };
            coaxial(circle.frame(), origin, axis).then(|| {
                if probe.distance(range.start()) <= TOLERANCE {
                    vec![Candidate::Along(range)]
                } else {
                    Vec::new()
                }
            })
        }
        _ => None,
    }
}

fn is_leaf(probe: &Probe, piece: Interval, radius: f64) -> bool {
    if radius <= TINY_LEAF {
        return true;
    }
    let tangents = [piece.start(), piece.middle(), piece.end()]
        .map(|parameter| probe.curve.evaluate(parameter).first);
    let [start, middle, end] = tangents;
    let turn = angle_between(start, end)
        .max(angle_between(start, middle))
        .max(angle_between(middle, end));
    let center = probe.curve.point(piece.middle());
    turn <= LEAF_TURN && radius * probe.surface_curvature(center) <= LEAF_CURVATURE
}

fn general(
    probe: &Probe,
    range: Interval,
    patch: Option<&SurfacePatch>,
) -> Result<Vec<Candidate>, IntersectionError> {
    if range
        .split(PRECHECK_SAMPLES)
        .all(|parameter| probe.distance(parameter) <= ON_SURFACE)
    {
        return Ok(vec![Candidate::Along(range)]);
    }
    let patch_box = patch.map(SurfacePatch::bounding_box);
    let mut pending = vec![(range, 0usize)];
    let mut leaves = Vec::new();
    let mut visited = 0usize;
    while let Some((piece, depth)) = pending.pop() {
        interrupt::check()?;
        visited += 1;
        if visited > MAX_PIECES {
            return Err(IntersectionError::TooComplex(MAX_PIECES));
        }
        let bounds = probe.curve.piece_bounds(piece);
        if let Some(patch_box) = &patch_box
            && !boxes_overlap(&bounds, patch_box, TOLERANCE)
        {
            continue;
        }
        let radius = bounds.half_extent().length();
        if probe.surface.distance(bounds.center()) > radius + TOLERANCE {
            continue;
        }
        if depth >= MAX_DEPTH || is_leaf(probe, piece, radius) {
            leaves.push(piece);
            continue;
        }
        let middle = piece.middle();
        match (
            Interval::new(piece.start(), middle),
            Interval::new(middle, piece.end()),
        ) {
            (Some(low), Some(high)) if middle > piece.start() && middle < piece.end() => {
                pending.push((high, depth + 1));
                pending.push((low, depth + 1));
            }
            _ => leaves.push(piece),
        }
    }
    leaves.sort_by(|a, b| a.start().total_cmp(&b.start()));
    let candidates: Vec<Candidate> = leaves
        .into_iter()
        .flat_map(|leaf| solve_leaf(probe, leaf))
        .collect();
    Ok(candidates
        .into_iter()
        .filter(|candidate| match candidate {
            Candidate::Root(at) | Candidate::Touch(at) => probe.distance(*at) <= TOLERANCE,
            Candidate::Along(_) | Candidate::End(_) => true,
        })
        .collect())
}

fn solve_leaf(probe: &Probe, piece: Interval) -> Vec<Candidate> {
    probe.forget();
    probe.local.set(true);
    let candidates = solve_leaf_locally(probe, piece);
    probe.local.set(false);
    probe.forget();
    candidates
}

fn solve_leaf_locally(probe: &Probe, piece: Interval) -> Vec<Candidate> {
    let samples: Vec<(f64, f64, f64)> = piece
        .split(LEAF_SAMPLES)
        .map(|parameter| {
            let (signed, distance) = probe.offset(parameter);
            (parameter, signed, distance)
        })
        .collect();
    if samples
        .iter()
        .all(|(_, _, distance)| *distance <= ON_SURFACE)
    {
        return vec![Candidate::Along(piece)];
    }
    let signed = |parameter: f64| probe.signed(parameter);
    let mut candidates = Vec::new();
    for pair in samples.windows(2) {
        let [(a, ga, _), (b, gb, _)] = pair else {
            continue;
        };
        if *ga == 0.0 {
            candidates.push(Candidate::Root(*a));
        } else if (*ga < 0.0) != (*gb < 0.0) && *gb != 0.0 {
            candidates.push(Candidate::Root(bracket_root(signed, *a, *b, *ga, *gb)));
        }
    }
    let count = samples.len();
    for index in 0..count {
        let Some(&(at, here, _)) = samples.get(index) else {
            continue;
        };
        let before = index.checked_sub(1).and_then(|before| samples.get(before));
        let after = samples.get(index + 1);
        let neighbours = [before, after];
        let lowest = neighbours
            .iter()
            .flatten()
            .all(|(_, value, _)| (*value < 0.0) == (here < 0.0) && value.abs() >= here.abs());
        let low = before.map_or(at, |sample| sample.0);
        let high = after.map_or(at, |sample| sample.0);
        let reach = probe.curve.point(low).distance(probe.curve.point(high));
        if !lowest || here.abs() > reach || high <= low {
            continue;
        }
        let side = if here < 0.0 { -1.0 } else { 1.0 };
        let (extremum, value) = minimize_bracket(|parameter| side * signed(parameter), low, high);
        if (-TOLERANCE..=TOLERANCE).contains(&value) {
            if probe.distance(extremum) <= TOLERANCE {
                candidates.push(Candidate::Touch(extremum));
            }
        } else if value < -TOLERANCE {
            let (vl, vh) = (signed(low), signed(high));
            let vm = signed(extremum);
            if (vl < 0.0) != (vm < 0.0) {
                candidates.push(Candidate::Root(bracket_root(signed, low, extremum, vl, vm)));
            }
            if (vh < 0.0) != (vm < 0.0) {
                candidates.push(Candidate::Root(bracket_root(
                    signed, extremum, high, vm, vh,
                )));
            }
        }
    }
    candidates
}

fn connected(probe: &Probe, low: f64, high: f64) -> bool {
    high <= low
        || CONNECTION_SAMPLES
            .iter()
            .all(|fraction| probe.distance(low + (high - low) * fraction) <= TOLERANCE)
}

fn overlap_like(probe: &Probe, low: f64, high: f64) -> bool {
    let Some(span) = Interval::new(low, high) else {
        return false;
    };
    let middle = span.middle();
    span.split(OVERLAP_SAMPLES)
        .all(|parameter| probe.distance(parameter) <= TOLERANCE)
        && probe.sine(middle) <= OVERLAP_SINE
        && probe.relative_curvature(middle) <= OVERLAP_CURVATURE
}

fn extend(probe: &Probe, inside: f64, limit: f64) -> f64 {
    if probe.distance(limit) <= TOLERANCE && connected(probe, inside, limit) {
        return limit;
    }
    let (mut inside, mut outside) = (inside, limit);
    for _ in 0..EXTENSION_BISECTIONS {
        let middle = 0.5 * (inside + outside);
        if middle == inside || middle == outside {
            break;
        }
        if probe.distance(middle) <= ON_SURFACE {
            inside = middle;
        } else {
            outside = middle;
        }
    }
    inside
}

fn distinct(probe: &Probe, cluster: &[Candidate]) -> usize {
    let mut spots: Vec<Point3> = Vec::new();
    let mut count = 0;
    for candidate in cluster {
        match candidate {
            Candidate::Along(_) => count += 1,
            Candidate::Root(at) | Candidate::Touch(at) => {
                let point = probe.curve.point(*at);
                if !spots.iter().any(|spot| spot.distance(point) <= TOLERANCE) {
                    spots.push(point);
                    count += 1;
                }
            }
            Candidate::End(_) => {}
        }
    }
    count
}

fn single_crossing(probe: &Probe, cluster: &[Candidate]) -> Option<f64> {
    let mut roots: Vec<(f64, Point3)> = Vec::new();
    for candidate in cluster {
        if let Candidate::Root(at) = candidate {
            let point = probe.curve.point(*at);
            if !roots
                .iter()
                .any(|(_, known)| known.distance(point) <= TOLERANCE)
            {
                roots.push((*at, point));
            }
        }
    }
    match roots.as_slice() {
        [(at, _)] => Some(*at),
        _ => None,
    }
}

enum Outcome {
    Point(f64, bool),
    Overlap(Interval),
}

fn finish(
    probe: &Probe,
    range: Interval,
    patch: Option<&SurfacePatch>,
    mut candidates: Vec<Candidate>,
) -> CurveSurfaceIntersection {
    for end in [range.start(), range.end()] {
        if probe.distance(end) <= TOLERANCE {
            candidates.push(Candidate::End(end));
        }
    }
    candidates
        .retain(|candidate| candidate.low() >= range.start() && candidate.high() <= range.end());
    candidates.sort_by(|a, b| a.low().total_cmp(&b.low()));
    let mut clusters: Vec<Vec<Candidate>> = Vec::new();
    for candidate in candidates {
        match clusters.last_mut() {
            Some(cluster)
                if cluster.iter().map(Candidate::high).fold(f64::MIN, f64::max)
                    >= candidate.low()
                    || connected(
                        probe,
                        cluster.iter().map(Candidate::high).fold(f64::MIN, f64::max),
                        candidate.low(),
                    ) =>
            {
                cluster.push(candidate);
            }
            _ => clusters.push(vec![candidate]),
        }
    }
    let lows: Vec<f64> = clusters
        .iter()
        .map(|cluster| cluster.iter().map(Candidate::low).fold(f64::MAX, f64::min))
        .collect();
    let mut outcomes = Vec::new();
    for (index, cluster) in clusters.iter().enumerate() {
        let low = lows.get(index).copied().unwrap_or(range.start());
        let high = cluster.iter().map(Candidate::high).fold(f64::MIN, f64::max);
        let along = cluster
            .iter()
            .any(|candidate| matches!(candidate, Candidate::Along(_)));
        let genuine = distinct(probe, cluster);
        let span = Interval::new(low, high).map_or(0.0, |span| probe.curve.length(span));
        let overlap =
            span > MIN_OVERLAP && (along || (genuine >= 2 && overlap_like(probe, low, high)));
        if overlap {
            let previous = index
                .checked_sub(1)
                .and_then(|before| clusters.get(before))
                .map_or(range.start(), |cluster| {
                    cluster.iter().map(Candidate::high).fold(f64::MIN, f64::max)
                });
            let next = lows.get(index + 1).copied().unwrap_or(range.end());
            let start = extend(probe, low, previous.max(range.start()));
            let end = extend(probe, high, next.min(range.end()));
            if let Some(extent) = Interval::new(start, end) {
                outcomes.push(Outcome::Overlap(extent));
            }
            continue;
        }
        let at_end = cluster.iter().find_map(|candidate| match candidate {
            Candidate::End(at) => Some(*at),
            _ => None,
        });
        let touch = cluster.iter().find_map(|candidate| match candidate {
            Candidate::Touch(at) => Some(*at),
            _ => None,
        });
        let nearest = cluster
            .iter()
            .map(|candidate| {
                let at = 0.5 * (candidate.low() + candidate.high());
                (at, probe.distance(at))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(low, |(at, _)| at);
        let crossing = single_crossing(probe, cluster);
        let parameter = at_end.or(crossing).or(touch).unwrap_or(nearest);
        let tangent = probe.sine(parameter) <= TANGENT_SINE
            || (crossing.is_none() && (genuine >= 2 || touch.is_some() || along));
        outcomes.push(Outcome::Point(parameter, tangent));
    }
    let locate = |point: Point3| match patch {
        Some(patch) => patch.locate(point),
        None => Some(probe.surface.project(point, None)),
    };
    let mut result = CurveSurfaceIntersection::default();
    for outcome in outcomes {
        match outcome {
            Outcome::Point(parameter, tangent) => {
                let point = probe.curve.point(parameter);
                if let Some(uv) = locate(point) {
                    result.points.push(CurveSurfacePoint {
                        parameter,
                        point,
                        uv,
                        tangent,
                    });
                }
            }
            Outcome::Overlap(extent) => {
                let pieces = match patch {
                    Some(patch) => guided_intervals(
                        extent,
                        CLIP_SAMPLES,
                        |parameter| {
                            let uv = patch.place(probe.curve.point(parameter));
                            (patch.contains(uv), uv)
                        },
                        |path| patch.path_misses(path),
                    ),
                    None => vec![extent],
                };
                for piece in pieces {
                    let (start, end) = (
                        probe.curve.point(piece.start()),
                        probe.curve.point(piece.end()),
                    );
                    let (Some(start_uv), Some(end_uv)) = (
                        locate(start).or_else(|| Some(probe.surface.project(start, None))),
                        locate(end).or_else(|| Some(probe.surface.project(end, None))),
                    ) else {
                        continue;
                    };
                    if probe.curve.length(piece) <= MIN_OVERLAP {
                        result.points.push(CurveSurfacePoint {
                            parameter: piece.start(),
                            point: start,
                            uv: start_uv,
                            tangent: true,
                        });
                    } else {
                        result.overlaps.push(CurveSurfaceOverlap {
                            range: piece,
                            start_uv,
                            end_uv,
                        });
                    }
                }
            }
        }
    }
    let overlaps = result.overlaps.clone();
    result.points.retain(|point| {
        !overlaps
            .iter()
            .any(|overlap| overlap.range.contains(point.parameter))
    });
    result
        .points
        .sort_by(|a, b| a.parameter.total_cmp(&b.parameter));
    result.points.dedup_by(|b, a| b.parameter == a.parameter);
    let full_period = probe
        .curve
        .period()
        .is_some_and(|period| range.length() >= period * (1.0 - 1e-12));
    if full_period
        && result.points.len() > 1
        && let (Some(first), Some(last)) = (result.points.first(), result.points.last())
        && first.point.distance(last.point) <= TOLERANCE
    {
        result.points.pop();
    }
    result
}
