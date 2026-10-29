use std::collections::BTreeMap;

use caditor_geometry::Point3;

use crate::tessellation::{MeshNormal, TessellationError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PatchPosition {
    Boundary(u32),
    Interior(u32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PatchVertex {
    pub position: PatchPosition,
    pub normal: MeshNormal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct PatchShape {
    pub grid: usize,
    pub points: usize,
    pub in_pieces: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct FacePatch {
    pub interior: Vec<Point3>,
    pub vertices: Vec<PatchVertex>,
    pub triangles: Vec<[u32; 3]>,
    pub shape: PatchShape,
}

impl FacePatch {
    pub(crate) fn push_interior(
        &mut self,
        point: Point3,
    ) -> Result<PatchPosition, TessellationError> {
        let index = u32::try_from(self.interior.len()).map_err(|_| TessellationError::TooLarge)?;
        self.interior.push(point);
        Ok(PatchPosition::Interior(index))
    }

    pub(crate) fn push_vertex(&mut self, vertex: PatchVertex) -> Result<u32, TessellationError> {
        let index = u32::try_from(self.vertices.len()).map_err(|_| TessellationError::TooLarge)?;
        self.vertices.push(vertex);
        Ok(index)
    }

    pub(crate) fn inner_pairs(&self) -> Vec<BoundaryPair> {
        let mut uses: BTreeMap<BoundaryPair, usize> = BTreeMap::new();
        for triangle in &self.triangles {
            for [from, to, _] in turns(*triangle) {
                if let Some(pair) = self.boundary_pair(from, to) {
                    *uses.entry(pair).or_default() += 1;
                }
            }
        }
        uses.into_iter()
            .filter(|(_, count)| *count == 2)
            .map(|(pair, _)| pair)
            .collect()
    }

    pub(crate) fn split(
        &mut self,
        pair: BoundaryPair,
        point: Point3,
        normal: MeshNormal,
    ) -> Result<(), TessellationError> {
        let mut beside = Vec::new();
        let mut kept = Vec::with_capacity(self.triangles.len());
        for triangle in &self.triangles {
            let found = turns(*triangle)
                .into_iter()
                .find(|[from, to, _]| self.boundary_pair(*from, *to) == Some(pair));
            match found {
                Some(corners) => beside.push(corners),
                None => kept.push(*triangle),
            }
        }
        self.triangles = kept;
        let position = self.push_interior(point)?;
        let middle = self.push_vertex(PatchVertex { position, normal })?;
        for [from, to, apex] in beside {
            self.triangles.push([apex, from, middle]);
            self.triangles.push([middle, to, apex]);
        }
        Ok(())
    }

    fn boundary_pair(&self, first: u32, second: u32) -> Option<BoundaryPair> {
        let position = |vertex: u32| match self.vertices.get(vertex as usize)?.position {
            PatchPosition::Boundary(position) => Some(position),
            PatchPosition::Interior(_) => None,
        };
        Some(BoundaryPair::of(position(first)?, position(second)?))
    }
}

fn turns([a, b, c]: [u32; 3]) -> [[u32; 3]; 3] {
    [[a, b, c], [b, c, a], [c, a, b]]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct BoundaryPair(pub u32, pub u32);

impl BoundaryPair {
    pub(crate) fn of(first: u32, second: u32) -> Self {
        Self(first.min(second), first.max(second))
    }
}
