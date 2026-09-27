use caditor_geometry::{Aabb, Aabb2, Plane, Point2, Point3, Vector3};

use crate::{
    curve::Circle,
    intersect::IntersectionError,
    interval::{Domain, Interval},
    surface::Surface,
};

const PARAMETER_SLACK: f64 = 1e-10;
const ROTATIONAL_STEP: f64 = 0.2;
const MAX_ROTATIONAL_SAMPLES: usize = 64;
const SAGITTA_SAFETY: f64 = 1.5;
const TINY_RADIUS: f64 = 1e-300;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfacePatch<'a> {
    surface: &'a Surface,
    bounds: Aabb2,
}

fn clamp_range(
    start: f64,
    end: f64,
    period: Option<f64>,
    domain: Domain,
) -> Result<(f64, f64), IntersectionError> {
    match period {
        Some(period) => Ok((start, end.min(start + period))),
        None => {
            let (low, high) = (domain.clamp(start), domain.clamp(end));
            if high < low {
                Err(IntersectionError::InvalidBounds)
            } else {
                Ok((low, high))
            }
        }
    }
}

impl<'a> SurfacePatch<'a> {
    pub fn new(surface: &'a Surface, bounds: Aabb2) -> Result<Self, IntersectionError> {
        let (min, max) = (bounds.min(), bounds.max());
        if !min.is_finite() || !max.is_finite() || max.x < min.x || max.y < min.y {
            return Err(IntersectionError::InvalidBounds);
        }
        let (u_start, u_end) = clamp_range(min.x, max.x, surface.u_period(), surface.u_domain())?;
        let (v_start, v_end) = clamp_range(min.y, max.y, surface.v_period(), surface.v_domain())?;
        Ok(Self {
            surface,
            bounds: Aabb2::from_point(Point2::new(u_start, v_start))
                .including(Point2::new(u_end, v_end)),
        })
    }

    pub fn surface(&self) -> &'a Surface {
        self.surface
    }

    pub fn bounds(&self) -> Aabb2 {
        self.bounds
    }

    pub fn u_range(&self) -> Interval {
        Interval::new(self.bounds.min().x, self.bounds.max().x).unwrap_or(Interval::UNIT)
    }

    pub fn v_range(&self) -> Interval {
        Interval::new(self.bounds.min().y, self.bounds.max().y).unwrap_or(Interval::UNIT)
    }

    pub fn wrap(&self, uv: Point2) -> Point2 {
        Point2::new(
            wrap_into(uv.x, self.u_range(), self.surface.u_period()),
            wrap_into(uv.y, self.v_range(), self.surface.v_period()),
        )
    }

    pub fn contains(&self, uv: Point2) -> bool {
        let wrapped = self.wrap(uv);
        let within = |value: f64, range: Interval| {
            let slack = PARAMETER_SLACK * (1.0 + range.start().abs().max(range.end().abs()));
            value >= range.start() - slack && value <= range.end() + slack
        };
        let v_inside = within(wrapped.y, self.v_range());
        if !v_inside {
            return false;
        }
        within(wrapped.x, self.u_range()) || self.surface.pole_at(uv).is_some()
    }

    pub fn locate(&self, point: Point3) -> Option<Point2> {
        let uv = self.wrap(self.surface.project(point, Some(self.bounds.center())));
        self.contains(uv).then_some(uv)
    }

    pub fn bounding_box(&self) -> Aabb {
        patch_bounds(self.surface, self.bounds)
    }

    pub(crate) fn halves(&self) -> [Self; 2] {
        let (min, max) = (self.bounds.min(), self.bounds.max());
        let du = (max.x - min.x) * self.u_scale();
        let dv = (max.y - min.y) * self.v_scale();
        let middle = self.bounds.center();
        let (first, second) = if du >= dv {
            (
                Aabb2::from_point(min).including(Point2::new(middle.x, max.y)),
                Aabb2::from_point(Point2::new(middle.x, min.y)).including(max),
            )
        } else {
            (
                Aabb2::from_point(min).including(Point2::new(max.x, middle.y)),
                Aabb2::from_point(Point2::new(min.x, middle.y)).including(max),
            )
        };
        [
            Self {
                surface: self.surface,
                bounds: first,
            },
            Self {
                surface: self.surface,
                bounds: second,
            },
        ]
    }

    fn u_scale(&self) -> f64 {
        let center = self.bounds.center();
        let (min, max) = (self.bounds.min(), self.bounds.max());
        [min.y, center.y, max.y]
            .into_iter()
            .map(|v| self.surface.evaluate(center.x, v).du.length())
            .fold(0.0, f64::max)
    }

    fn v_scale(&self) -> f64 {
        let center = self.bounds.center();
        let (min, max) = (self.bounds.min(), self.bounds.max());
        [min.x, center.x, max.x]
            .into_iter()
            .map(|u| self.surface.evaluate(u, center.y).dv.length())
            .fold(0.0, f64::max)
    }
}

pub(crate) fn wrap_into(value: f64, range: Interval, period: Option<f64>) -> f64 {
    let Some(period) = period.filter(|period| *period > 0.0) else {
        return value;
    };
    let slack = PARAMETER_SLACK * (1.0 + range.start().abs().max(range.end().abs()));
    let wrapped = range.start() + (value - range.start()).rem_euclid(period);
    if wrapped > range.end() + slack && wrapped >= range.start() + period - slack {
        wrapped - period
    } else {
        wrapped
    }
}

