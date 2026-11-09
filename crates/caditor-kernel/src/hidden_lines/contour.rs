use std::f64::consts::{PI, TAU};

use caditor_geometry::{Aabb2, Maths, Plane, Point2, Point3, Vector3};

use super::{HiddenLineError, flatten::polyline_curve, trace::traced_contours};
use crate::{
    curve::{Circle, Curve, Line},
    interrupt,
    interval::Interval,
    surface::{Cone, Cylinder, Extrusion, Sphere, Surface, Torus},
    tolerance::{LINEAR_RESOLUTION, SamplingTolerance},
    topology::{FaceContainment, FaceId, SolidClassifier},
};

const SEEN_ALONG_AXIS: f64 = 1e-9;
const ZERO_FACING: f64 = 1e-10;
const BISECTIONS: usize = 60;
const MIN_ROOT_SAMPLES: usize = 8;
const BOX_MARGIN_SHARE: f64 = 1e-3;
const BOX_MARGIN: f64 = 1e-3;
const TORUS_SEEDS: usize = 48;
const TORUS_DEPTH: usize = 12;
const MAX_TORUS_POINTS: usize = 1 << 14;
const EXACT_REACH: f64 = 4.0 * LINEAR_RESOLUTION;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Contour {
    pub curve: Curve,
    pub range: Interval,
    pub closed: bool,
    pub sampled: bool,
}

pub(super) fn facing(surface: &Surface, uv: Point2, toward: Vector3) -> f64 {
    surface
        .normal(uv.x, uv.y)
        .map_or(0.0, |normal| normal.dot(toward))
}

pub(super) fn roots_among(parameters: &[f64], mut value: impl FnMut(f64) -> f64) -> Vec<f64> {
    let values: Vec<(f64, f64)> = parameters
        .iter()
        .map(|parameter| (*parameter, value(*parameter)))
        .collect();
    let mut roots = Vec::new();
    for pair in values.windows(2) {
        let [(start, at_start), (end, at_end)] = pair else {
            continue;
        };
        let start_zero = at_start.abs() <= ZERO_FACING;
        let end_zero = at_end.abs() <= ZERO_FACING;
        if start_zero && !end_zero {
            roots.push(*start);
        } else if end_zero && !start_zero {
            roots.push(*end);
        } else if !start_zero && !end_zero && at_start * at_end < 0.0 {
            roots.push(bisected(*start, *end, *at_start, &mut value));
        }
    }
    roots.dedup();
    roots
}

fn bisected(mut low: f64, mut high: f64, at_low: f64, value: &mut impl FnMut(f64) -> f64) -> f64 {
    let sign = at_low.signum();
    for _ in 0..BISECTIONS {
        let middle = 0.5 * (low + high);
        if value(middle).signum() == sign {
            low = middle;
        } else {
            high = middle;
        }
    }
    0.5 * (low + high)
}

pub(super) fn sample_parameters(
    curve: &Curve,
    range: Interval,
    tolerance: &SamplingTolerance,
) -> Vec<f64> {
    let sampled: Vec<f64> = curve
        .sample(range, tolerance)
        .iter()
        .map(|sample| sample.parameter)
        .collect();
    if sampled.len() > MIN_ROOT_SAMPLES {
        return sampled;
    }
    range.split(MIN_ROOT_SAMPLES).collect()
}

fn margin_of(extent: f64) -> f64 {
    extent * BOX_MARGIN_SHARE + BOX_MARGIN
}

fn ruling(
    point: Point3,
    direction: Vector3,
    low: f64,
    high: f64,
) -> Result<Option<Contour>, HiddenLineError> {
    let line = Line::new(point, direction)?;
    Ok(Interval::new(low, high).map(|range| Contour {
        curve: Curve::Line(line),
        range,
        closed: false,
        sampled: false,
    }))
}

fn circle_contour(frame: Plane, radius: f64) -> Result<Contour, HiddenLineError> {
    Ok(Contour {
        curve: Curve::Circle(Circle::new(frame, radius)?),
        range: Interval::FULL_TURN,
        closed: true,
        sampled: false,
    })
}

fn level_frame(origin: Point3, normal: Vector3, x_axis: Vector3) -> Result<Plane, HiddenLineError> {
    Plane::with_x_axis(origin, normal, x_axis).ok_or(HiddenLineError::NoContourFrame)
}

