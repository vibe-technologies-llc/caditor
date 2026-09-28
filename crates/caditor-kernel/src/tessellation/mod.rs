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
    topology::{Edge, EdgeId, FaceId, Solid},
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

struct Tolerances {
    base: SamplingTolerance,
    faces: BTreeMap<FaceId, SamplingTolerance>,
}

impl Tolerances {
    fn face(&self, face: FaceId) -> SamplingTolerance {
        self.faces.get(&face).copied().unwrap_or(self.base)
    }

    fn edge(&self, solid: &Solid, edge: &Edge) -> SamplingTolerance {
        edge.coedges()
            .iter()
            .filter_map(|coedge| solid.coedge_face(*coedge))
            .map(|face| self.face(face))
            .fold(self.base, |finest, face| {
                SamplingTolerance::new(
                    finest.chord().min(face.chord()),
                    finest.angle().min(face.angle()),
                )
                .unwrap_or(finest)
            })
    }

    fn refine(&mut self, faces: &[FaceId]) -> bool {
        let mut refined = false;
        for face in faces {
            let current = self.face(*face);
            let finer =
                SamplingTolerance::new(current.chord() * REFINEMENT, current.angle() * REFINEMENT)
                    .filter(|finer| *finer != current);
            if let Some(finer) = finer {
                self.faces.insert(*face, finer);
                refined = true;
            }
        }
        refined
    }
}

enum Attempt {
    Meshed(Mesh),
    Crossed {
        faces: Vec<FaceId>,
        first: TessellationError,
    },
}

pub(crate) fn tessellate(
    solid: &Solid,
    tolerance: &SamplingTolerance,
) -> Result<Mesh, TessellationError> {
    let mut tolerances = Tolerances {
        base: *tolerance,
        faces: BTreeMap::new(),
    };
    for attempt in 0..=MAX_REFINEMENTS {
        match tessellate_once(solid, &tolerances)? {
            Attempt::Meshed(mesh) => return Ok(mesh),
            Attempt::Crossed { faces, first } => {
                if attempt == MAX_REFINEMENTS || !tolerances.refine(&faces) {
                    return Err(first);
                }
            }
        }
    }
    Err(TessellationError::MissingEntity)
}

fn tessellate_once(solid: &Solid, tolerances: &Tolerances) -> Result<Attempt, TessellationError> {
    let mut mesh = Mesh::default();
    let mut vertex_positions = Vec::new();
    for (_, vertex) in solid.vertices() {
        vertex_positions.push(mesh.push_position(vertex.point())?);
    }
    let mut least_segments: BTreeMap<EdgeId, usize> = BTreeMap::new();
    for (id, _) in solid.faces() {
        for (edge, segments) in face::pole_edge_segments(solid, id, &tolerances.face(id))? {
            let least = least_segments.entry(edge).or_default();
            *least = (*least).max(segments);
        }
    }
    let mut samplings = Vec::new();
    for (id, edge) in solid.edges() {
        let mut samples = edge
            .curve()
            .sample(edge.interval(), &tolerances.edge(solid, edge));
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
    let mut crossed = Vec::new();
    let mut first = None;
    for (id, _) in solid.faces() {
        interrupt::check()?;
        let start = mesh.triangles.len();
        match face::triangulate(solid, id, &samplings, &tolerances.face(id), &mut mesh) {
            Ok(()) => mesh.faces.push(FaceTriangles {
                face: id,
                triangles: start..mesh.triangles.len(),
            }),
            Err(
                error @ (TessellationError::SelfIntersectingBoundary(_)
                | TessellationError::DuplicateBoundaryPoint(_)),
            ) => {
                crossed.push(id);
                first.get_or_insert(error);
            }
            Err(error) if first.is_none() => return Err(error),
            Err(_) => break,
        }
    }
    Ok(match first {
        Some(first) => Attempt::Crossed {
            faces: crossed,
            first,
        },
        None => Attempt::Meshed(mesh),
    })
}
