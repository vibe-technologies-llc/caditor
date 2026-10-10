use caditor_geometry::{Point3, Vector3};

use crate::tessellation::TessellationError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PatchPosition {
    Boundary(u32),
    Interior(u32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PatchVertex {
    pub position: PatchPosition,
    pub normal: Vector3,
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
}