fn cylinder_contours(
    surface: &Surface,
    cylinder: &Cylinder,
    uv_box: Aabb2,
    toward: Vector3,
) -> Result<Vec<Contour>, HiddenLineError> {
    let frame = cylinder.frame();
    let (along_x, along_y) = (frame.x_axis().dot(toward), frame.y_axis().dot(toward));
    if along_x.hypot_libm(along_y) <= SEEN_ALONG_AXIS {
        return Ok(Vec::new());
    }
    let first = (-along_x).atan2_libm(along_y);
    let margin = margin_of(uv_box.size().y);
    let mut contours = Vec::with_capacity(2);
    for u in [first, first + PI] {
        let at = surface.evaluate(u, 0.0);
        contours.extend(ruling(
            at.point,
            at.dv,
            uv_box.min().y - margin,
            uv_box.max().y + margin,
        )?);
    }
    Ok(contours)
}

fn cone_contours(
    surface: &Surface,
    cone: &Cone,
    uv_box: Aabb2,
    toward: Vector3,
) -> Result<Vec<Contour>, HiddenLineError> {
    let frame = cone.frame();
    let (along_x, along_y) = (frame.x_axis().dot(toward), frame.y_axis().dot(toward));
    let turning = along_x.hypot_libm(along_y);
    if turning <= SEEN_ALONG_AXIS {
        return Ok(Vec::new());
    }
    let (sin, cos) = cone.half_angle().sin_cos_libm();
    let wanted = sin * frame.normal().dot(toward) / (cos * turning);
    if wanted.abs() >= 1.0 {
        return Ok(Vec::new());
    }
    let middle = along_y.atan2_libm(along_x);
    let spread = wanted.acos_libm();
    let margin = margin_of(uv_box.size().y);
    let domain = surface.v_domain();
    let (low, high) = (
        domain.clamp(uv_box.min().y - margin),
        domain.clamp(uv_box.max().y + margin),
    );
    let mut contours = Vec::with_capacity(2);
    for u in [middle - spread, middle + spread] {
        let at = surface.evaluate(u, 0.0);
        contours.extend(ruling(at.point, at.dv, low, high)?);
    }
    Ok(contours)
}

fn sphere_contours(sphere: &Sphere, view: &Plane) -> Result<Vec<Contour>, HiddenLineError> {
    let frame = level_frame(sphere.center(), view.normal(), view.x_axis())?;
    Ok(vec![circle_contour(frame, sphere.radius())?])
}

fn torus_contours(
    torus: &Torus,
    toward: Vector3,
    tolerance: &SamplingTolerance,
) -> Result<Vec<Contour>, HiddenLineError> {
    let frame = torus.frame();
    let (major, minor) = (torus.major_radius(), torus.minor_radius());
    let (along_x, along_y) = (frame.x_axis().dot(toward), frame.y_axis().dot(toward));
    let along_axis = frame.normal().dot(toward);
    if along_x.hypot_libm(along_y) <= SEEN_ALONG_AXIS {
        let mut contours = vec![circle_contour(*frame, major + minor)?];
        if major > minor {
            contours.push(circle_contour(*frame, major - minor)?);
        }
        return Ok(contours);
    }
    if along_axis.abs() <= SEEN_ALONG_AXIS {
        let lifted = |height: f64| {
            level_frame(
                frame.origin() + frame.normal() * height,
                frame.normal(),
                frame.x_axis(),
            )
        };
        let mut contours = vec![
            circle_contour(lifted(minor)?, major)?,
            circle_contour(lifted(-minor)?, major)?,
        ];
        let first = (-along_x).atan2_libm(along_y);
        for u in [first, first + PI] {
            let (sin, cos) = u.sin_cos_libm();
            let radial = frame.x_axis() * cos + frame.y_axis() * sin;
            let tangential = frame.y_axis() * cos - frame.x_axis() * sin;
            let tube = level_frame(frame.origin() + radial * major, tangential, radial)?;
            contours.push(circle_contour(tube, minor)?);
        }
        return Ok(contours);
    }
    let mut contours = Vec::with_capacity(2);
    for branch in [0.0, PI] {
        let tube = |u: f64| {
            let (sin, cos) = u.sin_cos_libm();
            let across = along_x * cos + along_y * sin;
            (-across).atan2_libm(along_axis) + branch
        };
        let points = torus_branch(torus, &tube, tolerance)?;
        if let Some((curve, range)) = polyline_curve(&points)? {
            contours.push(Contour {
                curve,
                range,
                closed: true,
                sampled: true,
            });
        }
    }
    Ok(contours)
}

fn torus_branch(
    torus: &Torus,
    tube: &impl Fn(f64) -> f64,
    tolerance: &SamplingTolerance,
) -> Result<Vec<Point3>, HiddenLineError> {
    let at = |u: f64| torus.evaluate(u, tube(u)).point;
    let mut points = vec![at(0.0)];
    let step = TAU / TORUS_SEEDS as f64;
    for seed in 0..TORUS_SEEDS {
        interrupt::check()?;
        let start = seed as f64 * step;
        let mut pending = vec![(start, start + step, 0_usize)];
        while let Some((low, high, depth)) = pending.pop() {
            let middle = 0.5 * (low + high);
            let (from, to, halfway) = (at(low), at(high), at(middle));
            let bowed = chord_distance(halfway, from, to) > tolerance.chord();
            if bowed && depth < TORUS_DEPTH && points.len() + pending.len() < MAX_TORUS_POINTS {
                pending.push((middle, high, depth + 1));
                pending.push((low, middle, depth + 1));
            } else {
                points.push(to);
            }
        }
    }
    Ok(points)
}

