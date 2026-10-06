mod arrange;
mod dxf;
#[cfg(test)]
mod entity_tests;
mod model;
mod sketch;
#[cfg(test)]
mod tests;

use std::{collections::BTreeSet, path::Path};

use caditor_geometry::Point2;
use caditor_step::ReadError;

pub use crate::import::{
    arrange::{DrawingOptions, DrawingUnit, MAX_SCALE, MIN_SCALE},
    dxf::parse_dxf,
    model::{
        ImportedBody, ModelImport, STEP_IMPORT_EXTENSIONS, bodies_transaction, parse_step,
        read_step_file,
    },
    sketch::{DrawingImport, SketchTarget, drawing_transaction},
};
use crate::{read::read_file, reason::ReadFailure};

pub const DXF_EXTENSION: &str = "dxf";
pub const MAX_DRAWING_CURVES: usize = 20_000;
pub const MAX_EXPANDED_OBJECTS: usize = 1_000_000;
pub const MAX_DRAWING_POINTS: usize = 2_000_000;
pub const MAX_DRAWING_VALUES: usize = 16_000_000;

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
    #[error("it is not a DXF drawing")]
    NotDxf,
    #[error("it is not a STEP file")]
    NotStep,
    #[error("{0}")]
    Step(ReadError),
    #[error("none of its bodies could be stored in the model")]
    NothingStorable,
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
}

pub fn read_dxf(path: &Path) -> Result<Drawing, ImportError> {
    let bytes = read_file(path).map_err(|error| ImportError::Reading(ReadFailure::of(&error)))?;
    parse_dxf(&bytes)
}
