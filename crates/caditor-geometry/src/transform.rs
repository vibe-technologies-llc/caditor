use crate::{Point2, Point3, Rotation3, Vector2, Vector3};

const UNIT_TOLERANCE: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RigidTransform {
    rotation: Rotation3,
    translation: Vector3,
}

impl RigidTransform {
    pub const IDENTITY: Self = Self {
        rotation: Rotation3::IDENTITY,
        translation: Vector3::ZERO,
    };

    pub fn new(rotation: Rotation3, translation: Vector3) -> Option<Self> {
        let length = rotation.length();
        let usable = rotation.is_finite() && translation.is_finite() && length > UNIT_TOLERANCE;
        usable.then(|| Self {
            rotation: rotation / length,
            translation,
        })
    }

    pub fn translation(offset: Vector3) -> Option<Self> {
        Self::new(Rotation3::IDENTITY, offset)
    }

    pub fn rotation_about(axis_origin: Point3, axis: Vector3, angle: f64) -> Option<Self> {
        let axis = axis.try_normalize()?;
        if !angle.is_finite() || !axis_origin.is_finite() {
            return None;
        }
        let rotation = Rotation3::from_axis_angle(axis, angle);
        Self::new(rotation, axis_origin - rotation * axis_origin)
    }

    pub fn rotation(&self) -> Rotation3 {
        self.rotation
    }

    pub fn offset(&self) -> Vector3 {
        self.translation
    }

    pub fn apply_point(&self, point: Point3) -> Point3 {
        self.rotation * point + self.translation
    }

    pub fn apply_vector(&self, vector: Vector3) -> Vector3 {
        self.rotation * vector
    }

    #[must_use]
    pub fn then(&self, next: &Self) -> Self {
        Self {
            rotation: (next.rotation * self.rotation).normalize(),
            translation: next.rotation * self.translation + next.translation,
        }
    }

    #[must_use]
    pub fn inverse(&self) -> Self {
        let rotation = self.rotation.conjugate();
        Self {
            rotation,
            translation: -(rotation * self.translation),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RigidTransform2 {
    rotation: Vector2,
    translation: Vector2,
}

impl RigidTransform2 {
    pub const IDENTITY: Self = Self {
        rotation: Vector2::X,
        translation: Vector2::ZERO,
    };

    pub fn new(angle: f64, translation: Vector2) -> Option<Self> {
        (angle.is_finite() && translation.is_finite()).then(|| Self {
            rotation: Vector2::from_angle(angle),
            translation,
        })
    }

    pub fn angle(&self) -> f64 {
        self.rotation.to_angle()
    }

    pub fn offset(&self) -> Vector2 {
        self.translation
    }

    pub fn apply_point(&self, point: Point2) -> Point2 {
        self.rotation.rotate(point) + self.translation
    }

    pub fn apply_vector(&self, vector: Vector2) -> Vector2 {
        self.rotation.rotate(vector)
    }

    #[must_use]
    pub fn then(&self, next: &Self) -> Self {
        Self {
            rotation: next.rotation.rotate(self.rotation).normalize(),
            translation: next.rotation.rotate(self.translation) + next.translation,
        }
    }

    #[must_use]
    pub fn inverse(&self) -> Self {
        let rotation = Vector2::new(self.rotation.x, -self.rotation.y);
        Self {
            rotation,
            translation: -rotation.rotate(self.translation),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::f64::consts::FRAC_PI_2;

    use super::*;

    const EPSILON: f64 = 1e-12;

    #[test]
    fn rotation_about_an_axis_keeps_the_axis_fixed() {
        let origin = Point3::new(1.0, 2.0, 3.0);
        let turn = RigidTransform::rotation_about(origin, Vector3::Z, FRAC_PI_2).unwrap();
        assert!(turn.apply_point(origin).distance(origin) < EPSILON);
        let moved = turn.apply_point(origin + Vector3::X);
        assert!(moved.distance(origin + Vector3::Y) < EPSILON);
        assert!(turn.apply_vector(Vector3::X).distance(Vector3::Y) < EPSILON);
    }

    #[test]
    fn composition_and_inverse_round_trip() {
        let a =
            RigidTransform::rotation_about(Point3::ONE, Vector3::new(1.0, 2.0, 0.5), 0.7).unwrap();
        let b = RigidTransform::translation(Vector3::new(-4.0, 1.0, 9.0)).unwrap();
        let point = Point3::new(0.3, -2.0, 5.0);
        let composed = a.then(&b);
        assert!(
            composed
                .apply_point(point)
                .distance(b.apply_point(a.apply_point(point)))
                < EPSILON
        );
        assert!(
            composed
                .inverse()
                .apply_point(composed.apply_point(point))
                .distance(point)
                < EPSILON
        );
    }

    #[test]
    fn rejects_non_finite_or_zero_input() {
        assert!(RigidTransform::translation(Vector3::new(f64::NAN, 0.0, 0.0)).is_none());
        assert!(
            RigidTransform::new(Rotation3::from_xyzw(0.0, 0.0, 0.0, 0.0), Vector3::ZERO).is_none()
        );
        assert!(RigidTransform::rotation_about(Point3::ZERO, Vector3::ZERO, 1.0).is_none());
        assert!(RigidTransform2::new(f64::INFINITY, Vector2::ZERO).is_none());
    }

    #[test]
    fn planar_transforms_compose_and_invert() {
        let a = RigidTransform2::new(FRAC_PI_2, Vector2::new(1.0, 0.0)).unwrap();
        let b = RigidTransform2::new(-0.3, Vector2::new(2.0, -5.0)).unwrap();
        let point = Point2::new(3.0, 4.0);
        assert!(a.apply_point(Point2::X).distance(Point2::new(1.0, 1.0)) < EPSILON);
        let composed = a.then(&b);
        assert!(
            composed
                .apply_point(point)
                .distance(b.apply_point(a.apply_point(point)))
                < EPSILON
        );
        assert!(
            composed
                .inverse()
                .apply_point(composed.apply_point(point))
                .distance(point)
                < EPSILON
        );
    }
}
