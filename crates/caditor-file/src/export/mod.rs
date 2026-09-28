mod stl;
#[cfg(test)]
mod tests;
mod three_mf;
mod zip;

use std::{
    panic::{self, AssertUnwindSafe},
    path::Path,
};

use caditor_document::CancelToken;
use caditor_geometry::Point3;
use caditor_kernel::{Mesh, SamplingTolerance, Solid};

use crate::{reason, save::write_atomically};

const APPLICATION: &str = concat!("caditor ", env!("CARGO_PKG_VERSION"));
const SMALLEST_EXTENT: f64 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MeshFormat {
    #[default]
    Stl,
    ThreeMf,
}

impl MeshFormat {
    pub const ALL: [Self; 2] = [Self::Stl, Self::ThreeMf];

    pub fn extension(self) -> &'static str {
        match self {
            Self::Stl => "stl",
            Self::ThreeMf => "3mf",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Stl => "STL",
            Self::ThreeMf => "3MF",
        }
    }

    pub fn matches(self, path: &Path) -> bool {
        path.extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case(self.extension()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MeshResolution {
    Coarse,
    #[default]
    Standard,
    Fine,
}

impl MeshResolution {
    pub const ALL: [Self; 3] = [Self::Coarse, Self::Standard, Self::Fine];

    pub fn name(self) -> &'static str {
        match self {
            Self::Coarse => "Coarse",
            Self::Standard => "Standard",
            Self::Fine => "Fine",
        }
    }

    fn chord_fraction(self) -> f64 {
        match self {
            Self::Coarse => 1e-3,
            Self::Standard => 2.5e-4,
            Self::Fine => 5e-5,
        }
    }

    fn angle_degrees(self) -> f64 {
        match self {
            Self::Coarse => 20.0,
            Self::Standard => 10.0,
            Self::Fine => 5.0,
        }
    }

    pub fn tolerance<'a>(self, solids: impl IntoIterator<Item = &'a Solid>) -> SamplingTolerance {
        let extent = extent(solids);
        SamplingTolerance::new(
            extent * self.chord_fraction(),
            self.angle_degrees().to_radians(),
        )
        .unwrap_or_else(|| SamplingTolerance::for_extent(extent))
    }
}

fn extent<'a>(solids: impl IntoIterator<Item = &'a Solid>) -> f64 {
    solids
        .into_iter()
        .filter_map(Solid::bounding_box)
        .map(|bounds| bounds.diagonal())
        .filter(|diagonal| diagonal.is_finite())
        .fold(SMALLEST_EXTENT, f64::max)
}

#[derive(Debug, Clone, Copy)]
pub struct ExportBody<'a> {
    pub name: &'a str,
    pub solid: &'a Solid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exported {
    pub bodies: usize,
    pub triangles: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExportError {
    #[error("there are no bodies to export")]
    Empty,
    #[error("the export was cancelled")]
    Cancelled,
    #[error(
        "the body of “{0}” could not be turned into triangles at this resolution; try another \
         resolution"
    )]
    Meshing(String),
    #[error("the model has more triangles than the file format can hold; try a coarser resolution")]
    TooLarge,
    #[error("the model could not be converted to the file format")]
    Encoding,
    #[error("{0}")]
    Writing(String),
}

pub fn export_mesh(
    path: &Path,
    format: MeshFormat,
    resolution: MeshResolution,
    bodies: &[ExportBody<'_>],
    cancel: &CancelToken,
) -> Result<Exported, ExportError> {
    if bodies.is_empty() {
        return Err(ExportError::Empty);
    }
    let tolerance = resolution.tolerance(bodies.iter().map(|body| body.solid));
    let mut meshes = Vec::with_capacity(bodies.len());
    for body in bodies {
        if cancel.is_cancelled() {
            return Err(ExportError::Cancelled);
        }
        meshes.push(MeshBody::tessellate(body, &tolerance)?);
    }
    if cancel.is_cancelled() {
        return Err(ExportError::Cancelled);
    }
    let contents = encode(format, &meshes)?;
    if cancel.is_cancelled() {
        return Err(ExportError::Cancelled);
    }
    write_atomically(path, &contents)
        .map_err(|error| ExportError::Writing(reason::writing(&error)))?;
    Ok(Exported {
        bodies: meshes.len(),
        triangles: meshes.iter().map(|mesh| mesh.triangles.len()).sum(),
    })
}

fn encode(format: MeshFormat, bodies: &[MeshBody<'_>]) -> Result<Vec<u8>, ExportError> {
    match format {
        MeshFormat::Stl => stl::encode(bodies),
        MeshFormat::ThreeMf => three_mf::encode(bodies),
    }
}

#[derive(Debug, Clone, PartialEq)]
struct MeshBody<'a> {
    name: &'a str,
    positions: Vec<Point3>,
    triangles: Vec<[u32; 3]>,
}

impl<'a> MeshBody<'a> {
    fn tessellate(
        body: &ExportBody<'a>,
        tolerance: &SamplingTolerance,
    ) -> Result<Self, ExportError> {
        let meshing = ExportError::Meshing(body.name.to_owned());
        let tessellated =
            panic::catch_unwind(AssertUnwindSafe(|| body.solid.tessellate(tolerance)));
        match tessellated {
            Ok(Ok(mesh)) => Self::compact(body.name, &mesh).ok_or(meshing),
            Ok(Err(error)) => {
                log::warn!("exporting {} failed: {error}", body.name);
                Err(meshing)
            }
            Err(_) => {
                log::error!("meshing {} for export panicked", body.name);
                Err(meshing)
            }
        }
    }

    fn compact(name: &'a str, mesh: &Mesh) -> Option<Self> {
        let mut remap = vec![None; mesh.positions().len()];
        let mut positions = Vec::new();
        let mut triangles = Vec::new();
        for triangle in mesh.position_triangles() {
            let [a, b, c] = triangle;
            if a == b || b == c || c == a {
                continue;
            }
            let mut compacted = [0; 3];
            for (slot, index) in compacted.iter_mut().zip(triangle) {
                let entry = remap.get_mut(index as usize)?;
                *slot = match *entry {
                    Some(existing) => existing,
                    None => {
                        let next = u32::try_from(positions.len()).ok()?;
                        positions.push(mesh.position(index)?);
                        *entry = Some(next);
                        next
                    }
                };
            }
            triangles.push(compacted);
        }
        Some(Self {
            name,
            positions,
            triangles,
        })
    }

    fn corners(&self) -> impl Iterator<Item = [Point3; 3]> + '_ {
        self.triangles.iter().filter_map(|triangle| {
            let [a, b, c] = triangle.map(|index| self.positions.get(index as usize).copied());
            Some([a?, b?, c?])
        })
    }
}
