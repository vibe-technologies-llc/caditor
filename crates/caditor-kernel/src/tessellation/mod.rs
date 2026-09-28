mod density;
mod face;
mod mass;
#[cfg(test)]
mod tests;

use std::{collections::BTreeMap, ops::Range};

use caditor_geometry::{Point3, Vector3};
use thiserror::Error;

pub use self::mass::MassProperties;
use crate::{
    interrupt::{self, Interrupted},
    tolerance::SamplingTolerance,
    topology::{EdgeId, FaceId, Solid},
};

const MIN_CLOSED_EDGE_SEGMENTS: usize = 3;
const MAX_REFINEMENTS: usize = 4;
const REFINEMENT: f64 = 0.5;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TessellationError {
    #[error("the solid refers to an entity it does not contain")]
    MissingEntity,
    #[error("the mesh has more vertices than it can index")]
    TooLarge,
    #[error("edge {0:?} could not be sampled")]
    EdgeSampling(EdgeId),
    #[error("face {0:?} has a point the triangulation cannot hold")]
    Triangulation(FaceId),
    #[error("the boundary of face {0:?} crosses itself in its parameter domain")]
    SelfIntersectingBoundary(FaceId),
    #[error("two different boundary points of face {0:?} share a parameter position")]
    DuplicateBoundaryPoint(FaceId),
    #[error(transparent)]
    Cancelled(#[from] Interrupted),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshVertex {
    pub position: u32,
    pub normal: Vector3,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaceTriangles {
    pub face: FaceId,
    pub triangles: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgePolyline {
    pub edge: EdgeId,
    pub positions: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Mesh {
    positions: Vec<Point3>,
    vertices: Vec<MeshVertex>,
    triangles: Vec<[u32; 3]>,
    faces: Vec<FaceTriangles>,
    edges: Vec<EdgePolyline>,
}

impl Mesh {
    pub fn positions(&self) -> &[Point3] {
        &self.positions
    }

    pub fn vertices(&self) -> &[MeshVertex] {
        &self.vertices
    }

    pub fn triangles(&self) -> &[[u32; 3]] {
        &self.triangles
    }

    pub fn faces(&self) -> &[FaceTriangles] {
        &self.faces
    }

    pub fn edges(&self) -> &[EdgePolyline] {
        &self.edges
    }

    pub fn position(&self, index: u32) -> Option<Point3> {
        self.positions.get(index as usize).copied()
    }

    pub fn triangle_positions(&self, triangle: [u32; 3]) -> Option<[u32; 3]> {
        let [a, b, c] = triangle.map(|corner| {
            self.vertices
                .get(corner as usize)
                .map(|vertex| vertex.position)
        });
        Some([a?, b?, c?])
    }

    pub fn position_triangles(&self) -> impl Iterator<Item = [u32; 3]> + '_ {
        self.triangles
            .iter()
            .filter_map(|triangle| self.triangle_positions(*triangle))
    }

    pub(crate) fn corner_points(&self, triangle: [u32; 3]) -> Option<[Point3; 3]> {
        let [a, b, c] = self
            .triangle_positions(triangle)?
            .map(|position| self.position(position));
        Some([a?, b?, c?])
    }

    pub(crate) fn face_triangles<'a>(
        &'a self,
        keep: impl Fn(FaceId) -> bool + 'a,
    ) -> impl Iterator<Item = [Point3; 3]> + 'a {
        self.faces
            .iter()
            .filter(move |face| keep(face.face))
            .flat_map(|face| {
                self.triangles
                    .get(face.triangles.clone())
                    .unwrap_or_default()
            })
            .filter_map(|triangle| self.corner_points(*triangle))
    }

    fn push_position(&mut self, point: Point3) -> Result<u32, TessellationError> {
        let index = u32::try_from(self.positions.len()).map_err(|_| TessellationError::TooLarge)?;
        self.positions.push(point);
        Ok(index)
    }

    fn push_vertex(&mut self, vertex: MeshVertex) -> Result<u32, TessellationError> {
        let index = u32::try_from(self.vertices.len()).map_err(|_| TessellationError::TooLarge)?;
        self.vertices.push(vertex);
        Ok(index)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EdgeSampling {
    pub parameters: Vec<f64>,
    pub points: Vec<Point3>,
    pub positions: Vec<u32>,
}

pub(crate) fn tessellate(
    solid: &Solid,
    tolerance: &SamplingTolerance,
) -> Result<Mesh, TessellationError> {
    let mut current = *tolerance;
    for _ in 0..MAX_REFINEMENTS {
        match tessellate_once(solid, &current) {
            Err(
                TessellationError::SelfIntersectingBoundary(_)
                | TessellationError::DuplicateBoundaryPoint(_),
            ) => {
                let Some(finer) = SamplingTolerance::new(
                    current.chord() * REFINEMENT,
                    current.angle() * REFINEMENT,
                )
                .filter(|finer| *finer != current) else {
                    break;
                };
                current = finer;
            }
            result => return result,
        }
    }
    tessellate_once(solid, &current)
}

fn tessellate_once(
    solid: &Solid,
    tolerance: &SamplingTolerance,
) -> Result<Mesh, TessellationError> {
    let mut mesh = Mesh::default();
    let mut vertex_positions = Vec::new();
    for (_, vertex) in solid.vertices() {
        vertex_positions.push(mesh.push_position(vertex.point())?);
    }
    let mut least_segments: BTreeMap<EdgeId, usize> = BTreeMap::new();
    for (id, _) in solid.faces() {
        for (edge, segments) in face::pole_edge_segments(solid, id, tolerance)? {
            let least = least_segments.entry(edge).or_default();
            *least = (*least).max(segments);
        }
    }
    let mut samplings = Vec::new();
    for (id, edge) in solid.edges() {
        let mut samples = edge.curve().sample(edge.interval(), tolerance);
        let pieces = if edge.is_closed() {
            MIN_CLOSED_EDGE_SEGMENTS
        } else {
            least_segments.get(&id).copied().unwrap_or(0)
        };
        if samples.len() <= pieces {
            samples = edge
                .interval()
                .split(pieces)
                .map(|parameter| crate::curve::CurveSample {
                    parameter,
                    point: edge.curve().point(parameter),
                })
                .collect();
        }
        let count = samples.len();
        if count < 2 {
            return Err(TessellationError::EdgeSampling(id));
        }
        let start = *vertex_positions
            .get(edge.start().index())
            .ok_or(TessellationError::MissingEntity)?;
        let end = *vertex_positions
            .get(edge.end().index())
            .ok_or(TessellationError::MissingEntity)?;
        let mut positions = Vec::with_capacity(count);
        for (index, sample) in samples.iter().enumerate() {
            let position = if index == 0 {
                start
            } else if index + 1 == count {
                end
            } else {
                mesh.push_position(sample.point)?
            };
            positions.push(position);
        }
        mesh.edges.push(EdgePolyline {
            edge: id,
            positions: positions.clone(),
        });
        samplings.push(EdgeSampling {
            parameters: samples.iter().map(|sample| sample.parameter).collect(),
            points: samples.iter().map(|sample| sample.point).collect(),
            positions,
        });
    }
    for (id, _) in solid.faces() {
        interrupt::check()?;
        let first = mesh.triangles.len();
        face::triangulate(solid, id, &samplings, tolerance, &mut mesh)?;
        mesh.faces.push(FaceTriangles {
            face: id,
            triangles: first..mesh.triangles.len(),
        });
    }
    Ok(mesh)
}
