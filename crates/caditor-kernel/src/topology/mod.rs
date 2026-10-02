mod builder;
mod classify;
#[cfg(test)]
mod classify_tests;
mod crossing;
mod pcurve;
#[cfg(test)]
mod tests;
mod validate;

use std::collections::{BTreeMap, BTreeSet};

use caditor_geometry::{Aabb, Point2, Point3, RigidTransform, Vector3};

pub use self::{
    builder::{BuildError, SolidBuilder},
    classify::{BoundaryClass, FaceContainment, PointClass, RayCrossing, SolidClassifier},
    crossing::{Crossing, CrossingCheck},
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
    naming::{EdgeName, FaceName, FaceOrigin, VertexName, occurrence_order},
    sense::Sense,
    surface::Surface,
    tessellation::{self, Mesh, TessellationError},
    tolerance::{MeshQuality, SamplingTolerance},
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
    pub fn approximate_size(&self) -> usize {
        let edges: usize = self
            .edges
            .iter()
            .map(|edge| edge.curve.heap_size() + size_of_val(edge.coedges.as_slice()))
            .sum();
        let coedges: usize = self
            .coedges
            .iter()
            .map(|coedge| coedge.pcurve.heap_size())
            .sum();
        let loops: usize = self
            .loops
            .iter()
            .map(|owned| size_of_val(owned.coedges.as_slice()))
            .sum();
        let faces: usize = self
            .faces
            .iter()
            .map(|face| face.surface.heap_size() + size_of_val(face.loops.as_slice()))
            .sum();
        let shells: usize = self
            .shells
            .iter()
            .map(|shell| size_of_val(shell.faces.as_slice()))
            .sum();
        size_of::<Self>()
            + size_of_val(self.vertices.as_slice())
            + size_of_val(self.edges.as_slice())
            + size_of_val(self.coedges.as_slice())
            + size_of_val(self.loops.as_slice())
            + size_of_val(self.faces.as_slice())
            + size_of_val(self.shells.as_slice())
            + edges
            + coedges
            + loops
            + faces
            + shells
    }

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

    pub(crate) fn outline_box(&self) -> Option<Aabb> {
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

    pub fn bounding_box(&self) -> Option<Aabb> {
        let outline = self.outline_box()?;
        let curved: Vec<FaceId> = self
            .faces()
            .filter(|(_, face)| doubly_curved(face.surface()))
            .map(|(id, _)| id)
            .collect();
        if curved.is_empty() {
            return Some(outline);
        }
        let classifier = self.classifier();
        let mut inside = Vec::new();
        for id in curved {
            let (Some(face), Some(uv_box)) = (self.face(id), classifier.face_uv_box(id)) else {
                continue;
            };
            let surface = face.surface();
            let step = |low: f64, high: f64, index: usize| {
                low + (high - low) * index as f64 / BOUNDS_GRID as f64
            };
            let grid = (0..=BOUNDS_GRID).flat_map(|row| {
                (0..=BOUNDS_GRID).map(move |column| {
                    Point2::new(
                        step(uv_box.min().x, uv_box.max().x, column),
                        step(uv_box.min().y, uv_box.max().y, row),
                    )
                })
            });
            let extremes: Vec<Point2> = match surface {
                Surface::Sphere(sphere) => [Vector3::X, Vector3::Y, Vector3::Z]
                    .into_iter()
                    .flat_map(|axis| [axis, -axis])
                    .map(|direction| {
                        surface.project(sphere.center() + direction * sphere.radius(), None)
                    })
                    .collect(),
                _ => Vec::new(),
            };
            for uv in grid.chain(extremes) {
                if matches!(
                    classifier.point_in_face(id, uv),
                    Some(FaceContainment::Inside | FaceContainment::OnBoundary)
                ) {
                    inside.push(surface.point_at(uv));
                }
            }
        }
        Some(match Aabb::from_points(inside) {
            Some(faces) => outline.union(faces),
            None => outline,
        })
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

    #[must_use]
    pub fn imported(mut self, feature: u64) -> Self {
        for (index, face) in self.faces.iter_mut().enumerate() {
            let index = u32::try_from(index).unwrap_or(u32::MAX);
            face.name = FaceName::imported(feature, index);
            face.origin = Some(FaceOrigin::Imported {
                feature,
                face: index,
            });
        }
        let names = self.derived_edge_names();
        for (edge, name) in self.edges.iter_mut().zip(names) {
            edge.name = name;
        }
        self
    }

    pub(crate) fn with_face_origins(mut self, change: impl Fn(FaceOrigin) -> FaceOrigin) -> Self {
        for face in &mut self.faces {
            face.origin = face.origin.map(&change);
        }
        self
    }

    pub(crate) fn with_face_names(mut self, rename: impl Fn(FaceName) -> FaceName) -> Self {
        for face in &mut self.faces {
            face.name = rename(face.name);
        }
        let names = self.derived_edge_names();
        for (edge, name) in self.edges.iter_mut().zip(names) {
            edge.name = name;
        }
        self
    }

    fn derived_edge_names(&self) -> Vec<EdgeName> {
        let sides: Vec<[FaceName; 2]> = self
            .edges
            .iter()
            .map(|edge| {
                let side = |same: bool| {
                    edge.coedges
                        .iter()
                        .filter_map(|coedge| self.coedge(*coedge))
                        .find(|coedge| coedge.sense.is_same() == same)
                        .and_then(|coedge| self.face(self.face_loop(coedge.owner)?.face))
                        .map_or(FaceName::NONE, Face::name)
                };
                [side(true), side(false)]
            })
            .collect();
        let mut names: Vec<EdgeName> = sides
            .iter()
            .map(|[left, right]| {
                if left == right {
                    EdgeName::seam(*left)
                } else {
                    EdgeName::between(*left, *right)
                }
            })
            .collect();
        let mut around: Vec<BTreeSet<FaceName>> = vec![BTreeSet::new(); self.vertices.len()];
        for (edge, faces) in self.edges.iter().zip(&sides) {
            for vertex in [edge.start, edge.end] {
                if let Some(set) = around.get_mut(vertex.index()) {
                    set.extend(faces.iter().copied());
                }
            }
        }
        let vertex_names: Vec<VertexName> = around.into_iter().map(VertexName::of_faces).collect();
        let mut groups: BTreeMap<EdgeName, Vec<usize>> = BTreeMap::new();
        for (index, name) in names.iter().enumerate() {
            groups.entry(*name).or_default().push(index);
        }
        for members in groups.values().filter(|members| members.len() > 1) {
            let mut named: Vec<(usize, EdgeName, Point3)> = members
                .iter()
                .filter_map(|index| {
                    let edge = self.edges.get(*index)?;
                    let [left, right] = *sides.get(*index)?;
                    let from = vertex_names
                        .get(edge.start.index())
                        .copied()
                        .unwrap_or_default();
                    let to = vertex_names
                        .get(edge.end.index())
                        .copied()
                        .unwrap_or_default();
                    Some((
                        *index,
                        EdgeName::between_at(left, right, from, to),
                        edge.curve.point(edge.interval.middle()),
                    ))
                })
                .collect();
            let mut counts: BTreeMap<EdgeName, usize> = BTreeMap::new();
            for (_, name, _) in &named {
                *counts.entry(*name).or_default() += 1;
            }
            named.sort_by_key(|(_, _, midpoint)| occurrence_order(*midpoint));
            let mut occurrences: BTreeMap<EdgeName, u32> = BTreeMap::new();
            for (index, name, _) in named {
                let unique = if counts.get(&name).copied().unwrap_or(0) > 1 {
                    let occurrence = occurrences.entry(name).or_insert(0);
                    let unique = EdgeName::occurrence(name, *occurrence);
                    *occurrence += 1;
                    unique
                } else {
                    name
                };
                if let Some(slot) = names.get_mut(index) {
                    *slot = unique;
                }
            }
        }
        names
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        validate::validate(self)
    }

    pub fn tessellate(&self, tolerance: &SamplingTolerance) -> Result<Mesh, TessellationError> {
        tessellation::tessellate(self, tolerance)
    }

    pub fn display_mesh(&self, quality: &MeshQuality) -> Result<Mesh, TessellationError> {
        tessellation::tessellate_for_display(
            self,
            self.extent(),
            quality,
            tessellation::DISPLAY_POINTS,
        )
    }

    pub fn tolerance_for(&self, quality: &MeshQuality) -> SamplingTolerance {
        quality.tolerance(self.extent())
    }

    pub fn default_tolerance(&self) -> SamplingTolerance {
        self.tolerance_for(&MeshQuality::COARSE)
    }

    fn extent(&self) -> f64 {
        self.bounding_box().map_or(1.0, |bounds| bounds.diagonal())
    }
}

const BOUNDS_GRID: usize = 12;

fn doubly_curved(surface: &Surface) -> bool {
    matches!(
        surface,
        Surface::Sphere(_) | Surface::Torus(_) | Surface::Revolution(_) | Surface::BSpline(_)
    )
}
