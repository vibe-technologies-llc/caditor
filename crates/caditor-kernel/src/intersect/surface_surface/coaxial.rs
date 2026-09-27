use std::f64::consts::FRAC_PI_2;

use caditor_geometry::{Aabb, Plane, Point2, Point3, Vector2, Vector3};

use crate::{
    curve::{Circle, Curve},
    curve2::{Circle2, Curve2, Line2},
    intersect::{
        SurfacePatch, intersect_curves2,
        surface_surface::{Raw, RawCurve, RawPoint},
    },
    interval::Interval,
    surface::Surface,
    tolerance::{LINEAR_RESOLUTION, parallel},
};

const TOLERANCE: f64 = LINEAR_RESOLUTION;
const PROFILE_CHECK_SAMPLES: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq)]
enum AxisKind {
    Fixed { origin: Point3, direction: Vector3 },
    Center(Point3),
    Normal(Vector3),
}

fn axis_kind(surface: &Surface) -> Option<AxisKind> {
    match surface {
        Surface::Plane(plane) => Some(AxisKind::Normal(plane.frame().normal())),
        Surface::Cylinder(cylinder) => Some(AxisKind::Fixed {
            origin: cylinder.frame().origin(),
            direction: cylinder.frame().normal(),
        }),
        Surface::Cone(cone) => Some(AxisKind::Fixed {
            origin: cone.frame().origin(),
            direction: cone.frame().normal(),
        }),
        Surface::Torus(torus) => Some(AxisKind::Fixed {
            origin: torus.frame().origin(),
            direction: torus.frame().normal(),
        }),
        Surface::Revolution(revolution) => Some(AxisKind::Fixed {
            origin: revolution.axis_origin(),
            direction: revolution.axis_direction(),
        }),
        Surface::Sphere(sphere) => Some(AxisKind::Center(sphere.center())),
        Surface::Extrusion(_) => None,
    }
}

fn off_axis(point: Point3, origin: Point3, direction: Vector3) -> f64 {
    let offset = point - origin;
    (offset - direction * offset.dot(direction)).length()
}

fn common_axis(first: AxisKind, second: AxisKind) -> Option<(Point3, Vector3)> {
    match (first, second) {
        (
            AxisKind::Fixed { origin, direction },
            AxisKind::Fixed {
                origin: other_origin,
                direction: other_direction,
            },
        ) => (parallel(direction, other_direction)
            && off_axis(other_origin, origin, direction) <= TOLERANCE)
            .then_some((origin, direction)),
        (AxisKind::Fixed { origin, direction }, AxisKind::Center(center))
        | (AxisKind::Center(center), AxisKind::Fixed { origin, direction }) => {
            (off_axis(center, origin, direction) <= TOLERANCE).then_some((origin, direction))
        }
        (AxisKind::Fixed { origin, direction }, AxisKind::Normal(normal))
        | (AxisKind::Normal(normal), AxisKind::Fixed { origin, direction }) => {
            parallel(direction, normal).then_some((origin, direction))
        }
        (AxisKind::Center(first), AxisKind::Center(second)) => {
            Some(match (second - first).try_normalize() {
                Some(direction) if first.distance(second) > TOLERANCE => (first, direction),
                _ => (first, Vector3::Z),
            })
        }
        (AxisKind::Center(center), AxisKind::Normal(normal))
        | (AxisKind::Normal(normal), AxisKind::Center(center)) => Some((center, normal)),
        (AxisKind::Normal(_), AxisKind::Normal(_)) => None,
    }
}

struct Axis {
    origin: Point3,
    direction: Vector3,
    reference: Vector3,
}

impl Axis {
    fn height(&self, point: Point3) -> f64 {
        (point - self.origin).dot(self.direction)
    }

    fn radius(&self, point: Point3) -> f64 {
        off_axis(point, self.origin, self.direction)
    }
}

struct Window {
    low: f64,
    high: f64,
    reach: f64,
}

fn window_of(axis: &Axis, bounds: &Aabb) -> Window {
    let corners = bounds.corners();
    let heights = corners.map(|corner| axis.height(corner));
    let low = heights.iter().copied().fold(f64::INFINITY, f64::min);
    let high = heights.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let reach = corners
        .iter()
        .map(|corner| axis.radius(*corner))
        .fold(0.0, f64::max)
        + bounds.diagonal();
    Window { low, high, reach }
}

fn meridian_of_profile(
    profile: &Curve,
    axis: &Axis,
    range: Interval,
) -> Option<(Curve2, Interval)> {
    let farthest = range
        .split(PROFILE_CHECK_SAMPLES)
        .map(|parameter| profile.point(parameter))
        .max_by(|a, b| axis.radius(*a).total_cmp(&axis.radius(*b)))?;
    let offset = farthest - axis.origin;
    let radial = (offset - axis.direction * offset.dot(axis.direction)).try_normalize()?;
    let across = radial.cross(axis.direction);
    let planar = range
        .split(PROFILE_CHECK_SAMPLES)
        .all(|parameter| (profile.point(parameter) - axis.origin).dot(across).abs() <= TOLERANCE);
    if !planar {
        return None;
    }
    let frame = Plane::from_frame(axis.origin, across, radial)?;
    let to_meridian = |point: Point3| frame.to_local(point);
    let to_direction =
        |vector: Vector3| Vector2::new(vector.dot(radial), vector.dot(axis.direction));
    let curve = match profile {
        Curve::Line(line) => Curve2::Line(
            Line2::new(to_meridian(line.origin()), to_direction(line.direction())).ok()?,
        ),
        Curve::Circle(circle) => {
            let own = circle.frame();
            let x_axis = to_direction(own.x_axis());
            let y_axis = to_direction(own.y_axis());
            Curve2::Circle(
                Circle2::with_axes(
                    to_meridian(own.origin()),
                    circle.radius(),
                    x_axis,
                    x_axis.perp_dot(y_axis) > 0.0,
                )
                .ok()?,
            )
        }
        Curve::BSpline(spline) => Curve2::BSpline(spline.map_points(to_meridian).ok()?),
        _ => return None,
    };
    Some((curve, range))
}

