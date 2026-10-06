mod coincidence;
mod elementary;
mod nurbs;
mod profile_spans;
mod projection;
mod swept;
#[cfg(test)]
mod tests;

use std::f64::consts::{FRAC_PI_2, TAU};

use caditor_geometry::{Point2, Point3, RigidTransform, Similarity, Vector3};

pub(crate) use self::projection::{periodic_near, refine as refine_projection};
pub use self::{
    elementary::{Cone, Cylinder, PlaneSurface, Sphere, Torus},
    nurbs::BSplineSurface,
    swept::{Extrusion, Revolution},
};
use crate::{
    error::GeometryError,
    interval::{Domain, Interval},
    mapping::{Affine, UvMap},
    sense::Sense,
    tolerance::LINEAR_RESOLUTION,
};

const LATITUDE: Interval = Interval::constant(-FRAC_PI_2, FRAC_PI_2);
const POLE_PARAMETER_TOLERANCE: f64 = 1e-6;
const NORMAL_NUDGE: f64 = 1e-7;
const HINT_PREFERENCE: f64 = 1e-3 * LINEAR_RESOLUTION;

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
    BSpline(BSplineSurface),
}

impl Surface {
    pub fn heap_size(&self) -> usize {
        match self {
            Self::Plane(_)
            | Self::Cylinder(_)
            | Self::Cone(_)
            | Self::Sphere(_)
            | Self::Torus(_) => 0,
            Self::Extrusion(extrusion) => extrusion.heap_size(),
            Self::Revolution(revolution) => revolution.heap_size(),
            Self::BSpline(spline) => spline.heap_size(),
        }
    }

    pub fn evaluate(&self, u: f64, v: f64) -> SurfaceDerivatives {
        match self {
            Self::Plane(plane) => plane.evaluate(u, v),
            Self::Cylinder(cylinder) => cylinder.evaluate(u, v),
            Self::Cone(cone) => cone.evaluate(u, v),
            Self::Sphere(sphere) => sphere.evaluate(u, v),
            Self::Torus(torus) => torus.evaluate(u, v),
            Self::Extrusion(extrusion) => extrusion.evaluate(u, v),
            Self::Revolution(revolution) => revolution.evaluate(u, v),
            Self::BSpline(spline) => spline.evaluate(u, v),
        }
    }

