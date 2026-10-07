use std::f64::consts::PI;

use caditor_geometry::{Aabb, Plane, Point3, Vector3};

use crate::{
    curve::{Circle, Curve, Ellipse, Line},
    intersect::{
        SurfacePatch, intersect_curve_surface,
        surface_surface::{Raw, RawCurve, RawPoint, coaxial},
    },
    interval::Interval,
    surface::{Cone, Cylinder, Extrusion, PlaneSurface, Surface, Torus},
    tolerance::LINEAR_RESOLUTION,
};

const TOLERANCE: f64 = LINEAR_RESOLUTION;
const TINY: f64 = 1e-14;
const ROUND_ELLIPSE: f64 = 1e-12;

pub(crate) fn intersect(first: &SurfacePatch, second: &SurfacePatch, window: &Aabb) -> Option<Raw> {
    if let Some(raw) = coaxial::intersect(first, second, window) {
        return Some(raw);
    }
    let (a, b) = (first.surface(), second.surface());
    let (a_patch, b_patch) = (first, second);
    match (a, b) {
        (Surface::Plane(one), Surface::Plane(other)) => Some(plane_plane(one, other, window)),
        (Surface::Plane(plane), Surface::Cylinder(cylinder))
        | (Surface::Cylinder(cylinder), Surface::Plane(plane)) => {
            Some(plane_cylinder(plane, cylinder, window))
        }
        (Surface::Plane(plane), Surface::Cone(cone))
        | (Surface::Cone(cone), Surface::Plane(plane)) => plane_cone(plane, cone, window),
        (Surface::Plane(plane), Surface::Torus(torus))
        | (Surface::Torus(torus), Surface::Plane(plane)) => plane_torus(plane, torus, window),
        (Surface::Plane(plane), Surface::Extrusion(extrusion)) => {
            plane_extrusion(plane, extrusion, b_patch, window)
        }
        (Surface::Extrusion(extrusion), Surface::Plane(plane)) => {
            plane_extrusion(plane, extrusion, a_patch, window)
        }
        (Surface::Cylinder(one), Surface::Cylinder(other)) => cylinder_cylinder(one, other, window),
        _ => None,
    }
}

pub(crate) fn line_window(origin: Point3, direction: Vector3, bounds: &Aabb) -> Option<Interval> {
    let mut low = f64::NEG_INFINITY;
    let mut high = f64::INFINITY;
    let starts = origin.to_array();
    let directions = direction.to_array();
    let mins = bounds.min().to_array();
    let maxs = bounds.max().to_array();
    for (((start, along), min), max) in starts.into_iter().zip(directions).zip(mins).zip(maxs) {
        if along.abs() <= TINY {
            if start < min || start > max {
                return None;
            }
            continue;
        }
        let (enter, leave) = ((min - start) / along, (max - start) / along);
        low = low.max(enter.min(leave));
        high = high.min(enter.max(leave));
    }
    Interval::new(low, high)
}

fn line(point: Point3, direction: Vector3, window: &Aabb, tangent: bool) -> Option<RawCurve> {
    let line = Line::new(point, direction).ok()?;
    let range = line_window(line.origin(), line.direction(), window)?;
    Some(RawCurve {
        curve: line.into(),
        range,
        tangent,
    })
}

fn ray(apex: Point3, direction: Vector3, window: &Aabb, tangent: bool) -> Option<RawCurve> {
    let line = Line::new(apex, direction).ok()?;
    let range = line_window(line.origin(), line.direction(), window)?;
    let range = Interval::new(range.start().max(0.0), range.end())?;
    Some(RawCurve {
        curve: line.into(),
        range,
        tangent,
    })
}

fn closed(curve: Curve) -> RawCurve {
    RawCurve {
        curve,
        range: Interval::FULL_TURN,
        tangent: false,
    }
}

fn plane_plane(first: &PlaneSurface, second: &PlaneSurface, window: &Aabb) -> Raw {
    let (a, b) = (first.frame(), second.frame());
    let direction = a.normal().cross(b.normal());
    let squared = direction.length_squared();
    if squared <= TINY * TINY {
        return Raw::default();
    }
    let (ha, hb) = (a.normal().dot(a.origin()), b.normal().dot(b.origin()));
    let point = (b.normal().cross(direction) * ha + direction.cross(a.normal()) * hb) / squared;
    Raw {
        curves: line(point, direction, window, false).into_iter().collect(),
        points: Vec::new(),
    }
}

