mod distance;
mod form;
#[cfg(test)]
mod tests;

use caditor_geometry::{Aabb, Aabb2, Point2, Point3, Vector3};
use thiserror::Error;

pub use self::{
    distance::distance,
    form::{
        Angle, AngleKind, Axis, EdgeForm, EdgeMeasure, FaceForm, angle, axis_of, axis_separation,
        edge_measure, face_form, planar_area,
    },
};
use crate::{
    curve::Curve,
    interval::Interval,
    surface::Surface,
    topology::{EdgeId, FaceContainment, FaceId, Solid, SolidClassifier},
};

#[derive(Debug, Clone, Copy)]
pub enum Element<'a> {
    Point(Point3),
    Edge { solid: &'a Solid, edge: EdgeId },
    Face { solid: &'a Solid, face: FaceId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum Accuracy {
    #[default]
    Exact,
    Approximate,
}

impl Accuracy {
    #[must_use]
    pub fn and(self, other: Self) -> Self {
        self.max(other)
    }

    fn of(exact: bool) -> Self {
        if exact {
            Self::Exact
        } else {
            Self::Approximate
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MeasureError {
    #[error("edge {0:?} is not part of the solid")]
    MissingEdge(EdgeId),
    #[error("face {0:?} is not part of the solid")]
    MissingFace(FaceId),
    #[error("no closest points were found")]
    NoClosestPoints,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Separation {
    pub distance: f64,
    pub from: Point3,
    pub to: Point3,
    pub accuracy: Accuracy,
}

impl Separation {
    fn between(from: Point3, to: Point3, accuracy: Accuracy) -> Self {
        Self {
            distance: from.distance(to),
            from,
            to,
            accuracy,
        }
    }

    fn swapped(self) -> Self {
        Self {
            from: self.to,
            to: self.from,
            ..self
        }
    }

    pub fn offset(&self) -> Vector3 {
        self.to - self.from
    }
}

struct EdgeShape<'a> {
    curve: &'a Curve,
    interval: Interval,
    start: Point3,
    end: Point3,
    bounds: Aabb,
}

impl<'a> EdgeShape<'a> {
    fn of(solid: &'a Solid, edge: EdgeId) -> Result<Self, MeasureError> {
        let definition = solid.edge(edge).ok_or(MeasureError::MissingEdge(edge))?;
        let curve = definition.curve();
        let interval = definition.interval();
        Ok(Self {
            curve,
            interval,
            start: curve.point(interval.start()),
            end: curve.point(interval.end()),
            bounds: curve.bounding_box(interval),
        })
    }

    fn point(&self, parameter: f64) -> Point3 {
        self.curve.point(parameter)
    }

    fn closest(&self, point: Point3) -> f64 {
        self.curve.closest_parameter(point, self.interval)
    }

    fn has_exact_closest_points(&self) -> bool {
        matches!(self.curve, Curve::Line(_) | Curve::Circle(_))
    }

    fn segment(&self) -> Option<[Point3; 2]> {
        matches!(self.curve, Curve::Line(_)).then_some([self.start, self.end])
    }
}

struct FaceShape<'a> {
    face: FaceId,
    surface: &'a Surface,
    classifier: SolidClassifier<'a>,
    edges: Vec<EdgeShape<'a>>,
}

impl<'a> FaceShape<'a> {
    fn of(solid: &'a Solid, face: FaceId) -> Result<Self, MeasureError> {
        let definition = solid.face(face).ok_or(MeasureError::MissingFace(face))?;
        let mut edge_ids: Vec<EdgeId> = definition
            .loops()
            .iter()
            .filter_map(|id| solid.face_loop(*id))
            .flat_map(|face_loop| face_loop.coedges())
            .filter_map(|coedge| solid.coedge(*coedge))
            .map(|coedge| coedge.edge())
            .collect();
        edge_ids.sort_unstable();
        edge_ids.dedup();
        let edges = edge_ids
            .into_iter()
            .map(|edge| EdgeShape::of(solid, edge))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            face,
            surface: definition.surface(),
            classifier: SolidClassifier::new(solid),
            edges,
        })
    }

    fn contains(&self, uv: Point2) -> bool {
        matches!(
            self.classifier.point_in_face(self.face, uv),
            Some(FaceContainment::Inside | FaceContainment::OnBoundary)
        )
    }

    fn projects_exactly(&self) -> bool {
        matches!(
            self.surface,
            Surface::Plane(_) | Surface::Cylinder(_) | Surface::Sphere(_)
        )
    }

    fn is_flat(&self) -> bool {
        matches!(self.surface, Surface::Plane(_))
    }

    fn uv_box(&self) -> Option<Aabb2> {
        self.classifier.face_uv_box(self.face)
    }
}

enum Shape<'a> {
    Point(Point3),
    Edge(EdgeShape<'a>),
    Face(FaceShape<'a>),
}

impl<'a> Shape<'a> {
    fn of(element: Element<'a>) -> Result<Self, MeasureError> {
        Ok(match element {
            Element::Point(point) => Self::Point(point),
            Element::Edge { solid, edge } => Self::Edge(EdgeShape::of(solid, edge)?),
            Element::Face { solid, face } => Self::Face(FaceShape::of(solid, face)?),
        })
    }
}

fn box_gap(first: &Aabb, second: &Aabb) -> f64 {
    let below = first.min() - second.max();
    let above = second.min() - first.max();
    below.max(above).max(Vector3::ZERO).length()
}
