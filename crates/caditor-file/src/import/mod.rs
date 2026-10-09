mod arrange;
mod dxf;
#[cfg(test)]
mod entity_tests;
mod mesh;
#[cfg(test)]
mod mesh_tests;
mod model;
mod sketch;
mod svg;
#[cfg(test)]
mod svg_tests;
#[cfg(test)]
mod tests;
mod zip_read;

use std::{collections::BTreeSet, path::Path};

use caditor_document::CancelToken;
use caditor_geometry::Point2;
use caditor_step::ReadError;

pub use crate::import::{
    arrange::{DrawingOptions, DrawingUnit, MAX_SCALE, MIN_SCALE},
    dxf::parse_dxf,
    mesh::{MESH_IMPORT_EXTENSIONS, MeshFormat, parse_mesh, read_mesh_file},
    model::{
        ImportedBody, ModelImport, STEP_IMPORT_EXTENSIONS, bodies_transaction, parse_step,
        read_step_file,
    },
    sketch::{DrawingImport, SketchTarget, drawing_transaction},
    svg::{SVG_EXTENSIONS, parse_svg},
};
use crate::{read::read_file, reason::ReadFailure};

pub const DXF_EXTENSION: &str = "dxf";
pub const DRAWING_IMPORT_EXTENSIONS: [&str; 3] = [DXF_EXTENSION, "svg", "svgz"];
pub const MAX_DRAWING_CURVES: usize = 20_000;
pub const MAX_READ_CURVES: usize = 5 * MAX_DRAWING_CURVES;
pub const MAX_EXPANDED_OBJECTS: usize = 1_000_000;
pub const MAX_DRAWING_POINTS: usize = 2_000_000;
pub const MAX_DRAWING_VALUES: usize = 16_000_000;
pub const MAX_DRAWING_ELEMENTS: usize = 4_000_000;

#[derive(Debug, Clone, PartialEq)]
pub enum DrawingCurve {
    Point(Point2),
    Line {
        start: Point2,
        end: Point2,
    },
    Circle {
        center: Point2,
        radius: f64,
    },
    Arc {
        center: Point2,
        start: Point2,
        end: Point2,
    },
    Spline {
        control_points: Vec<Point2>,
    },
}

impl DrawingCurve {
    pub fn ends(&self) -> Option<(Point2, Point2)> {
        match self {
            Self::Point(_) | Self::Circle { .. } => None,
            Self::Line { start, end } | Self::Arc { start, end, .. } => Some((*start, *end)),
            Self::Spline { control_points } => {
                Some((*control_points.first()?, *control_points.last()?))
            }
        }
    }

