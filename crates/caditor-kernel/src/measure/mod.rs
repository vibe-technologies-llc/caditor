mod distance;
mod form;
#[cfg(test)]
mod tests;

use std::borrow::Cow;

use caditor_geometry::{Aabb, Aabb2, Plane, Point2, Point3, Vector3};
use thiserror::Error;

pub use self::{
    distance::distance,
    form::{
        Angle, AngleKind, Axis, EdgeForm, EdgeMeasure, FaceForm, angle, axis_of, axis_separation,
        curve_measure, edge_measure, face_form, planar_area,
    },
};
use crate::{
    curve::{Curve, Line},
    interval::Interval,
    surface::{PlaneSurface, Surface},
    topology::{EdgeId, FaceContainment, FaceId, Solid, SolidClassifier},
};

#[derive(Debug, Clone, Copy)]
pub enum Element<'a> {
    Point(Point3),
    Edge {
        solid: &'a Solid,
        edge: EdgeId,
    },
    Face {
        solid: &'a Solid,
        face: FaceId,
    },
    Curve {
        curve: &'a Curve,
        interval: Interval,
    },
    Axis(Axis),
    Plane {
        origin: Point3,
        normal: Vector3,
    },
}

impl Element<'_> {
    fn bounds(&self) -> Option<Aabb> {
        match self {
            Self::Point(point) => Some(Aabb::from_point(*point)),
            Self::Edge { solid, edge } => {
                let definition = solid.edge(*edge)?;
                Some(definition.curve().bounding_box(definition.interval()))
            }
            Self::Face { solid, .. } => solid.bounding_box(),
            Self::Curve { curve, interval } => Some(curve.bounding_box(*interval)),
            Self::Axis(_) | Self::Plane { .. } => None,
        }
    }
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
    #[error("an axis or plane has no direction")]
    NoDirection,
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
    curve: Cow<'a, Curve>,
    interval: Interval,
    start: Point3,
    end: Point3,
    bounds: Aabb,
}

impl<'a> EdgeShape<'a> {
    fn of(solid: &'a Solid, edge: EdgeId) -> Result<Self, MeasureError> {
        let definition = solid.edge(edge).ok_or(MeasureError::MissingEdge(edge))?;
        Ok(Self::of_curve(
            Cow::Borrowed(definition.curve()),
            definition.interval(),
        ))
    }

    fn of_curve(curve: Cow<'a, Curve>, interval: Interval) -> Self {
        Self {
            start: curve.point(interval.start()),
            end: curve.point(interval.end()),
            bounds: curve.bounding_box(interval),
            curve,
            interval,
        }
    }

    fn axis_within(axis: Axis, bounds: &Aabb) -> Result<Self, MeasureError> {
        let line = Line::new(axis.origin, axis.direction).map_err(|_| MeasureError::NoDirection)?;
        let (low, high) = bounds.corners().iter().fold(
            (f64::INFINITY, f64::NEG_INFINITY),
            |(low, high), corner| {
                let along = (*corner - line.origin()).dot(line.direction());
                (low.min(along), high.max(along))
            },
        );
        let margin = AXIS_MARGIN.max(bounds.diagonal() * AXIS_MARGIN_FRACTION);
        let interval =
            Interval::new(low - margin, high + margin).ok_or(MeasureError::NoClosestPoints)?;
        Ok(Self::of_curve(Cow::Owned(Curve::Line(line)), interval))
    }

    fn point(&self, parameter: f64) -> Point3 {
        self.curve.point(parameter)
    }

    fn closest(&self, point: Point3) -> f64 {
        self.curve.closest_parameter(point, self.interval)
    }

    fn has_exact_closest_points(&self) -> bool {
        matches!(*self.curve, Curve::Line(_) | Curve::Circle(_))
    }

    fn segment(&self) -> Option<[Point3; 2]> {
        matches!(*self.curve, Curve::Line(_)).then_some([self.start, self.end])
    }
}

enum Extent<'a> {
    Face {
        face: FaceId,
        classifier: Box<SolidClassifier<'a>>,
    },
    Unbounded,
}

struct FaceShape<'a> {
    surface: Cow<'a, Surface>,
    extent: Extent<'a>,
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
            surface: Cow::Borrowed(definition.surface()),
            extent: Extent::Face {
                face,
                classifier: Box::new(SolidClassifier::new(solid)),
            },
            edges,
        })
    }

    fn plane(origin: Point3, normal: Vector3) -> Result<Self, MeasureError> {
        let frame = Plane::new(origin, normal).ok_or(MeasureError::NoDirection)?;
        let surface = PlaneSurface::new(frame).map_err(|_| MeasureError::NoDirection)?;
        Ok(Self {
            surface: Cow::Owned(Surface::Plane(surface)),
            extent: Extent::Unbounded,
            edges: Vec::new(),
        })
    }

    fn is_unbounded(&self) -> bool {
        matches!(self.extent, Extent::Unbounded)
    }

    fn contains(&self, uv: Point2) -> bool {
        match &self.extent {
            Extent::Face { face, classifier } => matches!(
                classifier.point_in_face(*face, uv),
                Some(FaceContainment::Inside | FaceContainment::OnBoundary)
            ),
            Extent::Unbounded => true,
        }
    }

    fn projects_exactly(&self) -> bool {
        matches!(
            *self.surface,
            Surface::Plane(_) | Surface::Cylinder(_) | Surface::Sphere(_)
        )
    }

    fn is_flat(&self) -> bool {
        matches!(*self.surface, Surface::Plane(_))
    }

    fn uv_box(&self) -> Option<Aabb2> {
        match &self.extent {
            Extent::Face { face, classifier } => classifier.face_uv_box(*face),
            Extent::Unbounded => None,
        }
    }
}

enum Shape<'a> {
    Point(Point3),
    Edge(EdgeShape<'a>),
    Face(FaceShape<'a>),
}

impl<'a> Shape<'a> {
    fn of(element: Element<'a>, other: Option<Aabb>) -> Result<Self, MeasureError> {
        Ok(match element {
            Element::Point(point) => Self::Point(point),
            Element::Edge { solid, edge } => Self::Edge(EdgeShape::of(solid, edge)?),
            Element::Face { solid, face } => Self::Face(FaceShape::of(solid, face)?),
            Element::Curve { curve, interval } => {
                Self::Edge(EdgeShape::of_curve(Cow::Borrowed(curve), interval))
            }
            Element::Axis(axis) => Self::Edge(EdgeShape::axis_within(
                axis,
                &other.ok_or(MeasureError::NoClosestPoints)?,
            )?),
            Element::Plane { origin, normal } => Self::Face(FaceShape::plane(origin, normal)?),
        })
    }
}

const AXIS_MARGIN: f64 = 1.0;
const AXIS_MARGIN_FRACTION: f64 = 0.01;

fn box_gap(first: &Aabb, second: &Aabb) -> f64 {
    let below = first.min() - second.max();
    let above = second.min() - first.max();
    below.max(above).max(Vector3::ZERO).length()
}
