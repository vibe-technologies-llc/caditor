mod builder;
mod classify;
#[cfg(test)]
mod classify_tests;
mod pcurve;
#[cfg(test)]
mod tests;
mod validate;

use caditor_geometry::{Aabb, Point3, RigidTransform};

pub use self::{
    builder::{BuildError, SolidBuilder},
    classify::{BoundaryClass, FaceContainment, PointClass, SolidClassifier},
    pcurve::{Pcurve, PcurveError, PcurveSample},
    validate::ValidationError,
};
pub(crate) use self::{
    pcurve::fit as fit_pcurve,
    validate::{continues, inside_polygon, signed_area},
};
use crate::{
    curve::Curve,
    error::GeometryError,
    interval::Interval,
    naming::{EdgeName, FaceName, FaceOrigin},
    sense::Sense,
    surface::Surface,
    tessellation::{self, Mesh, TessellationError},
    tolerance::SamplingTolerance,
};

macro_rules! typed_index {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u32);

        impl $name {
            pub fn index(self) -> usize {
                self.0 as usize
            }

            pub(crate) fn from_index(index: usize) -> Option<Self> {
                u32::try_from(index).ok().map(Self)
            }
        }
    };
}

typed_index!(VertexId);
typed_index!(EdgeId);
typed_index!(CoedgeId);
typed_index!(LoopId);
typed_index!(FaceId);
typed_index!(ShellId);

#[derive(Debug, Clone, PartialEq)]
pub struct Vertex {
    point: Point3,
}

impl Vertex {
    pub fn point(&self) -> Point3 {
        self.point
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    curve: Curve,
    interval: Interval,
    start: VertexId,
    end: VertexId,
    name: EdgeName,
    coedges: Vec<CoedgeId>,
}

impl Edge {
    pub fn curve(&self) -> &Curve {
        &self.curve
    }

    pub fn interval(&self) -> Interval {
        self.interval
    }

    pub fn start(&self) -> VertexId {
        self.start
    }

    pub fn end(&self) -> VertexId {
        self.end
    }

    pub fn is_closed(&self) -> bool {
        self.start == self.end
    }

    pub fn name(&self) -> EdgeName {
        self.name
    }

