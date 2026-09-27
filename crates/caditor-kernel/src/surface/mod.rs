mod coincidence;
mod elementary;
mod projection;
mod swept;
#[cfg(test)]
mod tests;

use std::f64::consts::{FRAC_PI_2, TAU};

use caditor_geometry::{Point2, Point3, RigidTransform, Vector3};

pub(crate) use self::projection::{periodic_near, refine as refine_projection};
pub use self::{
    elementary::{Cone, Cylinder, PlaneSurface, Sphere, Torus},
    swept::{Extrusion, Revolution},
};
use crate::{
    error::GeometryError,
    interval::{Domain, Interval},
    sense::Sense,
    tolerance::LINEAR_RESOLUTION,
};

const LATITUDE: Interval = Interval::constant(-FRAC_PI_2, FRAC_PI_2);
const POLE_PARAMETER_TOLERANCE: f64 = 1e-6;
const NORMAL_NUDGE: f64 = 1e-7;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceDerivatives {
    pub point: Point3,
    pub du: Vector3,
    pub dv: Vector3,
    pub duu: Vector3,
    pub duv: Vector3,
    pub dvv: Vector3,
}

impl SurfaceDerivatives {
    pub fn normal(&self) -> Option<Vector3> {
        self.du.cross(self.dv).try_normalize()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pole {
    pub v: f64,
    pub point: Point3,
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Surface {
    Plane(PlaneSurface),
    Cylinder(Cylinder),
    Cone(Cone),
    Sphere(Sphere),
    Torus(Torus),
    Extrusion(Extrusion),
    Revolution(Revolution),
}

impl Surface {
    pub fn evaluate(&self, u: f64, v: f64) -> SurfaceDerivatives {
        match self {
            Self::Plane(plane) => plane.evaluate(u, v),
            Self::Cylinder(cylinder) => cylinder.evaluate(u, v),
            Self::Cone(cone) => cone.evaluate(u, v),
            Self::Sphere(sphere) => sphere.evaluate(u, v),
            Self::Torus(torus) => torus.evaluate(u, v),
            Self::Extrusion(extrusion) => extrusion.evaluate(u, v),
            Self::Revolution(revolution) => revolution.evaluate(u, v),
        }
    }

    pub fn point(&self, u: f64, v: f64) -> Point3 {
        self.evaluate(u, v).point
    }

    pub fn point_at(&self, uv: Point2) -> Point3 {
        self.point(uv.x, uv.y)
    }

    pub fn normal(&self, u: f64, v: f64) -> Option<Vector3> {
        match self {
            Self::Plane(plane) => Some(plane.frame().normal()),
            Self::Cylinder(cylinder) => Some(cylinder.normal(u)),
            Self::Cone(cone) => Some(cone.normal(u)),
            Self::Sphere(sphere) => Some(sphere.normal(u, v)),
            Self::Torus(torus) => Some(torus.normal(u, v)),
            Self::Extrusion(_) | Self::Revolution(_) => self
                .evaluate(u, v)
                .normal()
                .or_else(|| self.nudged_normal(u, v)),
        }
    }

    fn nudged_normal(&self, u: f64, v: f64) -> Option<Vector3> {
        let nudge = |domain: Domain, value: f64| {
            let reach = domain
                .bounded()
                .map_or(1.0, |interval| interval.length().max(f64::MIN_POSITIVE));
            let step = NORMAL_NUDGE * reach;
            if value + step <= domain.end() {
                value + step
            } else {
                value - step
            }
        };
        let (u_domain, v_domain) = (self.u_domain(), self.v_domain());
        [
            (u, nudge(v_domain, v)),
            (nudge(u_domain, u), v),
            (nudge(u_domain, u), nudge(v_domain, v)),
        ]
        .into_iter()
        .find_map(|(u, v)| self.evaluate(u, v).normal())
    }

    pub fn u_period(&self) -> Option<f64> {
        match self {
            Self::Plane(_) => None,
            Self::Cylinder(_)
            | Self::Cone(_)
            | Self::Sphere(_)
            | Self::Torus(_)
            | Self::Revolution(_) => Some(TAU),
            Self::Extrusion(extrusion) => extrusion.profile().period(),
        }
    }

    pub fn v_period(&self) -> Option<f64> {
        match self {
            Self::Torus(_) => Some(TAU),
            Self::Revolution(revolution) => revolution.profile().period(),
            Self::Plane(_)
            | Self::Cylinder(_)
            | Self::Cone(_)
            | Self::Sphere(_)
            | Self::Extrusion(_) => None,
        }
    }

    pub fn u_domain(&self) -> Domain {
        match self {
            Self::Plane(_) => Domain::UNBOUNDED,
            Self::Cylinder(_)
            | Self::Cone(_)
            | Self::Sphere(_)
            | Self::Torus(_)
            | Self::Revolution(_) => Domain::from(Interval::FULL_TURN),
            Self::Extrusion(extrusion) => extrusion.profile().domain(),
        }
    }

    pub fn v_domain(&self) -> Domain {
        match self {
            Self::Plane(_) | Self::Cylinder(_) | Self::Extrusion(_) => Domain::UNBOUNDED,
            Self::Cone(cone) => {
                let apex = cone.apex_parameter();
                if cone.half_angle() > 0.0 {
                    Domain::new(apex, f64::INFINITY)
                } else {
                    Domain::new(f64::NEG_INFINITY, apex)
                }
                .unwrap_or(Domain::UNBOUNDED)
            }
            Self::Sphere(_) => Domain::from(LATITUDE),
            Self::Torus(_) => Domain::from(Interval::FULL_TURN),
            Self::Revolution(revolution) => revolution.profile().domain(),
        }
    }

    pub fn poles(&self) -> Vec<Pole> {
        match self {
            Self::Sphere(sphere) => vec![
                Pole {
                    v: LATITUDE.start(),
                    point: sphere.center() - sphere.frame().normal() * sphere.radius(),
                },
                Pole {
                    v: LATITUDE.end(),
                    point: sphere.center() + sphere.frame().normal() * sphere.radius(),
                },
            ],
            Self::Cone(cone) => vec![Pole {
                v: cone.apex_parameter(),
                point: cone.apex(),
            }],
            Self::Revolution(revolution) if revolution.profile().period().is_none() => revolution
                .profile()
                .domain()
                .bounded()
                .map(|range| {
                    [range.start(), range.end()]
                        .into_iter()
                        .map(|v| (v, revolution.profile().point(v)))
                        .filter(|(_, point)| {
                            revolution.distance_from_axis(*point) <= LINEAR_RESOLUTION
                        })
                        .map(|(v, point)| Pole {
                            v,
                            point: revolution.axis_point(point),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            Self::Plane(_)
            | Self::Cylinder(_)
            | Self::Torus(_)
            | Self::Extrusion(_)
            | Self::Revolution(_) => Vec::new(),
        }
    }

    pub fn pole_at(&self, uv: Point2) -> Option<Pole> {
        self.poles().into_iter().find(|pole| {
            (uv.y - pole.v).abs() <= POLE_PARAMETER_TOLERANCE * (1.0 + pole.v.abs())
                && self.point_at(uv).distance(pole.point) <= LINEAR_RESOLUTION
        })
    }

    pub fn project(&self, point: Point3, hint: Option<Point2>) -> Point2 {
        let hint = hint.filter(|hint| hint.is_finite());
        if !point.is_finite() {
            return hint.unwrap_or_else(|| {
                Point2::new(self.u_domain().clamp(0.0), self.v_domain().clamp(0.0))
            });
        }
        match self {
            Self::Plane(plane) => plane.project(point),
            Self::Cylinder(cylinder) => cylinder.project(point, hint),
            Self::Cone(cone) => cone.project(point, hint),
            Self::Sphere(sphere) => sphere.project(point, hint),
            Self::Torus(torus) => torus.project(point, hint),
            Self::Extrusion(extrusion) => extrusion.project(point, hint),
            Self::Revolution(revolution) => {
                let seed = revolution.project_seed(point, hint);
                let refined = projection::refine(self, point, seed);
                let u = periodic_near(refined.x, TAU, hint.map(|hint| hint.x));
                let v = match self.v_period() {
                    Some(period) => periodic_near(refined.y, period, hint.map(|hint| hint.y)),
                    None => refined.y,
                };
                let uv = Point2::new(u, v);
                match (self.pole_at(uv), hint) {
                    (Some(_), Some(hint)) => Point2::new(hint.x, v),
                    _ => uv,
                }
            }
        }
    }

    pub fn distance(&self, point: Point3) -> f64 {
        self.point_at(self.project(point, None)).distance(point)
    }

    pub fn same_surface(&self, other: &Self) -> Option<Sense> {
        coincidence::same_surface(self, other)
    }

    pub fn transformed(&self, transform: &RigidTransform) -> Result<Self, GeometryError> {
        Ok(match self {
            Self::Plane(plane) => {
                Self::Plane(PlaneSurface::new(plane.frame().transformed(transform))?)
            }
            Self::Cylinder(cylinder) => Self::Cylinder(Cylinder::new(
                cylinder.frame().transformed(transform),
                cylinder.radius(),
            )?),
            Self::Cone(cone) => Self::Cone(Cone::new(
                cone.frame().transformed(transform),
                cone.radius(),
                cone.half_angle(),
            )?),
            Self::Sphere(sphere) => Self::Sphere(Sphere::new(
                sphere.frame().transformed(transform),
                sphere.radius(),
            )?),
            Self::Torus(torus) => Self::Torus(Torus::new(
                torus.frame().transformed(transform),
                torus.major_radius(),
                torus.minor_radius(),
            )?),
            Self::Extrusion(extrusion) => Self::Extrusion(Extrusion::new(
                extrusion.profile().transformed(transform)?,
                transform.apply_vector(extrusion.direction()),
            )?),
            Self::Revolution(revolution) => Self::Revolution(Revolution::new(
                revolution.profile().transformed(transform)?,
                transform.apply_point(revolution.axis_origin()),
                transform.apply_vector(revolution.axis_direction()),
            )?),
        })
    }
}

impl From<PlaneSurface> for Surface {
    fn from(plane: PlaneSurface) -> Self {
        Self::Plane(plane)
    }
}

impl From<Cylinder> for Surface {
    fn from(cylinder: Cylinder) -> Self {
        Self::Cylinder(cylinder)
    }
}

impl From<Cone> for Surface {
    fn from(cone: Cone) -> Self {
        Self::Cone(cone)
    }
}

impl From<Sphere> for Surface {
    fn from(sphere: Sphere) -> Self {
        Self::Sphere(sphere)
    }
}

impl From<Torus> for Surface {
    fn from(torus: Torus) -> Self {
        Self::Torus(torus)
    }
}

impl From<Extrusion> for Surface {
    fn from(extrusion: Extrusion) -> Self {
        Self::Extrusion(extrusion)
    }
}

impl From<Revolution> for Surface {
    fn from(revolution: Revolution) -> Self {
        Self::Revolution(revolution)
    }
}
