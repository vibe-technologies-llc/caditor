use std::collections::BTreeMap;

use crate::{
    naming::FaceName,
    sense::Sense,
    surface::Surface,
    tessellation::{
        Mesh,
        density::Density,
        face::BoundaryPoint,
        patch::{FacePatch, PatchPosition, PatchShape, PatchVertex},
    },
    tolerance::SamplingTolerance,
};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FaceKey {
    pub surface: Surface,
    pub sense: Sense,
    pub tolerance: SamplingTolerance,
    pub density: Option<Density>,
    pub loops: Vec<Vec<BoundaryPoint>>,
}

impl FaceKey {
    fn relabelling(&self, later: &Self) -> Option<BTreeMap<u32, u32>> {
        let alike = self.surface == later.surface
            && self.sense == later.sense
            && self.tolerance == later.tolerance
            && self.density == later.density
            && self.loops.len() == later.loops.len();
        if !alike {
            return None;
        }
        let mut forward = BTreeMap::new();
        let mut backward = BTreeMap::new();
        for (earlier, later) in self.loops.iter().zip(&later.loops) {
            if earlier.len() != later.len() {
                return None;
            }
            for (earlier, later) in earlier.iter().zip(later) {
                let same_place = earlier.uv.x.to_bits() == later.uv.x.to_bits()
                    && earlier.uv.y.to_bits() == later.uv.y.to_bits();
                let onto = *forward.entry(earlier.position).or_insert(later.position);
                let from = *backward.entry(later.position).or_insert(earlier.position);
                if !same_place || onto != later.position || from != earlier.position {
                    return None;
                }
            }
        }
        Some(forward)
    }

    fn relabelled(mut self, renumbered: &[u32]) -> Self {
        for point in self.loops.iter_mut().flatten() {
            if let Some(position) = renumbered.get(point.position as usize) {
                point.position = *position;
            }
        }
        self
    }

    fn heap_size(&self) -> usize {
        self.density.as_ref().map_or(0, Density::heap_size)
            + size_of_val(self.loops.as_slice())
            + self
                .loops
                .iter()
                .map(|points| size_of_val(points.as_slice()))
                .sum::<usize>()
    }
}

#[derive(Debug, Clone, PartialEq)]
struct MeshedFace {
    key: FaceKey,
    slot: usize,
    shape: PatchShape,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct KeptFace {
    pub name: FaceName,
    pub key: FaceKey,
    pub slot: usize,
    pub shape: PatchShape,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DisplayMesh {
    mesh: Mesh,
    faces: BTreeMap<FaceName, Vec<MeshedFace>>,
    reused: usize,
}

impl DisplayMesh {
    pub(crate) fn new(
        mesh: Mesh,
        kept: impl IntoIterator<Item = KeptFace>,
        renumbered: Option<&[u32]>,
        reused: usize,
    ) -> Self {
        let mut faces: BTreeMap<FaceName, Vec<MeshedFace>> = BTreeMap::new();
        for KeptFace {
            name,
            key,
            slot,
            shape,
        } in kept
        {
            let key = match renumbered {
                Some(renumbered) => key.relabelled(renumbered),
                None => key,
            };
            faces
                .entry(name)
                .or_default()
                .push(MeshedFace { key, slot, shape });
        }
        Self {
            mesh,
            faces,
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
            + self
                .faces
                .values()
                .flatten()
                .map(|face| size_of_val(face) + face.key.heap_size())
                .sum::<usize>()
    }

    pub(crate) fn patch(&self, name: FaceName, key: &FaceKey) -> Option<FacePatch> {
        self.faces.get(&name)?.iter().find_map(|meshed| {
            let relabelling = meshed.key.relabelling(key)?;
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