    pub fn coedges(&self) -> &[CoedgeId] {
        &self.coedges
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Coedge {
    edge: EdgeId,
    sense: Sense,
    owner: LoopId,
    pcurve: Pcurve,
}

impl Coedge {
    pub fn edge(&self) -> EdgeId {
        self.edge
    }

    pub fn sense(&self) -> Sense {
        self.sense
    }

    pub fn owner(&self) -> LoopId {
        self.owner
    }

    pub fn pcurve(&self) -> &Pcurve {
        &self.pcurve
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Loop {
    face: FaceId,
    coedges: Vec<CoedgeId>,
}

impl Loop {
    pub fn face(&self) -> FaceId {
        self.face
    }

    pub fn coedges(&self) -> &[CoedgeId] {
        &self.coedges
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Face {
    surface: Surface,
    sense: Sense,
    loops: Vec<LoopId>,
    shell: ShellId,
    name: FaceName,
    origin: Option<FaceOrigin>,
}

impl Face {
    pub fn surface(&self) -> &Surface {
        &self.surface
    }

    pub fn sense(&self) -> Sense {
        self.sense
    }

    pub fn loops(&self) -> &[LoopId] {
        &self.loops
    }

    pub fn outer_loop(&self) -> Option<LoopId> {
        self.loops.first().copied()
    }

    pub fn inner_loops(&self) -> &[LoopId] {
        self.loops.get(1..).unwrap_or_default()
    }

    pub fn shell(&self) -> ShellId {
        self.shell
    }

    pub fn name(&self) -> FaceName {
        self.name
    }

    pub fn origin(&self) -> Option<FaceOrigin> {
        self.origin
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Shell {
    faces: Vec<FaceId>,
}

impl Shell {
    pub fn faces(&self) -> &[FaceId] {
        &self.faces
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Solid {
    vertices: Vec<Vertex>,
    edges: Vec<Edge>,
    coedges: Vec<Coedge>,
    loops: Vec<Loop>,
    faces: Vec<Face>,
    shells: Vec<Shell>,
}

fn enumerate<'a, Id, Item>(
    items: &'a [Item],
    id: impl Fn(usize) -> Option<Id> + 'a,
) -> impl Iterator<Item = (Id, &'a Item)> + 'a {
    items
        .iter()
        .enumerate()
        .filter_map(move |(index, item)| id(index).map(|id| (id, item)))
}

impl Solid {
    pub fn vertex(&self, id: VertexId) -> Option<&Vertex> {
        self.vertices.get(id.index())
    }

    pub fn edge(&self, id: EdgeId) -> Option<&Edge> {
        self.edges.get(id.index())
    }

    pub fn coedge(&self, id: CoedgeId) -> Option<&Coedge> {
        self.coedges.get(id.index())
    }

    pub fn face_loop(&self, id: LoopId) -> Option<&Loop> {
        self.loops.get(id.index())
    }

    pub fn face(&self, id: FaceId) -> Option<&Face> {
        self.faces.get(id.index())
    }

    pub fn shell(&self, id: ShellId) -> Option<&Shell> {
        self.shells.get(id.index())
    }

    pub fn vertices(&self) -> impl Iterator<Item = (VertexId, &Vertex)> {
        enumerate(&self.vertices, VertexId::from_index)
    }

    pub fn edges(&self) -> impl Iterator<Item = (EdgeId, &Edge)> {
        enumerate(&self.edges, EdgeId::from_index)
    }

    pub fn coedges(&self) -> impl Iterator<Item = (CoedgeId, &Coedge)> {
        enumerate(&self.coedges, CoedgeId::from_index)
    }

    pub fn loops(&self) -> impl Iterator<Item = (LoopId, &Loop)> {
        enumerate(&self.loops, LoopId::from_index)
    }

    pub fn faces(&self) -> impl Iterator<Item = (FaceId, &Face)> {
        enumerate(&self.faces, FaceId::from_index)
    }

    pub fn shells(&self) -> impl Iterator<Item = (ShellId, &Shell)> {
        enumerate(&self.shells, ShellId::from_index)
    }

    pub fn coedge_parameters(&self, id: CoedgeId) -> Option<(f64, f64)> {
        let coedge = self.coedge(id)?;
        let interval = self.edge(coedge.edge)?.interval;
        Some(match coedge.sense {
            Sense::Same => (interval.start(), interval.end()),
            Sense::Reversed => (interval.end(), interval.start()),
        })
    }

    pub fn coedge_vertices(&self, id: CoedgeId) -> Option<(VertexId, VertexId)> {
        let coedge = self.coedge(id)?;
        let edge = self.edge(coedge.edge)?;
        Some(match coedge.sense {
            Sense::Same => (edge.start, edge.end),
            Sense::Reversed => (edge.end, edge.start),
        })
    }

    pub fn coedge_face(&self, id: CoedgeId) -> Option<FaceId> {
        Some(self.face_loop(self.coedge(id)?.owner)?.face)
    }

    pub fn bounding_box(&self) -> Option<Aabb> {
        let edges = self
            .edges
            .iter()
            .map(|edge| edge.curve.bounding_box(edge.interval));
        let vertices = self
            .vertices
            .iter()
            .map(|vertex| Aabb::from_point(vertex.point));
        edges.chain(vertices).reduce(Aabb::union)
    }

    pub fn transformed(&self, transform: &RigidTransform) -> Result<Self, GeometryError> {
        let vertices = self
            .vertices
            .iter()
            .map(|vertex| {
                let point = transform.apply_point(vertex.point);
                if point.is_finite() {
                    Ok(Vertex { point })
                } else {
                    Err(GeometryError::NonFinite)
                }
            })
            .collect::<Result<_, _>>()?;
        let edges = self
            .edges
            .iter()
            .map(|edge| {
                Ok(Edge {
                    curve: edge.curve.transformed(transform)?,
                    ..edge.clone()
                })
            })
            .collect::<Result<_, GeometryError>>()?;
        let faces = self
            .faces
            .iter()
            .map(|face| {
                Ok(Face {
                    surface: face.surface.transformed(transform)?,
                    ..face.clone()
                })
            })
            .collect::<Result<_, GeometryError>>()?;
        Ok(Self {
            vertices,
            edges,
            coedges: self.coedges.clone(),
            loops: self.loops.clone(),
            faces,
            shells: self.shells.clone(),
        })
    }

    pub(crate) fn renamed(
        mut self,
        rename: impl Fn(FaceName, Option<FaceOrigin>) -> (FaceName, Option<FaceOrigin>),
    ) -> Self {
        for face in &mut self.faces {
            (face.name, face.origin) = rename(face.name, face.origin);
        }
        let names: Vec<EdgeName> = self
            .edges
            .iter()
            .map(|edge| {
                let faces: Vec<FaceName> = edge
                    .coedges
                    .iter()
                    .filter_map(|coedge| self.coedge_face(*coedge))
                    .filter_map(|face| self.face(face))
                    .map(Face::name)
                    .collect();
                match faces.as_slice() {
                    [first, second] if first != second => EdgeName::between(*first, *second),
                    [first, ..] => EdgeName::seam(*first),
                    [] => edge.name,
                }
            })
            .collect();
        for (edge, name) in self.edges.iter_mut().zip(names) {
            edge.name = name;
        }
        self
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        validate::validate(self)
    }

    pub fn tessellate(&self, tolerance: &SamplingTolerance) -> Result<Mesh, TessellationError> {
        tessellation::tessellate(self, tolerance)
    }

    pub fn default_tolerance(&self) -> SamplingTolerance {
        SamplingTolerance::for_extent(self.bounding_box().map_or(1.0, |bounds| bounds.diagonal()))
    }
}
