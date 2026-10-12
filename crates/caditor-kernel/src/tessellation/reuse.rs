use std::{collections::BTreeMap, ops::Range};

use crate::{
    naming::{Digest, FaceName},
    tessellation::{
        Mesh,
        density::Density,
        face::BoundaryPoint,
        patch::{FacePatch, PatchPosition, PatchShape, PatchVertex},
    },
    tolerance::SamplingTolerance,
    topology::{Face, Solid},
};

const BOUNDARY_KEY: u8 = 0x30;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FaceKey {
    pub tolerance: SamplingTolerance,
    pub density: Option<Density>,
    pub boundary: u128,
    pub positions: Vec<u32>,
}

impl FaceKey {
    pub(crate) fn of(
        tolerance: SamplingTolerance,
        density: Option<Density>,
        loops: &[Vec<BoundaryPoint>],
    ) -> Self {
        let mut digest = Digest::new(BOUNDARY_KEY);
        digest.count(loops.len());
        for points in loops {
            digest.count(points.len());
            for point in points {
                digest.u64(point.uv.x.to_bits());
                digest.u64(point.uv.y.to_bits());
            }
        }
        Self {
            tolerance,
            density,
            boundary: digest.finish(),
            positions: loops.iter().flatten().map(|point| point.position).collect(),
        }
    }
}

fn relabelling(earlier: &[u32], later: &[u32]) -> Option<BTreeMap<u32, u32>> {
    if earlier.len() != later.len() {
        return None;
    }
    let mut forward = BTreeMap::new();
    let mut backward = BTreeMap::new();
    for (earlier, later) in earlier.iter().zip(later) {
        let onto = *forward.entry(*earlier).or_insert(*later);
        let from = *backward.entry(*later).or_insert(*earlier);
        if onto != *later || from != *earlier {
            return None;
        }
    }
    Some(forward)
}

#[derive(Debug, Clone, PartialEq)]
struct MeshedFace {
    name: FaceName,
    slot: usize,
    shape: PatchShape,
    tolerance: SamplingTolerance,
    density: Option<Box<Density>>,
    boundary: u128,
    positions: Range<usize>,
}

impl MeshedFace {
    fn matches(&self, key: &FaceKey) -> bool {
        self.boundary == key.boundary
            && self.tolerance == key.tolerance
            && self.density.as_deref() == key.density.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct KeptFace {
    pub name: FaceName,
    pub key: FaceKey,
    pub slot: usize,
    pub shape: PatchShape,
}

#[derive(Debug, Clone, Copy)]
pub struct MeshedSolid<'a> {
    pub solid: &'a Solid,
    pub mesh: &'a DisplayMesh,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DisplayMesh {
    mesh: Mesh,
    faces: Vec<MeshedFace>,
    boundaries: Vec<u32>,
    reused: usize,
}

impl DisplayMesh {
    pub(crate) fn new(
        mesh: Mesh,
        kept: Vec<KeptFace>,
        renumbered: Option<&[u32]>,
        reused: usize,
    ) -> Self {
        let mut boundaries =
            Vec::with_capacity(kept.iter().map(|face| face.key.positions.len()).sum());
        let mut faces: Vec<MeshedFace> = kept
            .into_iter()
            .map(
                |KeptFace {
                     name,
                     key,
                     slot,
                     shape,
                 }| {
                    let first = boundaries.len();
                    boundaries.extend(key.positions.iter().map(|position| {
                        renumbered
                            .and_then(|renumbered| renumbered.get(*position as usize))
                            .copied()
                            .unwrap_or(*position)
                    }));
                    MeshedFace {
                        name,
                        slot,
                        shape,
                        tolerance: key.tolerance,
                        density: key.density.map(Box::new),
                        boundary: key.boundary,
                        positions: first..boundaries.len(),
                    }
                },
            )
            .collect();
        faces.sort_by_key(|face| face.name);
        Self {
            mesh,
            faces,
            boundaries,
            reused,
        }
    }

    pub fn mesh(&self) -> &Mesh {
        &self.mesh
    }

    pub fn into_mesh(self) -> Mesh {
        self.mesh
    }

    pub fn reused_faces(&self) -> usize {
        self.reused
    }

    pub fn approximate_size(&self) -> usize {
        size_of::<Self>() - size_of::<Mesh>()
            + self.mesh.approximate_size()
            + size_of_val(self.faces.as_slice())
            + size_of_val(self.boundaries.as_slice())
            + self
                .faces
                .iter()
                .filter_map(|face| face.density.as_deref())
                .map(|density| size_of::<Density>() + density.heap_size())
                .sum::<usize>()
    }

    pub(crate) fn patch(&self, solid: &Solid, face: &Face, key: &FaceKey) -> Option<FacePatch> {
        let name = face.name();
        let first = self.faces.partition_point(|meshed| meshed.name < name);
        self.faces
            .get(first..)?
            .iter()
            .take_while(|meshed| meshed.name == name)
            .filter(|meshed| meshed.matches(key))
            .find_map(|meshed| {
                let earlier = solid.face(self.mesh.faces.get(meshed.slot)?.face)?;
                if earlier.sense() != face.sense() || earlier.surface() != face.surface() {
                    return None;
                }
                let positions = self.boundaries.get(meshed.positions.clone())?;
                let relabelling = relabelling(positions, &key.positions)?;
                self.extract(meshed, &relabelling)
            })
    }

    fn extract(&self, meshed: &MeshedFace, relabelling: &BTreeMap<u32, u32>) -> Option<FacePatch> {
        let range = self.mesh.faces.get(meshed.slot)?.triangles.clone();
        let triangles = self.mesh.triangles.get(range)?;
        let mut patch = FacePatch {
            shape: meshed.shape,
            ..FacePatch::default()
        };
        let Some(first_vertex) = triangles.iter().flatten().min().copied() else {
            return Some(patch);
        };
        let mut first_interior = None;
        for triangle in triangles {
            let mut local = [0u32; 3];
            for (slot, corner) in local.iter_mut().zip(triangle) {
                *slot = corner.checked_sub(first_vertex)?;
                let created = patch.vertices.len();
                if (*slot as usize) < created {
                    continue;
                }
                if *slot as usize != created {
                    return None;
                }
                let vertex = self.mesh.vertices.get(*corner as usize)?;
                let position = match relabelling.get(&vertex.position) {
                    Some(position) => PatchPosition::Boundary(*position),
                    None => {
                        let first = *first_interior.get_or_insert(vertex.position);
                        let expected = u32::try_from(patch.interior.len()).ok()?;
                        if vertex.position.checked_sub(first)? != expected {
                            return None;
                        }
                        patch
                            .push_interior(self.mesh.position(vertex.position)?)
                            .ok()?
                    }
                };
                patch
                    .push_vertex(PatchVertex {
                        position,
                        normal: vertex.normal,
                    })
                    .ok()?;
            }
            patch.triangles.push(local);
        }
        Some(patch)
    }
}
