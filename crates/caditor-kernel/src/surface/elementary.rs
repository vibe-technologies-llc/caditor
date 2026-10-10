use std::f64::consts::FRAC_PI_2;

use caditor_geometry::{Plane, Point2, Point3, Vector3};

use crate::{
    checks::{checked_frame, size},
    error::GeometryError,
    surface::{
        SurfaceDerivatives,
        projection::{AXIS_EPSILON, periodic_near},
    },
    tolerance::MAX_SIZE,
};

const MIN_CONE_ANGLE: f64 = 1e-6;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaneSurface {
    frame: Plane,
}

impl PlaneSurface {
    pub fn new(frame: Plane) -> Result<Self, GeometryError> {
        Ok(Self {
            frame: checked_frame(frame)?,
        })
    }

    pub fn frame(&self) -> &Plane {
        &self.frame
    }

    pub(crate) fn evaluate(&self, u: f64, v: f64) -> SurfaceDerivatives {
        SurfaceDerivatives {
            point: self.frame.to_world(Point2::new(u, v)),
            du: self.frame.x_axis(),
            dv: self.frame.y_axis(),
            duu: Vector3::ZERO,
            duv: Vector3::ZERO,
            dvv: Vector3::ZERO,
        }
    }

    pub(crate) fn project(&self, point: Point3) -> Point2 {
        self.frame.to_local(point)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Axial {
    radial: Vector3,
    tangential: Vector3,
    axis: Vector3,
}

fn axial(frame: &Plane, u: f64) -> Axial {
    let (sin, cos) = u.sin_cos();
    Axial {
        radial: frame.x_axis() * cos + frame.y_axis() * sin,
        tangential: frame.y_axis() * cos - frame.x_axis() * sin,
        axis: frame.normal(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Cylindrical {
    angle: Option<f64>,
    distance: f64,
    height: f64,
}

fn cylindrical(frame: &Plane, point: Point3) -> Cylindrical {
    let offset = point - frame.origin();
    let (x, y) = (offset.dot(frame.x_axis()), offset.dot(frame.y_axis()));
    let distance = x.hypot(y);
    Cylindrical {
        angle: (distance > AXIS_EPSILON * (1.0 + offset.length())).then(|| y.atan2(x)),
        distance,
        height: offset.dot(frame.normal()),
    }
}

fn angle_near(angle: Option<f64>, hint: Option<f64>) -> f64 {
    match angle {
        Some(angle) => periodic_near(angle, std::f64::consts::TAU, hint),
        None => hint.filter(|hint| hint.is_finite()).unwrap_or(0.0),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cylinder {
    frame: Plane,
    radius: f64,
}

impl Cylinder {
    pub fn new(frame: Plane, radius: f64) -> Result<Self, GeometryError> {
        Ok(Self {
            frame: checked_frame(frame)?,
            radius: size(radius)?,
        })
    }

    pub fn frame(&self) -> &Plane {
        &self.frame
    }

    pub fn radius(&self) -> f64 {
        self.radius
    }

    pub(crate) fn evaluate(&self, u: f64, v: f64) -> SurfaceDerivatives {
        let Axial {
            radial,
            tangential,
            axis,
        } = axial(&self.frame, u);
        SurfaceDerivatives {
            point: self.frame.origin() + radial * self.radius + axis * v,
            du: tangential * self.radius,
            dv: axis,
            duu: -radial * self.radius,
            duv: Vector3::ZERO,
            dvv: Vector3::ZERO,
        }
    }

    pub(crate) fn normal(&self, u: f64) -> Vector3 {
        axial(&self.frame, u).radial
    }

    pub(crate) fn project(&self, point: Point3, hint: Option<Point2>) -> Point2 {
        let local = cylindrical(&self.frame, point);
        Point2::new(
            angle_near(local.angle, hint.map(|hint| hint.x)),
            local.height,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cone {
    frame: Plane,
    radius: f64,
    half_angle: f64,
}

impl Cone {
    pub fn new(frame: Plane, radius: f64, half_angle: f64) -> Result<Self, GeometryError> {
        if !radius.is_finite() || !half_angle.is_finite() {
            return Err(GeometryError::NonFinite);
        }
        if radius < 0.0 {
            return Err(GeometryError::NonPositive(radius));
        }
        if radius > MAX_SIZE {
            return Err(GeometryError::BeyondMaximum(radius));
        }
        if half_angle.abs() < MIN_CONE_ANGLE || half_angle.abs() > FRAC_PI_2 - MIN_CONE_ANGLE {
            return Err(GeometryError::ConeAngle(half_angle));
        }
        Ok(Self {
            frame: checked_frame(frame)?,
            radius,
            half_angle,
        })
    }

    pub fn frame(&self) -> &Plane {
        &self.frame
    }

    pub fn radius(&self) -> f64 {
        self.radius
    }

    pub fn half_angle(&self) -> f64 {
        self.half_angle
    }

    pub fn apex_parameter(&self) -> f64 {
        -self.radius / self.half_angle.sin()
    }

    pub fn apex(&self) -> Point3 {
        self.frame.origin() + self.frame.normal() * (self.apex_parameter() * self.half_angle.cos())
    }

    pub fn opening_direction(&self) -> Vector3 {
        self.frame.normal() * self.half_angle.sin().signum()
    }

    pub(crate) fn evaluate(&self, u: f64, v: f64) -> SurfaceDerivatives {
        let Axial {
            radial,
            tangential,
            axis,
        } = axial(&self.frame, u);
        let (sin, cos) = self.half_angle.sin_cos();
        let local_radius = self.radius + v * sin;
        SurfaceDerivatives {
            point: self.frame.origin() + radial * local_radius + axis * (v * cos),
            du: tangential * local_radius,
            dv: radial * sin + axis * cos,
            duu: -radial * local_radius,
            duv: tangential * sin,
            dvv: Vector3::ZERO,
        }
    }

    pub(crate) fn normal(&self, u: f64) -> Vector3 {
        let Axial { radial, axis, .. } = axial(&self.frame, u);
        let (sin, cos) = self.half_angle.sin_cos();
        radial * cos - axis * sin
    }

    pub(crate) fn project(&self, point: Point3, hint: Option<Point2>) -> Point2 {
        let local = cylindrical(&self.frame, point);
        let (sin, cos) = self.half_angle.sin_cos();
        let along = (local.distance - self.radius) * sin + local.height * cos;
        let v = if sin > 0.0 {
            along.max(self.apex_parameter())
        } else {
            along.min(self.apex_parameter())
        };
        let u = if v == self.apex_parameter() {
            hint.map_or_else(|| angle_near(local.angle, None), |hint| hint.x)
        } else {
            angle_near(local.angle, hint.map(|hint| hint.x))
        };
        Point2::new(u, v)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sphere {
    frame: Plane,
    radius: f64,
}

impl Sphere {
    pub fn new(frame: Plane, radius: f64) -> Result<Self, GeometryError> {
        Ok(Self {
            frame: checked_frame(frame)?,
            radius: size(radius)?,
        })
    }

    pub fn frame(&self) -> &Plane {
        &self.frame
    }

    pub fn center(&self) -> Point3 {
        self.frame.origin()
    }

    pub fn radius(&self) -> f64 {
        self.radius
    }

    pub(crate) fn evaluate(&self, u: f64, v: f64) -> SurfaceDerivatives {
        let Axial {
            radial,
            tangential,
            axis,
        } = axial(&self.frame, u);
        let (sin, cos) = v.sin_cos();
        let r = self.radius;
        let outward = radial * cos + axis * sin;
        SurfaceDerivatives {
            point: self.frame.origin() + outward * r,
            du: tangential * (r * cos),
            dv: (axis * cos - radial * sin) * r,
            duu: -radial * (r * cos),
            duv: -tangential * (r * sin),
            dvv: -outward * r,
        }
    }

    pub(crate) fn normal(&self, u: f64, v: f64) -> Vector3 {
        let Axial { radial, axis, .. } = axial(&self.frame, u);
        let (sin, cos) = v.sin_cos();
        radial * cos + axis * sin
    }

    pub(crate) fn project(&self, point: Point3, hint: Option<Point2>) -> Point2 {
        let local = cylindrical(&self.frame, point);
        let v = local.height.atan2(local.distance);
        let u = angle_near(local.angle, hint.map(|hint| hint.x));
        Point2::new(u, v)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Torus {
    frame: Plane,
    major_radius: f64,
    minor_radius: f64,
}

impl Torus {
    pub fn new(frame: Plane, major_radius: f64, minor_radius: f64) -> Result<Self, GeometryError> {
        Ok(Self {
            frame: checked_frame(frame)?,
            major_radius: size(major_radius)?,
            minor_radius: size(minor_radius)?,
        })
    }

    pub fn frame(&self) -> &Plane {
        &self.frame
    }

    pub fn major_radius(&self) -> f64 {
        self.major_radius
    }

    pub fn minor_radius(&self) -> f64 {
        self.minor_radius
    }

    pub(crate) fn evaluate(&self, u: f64, v: f64) -> SurfaceDerivatives {
        let Axial {
            radial,
            tangential,
            axis,
        } = axial(&self.frame, u);
        let (sin, cos) = v.sin_cos();
        let r = self.minor_radius;
        let ring = self.major_radius + r * cos;
        let tube = radial * cos + axis * sin;
        SurfaceDerivatives {
            point: self.frame.origin() + radial * ring + axis * (r * sin),
            du: tangential * ring,
            dv: (axis * cos - radial * sin) * r,
            duu: -radial * ring,
            duv: -tangential * (r * sin),
            dvv: -tube * r,
        }
    }

    pub(crate) fn normal(&self, u: f64, v: f64) -> Vector3 {
        let Axial { radial, axis, .. } = axial(&self.frame, u);
        let (sin, cos) = v.sin_cos();
        radial * cos + axis * sin
    }

    pub(crate) fn project(&self, point: Point3, hint: Option<Point2>) -> Point2 {
        let local = cylindrical(&self.frame, point);
        let u = angle_near(local.angle, hint.map(|hint| hint.x));
        let across = local.distance - self.major_radius;
        let tube_angle = (across.hypot(local.height) > AXIS_EPSILON * (1.0 + self.major_radius))
            .then(|| local.height.atan2(across));
        let v = angle_near(tube_angle, hint.map(|hint| hint.y));
        Point2::new(u, v)
    }
}
