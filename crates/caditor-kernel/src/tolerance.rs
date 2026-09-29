use std::f64::consts::FRAC_PI_4;

use caditor_geometry::{Point3, Vector3};

pub const LINEAR_RESOLUTION: f64 = 1e-6;
pub const MODEL_EXTENT: f64 = 1e4;
pub const MAX_SIZE: f64 = 1e6;
pub const ANGULAR_RESOLUTION: f64 = LINEAR_RESOLUTION / MODEL_EXTENT;
pub const PCURVE_TOLERANCE: f64 = 1e-4;
pub const INTERSECTION_TOLERANCE: f64 = 0.25 * LINEAR_RESOLUTION;

const MIN_CHORD_TOLERANCE: f64 = 10.0 * LINEAR_RESOLUTION;
const MIN_ANGLE_TOLERANCE: f64 = 1e-3;
const MAX_ANGLE_TOLERANCE: f64 = FRAC_PI_4;
const DEFAULT_CHORD_FRACTION: f64 = 1e-3;
const DEFAULT_ANGLE_TOLERANCE: f64 = 0.35;

pub fn same_point(a: Point3, b: Point3) -> bool {
    a.distance_squared(b) <= LINEAR_RESOLUTION * LINEAR_RESOLUTION
}

pub fn parallel(a: Vector3, b: Vector3) -> bool {
    match (a.try_normalize(), b.try_normalize()) {
        (Some(a), Some(b)) => a.cross(b).length() <= ANGULAR_RESOLUTION,
        _ => false,
    }
}

pub fn same_direction(a: Vector3, b: Vector3) -> bool {
    parallel(a, b) && a.dot(b) > 0.0
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SamplingTolerance {
    chord: f64,
    angle: f64,
}

impl SamplingTolerance {
    pub fn new(chord: f64, angle: f64) -> Option<Self> {
        let usable = chord.is_finite() && chord > 0.0 && angle.is_finite() && angle > 0.0;
        usable.then(|| Self {
            chord: chord.max(MIN_CHORD_TOLERANCE),
            angle: angle.clamp(MIN_ANGLE_TOLERANCE, MAX_ANGLE_TOLERANCE),
        })
    }

    pub fn for_extent(extent: f64) -> Self {
        let chord = if extent.is_finite() && extent > 0.0 {
            extent * DEFAULT_CHORD_FRACTION
        } else {
            1.0
        };
        Self {
            chord: chord.max(MIN_CHORD_TOLERANCE),
            angle: DEFAULT_ANGLE_TOLERANCE,
        }
    }

    pub fn chord(&self) -> f64 {
        self.chord
    }

    pub fn angle(&self) -> f64 {
        self.angle
    }

    pub(crate) fn step_on_radius(&self, radius: f64) -> f64 {
        let by_angle = radius * self.angle;
        let by_chord = if self.chord >= 2.0 * radius {
            f64::INFINITY
        } else {
            2.0 * radius * (1.0 - self.chord / radius).clamp(-1.0, 1.0).acos()
        };
        by_angle.min(by_chord)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angular_resolution_moves_a_point_at_model_extent_by_the_linear_resolution() {
        assert!((ANGULAR_RESOLUTION * MODEL_EXTENT - LINEAR_RESOLUTION).abs() < 1e-18);
        assert!(same_point(Point3::ZERO, Point3::new(0.0, 0.0, 0.5e-6)));
        assert!(!same_point(Point3::ZERO, Point3::new(0.0, 0.0, 2e-6)));
        assert!(same_direction(Vector3::X, Vector3::new(2.0, 1e-12, 0.0)));
        assert!(parallel(Vector3::X, Vector3::NEG_X));
        assert!(!same_direction(Vector3::X, Vector3::NEG_X));
        assert!(!parallel(Vector3::X, Vector3::ZERO));
    }

    #[test]
    fn sampling_tolerances_are_clamped_and_reject_bad_input() {
        assert!(SamplingTolerance::new(0.0, 0.1).is_none());
        assert!(SamplingTolerance::new(0.1, f64::NAN).is_none());
        let clamped = SamplingTolerance::new(1e-12, 10.0).unwrap();
        assert_eq!(clamped.chord(), MIN_CHORD_TOLERANCE);
        assert_eq!(clamped.angle(), MAX_ANGLE_TOLERANCE);
        let derived = SamplingTolerance::for_extent(100.0);
        assert!((derived.chord() - 0.1).abs() < 1e-12);
        assert_eq!(SamplingTolerance::for_extent(f64::NAN).chord(), 1.0);
    }

    #[test]
    fn step_on_radius_holds_both_limits() {
        let tolerance = SamplingTolerance::new(0.01, 0.2).unwrap();
        let step = tolerance.step_on_radius(10.0);
        assert!(step <= 10.0 * 0.2 + 1e-12);
        let sagitta = 10.0 * (1.0 - (step / 20.0).cos());
        assert!(sagitta <= 0.01 + 1e-12);
        assert!(tolerance.step_on_radius(0.001) <= 0.001 * 0.2 + 1e-15);
    }
}
