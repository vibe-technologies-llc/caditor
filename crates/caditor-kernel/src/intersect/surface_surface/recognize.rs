use std::f64::consts::TAU;

use caditor_geometry::{Plane, Point3, Vector3};

use crate::{
    curve::{Circle, Curve, Ellipse, IntersectionCurve, Line},
    intersect::linear::damped_least_squares,
    interval::Interval,
    tolerance::LINEAR_RESOLUTION,
};

const FIT: f64 = 0.5 * LINEAR_RESOLUTION;
const MIN_NODES: usize = 6;
const MIN_BULGE: f64 = 1e3 * FIT;
const CHECK_SAMPLES: usize = 64;

pub(crate) fn recognize(curve: &IntersectionCurve) -> Option<(Curve, Interval)> {
    let (found, range) = candidate(curve)?;
    let [first, second] = curve.surfaces();
    let on_both = range.split(CHECK_SAMPLES).all(|parameter| {
        let point = found.point(parameter);
        first.distance(point) <= FIT && second.distance(point) <= FIT
    });
    let start = curve.nodes().first()?.point;
    let end = curve.nodes().last()?.point;
    let same_ends = found.point(range.start()).distance(start) <= FIT
        && found.point(range.end()).distance(end) <= FIT;
    (on_both && same_ends).then_some((found, range))
}

fn candidate(curve: &IntersectionCurve) -> Option<(Curve, Interval)> {
    let points: Vec<Point3> = curve.nodes().iter().map(|node| node.point).collect();
    if points.len() < MIN_NODES {
        return None;
    }
    let first = curve.nodes().first()?;
    let last = curve.nodes().last()?;
    if !curve.is_closed()
        && let Some(line) = as_line(&points, first.point, last.point)
    {
        return Some(line);
    }
    let chord = Line::through(first.point, last.point).ok();
    let bulge = points
        .iter()
        .map(|point| match &chord {
            Some(line) => line.point(line.parameter_of(*point)).distance(*point),
            None => point.distance(first.point),
        })
        .fold(0.0, f64::max);
    if bulge < MIN_BULGE {
        return None;
    }
    let normal = planar_normal(&points)?;
    let centroid =
        points.iter().fold(Point3::ZERO, |sum, point| sum + *point) / points.len() as f64;
    if points
        .iter()
        .any(|point| (*point - centroid).dot(normal).abs() > FIT)
    {
        return None;
    }
    as_circle(curve, &points, normal).or_else(|| as_ellipse(curve, &points, normal, centroid))
}

fn as_line(points: &[Point3], start: Point3, end: Point3) -> Option<(Curve, Interval)> {
    let line = Line::through(start, end).ok()?;
    let straight = points.iter().all(|point| {
        let along = line.parameter_of(*point);
        line.point(along).distance(*point) <= FIT
    });
    straight.then(|| {
        (
            line.into(),
            Interval::new(0.0, start.distance(end)).unwrap_or(Interval::UNIT),
        )
    })
}

fn planar_normal(points: &[Point3]) -> Option<Vector3> {
    let centroid =
        points.iter().fold(Point3::ZERO, |sum, point| sum + *point) / points.len() as f64;
    let count = points.len();
    let sum = points
        .iter()
        .enumerate()
        .filter_map(|(index, point)| {
            let next = points.get((index + 1) % count)?;
            Some((*point - centroid).cross(*next - centroid))
        })
        .fold(Vector3::ZERO, |sum, cross| sum + cross);
    sum.try_normalize()
}

fn oriented_frame(
    center: Point3,
    normal: Vector3,
    x_axis: Vector3,
    direction: Vector3,
    at: Point3,
) -> Option<Plane> {
    let frame = Plane::from_frame(center, normal, x_axis)?;
    let radial = at - center;
    let tangent = frame.normal().cross(radial);
    if tangent.dot(direction) >= 0.0 {
        Some(frame)
    } else {
        Plane::from_frame(center, -normal, x_axis)
    }
}

fn sweep(curve: &IntersectionCurve, start: f64, end: f64) -> Option<Interval> {
    if curve.is_closed() {
        return Interval::new(start, start + TAU);
    }
    let mut span = (end - start).rem_euclid(TAU);
    if span <= 1e-12 {
        span = TAU;
    }
    Interval::new(start, start + span)
}

