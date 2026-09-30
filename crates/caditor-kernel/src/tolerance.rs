use std::f64::consts::{FRAC_PI_4, PI};

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
const COARSE_CHORD_FRACTION: f64 = 1e-3;
const COARSE_ANGLE: f64 = 0.35;
const SMOOTH_CHORD_FRACTION: f64 = 2.5e-4;
const SMOOTH_ANGLE: f64 = 6.0 * PI / 180.0;
const UNKNOWN_EXTENT_CHORD: f64 = 1.0;

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
        MeshQuality::COARSE.tolerance(extent)
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshQuality {
    chord_fraction: f64,
    angle: f64,
}

impl MeshQuality {
    pub const COARSE: Self = Self {
        chord_fraction: COARSE_CHORD_FRACTION,
        angle: COARSE_ANGLE,
    };
    pub const SMOOTH: Self = Self {
        chord_fraction: SMOOTH_CHORD_FRACTION,
        angle: SMOOTH_ANGLE,
    };

    pub fn new(chord_fraction: f64, angle: f64) -> Option<Self> {
        let usable =
            chord_fraction.is_finite() && chord_fraction > 0.0 && angle.is_finite() && angle > 0.0;
        usable.then(|| Self {
            chord_fraction,
            angle: angle.clamp(MIN_ANGLE_TOLERANCE, MAX_ANGLE_TOLERANCE),
        })
    }

    pub fn chord_fraction(&self) -> f64 {
        self.chord_fraction
    }

    pub fn angle(&self) -> f64 {
        self.angle
    }

    #[must_use]
    pub fn at_least(&self, coarse: &Self) -> Self {
        Self {
            chord_fraction: self.chord_fraction.max(coarse.chord_fraction),
            angle: self.angle.max(coarse.angle),
        }
    }

    pub fn tolerance(&self, extent: f64) -> SamplingTolerance {
        let chord = if extent.is_finite() && extent > 0.0 {
            extent * self.chord_fraction
        } else {
            UNKNOWN_EXTENT_CHORD
        };
        SamplingTolerance {
            chord: chord.max(MIN_CHORD_TOLERANCE),
            angle: self.angle,
        }
    }
}

impl Default for MeshQuality {
    fn default() -> Self {
        Self::SMOOTH
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
    fn mesh_qualities_scale_the_chord_with_the_extent_and_keep_their_angle() {
        let smooth = MeshQuality::SMOOTH.tolerance(200.0);
        let coarse = MeshQuality::COARSE.tolerance(200.0);

        assert!((smooth.chord() - 0.05).abs() < 1e-12);
        assert!((smooth.angle().to_degrees() - 6.0).abs() < 1e-9);
        assert!(smooth.chord() < coarse.chord() && smooth.angle() < coarse.angle());
        assert_eq!(coarse, SamplingTolerance::for_extent(200.0));
        assert_eq!(MeshQuality::default(), MeshQuality::SMOOTH);

        assert!(MeshQuality::new(0.0, 0.1).is_none());
        assert!(MeshQuality::new(1e-3, f64::INFINITY).is_none());
        let clamped = MeshQuality::new(1e-3, 10.0).unwrap();
        assert_eq!(clamped.angle(), MAX_ANGLE_TOLERANCE);
        assert_eq!(clamped.tolerance(1e-9).chord(), MIN_CHORD_TOLERANCE);
        assert_eq!(clamped.tolerance(f64::NAN).chord(), 1.0);
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
