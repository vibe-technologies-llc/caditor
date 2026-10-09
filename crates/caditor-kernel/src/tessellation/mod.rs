mod density;
mod face;
mod insertion;
mod mass;
#[cfg(test)]
mod tests;

use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};

use caditor_geometry::{Point3, Vector3};
use thiserror::Error;

pub use self::mass::{MassProperties, SecondMoment};
pub(crate) use self::mass::{Moments, TriangleIndex};
use crate::{
    interrupt::{self, Interrupted},
    tolerance::{MeshQuality, SamplingTolerance},
    topology::{Edge, EdgeId, FaceId, Solid},
};

const MIN_CLOSED_EDGE_SEGMENTS: usize = 3;
const MAX_REFINEMENTS: usize = 4;
const MAX_END_PARTINGS: usize = 8;
const REFINEMENT: f64 = 0.5;
pub(crate) const MAX_POINTS: usize = 1 << 22;
pub(crate) const DISPLAY_POINTS: usize = 1 << 20;
const POLL_EVERY: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TessellationError {
    #[error("the solid refers to an entity it does not contain")]
    MissingEntity,
    #[error("the mesh would need more than {} points", MAX_POINTS)]
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
    chord: f64,
}

impl Mesh {
    pub fn chord(&self) -> f64 {
        self.chord
    }

    pub fn approximate_size(&self) -> usize {
        size_of::<Self>()
            + size_of_val(self.positions.as_slice())
            + size_of_val(self.vertices.as_slice())
            + size_of_val(self.triangles.as_slice())
            + size_of_val(self.faces.as_slice())
            + self
                .edges
                .iter()
                .map(|edge| size_of_val(edge) + size_of_val(edge.positions.as_slice()))
                .sum::<usize>()
    }

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

    fn drop_unused(&mut self) {
        let mut used_vertices = vec![false; self.vertices.len()];
        let mut used_positions = vec![false; self.positions.len()];
        for corner in self.triangles.iter().flatten() {
            if let Some(used) = used_vertices.get_mut(*corner as usize) {
                *used = true;
            }
            if let Some(vertex) = self.vertices.get(*corner as usize)
                && let Some(used) = used_positions.get_mut(vertex.position as usize)
            {
                *used = true;
            }
        }
        for position in self.edges.iter().flat_map(|edge| &edge.positions) {
            if let Some(used) = used_positions.get_mut(*position as usize) {
                *used = true;
            }
        }
        let position_map = renumbering(&used_positions);
        let vertex_map = renumbering(&used_vertices);
        let renumber = |map: &[u32], index: u32| map.get(index as usize).copied().unwrap_or(index);
        self.positions = kept(std::mem::take(&mut self.positions), &used_positions);
        self.vertices = kept(std::mem::take(&mut self.vertices), &used_vertices)
            .into_iter()
            .map(|vertex| MeshVertex {
                position: renumber(&position_map, vertex.position),
                ..vertex
            })
            .collect();
        for corner in self.triangles.iter_mut().flatten() {
            *corner = renumber(&vertex_map, *corner);
        }
        for position in self.edges.iter_mut().flat_map(|edge| &mut edge.positions) {
            *position = renumber(&position_map, *position);
        }
    }

    fn push_vertex(&mut self, vertex: MeshVertex) -> Result<u32, TessellationError> {
        let index = u32::try_from(self.vertices.len()).map_err(|_| TessellationError::TooLarge)?;
        self.vertices.push(vertex);
        Ok(index)
    }
}

fn renumbering(used: &[bool]) -> Vec<u32> {
    let mut next = 0u32;
    used.iter()
        .map(|used| {
            let index = next;
            if *used {
                next = next.saturating_add(1);
            }
            index
        })
        .collect()
}

