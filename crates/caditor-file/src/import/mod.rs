mod dxf;
mod model;
mod sketch;
#[cfg(test)]
mod tests;

use std::path::Path;

use caditor_geometry::Point2;

pub use crate::import::{
    dxf::parse_dxf,
    model::{
        ImportedBody, ModelImport, STEP_IMPORT_EXTENSIONS, bodies_transaction, parse_step,
        read_step_file,
    },
    sketch::{DrawingImport, SketchTarget, drawing_transaction},
};
use crate::{read::read_file, reason};

pub const DXF_EXTENSION: &str = "dxf";
pub const MAX_DRAWING_CURVES: usize = 20_000;
pub const MAX_EXPANDED_OBJECTS: usize = 1_000_000;

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

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Drawing {
    pub curves: Vec<DrawingCurve>,
    pub notes: Vec<String>,
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
    Reading(String),
    #[error("it is not a DXF drawing")]
    NotDxf,
    #[error("it is not a STEP file")]
    NotStep,
    #[error("{0}")]
    Model(String),
    #[error("the drawing is damaged near line {0}")]
    DamagedAt(usize),
    #[error("the drawing is damaged")]
    Damaged,
    #[error("the drawing has no lines, arcs, circles or splines to import")]
    Empty { left_out: Vec<String> },
    #[error(
        "the drawing has more than {MAX_DRAWING_CURVES} curves, more than one sketch can hold; \
         split it into smaller drawings"
    )]
    TooLarge,
    #[error(
        "its blocks repeat into more than {MAX_EXPANDED_OBJECTS} objects, more than caditor reads \
         from one drawing"
    )]
    TooManyObjects,
}

pub fn read_dxf(path: &Path) -> Result<Drawing, ImportError> {
    let bytes = read_file(path).map_err(|error| ImportError::Reading(reason::reading(&error)))?;
    parse_dxf(&bytes)
}
