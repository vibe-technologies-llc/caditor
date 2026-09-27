use std::f64::consts::{PI, TAU};

use caditor_geometry::{Plane, Point2, Point3, Vector3};

use crate::{
    checks::{finite_point, unit},
    curve::Curve,
    error::GeometryError,
    interval::Interval,
    numeric::{Taylor, minimize_near},
    parametric::{closest_parameter_near, refined_seeds},
    surface::{
        SurfaceDerivatives,
        projection::{AXIS_EPSILON, periodic_near},
    },
    tolerance::{LINEAR_RESOLUTION, parallel},
};

const PROFILE_SEED_REFINEMENT: usize = 4;
const DEGENERACY_SAMPLES: usize = 16;
const UNBOUNDED_REACH: f64 = 1.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Extrusion {
    profile: Curve,
    direction: Vector3,
}

impl Extrusion {
    pub fn new(profile: Curve, direction: Vector3) -> Result<Self, GeometryError> {
        let direction = unit(direction)?;
        let range = profile.domain().clipped(UNBOUNDED_REACH);
        let sweeps = range.split(DEGENERACY_SAMPLES).any(|parameter| {
            let tangent = profile.evaluate(parameter).first;
            tangent.length() > 0.0 && !parallel(tangent, direction)
        });
        if !sweeps {
            return Err(GeometryError::DegenerateSurface);
        }
        Ok(Self { profile, direction })
    }

    pub fn profile(&self) -> &Curve {
        &self.profile
    }

    pub fn direction(&self) -> Vector3 {
        self.direction
    }

    pub(crate) fn evaluate(&self, u: f64, v: f64) -> SurfaceDerivatives {
        let curve = self.profile.evaluate(u);
        SurfaceDerivatives {
            point: curve.point + self.direction * v,
            du: curve.first,
            dv: self.direction,
            duu: curve.second,
            duv: Vector3::ZERO,
            dvv: Vector3::ZERO,
        }
    }

