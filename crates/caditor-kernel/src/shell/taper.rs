use caditor_geometry::{Plane, Point3, Vector3};

use crate::{
    curve::{Circle, Curve, Line},
    interval::Interval,
    surface::{Cone, Cylinder, Surface},
    tolerance::{ANGULAR_RESOLUTION, LINEAR_RESOLUTION},
    topology::Edge,
};

const MERIDIAN: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Slopes {
    pub(super) from: f64,
    pub(super) to: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Taper {
    axis: Plane,
    hinge: f64,
    slopes: Slopes,
    surface: Surface,
}

struct Split {
    height: f64,
    radial: Vector3,
}

impl Taper {
    pub(super) fn new(axis: Plane, hinge: f64, slopes: Slopes, middle: f64) -> Option<Self> {
        let along = axis.normal();
        let radius = hinge + middle * slopes.to;
        let anchor = Plane::from_frame(axis.origin() + along * middle, along, axis.x_axis())?;
        let half_angle = slopes.to.atan();
        let surface: Surface = if half_angle.abs() <= ANGULAR_RESOLUTION {
            Cylinder::new(anchor, radius).ok()?.into()
        } else {
            Cone::new(anchor, radius, half_angle).ok()?.into()
        };
        Some(Self {
            axis,
            hinge,
            slopes,
            surface,
        })
    }

    pub(super) fn surface(&self) -> &Surface {
        &self.surface
    }

    pub(super) fn same_as(&self, other: &Self) -> bool {
        let (here, there) = (self.axis.normal(), other.axis.normal());
        let off_axis = (other.axis.origin() - self.axis.origin()).reject_from(here);
        here.dot(there) >= 1.0 - ANGULAR_RESOLUTION
            && off_axis.length() <= LINEAR_RESOLUTION
            && (self.hinge - other.hinge).abs() <= LINEAR_RESOLUTION
            && (self.slopes.from - other.slopes.from).abs() <= ANGULAR_RESOLUTION
            && (self.slopes.to - other.slopes.to).abs() <= ANGULAR_RESOLUTION
    }

    fn split(&self, point: Point3) -> Split {
        let local = point - self.axis.origin();
        let height = local.dot(self.axis.normal());
        Split {
            height,
            radial: local - self.axis.normal() * height,
        }
    }

    fn radius(&self, height: f64, slope: f64) -> f64 {
        self.hinge + height * slope
    }

    fn scaled(&self, point: Point3, from: f64, to: f64) -> Point3 {
        let Split { height, radial } = self.split(point);
        let (before, after) = (self.radius(height, from), self.radius(height, to));
        if before.abs() <= LINEAR_RESOLUTION {
            return point;
        }
        self.axis.origin() + self.axis.normal() * height + radial * (after / before)
    }

    pub(super) fn carried(&self, point: Point3) -> Point3 {
        self.scaled(point, self.slopes.from, self.slopes.to)
    }

    pub(super) fn pulled_back(&self, point: Point3) -> Point3 {
        self.scaled(point, self.slopes.to, self.slopes.from)
    }

    pub(super) fn residual(&self, point: Point3) -> (f64, Vector3) {
        let Split { height, radial } = self.split(point);
        let outward = radial.try_normalize().unwrap_or(self.axis.x_axis());
        let slant = (1.0 + self.slopes.to * self.slopes.to).sqrt();
        let beyond = radial.length() - self.radius(height, self.slopes.to);
        (
            beyond / slant,
            (outward - self.axis.normal() * self.slopes.to) / slant,
        )
    }

    pub(super) fn carried_edge(&self, edge: &Edge) -> Option<(Curve, Interval)> {
        let interval = edge.interval();
        match edge.curve() {
            Curve::Circle(circle) => {
                let frame = circle.frame();
                let Split { height, radial } = self.split(circle.center());
                let coaxial = frame.normal().cross(self.axis.normal()).length()
                    <= ANGULAR_RESOLUTION
                    && radial.length() <= LINEAR_RESOLUTION;
                if !coaxial {
                    return None;
                }
                let before = self.radius(height, self.slopes.from);
                if before.abs() <= LINEAR_RESOLUTION {
                    return None;
                }
                let radius = circle.radius() * self.radius(height, self.slopes.to) / before;
                let carried = Circle::new(*frame, radius).ok()?;
                Some((carried.into(), interval))
            }
            Curve::Line(line) => {
                let start = edge.curve().point(interval.start());
                let end = edge.curve().point(interval.end());
                let Split { radial, .. } = self.split(start);
                let meridian = self.axis.normal().cross(radial);
                if line.direction().dot(meridian).abs() > MERIDIAN * meridian.length().max(1.0) {
                    return None;
                }
                let (from, to) = (self.carried(start), self.carried(end));
                let carried = Line::through(from, to).ok()?;
                Some((carried.into(), Interval::new(0.0, from.distance(to))?))
            }
            _ => None,
        }
    }
}