    fn points(&self) -> Vec<Point2> {
        match self {
            Self::Point(point) => vec![*point],
            Self::Line { start, end } => vec![*start, *end],
            Self::Circle { center, radius } => vec![
                *center - Point2::splat(*radius),
                *center + Point2::splat(*radius),
            ],
            Self::Arc { center, start, end } => {
                let radius = start.distance(*center);
                vec![
                    *center - Point2::splat(radius),
                    *center + Point2::splat(radius),
                    *end,
                ]
            }
            Self::Spline { control_points } => control_points.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Drawing {
    pub curves: Vec<DrawingCurve>,
    pub construction: BTreeSet<usize>,
    pub notes: Vec<String>,
    pub unit_scale: f64,
    pub layers: Vec<String>,
    pub curve_layers: Vec<usize>,
}

impl Default for Drawing {
    fn default() -> Self {
        Self {
            curves: Vec::new(),
            construction: BTreeSet::new(),
            notes: Vec::new(),
            unit_scale: 1.0,
            layers: Vec::new(),
            curve_layers: Vec::new(),
        }
    }
}

impl Drawing {
    pub fn extent(&self) -> f64 {
        let mut points = self.curves.iter().flat_map(DrawingCurve::points);
        let Some(first) = points.next() else {
            return 0.0;
        };
        let (low, high) = points.fold((first, first), |(low, high), point| {
            (low.min(point), high.max(point))
        });
        low.distance(high)
    }

    pub fn curve_count(&self) -> usize {
        self.curves
            .iter()
            .filter(|curve| !matches!(curve, DrawingCurve::Point(_)))
            .count()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImportError {
    #[error("{0}")]
    Reading(ReadFailure),
    #[error("caditor ran into an internal error while reading it")]
    Crashed,
    #[error("the import was stopped")]
    Cancelled,
    #[error("it is not a DXF drawing")]
    NotDxf,
    #[error("it is not an SVG drawing")]
    NotSvg,
    #[error("it is not a STEP file")]
    NotStep,
    #[error("{0}")]
    Step(ReadError),
    #[error("none of its bodies could be stored in the model")]
    NothingStorable,
    #[error("it is not an STL, OBJ or 3MF mesh")]
    NotMesh,
    #[error("it is not a readable {0} mesh")]
    DamagedMesh(&'static str),
    #[error("the 3MF package holds no 3D model")]
    NoModelInPackage,
    #[error(
        "no part of it is a closed surface ({open_edges} of its edges border only one triangle), \
         so it does not enclose a solid"
    )]
    MeshNotClosed { open_edges: usize },
    #[error(
        "it has {faces} faces even after its flat areas are joined, more than the {} caditor \
         takes from one mesh; reduce its triangles in a mesh editor first",
        caditor_kernel::MAX_FACETED_FACES
    )]
    MeshTooDetailed { faces: usize },
    #[error("its surface could not be made into a valid solid")]
    MeshNotSolid,
    #[error("the compressed file is damaged, so it cannot be unpacked")]
    DamagedArchive,
    #[error(
        "the compressed file unpacks to more than {} GiB, more than caditor reads",
        crate::read::MAX_FILE_SIZE >> 30
    )]
    UnpacksTooLarge,
    #[error("the drawing is damaged near line {0}")]
    DamagedAt(usize),
    #[error("the drawing is damaged")]
    Damaged,
    #[error("the drawing has no lines, arcs, circles or splines to import")]
    Empty { left_out: Vec<String> },
    #[error(
        "its blocks repeat into more than {MAX_EXPANDED_OBJECTS} objects, more than caditor reads \
         from one drawing"
    )]
    TooManyObjects,
    #[error(
        "its curves hold more than {MAX_DRAWING_POINTS} points in all, more than caditor reads \
         from one drawing"
    )]
    TooDetailed,
    #[error(
        "its blocks and entities hold more than {MAX_DRAWING_VALUES} values, more than caditor \
         reads from one drawing; split it into smaller drawings"
    )]
    TooManyValues,
    #[error(
        "it holds more than {MAX_DRAWING_ELEMENTS} elements, more than caditor reads from one \
         drawing; split it into smaller drawings"
    )]
    TooManyElements,
    #[error(
        "its reused elements repeat into more than {MAX_EXPANDED_OBJECTS} objects, more than \
         caditor reads from one drawing"
    )]
    TooManyCopies,
}

pub fn read_dxf(path: &Path, cancel: &CancelToken) -> Result<Drawing, ImportError> {
    let bytes = read_file(path).map_err(|error| ImportError::Reading(ReadFailure::of(&error)))?;
    ensure_going(cancel)?;
    let drawing = parse_dxf(&bytes)?;
    ensure_going(cancel)?;
    Ok(drawing)
}

pub fn read_drawing(path: &Path, cancel: &CancelToken) -> Result<Drawing, ImportError> {
    let bytes = read_file(path).map_err(|error| ImportError::Reading(ReadFailure::of(&error)))?;
    ensure_going(cancel)?;
    let named = |extensions: &[&str]| {
        path.extension().is_some_and(|extension| {
            extensions
                .iter()
                .any(|known| extension.eq_ignore_ascii_case(known))
        })
    };
    let is_svg =
        named(&SVG_EXTENSIONS) || (!named(&[DXF_EXTENSION]) && svg::looks_like_svg(&bytes));
    let drawing = if is_svg {
        parse_svg(&bytes)
    } else {
        parse_dxf(&bytes)
    }?;
    ensure_going(cancel)?;
    Ok(drawing)
}

pub(crate) fn ensure_going(cancel: &CancelToken) -> Result<(), ImportError> {
    if cancel.is_cancelled() {
        Err(ImportError::Cancelled)
    } else {
        Ok(())
    }
}