pub(crate) fn plane_cylinder_ellipse(
    frame: &Plane,
    origin: Point3,
    axis: Vector3,
    radius: f64,
) -> Option<Curve> {
    let normal = frame.normal();
    let cosine = normal.dot(axis);
    if cosine.abs() <= TINY {
        return None;
    }
    let along = normal.dot(frame.origin() - origin) / cosine;
    let center = origin + axis * along;
    let Some(minor) = normal.cross(axis).try_normalize() else {
        let circle_frame = Plane::from_frame(center, normal, frame.x_axis())?;
        return Circle::new(circle_frame, radius).ok().map(Curve::from);
    };
    let major = normal.cross(minor);
    let major_radius = radius / cosine.abs();
    if major_radius - radius <= ROUND_ELLIPSE * radius {
        let circle_frame = Plane::from_frame(center, normal, major)?;
        return Circle::new(circle_frame, radius).ok().map(Curve::from);
    }
    let ellipse_frame = Plane::from_frame(center, normal, major)?;
    Ellipse::new(ellipse_frame, major_radius, radius)
        .ok()
        .map(Curve::from)
}

fn plane_cylinder(plane: &PlaneSurface, cylinder: &Cylinder, window: &Aabb) -> Raw {
    let frame = plane.frame();
    let normal = frame.normal();
    let axis = cylinder.frame().normal();
    let origin = cylinder.frame().origin();
    let radius = cylinder.radius();
    let cosine = normal.dot(axis);
    let mut raw = Raw::default();
    if cosine.abs() * window.diagonal() <= TOLERANCE {
        let height = frame.signed_distance(origin);
        let foot = origin - normal * height;
        let across = normal.cross(axis).normalize_or_zero();
        if (height.abs() - radius).abs() <= TOLERANCE {
            raw.curves.extend(line(foot, axis, window, true));
        } else if height.abs() < radius {
            let half = (radius * radius - height * height).sqrt();
            raw.curves
                .extend(line(foot + across * half, axis, window, false));
            raw.curves
                .extend(line(foot - across * half, axis, window, false));
        }
        return raw;
    }
    raw.curves
        .extend(plane_cylinder_ellipse(frame, origin, axis, radius).map(closed));
    raw
}

fn reach_from(point: Point3, window: &Aabb) -> f64 {
    window
        .corners()
        .iter()
        .map(|corner| corner.distance(point))
        .fold(0.0, f64::max)
}