fn as_circle(
    curve: &IntersectionCurve,
    points: &[Point3],
    normal: Vector3,
) -> Option<(Curve, Interval)> {
    let count = points.len();
    let a = *points.first()?;
    let b = *points.get(count / 3)?;
    let c = *points.get(2 * count / 3)?;
    let (u, v) = (a - c, b - c);
    let cross = u.cross(v);
    let squared = cross.length_squared();
    if squared <= f64::MIN_POSITIVE {
        return None;
    }
    let center =
        c + (v * u.length_squared() - u * v.length_squared()).cross(cross) / (2.0 * squared);
    let radius = a.distance(center);
    if points
        .iter()
        .any(|point| (point.distance(center) - radius).abs() > FIT)
    {
        return None;
    }
    let first = curve.nodes().first()?;
    let last = curve.nodes().last()?;
    let x_axis = (first.point - center).try_normalize()?;
    let frame = oriented_frame(center, normal, x_axis, first.derivative, first.point)?;
    let circle = Circle::new(frame, radius).ok()?;
    let local = last.point - center;
    let end = local.dot(frame.y_axis()).atan2(local.dot(frame.x_axis()));
    Some((circle.into(), sweep(curve, 0.0, end)?))
}

fn as_ellipse(
    curve: &IntersectionCurve,
    points: &[Point3],
    normal: Vector3,
    centroid: Point3,
) -> Option<(Curve, Interval)> {
    let e1 = normal.any_orthonormal_vector();
    let e2 = normal.cross(e1);
    let local: Vec<(f64, f64)> = points
        .iter()
        .map(|point| ((*point - centroid).dot(e1), (*point - centroid).dot(e2)))
        .collect();
    let scale = local
        .iter()
        .fold(0.0f64, |largest, (x, y)| largest.max(x.abs()).max(y.abs()));
    if scale <= f64::MIN_POSITIVE {
        return None;
    }
    let rows: Vec<Vec<f64>> = local
        .iter()
        .map(|(x, y)| {
            let (x, y) = (x / scale, y / scale);
            vec![x * x, x * y, y * y, x, y]
        })
        .collect();
    let targets = vec![-1.0; rows.len()];
    let fitted = damped_least_squares(&rows, &targets)?;
    let [a, b, c, d, e] = fitted.as_slice() else {
        return None;
    };
    let (a, b, c, d, e) = (
        a / (scale * scale),
        b / (scale * scale),
        c / (scale * scale),
        d / scale,
        e / scale,
    );
    let determinant = 4.0 * a * c - b * b;
    if determinant <= 0.0 {
        return None;
    }
    let x0 = (b * e - 2.0 * c * d) / determinant;
    let y0 = (b * d - 2.0 * a * e) / determinant;
    let level = 1.0 - (a * x0 * x0 + b * x0 * y0 + c * y0 * y0 + d * x0 + e * y0);
    let angle = 0.5 * b.atan2(a - c);
    let (sin, cos) = angle.sin_cos();
    let first_value = a * cos * cos + b * sin * cos + c * sin * sin;
    let second_value = a * sin * sin - b * sin * cos + c * cos * cos;
    if level <= 0.0 || first_value <= 0.0 || second_value <= 0.0 {
        return None;
    }
    let (first_radius, second_radius) =
        ((level / first_value).sqrt(), (level / second_value).sqrt());
    let first_axis = e1 * cos + e2 * sin;
    let second_axis = e2 * cos - e1 * sin;
    let (major, minor, major_axis) = if first_radius >= second_radius {
        (first_radius, second_radius, first_axis)
    } else {
        (second_radius, first_radius, second_axis)
    };
    let center = centroid + e1 * x0 + e2 * y0;
    let start = curve.nodes().first()?;
    let end = curve.nodes().last()?;
    let frame = oriented_frame(center, normal, major_axis, start.derivative, start.point)?;
    let ellipse = Ellipse::new(frame, major, minor).ok()?;
    let parameter_of = |point: Point3| {
        let offset = point - center;
        (offset.dot(frame.y_axis()) / minor).atan2(offset.dot(frame.x_axis()) / major)
    };
    let as_curve = Curve::Ellipse(ellipse);
    let fits = points.iter().all(|point| {
        let parameter = parameter_of(*point);
        let range = Interval::new(parameter - 0.2, parameter + 0.2).unwrap_or(Interval::UNIT);
        let closest = as_curve.closest_parameter(*point, range);
        as_curve.point(closest).distance(*point) <= FIT
    });
    if !fits {
        return None;
    }
    let first = parameter_of(start.point);
    Some((as_curve, sweep(curve, first, parameter_of(end.point))?))
}
