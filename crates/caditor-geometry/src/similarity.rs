use glam::DMat3;

use crate::{Plane, Point3, RigidTransform, Rotation3, Vector3};

const SCALE_LIMIT: f64 = 1e6;
const MIRROR: DMat3 = DMat3::from_diagonal(Vector3::new(1.0, 1.0, -1.0));

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Similarity {
    rotation: Rotation3,
    mirrored: bool,
    scale: f64,
    translation: Vector3,
}

impl Similarity {
    pub const IDENTITY: Self = Self {
        rotation: Rotation3::IDENTITY,
        mirrored: false,
        scale: 1.0,
        translation: Vector3::ZERO,
    };

    pub fn reflection(plane: &Plane) -> Option<Self> {
        let normal = plane.normal().try_normalize()?;
        if !plane.origin().is_finite() {
            return None;
        }
        let linear = DMat3::IDENTITY - 2.0 * outer(normal, normal);
        Some(Self::from_linear(
            linear,
            1.0,
            2.0 * plane.origin().dot(normal) * normal,
        ))
    }

    pub fn scaling(center: Point3, factor: f64) -> Option<Self> {
        let usable = center.is_finite()
            && factor.is_finite()
            && (1.0 / SCALE_LIMIT..=SCALE_LIMIT).contains(&factor);
        usable.then(|| Self {
            scale: factor,
            translation: center - center * factor,
            ..Self::IDENTITY
        })
    }

    fn from_linear(linear: DMat3, scale: f64, translation: Vector3) -> Self {
        let mirrored = linear.determinant() < 0.0;
        let proper = if mirrored { linear * MIRROR } else { linear };
        Self {
            rotation: Rotation3::from_mat3(&proper).normalize(),
            mirrored,
            scale,
            translation,
        }
    }

    fn linear(&self) -> DMat3 {
        let rotation = DMat3::from_quat(self.rotation);
        if self.mirrored {
            rotation * MIRROR
        } else {
            rotation
        }
    }

    pub fn is_mirrored(&self) -> bool {
        self.mirrored
    }

    pub fn scale(&self) -> f64 {
        self.scale
    }

    pub fn is_rigid(&self) -> bool {
        !self.mirrored && self.scale == 1.0
    }

    pub fn offset(&self) -> Vector3 {
        self.translation
    }

    pub fn apply_point(&self, point: Point3) -> Point3 {
        self.rotation * (self.mirror(point) * self.scale) + self.translation
    }

    pub fn apply_vector(&self, vector: Vector3) -> Vector3 {
        self.apply_direction(vector) * self.scale
    }

    pub fn apply_direction(&self, direction: Vector3) -> Vector3 {
        self.rotation * self.mirror(direction)
    }

    fn mirror(&self, vector: Vector3) -> Vector3 {
        if self.mirrored {
            Vector3::new(vector.x, vector.y, -vector.z)
        } else {
            vector
        }
    }

    #[must_use]
    pub fn then(&self, next: &Self) -> Self {
        Self::from_linear(
            next.linear() * self.linear(),
            self.scale * next.scale,
            next.apply_vector(self.translation) + next.translation,
        )
    }

    #[must_use]
    pub fn inverse(&self) -> Self {
        let linear = self.linear().transpose();
        Self::from_linear(
            linear,
            1.0 / self.scale,
            -(linear * self.translation) / self.scale,
        )
    }
}

impl From<RigidTransform> for Similarity {
    fn from(rigid: RigidTransform) -> Self {
        Self {
            rotation: rigid.rotation(),
            translation: rigid.offset(),
            ..Self::IDENTITY
        }
    }
}

fn outer(a: Vector3, b: Vector3) -> DMat3 {
    DMat3::from_cols(a * b.x, a * b.y, a * b.z)
}

#[cfg(test)]
mod tests {
    use std::f64::consts::FRAC_PI_3;

    use super::*;

    const EPSILON: f64 = 1e-12;

    fn tilted_plane() -> Plane {
        Plane::new(Point3::new(1.0, -2.0, 3.0), Vector3::new(1.0, 2.0, -0.5)).unwrap()
    }