fn plane_cone(plane: &PlaneSurface, cone: &Cone, window: &Aabb) -> Option<Raw> {
    let frame = plane.frame();
    let normal = frame.normal();
    let apex = cone.apex();
    let axis = cone.opening_direction();
    let (sine, cosine) = cone.half_angle().abs().sin_cos();
    let reach = reach_from(apex, window);
    let height = frame.signed_distance(apex);
    let along = normal.dot(axis);
    let across = normal - axis * along;
    let spread = across.length();
    let mut raw = Raw::default();
    if height.abs() <= TOLERANCE {
        let Some(radial) = across.try_normalize() else {
            raw.points.push(RawPoint {
                point: apex,
                tangent: true,
            });
            return Some(raw);
        };
        let side = axis.cross(radial);
        let ruling =
            |angle: f64| axis * cosine + (radial * angle.cos() + side * angle.sin()) * sine;
        let ratio = -cosine * along / (sine * spread);
        let nearest = if ratio >= 0.0 { 0.0 } else { PI };
        let gap = (cosine * along + sine * spread * nearest.cos()).abs() * reach;
        if ratio.abs() > 1.0 {
            if gap <= TOLERANCE {
                raw.curves.extend(ray(apex, ruling(nearest), window, true));
            } else {
                raw.points.push(RawPoint {
                    point: apex,
                    tangent: true,
                });
            }
            return Some(raw);
        }
        let opening = ratio.acos();
        if 2.0 * reach * sine * opening.sin() <= TOLERANCE {
            raw.curves.extend(ray(apex, ruling(nearest), window, true));
        } else {
            raw.curves.extend(ray(apex, ruling(opening), window, false));
            raw.curves
                .extend(ray(apex, ruling(-opening), window, false));
        }
        return Some(raw);
    }
    if along.abs() <= sine {
        return None;
    }
    let crossing = -height / along;
    if crossing <= 0.0 {
        return Some(raw);
    }
    let radial = across.try_normalize()?;
    let vertex = |direction: Vector3| {
        let rate = normal.dot(direction);
        let distance = -height / rate;
        (rate.abs() > TINY && distance > 0.0).then(|| apex + direction * distance)
    };
    let near = vertex(axis * cosine + radial * sine)?;
    let far = vertex(axis * cosine - radial * sine)?;
    let center = near.lerp(far, 0.5);
    let major_radius = 0.5 * near.distance(far);
    let x_axis = (far - near).try_normalize()?;
    let y_axis = normal.cross(x_axis);
    let offset = center - apex;
    let cos2 = cosine * cosine;
    let quadratic = y_axis.dot(axis).powi(2) - cos2;
    let constant = offset.dot(axis).powi(2) - cos2 * offset.length_squared();
    let minor_squared = -constant / quadratic;
    if minor_squared.is_nan() || minor_squared <= 0.0 {
        return None;
    }
    let minor_radius = minor_squared.sqrt();
    let ellipse_frame = Plane::from_frame(center, normal, x_axis)?;
    let curve = if (major_radius - minor_radius).abs() <= ROUND_ELLIPSE * major_radius {
        Circle::new(ellipse_frame, major_radius).ok()?.into()
    } else if major_radius >= minor_radius {
        Ellipse::new(ellipse_frame, major_radius, minor_radius)
            .ok()?
            .into()
    } else {
        let turned = Plane::from_frame(center, normal, y_axis)?;
        Ellipse::new(turned, minor_radius, major_radius)
            .ok()?
            .into()
    };
    raw.curves.push(closed(curve));
    Some(raw)
}

fn plane_torus(plane: &PlaneSurface, torus: &Torus, window: &Aabb) -> Option<Raw> {
    let frame = plane.frame();
    let axis = torus.frame().normal();
    let origin = torus.frame().origin();
    let contains_axis = frame.normal().dot(axis).abs() * window.diagonal() <= TOLERANCE
        && frame.signed_distance(origin).abs() <= TOLERANCE;
    if !contains_axis {
        return None;
    }
    let outward = frame.normal().cross(axis).try_normalize()?;
    let mut raw = Raw::default();
    for side in [1.0, -1.0] {
        let center = origin + outward * (side * torus.major_radius());
        let circle_frame = Plane::from_frame(center, frame.normal(), outward)?;
        raw.curves.push(closed(
            Circle::new(circle_frame, torus.minor_radius()).ok()?.into(),
        ));
    }
    Some(raw)
}

pub(crate) fn conjugate_ellipse(center: Point3, first: Vector3, second: Vector3) -> Option<Curve> {
    let turn =
        0.5 * (2.0 * first.dot(second)).atan2(first.length_squared() - second.length_squared());
    let (sin, cos) = turn.sin_cos();
    let major = first * cos + second * sin;
    let minor = second * cos - first * sin;
    let normal = major.cross(minor).try_normalize()?;
    let (major_radius, minor_radius) = (major.length(), minor.length());
    let frame = Plane::from_frame(center, normal, major.try_normalize()?)?;
    if major_radius - minor_radius <= ROUND_ELLIPSE * major_radius {
        Circle::new(frame, major_radius).ok().map(Curve::from)
    } else {
        Ellipse::new(frame, major_radius, minor_radius)
            .ok()
            .map(Curve::from)
    }
}

