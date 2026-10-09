use std::f64::consts::TAU;

use caditor_geometry::{Aabb2, Plane, Point2, Point3, Vector3};

use super::{OFFSET_TOLERANCE, Offsets, ShellError};
use crate::{
    curve::{Circle, Curve, Line},
    intersect::{IntersectionBranch, IntersectionError, SurfacePatch, intersect_surfaces},
    interval::Interval,
    surface::{Surface, SurfaceDerivatives, periodic_near},
    tolerance::LINEAR_RESOLUTION,
    topology::{Edge, EdgeId, FaceId},
};

const EDGE_SAMPLES: [f64; 3] = [0.25, 0.5, 0.75];
const WINDOW_SAMPLES: usize = 16;
const WINDOW_GROWTH: f64 = 0.25;
const WINDOW_REACH: f64 = 3.0;
const SLOWEST_SPEED: f64 = 1e-3;

pub(super) fn follows(
    offsets: &Offsets<'_>,
    faces: &[FaceId],
    curve: &Curve,
    interval: Interval,
) -> bool {
    EDGE_SAMPLES
        .iter()
        .all(|fraction| offsets.on_both(faces, curve.point(interval.at(*fraction))))
}

fn angle_about(circle: &Circle, point: Point3, near: f64) -> f64 {
    let frame = circle.frame();
    let local = point - circle.center();
    let angle = local.dot(frame.y_axis()).atan2(local.dot(frame.x_axis()));
    periodic_near(angle, TAU, Some(near))
}

fn parameter_near(curve: &Curve, point: Point3, near: f64, reach: f64) -> f64 {
    let (low, high) = (near - reach, near + reach);
    let window = match curve.period() {
        Some(_) => Interval::new(low, high),
        None => {
            let domain = curve.domain();
            Interval::new(domain.clamp(low), domain.clamp(high))
        }
    };
    window.map_or(near, |window| curve.closest_parameter(point, window))
}

pub(super) fn reverses(edge: &Edge, from: Point3, to: Point3) -> bool {
    let interval = edge.interval();
    let advance = match edge.curve() {
        Curve::Line(line) => (to - from).dot(line.direction()),
        Curve::Circle(circle) => {
            let sweep = angle_about(circle, to, interval.end())
                - angle_about(circle, from, interval.start());
            sweep * circle.radius()
        }
        curve => {
            let reach = interval.length();
            let (start, end) = (
                parameter_near(curve, from, interval.start(), reach),
                parameter_near(curve, to, interval.end(), reach),
            );
            curve.length(Interval::new(start.min(end), start.max(end)).unwrap_or(interval))
                * (end - start).signum()
        }
    };
    advance <= LINEAR_RESOLUTION
}

fn rebuilt(
    edge: &Edge,
    source: EdgeId,
    from: Point3,
    to: Point3,
) -> Result<Option<(Curve, Interval)>, ShellError> {
    let collapses = || ShellError::EdgeCollapses(source);
    match edge.curve() {
        Curve::Line(_) => {
            let line = Line::through(from, to).map_err(|_| collapses())?;
            let interval = Interval::new(0.0, from.distance(to)).ok_or_else(collapses)?;
            Ok(Some((line.into(), interval)))
        }
        Curve::Circle(circle) => {
            let axis = circle.frame().normal();
            let center = circle.center() + axis * (from - circle.center()).dot(axis);
            let radial = from - center;
            let radius = radial.length();
            if radius <= LINEAR_RESOLUTION {
                return Err(collapses());
            }
            let x_axis = radial.try_normalize().ok_or_else(collapses)?;
            let frame = Plane::from_frame(center, axis, x_axis).ok_or_else(collapses)?;
            let sweep = if edge.is_closed() {
                TAU
            } else {
                let local = to - center;
                let angle = axis
                    .cross(x_axis)
                    .dot(local)
                    .atan2(x_axis.dot(local))
                    .rem_euclid(TAU);
                if angle <= f64::EPSILON { TAU } else { angle }
            };
            let circle = Circle::new(frame, radius).map_err(|_| collapses())?;
            let interval = Interval::new(0.0, sweep).ok_or_else(collapses)?;
            Ok(Some((circle.into(), interval)))
        }
        _ => Ok(None),
    }
}

fn window(surface: &Surface, points: &[Point3], reach: f64) -> Option<Aabb2> {
    let mut hint: Option<Point2> = None;
    let mut uvs = Vec::with_capacity(points.len());
    for point in points {
        let uv = surface.project(*point, hint);
        hint = Some(uv);
        uvs.push(uv);
    }
    let bounds = Aabb2::from_points(uvs)?;
    let (min, max, center) = (bounds.min(), bounds.max(), bounds.center());
    let speeds = [
        min,
        center,
        max,
        Point2::new(min.x, max.y),
        Point2::new(max.x, min.y),
    ]
    .map(|uv| surface.evaluate(uv.x, uv.y));
    let fastest = |pick: fn(&SurfaceDerivatives) -> f64| {
        speeds.iter().map(pick).fold(SLOWEST_SPEED, f64::max)
    };
    let (u_speed, v_speed) = (fastest(|at| at.du.length()), fastest(|at| at.dv.length()));
    let margin = |extent: f64, speed: f64, period: Option<f64>| {
        let margin = extent * WINDOW_GROWTH + reach / speed;
        period.map_or(margin, |period| margin.min(period))
    };
    let size = bounds.size();
    let (du, dv) = (
        margin(size.x, u_speed, surface.u_period()),
        margin(size.y, v_speed, surface.v_period()),
    );
    let grown = Aabb2::from_point(Point2::new(min.x - du, min.y - dv))
        .including(Point2::new(max.x + du, max.y + dv));
    grown.min().is_finite().then_some(grown)
}