    #[test]
    fn a_reflection_keeps_its_plane_and_mirrors_across_it() {
        let plane = tilted_plane();
        let mirror = Similarity::reflection(&plane).unwrap();
        let on_plane = plane.to_world(crate::Point2::new(4.0, -7.0));
        let off_plane = on_plane + plane.normal() * 2.5;

        assert!(mirror.is_mirrored());
        assert_eq!(mirror.scale(), 1.0);
        assert!(mirror.apply_point(on_plane).distance(on_plane) < EPSILON);
        assert!(
            mirror
                .apply_point(off_plane)
                .distance(on_plane - plane.normal() * 2.5)
                < EPSILON
        );
        assert!(
            mirror
                .apply_direction(plane.normal())
                .distance(-plane.normal())
                < EPSILON
        );
        assert!(
            mirror
                .apply_direction(plane.x_axis())
                .distance(plane.x_axis())
                < EPSILON
        );
    }

    #[test]
    fn a_scaling_keeps_its_center_and_stretches_distances_from_it() {
        let center = Point3::new(2.0, 3.0, -1.0);
        let scaling = Similarity::scaling(center, 2.5).unwrap();
        let point = center + Vector3::new(1.0, -2.0, 4.0);

        assert!(!scaling.is_mirrored());
        assert!(scaling.apply_point(center).distance(center) < EPSILON);
        assert!(
            (scaling.apply_point(point).distance(center) - 2.5 * point.distance(center)).abs()
                < EPSILON
        );
        assert!(scaling.apply_direction(Vector3::X).distance(Vector3::X) < EPSILON);
        assert!(scaling.apply_vector(Vector3::X).distance(Vector3::X * 2.5) < EPSILON);
    }

    #[test]
    fn a_rigid_transform_maps_as_it_did() {
        let rigid = RigidTransform::rotation_about(Point3::ONE, Vector3::new(1.0, 2.0, 0.5), 0.7)
            .unwrap()
            .then(&RigidTransform::translation(Vector3::new(-4.0, 1.0, 9.0)).unwrap());
        let similarity = Similarity::from(rigid);
        let point = Point3::new(0.3, -2.0, 5.0);

        assert!(similarity.is_rigid());
        assert_eq!(similarity.apply_point(point), rigid.apply_point(point));
        assert_eq!(similarity.apply_direction(point), rigid.apply_vector(point));
    }

    #[test]
    fn composition_and_inverse_round_trip() {
        let mirror = Similarity::reflection(&tilted_plane()).unwrap();
        let turn = Similarity::from(
            RigidTransform::rotation_about(Point3::ZERO, Vector3::Z, FRAC_PI_3).unwrap(),
        );
        let scaling = Similarity::scaling(Point3::new(0.0, 1.0, 2.0), 0.4).unwrap();
        let point = Point3::new(0.3, -2.0, 5.0);
        let composed = mirror.then(&turn).then(&scaling);
        let stepwise = scaling.apply_point(turn.apply_point(mirror.apply_point(point)));

        assert!(composed.is_mirrored());
        assert!((composed.scale() - 0.4).abs() < EPSILON);
        assert!(composed.apply_point(point).distance(stepwise) < EPSILON);
        assert!(
            composed
                .inverse()
                .apply_point(composed.apply_point(point))
                .distance(point)
                < EPSILON
        );
        assert!(!mirror.then(&mirror).is_mirrored());
        assert!(mirror.then(&mirror).apply_point(point).distance(point) < EPSILON);
    }

    #[test]
    fn rejects_degenerate_input() {
        let flat = Plane::XY;

        assert!(Similarity::scaling(Point3::ZERO, 0.0).is_none());
        assert!(Similarity::scaling(Point3::ZERO, -2.0).is_none());
        assert!(Similarity::scaling(Point3::ZERO, f64::NAN).is_none());
        assert!(Similarity::scaling(Point3::new(f64::INFINITY, 0.0, 0.0), 2.0).is_none());
        assert!(Similarity::reflection(&flat).is_some());
    }
}