pub(crate) fn boxes_overlap(a: &Aabb, b: &Aabb, margin: f64) -> bool {
    let low = a.min().max(b.min());
    let high = a.max().min(b.max());
    (high - low).min_element() >= -margin
}

fn translated(bounds: Aabb, offset: Vector3) -> Aabb {
    Aabb::from_point(bounds.min() + offset).including(bounds.max() + offset)
}

fn arc_bounds(
    center: Point3,
    axis: Vector3,
    x_axis: Vector3,
    radius: f64,
    range: Interval,
) -> Aabb {
    let frame = Plane::from_frame(center, axis, x_axis);
    match frame.map(|frame| Circle::new(frame, radius)) {
        Some(Ok(circle)) if radius > TINY_RADIUS => circle.bounds(range),
        _ => Aabb::from_point(center),
    }
}

fn rotational_bounds(
    u_range: Interval,
    v_range: Interval,
    ring: impl Fn(f64) -> (Point3, Vector3, Vector3, f64),
    second_derivative: impl Fn(f64) -> f64,
) -> Aabb {
    let pieces = (v_range.length() / ROTATIONAL_STEP).ceil();
    let pieces = if pieces.is_finite() && pieces >= 1.0 {
        (pieces as usize).min(MAX_ROTATIONAL_SAMPLES)
    } else {
        1
    };
    let step = v_range.length() / pieces as f64;
    let mut bounds: Option<Aabb> = None;
    let mut curvature: f64 = 0.0;
    for v in v_range.split(pieces) {
        let (center, axis, x_axis, radius) = ring(v);
        let arc = arc_bounds(center, axis, x_axis, radius, u_range);
        bounds = Some(bounds.map_or(arc, |bounds| bounds.union(arc)));
        curvature = curvature.max(second_derivative(v));
    }
    let margin = SAGITTA_SAFETY * step * step * curvature / 8.0;
    bounds
        .unwrap_or_else(|| Aabb::from_point(Point3::ZERO))
        .expanded(margin)
}

pub(crate) fn patch_bounds(surface: &Surface, bounds: Aabb2) -> Aabb {
    let (min, max) = (bounds.min(), bounds.max());
    let u_range = Interval::new(min.x, max.x).unwrap_or(Interval::UNIT);
    let v_range = Interval::new(min.y, max.y).unwrap_or(Interval::UNIT);
    match surface {
        Surface::Plane(_) => Aabb::from_points(
            [
                min,
                max,
                Point2::new(min.x, max.y),
                Point2::new(max.x, min.y),
            ]
            .map(|uv| surface.point_at(uv)),
        )
        .unwrap_or_else(|| Aabb::from_point(Point3::ZERO)),
        Surface::Cylinder(_) | Surface::Cone(_) => {
            rotational_bounds(u_range, v_range, |v| rotational_ring(surface, v), |_| 0.0)
        }
        Surface::Sphere(sphere) => {
            let radius = sphere.radius();
            rotational_bounds(
                u_range,
                v_range,
                |v| rotational_ring(surface, v),
                |_| radius,
            )
        }
        Surface::Torus(torus) => {
            let radius = torus.minor_radius();
            rotational_bounds(
                u_range,
                v_range,
                |v| rotational_ring(surface, v),
                |_| radius,
            )
        }
        Surface::Revolution(revolution) => rotational_bounds(
            u_range,
            v_range,
            |v| rotational_ring(surface, v),
            |v| revolution.profile().evaluate(v).second.length(),
        ),
        Surface::Extrusion(extrusion) => {
            let profile = extrusion.profile().piece_bounds(u_range);
            let direction = extrusion.direction();
            translated(profile, direction * v_range.start())
                .union(translated(profile, direction * v_range.end()))
        }
    }
}

pub(crate) fn rotational_ring(surface: &Surface, v: f64) -> (Point3, Vector3, Vector3, f64) {
    let at_zero = surface.evaluate(0.0, v);
    let turned = surface.evaluate(std::f64::consts::FRAC_PI_2, v);
    let (axis_origin, axis) = match surface {
        Surface::Cylinder(cylinder) => (cylinder.frame().origin(), cylinder.frame().normal()),
        Surface::Cone(cone) => (cone.frame().origin(), cone.frame().normal()),
        Surface::Sphere(sphere) => (sphere.frame().origin(), sphere.frame().normal()),
        Surface::Torus(torus) => (torus.frame().origin(), torus.frame().normal()),
        Surface::Revolution(revolution) => (revolution.axis_origin(), revolution.axis_direction()),
        Surface::Plane(plane) => (plane.frame().origin(), plane.frame().normal()),
        Surface::Extrusion(extrusion) => (at_zero.point, extrusion.direction()),
    };
    let center = axis_origin + axis * (at_zero.point - axis_origin).dot(axis);
    let radial = at_zero.point - center;
    let radius = radial.length();
    let x_axis = radial
        .try_normalize()
        .or_else(|| (turned.point - center).try_normalize())
        .unwrap_or_else(|| axis.any_orthonormal_vector());
    (center, axis, x_axis, radius)
}