fn meridian(
    surface: &Surface,
    patch: &SurfacePatch,
    axis: &Axis,
    window: &Window,
) -> Option<(Curve2, Interval)> {
    let span = Interval::new(window.low, window.high)?;
    match surface {
        Surface::Plane(plane) => {
            let height = axis.height(plane.frame().origin());
            Some((
                Line2::new(Point2::new(0.0, height), Vector2::X)
                    .ok()?
                    .into(),
                Interval::new(0.0, window.reach)?,
            ))
        }
        Surface::Cylinder(cylinder) => Some((
            Line2::new(Point2::new(cylinder.radius(), 0.0), Vector2::Y)
                .ok()?
                .into(),
            span,
        )),
        Surface::Cone(cone) => {
            let apex = axis.height(cone.apex());
            let (sine, cosine) = cone.half_angle().abs().sin_cos();
            let opening = cone.opening_direction().dot(axis.direction).signum();
            let length = window.reach / sine;
            Some((
                Line2::new(Point2::new(0.0, apex), Vector2::new(sine, opening * cosine))
                    .ok()?
                    .into(),
                Interval::new(0.0, length)?,
            ))
        }
        Surface::Sphere(sphere) => Some((
            Circle2::new(
                Point2::new(0.0, axis.height(sphere.center())),
                sphere.radius(),
            )
            .ok()?
            .into(),
            Interval::new(-FRAC_PI_2, FRAC_PI_2)?,
        )),
        Surface::Torus(torus) => Some((
            Circle2::new(
                Point2::new(torus.major_radius(), axis.height(torus.frame().origin())),
                torus.minor_radius(),
            )
            .ok()?
            .into(),
            Interval::FULL_TURN,
        )),
        Surface::Revolution(revolution) => {
            let range = match revolution.profile().period() {
                Some(period) => Interval::new(0.0, period)?,
                None => patch.v_range(),
            };
            meridian_of_profile(revolution.profile(), axis, range)
        }
        Surface::Extrusion(_) => None,
    }
}

fn reference_of(surface: &Surface, direction: Vector3) -> Option<Vector3> {
    let x_axis = match surface {
        Surface::Plane(plane) => plane.frame().x_axis(),
        Surface::Cylinder(cylinder) => cylinder.frame().x_axis(),
        Surface::Cone(cone) => cone.frame().x_axis(),
        Surface::Sphere(sphere) => sphere.frame().x_axis(),
        Surface::Torus(torus) => torus.frame().x_axis(),
        Surface::Revolution(_) | Surface::Extrusion(_) => direction.any_orthonormal_vector(),
    };
    (x_axis - direction * x_axis.dot(direction)).try_normalize()
}

pub(crate) fn intersect(first: &SurfacePatch, second: &SurfacePatch, bounds: &Aabb) -> Option<Raw> {
    let (a, b) = (first.surface(), second.surface());
    let (origin, direction) = common_axis(axis_kind(a)?, axis_kind(b)?)?;
    let reference = reference_of(a, direction)
        .or_else(|| reference_of(b, direction))
        .unwrap_or_else(|| direction.any_orthonormal_vector());
    let axis = Axis {
        origin,
        direction,
        reference,
    };
    let window = window_of(&axis, bounds);
    let (first_meridian, first_range) = meridian(a, first, &axis, &window)?;
    let (second_meridian, second_range) = meridian(b, second, &axis, &window)?;
    let found =
        intersect_curves2(&first_meridian, first_range, &second_meridian, second_range).ok()?;
    let mut raw = Raw::default();
    let ring = |radius: f64, height: f64, tangent: bool, raw: &mut Raw| {
        let center = axis.origin + axis.direction * height;
        if radius <= TOLERANCE {
            raw.points.push(RawPoint {
                point: center,
                tangent: true,
            });
            return;
        }
        let circle = Plane::from_frame(center, axis.direction, axis.reference)
            .and_then(|frame| Circle::new(frame, radius).ok());
        if let Some(circle) = circle {
            raw.curves.push(RawCurve {
                curve: circle.into(),
                range: Interval::FULL_TURN,
                tangent,
            });
        }
    };
    for point in &found.points {
        if point.point.x < -TOLERANCE {
            continue;
        }
        ring(point.point.x, point.point.y, point.tangent, &mut raw);
    }
    for overlap in &found.overlaps {
        for parameter in [overlap.first.start(), overlap.first.end()] {
            let at = first_meridian.point(parameter);
            if at.x >= -TOLERANCE {
                ring(at.x, at.y, true, &mut raw);
            }
        }
    }
    Some(raw)
}