pub(super) fn chord_distance(point: Point3, from: Point3, to: Point3) -> f64 {
    let chord = to - from;
    let length_squared = chord.length_squared();
    if length_squared <= 0.0 {
        return point.distance(from);
    }
    let fraction = ((point - from).dot(chord) / length_squared).clamp(0.0, 1.0);
    point.distance(from + chord * fraction)
}

fn extrusion_contours(
    extrusion: &Extrusion,
    uv_box: Aabb2,
    toward: Vector3,
    tolerance: &SamplingTolerance,
) -> Result<Vec<Contour>, HiddenLineError> {
    let direction = extrusion.direction();
    if direction.cross(toward).length() <= SEEN_ALONG_AXIS {
        return Ok(Vec::new());
    }
    let profile = extrusion.profile();
    let span = uv_box.size().x;
    let (low, high) = match profile.period() {
        Some(period) if span >= period => (uv_box.min().x, uv_box.min().x + period),
        _ => {
            let margin = margin_of(span);
            let domain = profile.domain();
            (
                domain.clamp(uv_box.min().x - margin),
                domain.clamp(uv_box.max().x + margin),
            )
        }
    };
    let Some(range) = Interval::new(low, high) else {
        return Ok(Vec::new());
    };
    let parameters = sample_parameters(profile, range, tolerance);
    let roots = roots_among(&parameters, |u| {
        profile
            .evaluate(u)
            .first
            .cross(direction)
            .try_normalize()
            .map_or(0.0, |normal| normal.dot(toward))
    });
    let margin = margin_of(uv_box.size().y);
    let mut contours = Vec::with_capacity(roots.len());
    for root in roots {
        interrupt::check()?;
        contours.extend(ruling(
            profile.point(root),
            direction,
            uv_box.min().y - margin,
            uv_box.max().y + margin,
        )?);
    }
    Ok(contours)
}

pub(super) fn contours(
    surface: &Surface,
    uv_box: Aabb2,
    view: &Plane,
    tolerance: &SamplingTolerance,
) -> Result<Vec<Contour>, HiddenLineError> {
    let toward = view.normal();
    match surface {
        Surface::Plane(_) => Ok(Vec::new()),
        Surface::Cylinder(cylinder) => cylinder_contours(surface, cylinder, uv_box, toward),
        Surface::Cone(cone) => cone_contours(surface, cone, uv_box, toward),
        Surface::Sphere(sphere) => sphere_contours(sphere, view),
        Surface::Torus(torus) => torus_contours(torus, toward, tolerance),
        Surface::Extrusion(extrusion) => extrusion_contours(extrusion, uv_box, toward, tolerance),
        Surface::Revolution(_) | Surface::BSpline(_) => {
            traced_contours(surface, uv_box, toward, tolerance)
        }
    }
}

pub(super) fn kept_ranges(
    contour: &Contour,
    surface: &Surface,
    face: FaceId,
    classifier: &SolidClassifier<'_>,
    boundary_points: &[Point3],
    chord: f64,
) -> Vec<Interval> {
    let reach = if contour.sampled {
        chord + LINEAR_RESOLUTION
    } else {
        EXACT_REACH
    };
    let mut parameters = vec![contour.range.start(), contour.range.end()];
    for point in boundary_points {
        let parameter = contour.curve.closest_parameter(*point, contour.range);
        if contour.curve.point(parameter).distance(*point) <= reach {
            parameters.push(parameter);
        }
    }
    parameters.sort_by(f64::total_cmp);
    parameters.dedup();
    let mut kept: Vec<Interval> = Vec::new();
    for pair in parameters.windows(2) {
        let [start, end] = pair else {
            continue;
        };
        let Some(piece) = Interval::new(*start, *end) else {
            continue;
        };
        let middle = contour.curve.point(piece.middle());
        let uv = surface.project(middle, None);
        let on_surface = surface.point_at(uv).distance(middle) <= reach;
        let inside = classifier.point_in_face(face, uv) == Some(FaceContainment::Inside);
        if !(on_surface && inside) {
            continue;
        }
        match kept.last_mut() {
            Some(last) if last.end() == piece.start() => {
                if let Some(joined) = Interval::new(last.start(), piece.end()) {
                    *last = joined;
                }
            }
            _ => kept.push(piece),
        }
    }
    kept
}