    pub(crate) fn project(&self, point: Point3, hint: Option<Point2>) -> Point2 {
        let across = |vector: Vector3| vector - self.direction * vector.dot(self.direction);
        let u = match &self.profile {
            Curve::Line(line) => {
                let offset = point - line.origin();
                let tilt = line.direction().dot(self.direction);
                let denominator = 1.0 - tilt * tilt;
                if denominator > 0.0 {
                    (offset.dot(line.direction()) - offset.dot(self.direction) * tilt) / denominator
                } else {
                    0.0
                }
            }
            profile => {
                let range = profile_search_range(profile, hint.map(|hint| hint.x));
                let samples = refined_seeds(profile, range, PROFILE_SEED_REFINEMENT);
                let objective = |parameter: f64| {
                    let curve = profile.evaluate(range.clamp(parameter));
                    let offset = across(curve.point - point);
                    let first = across(curve.first);
                    let second = across(curve.second);
                    Taylor {
                        value: offset.dot(offset),
                        slope: 2.0 * first.dot(offset),
                        curvature: 2.0 * (first.dot(first) + second.dot(offset)),
                    }
                };
                let found = range.clamp(
                    minimize_near(&samples, objective, range, hint.map(|hint| hint.x))
                        .unwrap_or(range.start()),
                );
                match profile.period() {
                    Some(period) => periodic_near(found, period, hint.map(|hint| hint.x)),
                    None => found,
                }
            }
        };
        let v = (point - self.profile.point(u)).dot(self.direction);
        Point2::new(u, v)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Revolution {
    profile: Curve,
    axis: Plane,
    meridian_angle: f64,
}

impl Revolution {
    pub fn new(
        profile: Curve,
        axis_origin: Point3,
        axis_direction: Vector3,
    ) -> Result<Self, GeometryError> {
        let axis = Plane::new(finite_point(axis_origin)?, unit(axis_direction)?)
            .ok_or(GeometryError::ZeroDirection)?;
        let range = profile.domain().clipped(UNBOUNDED_REACH);
        let farthest = range
            .split(DEGENERACY_SAMPLES)
            .map(|parameter| profile.point(parameter) - axis.origin())
            .map(|offset| offset - axis.normal() * offset.dot(axis.normal()))
            .max_by(|a, b| a.length().total_cmp(&b.length()))
            .filter(|offset| offset.length() > LINEAR_RESOLUTION)
            .ok_or(GeometryError::DegenerateSurface)?;
        let meridian_angle = farthest
            .dot(axis.y_axis())
            .atan2(farthest.dot(axis.x_axis()));
        Ok(Self {
            profile,
            axis,
            meridian_angle,
        })
    }

    pub fn profile(&self) -> &Curve {
        &self.profile
    }

    pub fn axis_origin(&self) -> Point3 {
        self.axis.origin()
    }

    pub fn axis_direction(&self) -> Vector3 {
        self.axis.normal()
    }

    pub(crate) fn rotate(&self, vector: Vector3, angle: f64) -> Vector3 {
        let axis = self.axis.normal();
        let (sin, cos) = angle.sin_cos();
        vector * cos + axis.cross(vector) * sin + axis * (axis.dot(vector) * (1.0 - cos))
    }

    pub(crate) fn evaluate(&self, u: f64, v: f64) -> SurfaceDerivatives {
        let axis = self.axis.normal();
        let curve = self.profile.evaluate(v);
        let offset = curve.point - self.axis.origin();
        let around = axis.cross(offset);
        SurfaceDerivatives {
            point: self.axis.origin() + self.rotate(offset, u),
            du: self.rotate(around, u),
            dv: self.rotate(curve.first, u),
            duu: self.rotate(axis.cross(around), u),
            duv: self.rotate(axis.cross(curve.first), u),
            dvv: self.rotate(curve.second, u),
        }
    }

    pub(crate) fn distance_from_axis(&self, point: Point3) -> f64 {
        let offset = point - self.axis.origin();
        (offset - self.axis.normal() * offset.dot(self.axis.normal())).length()
    }

    pub(crate) fn axis_point(&self, point: Point3) -> Point3 {
        let offset = point - self.axis.origin();
        self.axis.origin() + self.axis.normal() * offset.dot(self.axis.normal())
    }

    pub(crate) fn project_seed(&self, point: Point3, hint: Option<Point2>) -> Point2 {
        let offset = point - self.axis.origin();
        let (x, y) = (
            offset.dot(self.axis.x_axis()),
            offset.dot(self.axis.y_axis()),
        );
        let turn = if x.hypot(y) > AXIS_EPSILON * (1.0 + offset.length()) {
            y.atan2(x) - self.meridian_angle
        } else {
            hint.map_or(0.0, |hint| hint.x)
        };
        [turn, turn + PI]
            .into_iter()
            .map(|angle| {
                let unrotated = self.axis.origin() + self.rotate(offset, -angle);
                let v = closest_on_profile(&self.profile, unrotated, hint.map(|hint| hint.y));
                let distance = self.profile.point(v).distance_squared(unrotated);
                (Point2::new(angle.rem_euclid(TAU), v), distance)
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(Point2::ZERO, |(uv, _)| uv)
    }
}

pub(crate) fn profile_search_range(profile: &Curve, hint: Option<f64>) -> Interval {
    match (profile.period(), hint.filter(|hint| hint.is_finite())) {
        (Some(period), Some(hint)) => {
            Interval::new(hint - 0.5 * period, hint + 0.5 * period).unwrap_or(Interval::FULL_TURN)
        }
        _ => profile.domain().clipped(UNBOUNDED_REACH),
    }
}

pub(crate) fn closest_on_profile(profile: &Curve, point: Point3, hint: Option<f64>) -> f64 {
    match profile {
        Curve::Line(line) => line.parameter_of(point),
        Curve::Circle(_) => {
            let range = profile_search_range(profile, hint);
            let found = profile.closest_parameter(point, range);
            periodic_near(found, TAU, hint)
        }
        profile => {
            let range = profile_search_range(profile, hint);
            let found = closest_parameter_near(profile, point, range, hint);
            match profile.period() {
                Some(period) => periodic_near(found, period, hint),
                None => found,
            }
        }
    }
}
