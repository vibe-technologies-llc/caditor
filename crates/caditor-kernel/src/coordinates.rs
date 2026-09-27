use std::{
    fmt::Debug,
    ops::{Add, Mul, Neg, Sub},
};

use caditor_geometry::{Point2, Point3};

pub trait Coordinates:
    Copy
    + Debug
    + PartialEq
    + Add<Output = Self>
    + Sub<Output = Self>
    + Mul<f64, Output = Self>
    + Neg<Output = Self>
{
    const ORIGIN: Self;

    fn dot(self, other: Self) -> f64;

    fn all_finite(self) -> bool;

    fn component_min(self, other: Self) -> Self;

    fn component_max(self, other: Self) -> Self;

    fn min_component(self) -> f64;

    fn splat(value: f64) -> Self;

    fn norm(self) -> f64 {
        self.dot(self).max(0.0).sqrt()
    }

    fn distance_to(self, other: Self) -> f64 {
        (self - other).norm()
    }
}

impl Coordinates for Point2 {
    const ORIGIN: Self = Point2::ZERO;

    fn dot(self, other: Self) -> f64 {
        Point2::dot(self, other)
    }

    fn all_finite(self) -> bool {
        self.is_finite()
    }

    fn component_min(self, other: Self) -> Self {
        Point2::min(self, other)
    }

    fn component_max(self, other: Self) -> Self {
        Point2::max(self, other)
    }

    fn min_component(self) -> f64 {
        self.min_element()
    }

    fn splat(value: f64) -> Self {
        Point2::splat(value)
    }
}

impl Coordinates for Point3 {
    const ORIGIN: Self = Point3::ZERO;

    fn dot(self, other: Self) -> f64 {
        Point3::dot(self, other)
    }

    fn all_finite(self) -> bool {
        self.is_finite()
    }

    fn component_min(self, other: Self) -> Self {
        Point3::min(self, other)
    }

    fn component_max(self, other: Self) -> Self {
        Point3::max(self, other)
    }

    fn min_component(self) -> f64 {
        self.min_element()
    }

    fn splat(value: f64) -> Self {
        Point3::splat(value)
    }
}

pub(crate) fn angle_between<P: Coordinates>(a: P, b: P) -> f64 {
    let (aa, bb, ab) = (a.dot(a), b.dot(b), a.dot(b));
    if aa <= 0.0 || bb <= 0.0 {
        return 0.0;
    }
    let cross = (aa * bb - ab * ab).max(0.0).sqrt();
    cross.atan2(ab)
}

pub(crate) fn distance_to_segment<P: Coordinates>(point: P, start: P, end: P) -> f64 {
    let along = end - start;
    let length_squared = along.dot(along);
    let fraction = if length_squared > 0.0 {
        ((point - start).dot(along) / length_squared).clamp(0.0, 1.0)
    } else {
        0.0
    };
    point.distance_to(start + along * fraction)
}

#[cfg(test)]
mod tests {
    use std::f64::consts::FRAC_PI_2;

    use super::*;

    #[test]
    fn angles_and_segment_distances() {
        assert!((angle_between(Point2::X, Point2::Y) - FRAC_PI_2).abs() < 1e-15);
        assert_eq!(angle_between(Point3::ZERO, Point3::X), 0.0);
        assert!((angle_between(Point3::X, Point3::NEG_X) - std::f64::consts::PI).abs() < 1e-15);
        assert_eq!(
            distance_to_segment(Point2::new(0.5, 2.0), Point2::ZERO, Point2::X),
            2.0
        );
        assert_eq!(
            distance_to_segment(Point2::new(3.0, 0.0), Point2::ZERO, Point2::X),
            2.0
        );
        assert_eq!(
            distance_to_segment(Point2::new(3.0, 4.0), Point2::ZERO, Point2::ZERO),
            5.0
        );
    }
}