fn kept<T>(items: Vec<T>, used: &[bool]) -> Vec<T> {
    items
        .into_iter()
        .zip(used)
        .filter(|(_, used)| **used)
        .map(|(item, _)| item)
        .collect()
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

pub(crate) fn tessellate(
    solid: &Solid,
    tolerance: &SamplingTolerance,
) -> Result<Mesh, TessellationError> {
    tessellate_within(solid, tolerance, MAX_POINTS)
}

pub(crate) fn tessellate_for_display(
    solid: &Solid,
    extent: f64,
    quality: &MeshQuality,
    limit: usize,
) -> Result<Mesh, TessellationError> {
    let fallback = quality.at_least(&MeshQuality::COARSE);
    match tessellate_within(solid, &quality.tolerance(extent), limit) {
        Err(error) if !matches!(error, TessellationError::Cancelled(_)) && fallback != *quality => {
            tessellate(solid, &fallback.tolerance(extent))
        }
        meshed => meshed,
    }
}

pub(crate) fn tessellate_within(
    solid: &Solid,
    tolerance: &SamplingTolerance,
    limit: usize,
) -> Result<Mesh, TessellationError> {
    let mut tessellator = Tessellator::new(solid, tolerance, limit)?;
    let mut crossed = tessellator.first_pass()?;
    for _ in 0..MAX_REFINEMENTS {
        let Some(faces) = crossed.faces() else {
            break;
        };
        if !tessellator.tolerances.refine(&faces) {
            break;
        }
        crossed = tessellator.refine(&faces)?;
    }
    match crossed.first {
        Some(first) => Err(first),
        None => Ok(tessellator.finish()),
    }
}

#[derive(Debug, Default)]
struct Crossed {
    faces: Vec<FaceId>,
    first: Option<TessellationError>,
}

impl Crossed {
    fn faces(&self) -> Option<Vec<FaceId>> {
        self.first.is_some().then(|| self.faces.clone())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct SampledWith {
    tolerance: SamplingTolerance,
    least: usize,
}

struct Tessellator<'a> {
    solid: &'a Solid,
    limit: usize,
    tolerances: Tolerances,
    mesh: Mesh,
    vertex_positions: Vec<u32>,
    poles: BTreeMap<FaceId, face::PoleSampling>,
    samplings: Vec<EdgeSampling>,
    sampled_with: Vec<SampledWith>,
    triangles: BTreeMap<FaceId, Vec<[u32; 3]>>,
    orphans: bool,
}

impl<'a> Tessellator<'a> {
    fn new(
        solid: &'a Solid,
        tolerance: &SamplingTolerance,
        limit: usize,
    ) -> Result<Self, TessellationError> {
        let mut mesh = Mesh::default();
        let mut vertex_positions = Vec::new();
        for (_, vertex) in solid.vertices() {
            vertex_positions.push(mesh.push_position(vertex.point())?);
        }
        Ok(Self {
            solid,
            limit,
            tolerances: Tolerances {
                base: *tolerance,
                faces: BTreeMap::new(),
            },
            mesh,
            vertex_positions,
            poles: BTreeMap::new(),
            samplings: Vec::new(),
            sampled_with: Vec::new(),
            triangles: BTreeMap::new(),
            orphans: false,
        })
    }

    fn first_pass(&mut self) -> Result<Crossed, TessellationError> {
        let faces: Vec<FaceId> = self.solid.faces().map(|(id, _)| id).collect();
        self.find_poles(&faces)?;
        for (id, edge) in self.solid.edges() {
            interrupt::check()?;
            let wanted = self.wanted(id, edge);
            let sampling = self.sample(id, edge, wanted)?;
            self.samplings.push(sampling);
            self.sampled_with.push(wanted);
        }
        self.triangulate(&faces)
    }

    fn refine(&mut self, crossed: &[FaceId]) -> Result<Crossed, TessellationError> {
        self.find_poles(crossed)?;
        let mut redo: BTreeSet<FaceId> = crossed.iter().copied().collect();
        for (id, edge) in self.solid.edges() {
            interrupt::check()?;
            let wanted = self.wanted(id, edge);
            if self.sampled_with.get(id.index()) == Some(&wanted) {
                continue;
            }
            let sampling = self.sample(id, edge, wanted)?;
            if let (Some(slot), Some(with)) = (
                self.samplings.get_mut(id.index()),
                self.sampled_with.get_mut(id.index()),
            ) {
                *slot = sampling;
                *with = wanted;
            }
            self.orphans = true;
            redo.extend(
                edge.coedges()
                    .iter()
                    .filter_map(|coedge| self.solid.coedge_face(*coedge)),
            );
        }
        let faces: Vec<FaceId> = self
            .solid
            .faces()
            .map(|(id, _)| id)
            .filter(|id| redo.contains(id))
            .collect();
        for face in &faces {
            if self.triangles.remove(face).is_some() {
                self.orphans = true;
            }
        }
        self.triangulate(&faces)
    }

    fn find_poles(&mut self, faces: &[FaceId]) -> Result<(), TessellationError> {
        for face in faces {
            let found = face::pole_sampling(self.solid, *face, &self.tolerances.face(*face))?;
            match found {
                Some(found) => self.poles.insert(*face, found),
                None => self.poles.remove(face),
            };
        }
        Ok(())
    }

    fn wanted(&self, id: EdgeId, edge: &Edge) -> SampledWith {
        let least = if edge.is_closed() {
            MIN_CLOSED_EDGE_SEGMENTS
        } else {
            self.poles
                .values()
                .flat_map(|poles| poles.edges.iter())
                .filter(|(edge, _)| *edge == id)
                .map(|(_, segments)| *segments)
                .max()
                .unwrap_or(0)
        };
        SampledWith {
            tolerance: self.tolerances.edge(self.solid, edge),
            least,
        }
    }

    fn sample(
        &mut self,
        id: EdgeId,
        edge: &Edge,
        wanted: SampledWith,
    ) -> Result<EdgeSampling, TessellationError> {
        let mut samples = edge.curve().sample(edge.interval(), &wanted.tolerance);
        if samples.len() <= wanted.least {
            samples = edge
                .interval()
                .split(wanted.least)
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
        let start = *self
            .vertex_positions
            .get(edge.start().index())
            .ok_or(TessellationError::MissingEntity)?;
        let end = *self
            .vertex_positions
            .get(edge.end().index())
            .ok_or(TessellationError::MissingEntity)?;
        let mut positions = Vec::with_capacity(count);
        for (index, sample) in samples.iter().enumerate() {
            let position = if index == 0 {
                start
            } else if index + 1 == count {
                end
            } else {
                self.mesh.push_position(sample.point)?
            };
            positions.push(position);
        }
        if self.mesh.positions.len() > self.limit {
            return Err(TessellationError::TooLarge);
        }
        Ok(EdgeSampling {
            parameters: samples.iter().map(|sample| sample.parameter).collect(),
            points: samples.iter().map(|sample| sample.point).collect(),
            positions,
        })
    }

    fn part_overlapping_ends(
        &mut self,
        faces: &[FaceId],
    ) -> Result<Vec<FaceId>, TessellationError> {
        let mut checked: BTreeSet<FaceId> = faces.iter().copied().collect();
        for _ in 0..MAX_END_PARTINGS {
            let mut ends = BTreeSet::new();
            for face in &checked {
                interrupt::check()?;
                ends.extend(face::overlapping_ends(self.solid, *face, &self.samplings)?);
            }
            let mut parted = false;
            for (id, end) in ends {
                if !self.bisect_end(id, end)? {
                    continue;
                }
                parted = true;
                let edge = self
                    .solid
                    .edge(id)
                    .ok_or(TessellationError::MissingEntity)?;
                checked.extend(
                    edge.coedges()
                        .iter()
                        .filter_map(|coedge| self.solid.coedge_face(*coedge)),
                );
            }
            if !parted {
                break;
            }
        }
        Ok(self
            .solid
            .faces()
            .map(|(id, _)| id)
            .filter(|id| checked.contains(id))
            .collect())
    }

    fn bisect_end(&mut self, id: EdgeId, end: face::EdgeEnd) -> Result<bool, TessellationError> {
        let edge = self
            .solid
            .edge(id)
            .ok_or(TessellationError::MissingEntity)?;
        let sampling = self
            .samplings
            .get_mut(id.index())
            .ok_or(TessellationError::MissingEntity)?;
        let count = sampling.parameters.len();
        let after = match end {
            face::EdgeEnd::Start => 1,
            face::EdgeEnd::End => count.saturating_sub(1),
        };
        let (Some(low), Some(high)) = (
            sampling.parameters.get(after.wrapping_sub(1)).copied(),
            sampling.parameters.get(after).copied(),
        ) else {
            return Err(TessellationError::MissingEntity);
        };
        let parameter = 0.5 * (low + high);
        if parameter == low || parameter == high {
            return Ok(false);
        }
        let point = edge.curve().point(parameter);
        let position = self.mesh.push_position(point)?;
        if self.mesh.positions.len() > self.limit {
            return Err(TessellationError::TooLarge);
        }
        sampling.parameters.insert(after, parameter);
        sampling.points.insert(after, point);
        sampling.positions.insert(after, position);
        Ok(true)
    }

    fn triangulate(&mut self, faces: &[FaceId]) -> Result<Crossed, TessellationError> {
        let faces = self.part_overlapping_ends(faces)?;
        for face in &faces {
            if self.triangles.remove(face).is_some() {
                self.orphans = true;
            }
        }
        let mut crossed = Crossed::default();
        for id in &faces {
            interrupt::check()?;
            let budget = face::Budget {
                limit: self.limit,
                tolerance: self.tolerances.face(*id),
                density: self.poles.get(id).map(|poles| poles.density.clone()),
            };
            let start = self.mesh.triangles.len();
            match face::triangulate(self.solid, *id, &self.samplings, &budget, &mut self.mesh) {
                Ok(()) => {
                    let triangles = self.mesh.triangles.split_off(start);
                    self.triangles.insert(*id, triangles);
                }
                Err(
                    error @ (TessellationError::SelfIntersectingBoundary(_)
                    | TessellationError::DuplicateBoundaryPoint(_)),
                ) => {
                    self.mesh.triangles.truncate(start);
                    self.orphans = true;
                    crossed.faces.push(*id);
                    crossed.first.get_or_insert(error);
                }
                Err(error) => return Err(error),
            }
        }
        Ok(crossed)
    }

    fn finish(self) -> Mesh {
        let mut mesh = self.mesh;
        mesh.chord = self.tolerances.base.chord();
        mesh.triangles.clear();
        for (id, _) in self.solid.faces() {
            let start = mesh.triangles.len();
            mesh.triangles
                .extend(self.triangles.get(&id).into_iter().flatten());
            mesh.faces.push(FaceTriangles {
                face: id,
                triangles: start..mesh.triangles.len(),
            });
        }
        mesh.edges = self
            .solid
            .edges()
            .zip(&self.samplings)
            .map(|((id, _), sampling)| EdgePolyline {
                edge: id,
                positions: sampling.positions.clone(),
            })
            .collect();
        if self.orphans {
            mesh.drop_unused();
        }
        mesh
    }
}
