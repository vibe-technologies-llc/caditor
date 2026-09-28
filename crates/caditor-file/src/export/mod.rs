mod stl;
#[cfg(test)]
mod tests;
mod three_mf;
mod zip;

use std::{
    panic::{self, AssertUnwindSafe},
    path::Path,
    time::SystemTime,
};

use caditor_document::CancelToken;
use caditor_geometry::Point3;
use caditor_kernel::{Mesh, SamplingTolerance, Solid};
use caditor_step::{StepBody, WriteError, write_step};

use crate::{reason, save::write_atomically};

const APPLICATION: &str = concat!("caditor ", env!("CARGO_PKG_VERSION"));
const SMALLEST_EXTENT: f64 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ExportFormat {
    #[default]
    Stl,
    ThreeMf,
    Step,
}

impl ExportFormat {
    pub const ALL: [Self; 3] = [Self::Stl, Self::ThreeMf, Self::Step];

    pub fn extension(self) -> &'static str {
        match self {
            Self::Stl => "stl",
            Self::ThreeMf => "3mf",
            Self::Step => STEP_EXTENSION,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Stl => "STL",
            Self::ThreeMf => "3MF",
            Self::Step => "STEP",
        }
    }

    pub fn is_mesh(self) -> bool {
        match self {
            Self::Stl | Self::ThreeMf => true,
            Self::Step => false,
        }
    }

    pub fn matches(self, path: &Path) -> bool {
        let accepted: &[&str] = match self {
            Self::Step => &STEP_EXTENSIONS,
            Self::Stl | Self::ThreeMf => &[self.extension()],
        };
        path.extension().is_some_and(|extension| {
            accepted
                .iter()
                .any(|accepted| extension.eq_ignore_ascii_case(accepted))
        })
    }
}

pub const STEP_EXTENSION: &str = "step";
pub const STEP_EXTENSIONS: [&str; 2] = ["step", "stp"];

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
    pub triangles: Option<usize>,
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
    Step(String),
    #[error("{0}")]
    Writing(String),
}

pub fn export_bodies(
    path: &Path,
    format: ExportFormat,
    resolution: MeshResolution,
    bodies: &[ExportBody<'_>],
    cancel: &CancelToken,
) -> Result<Exported, ExportError> {
    if bodies.is_empty() {
        return Err(ExportError::Empty);
    }
    if !format.is_mesh() {
        return export_step(path, bodies, cancel);
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
        triangles: Some(meshes.iter().map(|mesh| mesh.triangles.len()).sum()),
    })
}

fn export_step(
    path: &Path,
    bodies: &[ExportBody<'_>],
    cancel: &CancelToken,
) -> Result<Exported, ExportError> {
    let step_bodies: Vec<StepBody<'_>> = bodies
        .iter()
        .map(|body| StepBody {
            name: body.name,
            solid: body.solid,
        })
        .collect();
    let model_name = path
        .file_stem()
        .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
    let written = panic::catch_unwind(AssertUnwindSafe(|| {
        write_step(&step_bodies, &model_name, SystemTime::now())
    }));
    let contents = match written {
        Ok(Ok(contents)) => contents,
        Ok(Err(WriteError::Empty)) => return Err(ExportError::Empty),
        Ok(Err(error)) => return Err(ExportError::Step(error.to_string())),
        Err(_) => {
            log::error!("writing STEP panicked");
            return Err(ExportError::Encoding);
        }
    };
    if cancel.is_cancelled() {
        return Err(ExportError::Cancelled);
    }
    write_atomically(path, contents.as_bytes())
        .map_err(|error| ExportError::Writing(reason::writing(&error)))?;
    Ok(Exported {
        bodies: bodies.len(),
        triangles: None,
    })
}

fn encode(format: ExportFormat, bodies: &[MeshBody<'_>]) -> Result<Vec<u8>, ExportError> {
    match format {
        ExportFormat::Stl => stl::encode(bodies),
        ExportFormat::ThreeMf => three_mf::encode(bodies),
        ExportFormat::Step => Err(ExportError::Encoding),
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
