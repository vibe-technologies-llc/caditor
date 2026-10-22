mod clip;
mod curve_curve;
mod curve_surface;
mod linear;
mod patch;
pub(crate) mod solve;
mod surface_surface;
#[cfg(test)]
mod tests;

use caditor_geometry::Point3;
use thiserror::Error;

pub(crate) use self::{
    clip::{guided_intervals, inside_intervals},
    linear::solve_dense,
    patch::{boxes_overlap, parameter_slack, patch_bounds, wrap_into},
    surface_surface::line_window,
};
pub use self::{
    curve_curve::{
        CurveCurveIntersection, CurveCurveOverlap, CurveCurvePoint, intersect_curves,
        intersect_curves2,
    },
    curve_surface::{
        CurveSurfaceIntersection, CurveSurfaceOverlap, CurveSurfacePoint, intersect_curve_surface,
    },
    patch::SurfacePatch,
    surface_surface::{
        IntersectionBranch, IntersectionPoint, SurfaceIntersection, intersect_surfaces,
        intersect_surfaces_through,
    },
};
use crate::{error::GeometryError, interrupt::Interrupted};

#[derive(Debug, Clone, PartialEq, Error)]
pub enum IntersectionError {
    #[error("a parameter box is empty or not finite")]
    InvalidBounds,
    #[error("the intersection needs more than {0} subdivisions")]
    TooComplex(usize),
    #[error("an intersection curve could not be followed to its end")]
    Unfollowable,
    #[error(
        "an intersection curve strays {distance} from its surfaces near {point}, where it could not be refined ({cause})"
    )]
    Unrefined {
        point: Point3,
        distance: f64,
        cause: RefineStop,
    },
    #[error(transparent)]
    Geometry(#[from] GeometryError),
    #[error(transparent)]
    Cancelled(#[from] Interrupted),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RefineStop {
    #[error("no point on both surfaces lies across its middle")]
    Unsolved,
    #[error("the point across its middle lies at one of its ends")]
    AtEnd,
    #[error("it was halved {0} times")]
    TooDeep(usize),
    #[error("the curve reached {0} points")]
    TooManyNodes(usize),
}