    pub fn point(&self, u: f64, v: f64) -> Point3 {
        match self {
            Self::BSpline(spline) => spline.point(u, v),
            _ => self.evaluate(u, v).point,
        }
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
            Self::Extrusion(_) | Self::Revolution(_) | Self::BSpline(_) => self
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
            Self::BSpline(spline) => spline.u_period(),
        }
    }

    pub fn v_period(&self) -> Option<f64> {
        match self {
            Self::Torus(_) => Some(TAU),
            Self::Revolution(revolution) => revolution.profile().period(),
            Self::BSpline(spline) => spline.v_period(),
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
            Self::BSpline(spline) => Domain::from(spline.u_domain()),
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
            Self::BSpline(spline) => Domain::from(spline.v_domain()),
        }
    }

    pub fn poles(&self) -> Vec<Pole> {
        self.pole_slots().into_iter().flatten().collect()
    }

    fn pole_slots(&self) -> [Option<Pole>; 2] {
        match self {
            Self::Sphere(sphere) => [
                Some(Pole {
                    v: LATITUDE.start(),
                    point: sphere.center() - sphere.frame().normal() * sphere.radius(),
                }),
                Some(Pole {
                    v: LATITUDE.end(),
                    point: sphere.center() + sphere.frame().normal() * sphere.radius(),
                }),
            ],
            Self::Cone(cone) => [
                Some(Pole {
                    v: cone.apex_parameter(),
                    point: cone.apex(),
                }),
                None,
            ],
            Self::Revolution(revolution) if revolution.profile().period().is_none() => {
                let Some(range) = revolution.profile().domain().bounded() else {
                    return [None; 2];
                };
                [range.start(), range.end()].map(|v| {
                    let point = revolution.profile().point(v);
                    (revolution.distance_from_axis(point) <= LINEAR_RESOLUTION).then(|| Pole {
                        v,
                        point: revolution.axis_point(point),
                    })
                })
            }
            Self::BSpline(spline) => spline.pole_slots(),
            Self::Plane(_)
            | Self::Cylinder(_)
            | Self::Torus(_)
            | Self::Extrusion(_)
            | Self::Revolution(_) => [None; 2],
        }
    }

    pub fn pole_at(&self, uv: Point2) -> Option<Pole> {
        self.pole_slots().into_iter().flatten().find(|pole| {
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
            Self::Cone(cone) => self.at_pole(cone.project(point, hint), hint),
            Self::Sphere(sphere) => self.at_pole(sphere.project(point, hint), hint),
            Self::Torus(torus) => torus.project(point, hint),
            Self::Extrusion(extrusion) => extrusion.project(point, hint),
            Self::Revolution(_) if let Some(on_surface) = self.project_on_surface(point, hint) => {
                on_surface
            }
            Self::Revolution(revolution) => {
                let seed = revolution.project_seed(point, hint);
                let refined = projection::refine(self, point, seed);
                let u = periodic_near(refined.x, TAU, hint.map(|hint| hint.x));
                let v = match self.v_period() {
                    Some(period) => periodic_near(refined.y, period, hint.map(|hint| hint.y)),
                    None => refined.y,
                };
                self.at_pole(Point2::new(u, v), hint)
            }
            Self::BSpline(spline) => self.project_on_spline(spline, point, hint),
        }
    }

    fn project_on_spline(
        &self,
        spline: &BSplineSurface,
        point: Point3,
        hint: Option<Point2>,
    ) -> Point2 {
        let foot = |seed: Point2, known: &[Point2]| {
            let refined = projection::refine_among(self, point, seed, known);
            (self.point_at(refined).distance(point), refined)
        };
        let from_hint = hint
            .map(|hint| foot(hint, &[]))
            .filter(|(distance, uv)| distance.is_finite() && uv.is_finite());
        if let (Some(hint), Some((distance, refined))) = (hint, from_hint)
            && distance <= LINEAR_RESOLUTION
        {
            return self.at_pole(spline.place(refined, Some(hint)), Some(hint));
        }
        let mut known: Vec<Point2> = from_hint.iter().map(|(_, uv)| *uv).collect();
        let mut best = from_hint;
        for seed in spline.project_seed(point) {
            let (distance, refined) = foot(seed, &known);
            known.push(refined);
            if best.is_none_or(|(closest, _)| distance < closest) {
                best = Some((distance, refined));
            }
        }
        if best.is_none_or(|(closest, _)| closest > LINEAR_RESOLUTION) {
            for seed in spline.span_seeds(point) {
                let (distance, refined) = foot(seed, &known);
                known.push(refined);
                if best.is_none_or(|(closest, _)| distance < closest) {
                    best = Some((distance, refined));
                }
                if distance <= LINEAR_RESOLUTION {
                    break;
                }
            }
        }
        let chosen = match (from_hint, best) {
            (Some((near, uv)), Some((closest, _))) if near <= closest + HINT_PREFERENCE => uv,
            (_, Some((_, uv))) => uv,
            (_, None) => hint.unwrap_or(Point2::ZERO),
        };
        self.at_pole(spline.place(chosen, hint), hint)
    }

    fn project_on_surface(&self, point: Point3, hint: Option<Point2>) -> Option<Point2> {
        let hint = hint?;
        let refined = projection::refine(self, point, hint);
        if !refined.is_finite() || self.point_at(refined).distance(point) > LINEAR_RESOLUTION {
            return None;
        }
        let u = periodic_near(refined.x, TAU, Some(hint.x));
        let v = match self.v_period() {
            Some(period) => periodic_near(refined.y, period, Some(hint.y)),
            None => refined.y,
        };
        Some(self.at_pole(Point2::new(u, v), Some(hint)))
    }

    fn at_pole(&self, uv: Point2, hint: Option<Point2>) -> Point2 {
        match (self.pole_at(uv), hint) {
            (Some(_), Some(hint)) => Point2::new(hint.x, uv.y),
            _ => uv,
        }
    }

    pub fn distance(&self, point: Point3) -> f64 {
        self.point_at(self.project(point, None)).distance(point)
    }

    pub fn same_surface(&self, other: &Self) -> Option<Sense> {
        coincidence::same_surface(self, other)
    }

    pub fn transformed(&self, transform: &RigidTransform) -> Result<Self, GeometryError> {
        self.mapped(&Similarity::from(*transform))
            .map(|(surface, _)| surface)
    }

    pub(crate) fn mapped(&self, similarity: &Similarity) -> Result<(Self, UvMap), GeometryError> {
        let scale = similarity.scale();
        let stretch = Affine::scaling(scale);
        let around = if similarity.is_mirrored() {
            Affine::TURNED_BACK
        } else {
            Affine::IDENTITY
        };
        let turned = |v: Affine| UvMap { u: around, v };
        Ok(match self {
            Self::Plane(plane) => (
                Self::Plane(PlaneSurface::new(plane.frame().mapped(similarity))?),
                UvMap {
                    u: stretch,
                    v: if similarity.is_mirrored() {
                        Affine::scaling(-scale)
                    } else {
                        stretch
                    },
                },
            ),
            Self::Cylinder(cylinder) => (
                Self::Cylinder(Cylinder::new(
                    cylinder.frame().mapped(similarity),
                    cylinder.radius() * scale,
                )?),
                turned(stretch),
            ),
            Self::Cone(cone) => (
                Self::Cone(Cone::new(
                    cone.frame().mapped(similarity),
                    cone.radius() * scale,
                    cone.half_angle(),
                )?),
                turned(stretch),
            ),
            Self::Sphere(sphere) => (
                Self::Sphere(Sphere::new(
                    sphere.frame().mapped(similarity),
                    sphere.radius() * scale,
                )?),
                turned(Affine::IDENTITY),
            ),
            Self::Torus(torus) => (
                Self::Torus(Torus::new(
                    torus.frame().mapped(similarity),
                    torus.major_radius() * scale,
                    torus.minor_radius() * scale,
                )?),
                turned(Affine::IDENTITY),
            ),
            Self::Extrusion(extrusion) => {
                let (profile, along) = extrusion.profile().mapped(similarity)?;
                (
                    Self::Extrusion(Extrusion::new(
                        profile,
                        similarity.apply_direction(extrusion.direction()),
                    )?),
                    UvMap {
                        u: along,
                        v: stretch,
                    },
                )
            }
            Self::Revolution(revolution) => {
                let (profile, along) = revolution.profile().mapped(similarity)?;
                (
                    Self::Revolution(Revolution::new(
                        profile,
                        similarity.apply_point(revolution.axis_origin()),
                        similarity.apply_direction(revolution.axis_direction()),
                    )?),
                    turned(along),
                )
            }
            Self::BSpline(spline) => (
                Self::BSpline(spline.mapped(|point| similarity.apply_point(point))?),
                UvMap::IDENTITY,
            ),
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

impl From<BSplineSurface> for Surface {
    fn from(spline: BSplineSurface) -> Self {
        Self::BSpline(spline)
    }
}
