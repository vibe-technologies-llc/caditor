use thiserror::Error;

use crate::tolerance::{LINEAR_RESOLUTION, MAX_SIZE};

#[derive(Debug, Clone, PartialEq, Error)]
pub enum GeometryError {
    #[error("a coordinate or parameter is not finite")]
    NonFinite,
    #[error("a direction has zero length")]
    ZeroDirection,
    #[error("a size that must be positive is {0}")]
    NonPositive(f64),
    #[error("a size of {0} mm is below the resolution of {LINEAR_RESOLUTION} mm")]
    BelowResolution(f64),
    #[error("a size of {0} mm is beyond the largest of {MAX_SIZE} mm")]
    BeyondMaximum(f64),
    #[error("the torus tube radius {minor} is not smaller than its ring radius {major}")]
    SelfIntersectingTorus { major: f64, minor: f64 },
    #[error("the cone half angle {0} is not strictly between zero and a right angle")]
    ConeAngle(f64),
    #[error("a B-spline of degree {0} is not supported")]
    SplineDegree(usize),
    #[error("a B-spline of degree {degree} needs more than {points} control points")]
    TooFewControlPoints { degree: usize, points: usize },
    #[error("a B-spline with {points} control points of degree {degree} has {knots} knots")]
    KnotCount {
        degree: usize,
        points: usize,
        knots: usize,
    },
    #[error("the B-spline knots are not clamped, non-decreasing and of limited multiplicity")]
    Knots,
    #[error("a B-spline has {points} control points but {weights} weights")]
    WeightCount { points: usize, weights: usize },
    #[error("a B-spline weight {0} is not positive")]
    Weight(f64),
    #[error("the surface collapses to a curve or a point")]
    DegenerateSurface,
}
