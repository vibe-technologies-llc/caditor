use std::{borrow::Cow, f64::consts::FRAC_PI_8};

use caditor_geometry::{Point3, Vector3};

use super::{Accuracy, EdgeShape, Element, MeasureError, Separation};
use crate::{
    curve::Curve,
    interval::Interval,
    numeric,
    surface::Surface,
    tolerance::LINEAR_RESOLUTION,
    topology::{EdgeId, FaceId, Solid},
};

const CURVED_PIECES: usize = 64;
const PARALLEL_SINE: f64 = 1e-12;
const SHARED_CORNER: f64 = 10.0 * LINEAR_RESOLUTION;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Axis {
    pub origin: Point3,
    pub direction: Vector3,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EdgeForm {
    Line {
        start: Point3,
        end: Point3,
    },
    Circle {
        center: Point3,
        radius: f64,
        normal: Vector3,
        sweep: f64,
    },
    Ellipse {
        center: Point3,
        major_radius: f64,
        minor_radius: f64,
    },
    Curve,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeMeasure {
    pub length: f64,
    pub length_accuracy: Accuracy,
    pub form: EdgeForm,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FaceForm {
    Plane {
        origin: Point3,
        normal: Vector3,
    },
    Cylinder {
        axis: Axis,
        radius: f64,
    },
    Cone {
        apex: Point3,
        axis: Axis,
        half_angle: f64,
    },
    Sphere {
        center: Point3,
        radius: f64,
    },
    Torus {
        axis: Axis,
        major_radius: f64,
        minor_radius: f64,
    },
    Surface,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AngleKind {
    Corner,
    Lines,
    Planes,
    LineAndPlane,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Angle {
    pub radians: f64,
    pub kind: AngleKind,
}

pub fn edge_measure(solid: &Solid, edge: EdgeId) -> Result<EdgeMeasure, MeasureError> {
    let definition = solid.edge(edge).ok_or(MeasureError::MissingEdge(edge))?;
    Ok(curve_measure(definition.curve(), definition.interval()))
}

pub fn curve_measure(curve: &Curve, interval: Interval) -> EdgeMeasure {
    let shape = EdgeShape::of_curve(Cow::Borrowed(curve), interval);
    let form = match curve {
        Curve::Line(_) => EdgeForm::Line {
            start: shape.start,
            end: shape.end,
        },
        Curve::Circle(circle) => EdgeForm::Circle {
            center: circle.center(),
            radius: circle.radius(),
            normal: circle.frame().normal(),
            sweep: shape.interval.length(),
        },
        Curve::Ellipse(ellipse) => EdgeForm::Ellipse {
            center: ellipse.center(),
            major_radius: ellipse.major_radius(),
            minor_radius: ellipse.minor_radius(),
        },
        _ => EdgeForm::Curve,
    };
    EdgeMeasure {
        length: curve.length(interval),
        length_accuracy: Accuracy::of(matches!(curve, Curve::Line(_) | Curve::Circle(_))),
        form,
    }
}

pub fn face_form(solid: &Solid, face: FaceId) -> Result<FaceForm, MeasureError> {
    let definition = solid.face(face).ok_or(MeasureError::MissingFace(face))?;
    let axis_of_frame = |frame: &caditor_geometry::Plane| Axis {
        origin: frame.origin(),
        direction: frame.normal(),
    };
    Ok(match definition.surface() {
        Surface::Plane(plane) => FaceForm::Plane {
            origin: plane.frame().origin(),
            normal: plane.frame().normal() * definition.sense().sign(),
        },
        Surface::Cylinder(cylinder) => FaceForm::Cylinder {
            axis: axis_of_frame(cylinder.frame()),
            radius: cylinder.radius(),
        },
        Surface::Cone(cone) => FaceForm::Cone {
            apex: cone.apex(),
            axis: axis_of_frame(cone.frame()),
            half_angle: cone.half_angle().abs(),
        },
        Surface::Sphere(sphere) => FaceForm::Sphere {
            center: sphere.center(),
            radius: sphere.radius(),
        },
        Surface::Torus(torus) => FaceForm::Torus {
            axis: axis_of_frame(torus.frame()),
            major_radius: torus.major_radius(),
            minor_radius: torus.minor_radius(),
        },
        _ => FaceForm::Surface,
    })
}

pub fn planar_area(solid: &Solid, face: FaceId) -> Result<Option<(f64, Accuracy)>, MeasureError> {
    let definition = solid.face(face).ok_or(MeasureError::MissingFace(face))?;
    let Surface::Plane(plane) = definition.surface() else {
        return Ok(None);
    };
    let origin = plane.frame().origin();
    let normal = plane.frame().normal();
    let mut twice = 0.0;
    let mut accuracy = Accuracy::Exact;
    let coedges = definition
        .loops()
        .iter()
        .filter_map(|id| solid.face_loop(*id))
        .flat_map(|face_loop| face_loop.coedges())
        .filter_map(|id| solid.coedge(*id));
    for coedge in coedges {
        let edge = solid
            .edge(coedge.edge())
            .ok_or(MeasureError::MissingEdge(coedge.edge()))?;
        let curve = edge.curve();
        accuracy = accuracy.and(Accuracy::of(matches!(
            curve,
            Curve::Line(_) | Curve::Circle(_)
        )));
        twice += coedge.sense().sign() * swept_area(curve, edge.interval(), origin, normal);
    }
    Ok(Some((0.5 * twice.abs(), accuracy)))
}

fn swept_area(curve: &Curve, interval: Interval, origin: Point3, normal: Vector3) -> f64 {
    let pieces = match curve {
        Curve::Line(_) => 1,
        Curve::Circle(_) | Curve::Ellipse(_) => {
            (interval.length() / FRAC_PI_8).ceil().max(1.0) as usize
        }
        _ => CURVED_PIECES,
    };
    let breaks: Vec<f64> = interval.split(pieces).collect();
    numeric::integrate(&breaks, |parameter| {
        let derivatives = curve.evaluate(parameter);
        normal.dot((derivatives.point - origin).cross(derivatives.first))
    })
}

fn circle_axis(measure: EdgeMeasure) -> Option<Axis> {
    match measure.form {
        EdgeForm::Circle { center, normal, .. } => Some(Axis {
            origin: center,
            direction: normal,
        }),
        _ => None,
    }
}

pub fn axis_of(element: Element<'_>) -> Result<Option<Axis>, MeasureError> {
    match element {
        Element::Point(_) | Element::Plane { .. } => Ok(None),
        Element::Axis(axis) => Ok(Some(axis)),
        Element::Edge { solid, edge } => Ok(circle_axis(edge_measure(solid, edge)?)),
        Element::Curve { curve, interval } => Ok(circle_axis(curve_measure(curve, interval))),
        Element::Face { solid, face } => Ok(match face_form(solid, face)? {
            FaceForm::Cylinder { axis, .. }
            | FaceForm::Cone { axis, .. }
            | FaceForm::Torus { axis, .. } => Some(axis),
            _ => None,
        }),
    }
}

pub fn axis_separation(first: Axis, second: Axis) -> Separation {
    let offset = second.origin - first.origin;
    let across = first.direction.cross(second.direction);
    let squared_sine = across.length_squared()
        / (first.direction.length_squared() * second.direction.length_squared())
            .max(f64::MIN_POSITIVE);
    let (from, to) = if squared_sine <= PARALLEL_SINE * PARALLEL_SINE {
        let along = first.direction.dot(offset) / first.direction.length_squared();
        (first.origin + first.direction * along, second.origin)
    } else {
        let squared = across.length_squared();
        let s = offset.cross(second.direction).dot(across) / squared;
        let t = offset.cross(first.direction).dot(across) / squared;
        (
            first.origin + first.direction * s,
            second.origin + second.direction * t,
        )
    };
    Separation::between(from, to, Accuracy::Exact)
}

enum Direction {
    Straight([Point3; 2]),
    Unbounded(Vector3),
    Flat(Vector3),
}

fn straight(measure: EdgeMeasure) -> Option<Direction> {
    match measure.form {
        EdgeForm::Line { start, end } => Some(Direction::Straight([start, end])),
        _ => None,
    }
}

fn direction_of(element: Element<'_>) -> Result<Option<Direction>, MeasureError> {
    match element {
        Element::Point(_) => Ok(None),
        Element::Axis(axis) => Ok(Some(Direction::Unbounded(axis.direction))),
        Element::Plane { normal, .. } => Ok(Some(Direction::Flat(normal))),
        Element::Edge { solid, edge } => Ok(straight(edge_measure(solid, edge)?)),
        Element::Curve { curve, interval } => Ok(straight(curve_measure(curve, interval))),
        Element::Face { solid, face } => Ok(match face_form(solid, face)? {
            FaceForm::Plane { normal, .. } => Some(Direction::Flat(normal)),
            _ => None,
        }),
    }
}

pub fn angle(first: Element<'_>, second: Element<'_>) -> Result<Option<Angle>, MeasureError> {
    let (Some(first), Some(second)) = (direction_of(first)?, direction_of(second)?) else {
        return Ok(None);
    };
    let angle = match (first, second) {
        (Direction::Straight(first), Direction::Straight(second)) => {
            match shared_corner(first, second) {
                Some([corner, first_end, second_end]) => Angle {
                    radians: between(first_end - corner, second_end - corner),
                    kind: AngleKind::Corner,
                },
                None => Angle {
                    radians: acute(between(first[1] - first[0], second[1] - second[0])),
                    kind: AngleKind::Lines,
                },
            }
        }
        (Direction::Flat(first), Direction::Flat(second)) => Angle {
            radians: acute(between(first, second)),
            kind: AngleKind::Planes,
        },
        (Direction::Flat(normal), line) | (line, Direction::Flat(normal)) => {
            let Some(along) = line.along() else {
                return Ok(None);
            };
            Angle {
                radians: std::f64::consts::FRAC_PI_2 - acute(between(along, normal)),
                kind: AngleKind::LineAndPlane,
            }
        }
        (first, second) => {
            let (Some(first), Some(second)) = (first.along(), second.along()) else {
                return Ok(None);
            };
            Angle {
                radians: acute(between(first, second)),
                kind: AngleKind::Lines,
            }
        }
    };
    Ok(Some(angle))
}

impl Direction {
    fn along(&self) -> Option<Vector3> {
        match self {
            Self::Straight([start, end]) => Some(*end - *start),
            Self::Unbounded(direction) => Some(*direction),
            Self::Flat(_) => None,
        }
    }
}

fn shared_corner(first: [Point3; 2], second: [Point3; 2]) -> Option<[Point3; 3]> {
    let [a, b] = first;
    let [c, d] = second;
    [(a, b, c, d), (a, b, d, c), (b, a, c, d), (b, a, d, c)]
        .into_iter()
        .find(|(corner, _, other, _)| corner.distance(*other) <= SHARED_CORNER)
        .map(|(corner, first_end, _, second_end)| [corner, first_end, second_end])
}

fn between(first: Vector3, second: Vector3) -> f64 {
    first.cross(second).length().atan2(first.dot(second))
}

fn acute(radians: f64) -> f64 {
    if radians > std::f64::consts::FRAC_PI_2 {
        std::f64::consts::PI - radians
    } else {
        radians
    }
}