fn plane_extrusion(
    plane: &PlaneSurface,
    extrusion: &Extrusion,
    patch: &SurfacePatch,
    window: &Aabb,
) -> Option<Raw> {
    let frame = plane.frame();
    let normal = frame.normal();
    let direction = extrusion.direction();
    let rate = normal.dot(direction);
    let profile = extrusion.profile();
    let u_range = patch.u_range();
    let mut raw = Raw::default();
    if rate.abs() * window.diagonal() <= TOLERANCE {
        let surface = Surface::Plane(*plane);
        let found = intersect_curve_surface(profile, u_range, &surface, None).ok()?;
        for point in &found.points {
            raw.curves
                .extend(line(point.point, direction, window, point.tangent));
        }
        for overlap in &found.overlaps {
            for parameter in [overlap.range.start(), overlap.range.end()] {
                raw.curves
                    .extend(line(profile.point(parameter), direction, window, true));
            }
        }
        return Some(raw);
    }
    let map = |point: Point3| point - direction * (frame.signed_distance(point) / rate);
    let map_vector = |vector: Vector3| vector - direction * (normal.dot(vector) / rate);
    let curve = match profile {
        Curve::Line(profile_line) => {
            let mapped = map_vector(profile_line.direction());
            raw.curves
                .extend(line(map(profile_line.origin()), mapped, window, false));
            return Some(raw);
        }
        Curve::Circle(circle) => {
            let own = circle.frame();
            conjugate_ellipse(
                map(own.origin()),
                map_vector(own.x_axis() * circle.radius()),
                map_vector(own.y_axis() * circle.radius()),
            )?
        }
        Curve::Ellipse(ellipse) => {
            let own = ellipse.frame();
            conjugate_ellipse(
                map(own.origin()),
                map_vector(own.x_axis() * ellipse.major_radius()),
                map_vector(own.y_axis() * ellipse.minor_radius()),
            )?
        }
        Curve::BSpline(spline) => {
            let mapped = spline.map_points(map).ok()?;
            let domain = mapped.domain();
            let start = domain.clamp(u_range.start());
            let range = Interval::new(start, domain.clamp(u_range.end()).max(start))?;
            raw.curves.push(RawCurve {
                curve: mapped.into(),
                range,
                tangent: false,
            });
            return Some(raw);
        }
        _ => return None,
    };
    raw.curves.push(closed(curve));
    Some(raw)
}

fn cylinder_cylinder(first: &Cylinder, second: &Cylinder, window: &Aabb) -> Option<Raw> {
    let (a1, a2) = (first.frame().normal(), second.frame().normal());
    let (c1, c2) = (first.frame().origin(), second.frame().origin());
    let (r1, r2) = (first.radius(), second.radius());
    let mut raw = Raw::default();
    let cross = a1.cross(a2);
    if cross.length() * window.diagonal() <= TOLERANCE {
        let offset = c2 - c1;
        let across = offset - a1 * offset.dot(a1);
        let apart = across.length();
        if apart <= TOLERANCE {
            return None;
        }
        let outward = across / apart;
        let side = a1.cross(outward);
        let along = (apart * apart + r1 * r1 - r2 * r2) / (2.0 * apart);
        let touching =
            (apart - (r1 + r2)).abs() <= TOLERANCE || (apart - (r1 - r2).abs()).abs() <= TOLERANCE;
        if touching {
            let sign = if along < 0.0 { -1.0 } else { 1.0 };
            raw.curves
                .extend(line(c1 + outward * (r1 * sign), a1, window, true));
        } else if along.abs() < r1 {
            let half = (r1 * r1 - along * along).sqrt();
            let base = c1 + outward * along;
            raw.curves
                .extend(line(base + side * half, a1, window, false));
            raw.curves
                .extend(line(base - side * half, a1, window, false));
        }
        return Some(raw);
    }
    if (r1 - r2).abs() > TOLERANCE {
        return None;
    }
    let offset = c2 - c1;
    let squared = cross.length_squared();
    let s = offset.cross(a2).dot(cross) / squared;
    let t = offset.cross(a1).dot(cross) / squared;
    let (p1, p2) = (c1 + a1 * s, c2 + a2 * t);
    if p1.distance(p2) > TOLERANCE {
        return None;
    }
    let center = p1.lerp(p2, 0.5);
    for normal in [a1 - a2, a1 + a2] {
        let frame = Plane::new(center, normal)?;
        raw.curves
            .extend(plane_cylinder_ellipse(&frame, c1, a1, r1).map(closed));
    }
    let touching = cross.normalize();
    for sign in [1.0, -1.0] {
        raw.points.push(RawPoint {
            point: center + touching * (sign * r1),
            tangent: true,
        });
    }
    Some(raw)
}