fn located(curve: &Curve, range: Interval, point: Point3) -> Option<f64> {
    let parameter = curve.closest_parameter(point, range);
    (curve.point(parameter).distance(point) <= OFFSET_TOLERANCE).then_some(parameter)
}

fn piece_between(
    branch: &IntersectionBranch,
    (from, to): (Point3, Point3),
    along: Vector3,
    closed: bool,
) -> Option<(Curve, Interval)> {
    let start = located(&branch.curve, branch.range, from)?;
    let forward = branch.curve.evaluate(start).first.dot(along) >= 0.0;
    let (curve, range) = if forward {
        (branch.curve.clone(), branch.range)
    } else {
        (
            branch.curve.reversed(),
            branch.curve.reversed_range(branch.range),
        )
    };
    let start = located(&curve, range, from)?;
    let end = located(&curve, range, to)?;
    let period = curve.period().filter(|_| branch.closed);
    let interval = match (period, closed) {
        (Some(period), true) => Interval::new(start, start + period),
        (Some(period), false) if end <= start => Interval::new(start, end + period),
        (_, false) => Interval::new(start, end),
        (None, true) => None,
    }?;
    (curve.length(interval) > LINEAR_RESOLUTION).then_some((curve, interval))
}

fn crossing(
    offsets: &Offsets<'_>,
    edge: &Edge,
    faces: &[FaceId],
    ends: (Point3, Point3),
) -> Result<Option<(Curve, Interval)>, ShellError> {
    let [first, second] = faces else {
        return Ok(None);
    };
    let surfaces = [offsets.surface(*first)?, offsets.surface(*second)?];
    let interval = edge.interval();
    let mut points: Vec<Point3> = interval
        .split(WINDOW_SAMPLES)
        .map(|parameter| edge.curve().point(parameter))
        .collect();
    points.extend([ends.0, ends.1]);
    let reach = WINDOW_REACH * offsets.reach;
    let [first_surface, second_surface] = &surfaces;
    let (Some(first_window), Some(second_window)) = (
        window(first_surface, &points, reach),
        window(second_surface, &points, reach),
    ) else {
        return Ok(None);
    };
    let (Ok(first_patch), Ok(second_patch)) = (
        SurfacePatch::new(first_surface, first_window),
        SurfacePatch::new(second_surface, second_window),
    ) else {
        return Ok(None);
    };
    let found = match intersect_surfaces(&first_patch, &second_patch) {
        Ok(found) => found,
        Err(IntersectionError::Cancelled(interrupted)) => {
            return Err(ShellError::Cancelled(interrupted));
        }
        Err(_) => return Ok(None),
    };
    let along = edge.curve().evaluate(interval.start()).first;
    Ok(found.branches().iter().find_map(|branch| {
        piece_between(branch, ends, along, edge.is_closed())
            .filter(|(curve, interval)| follows(offsets, faces, curve, *interval))
    }))
}

fn untouched(offsets: &Offsets<'_>, edge: &Edge, faces: &[FaceId], ends: (Point3, Point3)) -> bool {
    let (start, end) = (
        edge.curve().point(edge.interval().start()),
        edge.curve().point(edge.interval().end()),
    );
    faces.iter().all(|face| offsets.distance(*face) == 0.0)
        && start.distance(ends.0) <= LINEAR_RESOLUTION
        && end.distance(ends.1) <= LINEAR_RESOLUTION
}

pub(super) fn offset_edge(
    offsets: &Offsets<'_>,
    source: EdgeId,
    faces: &[FaceId],
    (from, to): (Point3, Point3),
) -> Result<(Curve, Interval), ShellError> {
    let edge = offsets
        .solid
        .edge(source)
        .ok_or(ShellError::UnsupportedEdge(source))?;
    if !edge.is_closed() && reverses(edge, from, to) {
        return Err(ShellError::EdgeCollapses(source));
    }
    if untouched(offsets, edge, faces, (from, to)) {
        return Ok((edge.curve().clone(), edge.interval()));
    }
    if let Some((curve, interval)) = rebuilt(edge, source, from, to)?
        && follows(offsets, faces, &curve, interval)
    {
        return Ok((curve, interval));
    }
    crossing(offsets, edge, faces, (from, to))?.ok_or(ShellError::UnsupportedEdge(source))
}
