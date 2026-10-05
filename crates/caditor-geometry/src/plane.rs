use crate::{Point2, Point3, RigidTransform, Similarity, Vector3};

const FRAME_TOLERANCE: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plane {
    origin: Point3,
    normal: Vector3,
    x_axis: Vector3,
}

impl Plane {
    pub const XY: Self = Self::through_origin(Vector3::Z, Vector3::X);
    pub const XZ: Self = Self::through_origin(Vector3::NEG_Y, Vector3::X);
    pub const YZ: Self = Self::through_origin(Vector3::X, Vector3::Y);

    const fn through_origin(normal: Vector3, x_axis: Vector3) -> Self {
        Self {
            origin: Point3::ZERO,
            normal,
            x_axis,
        }
    }

    pub fn new(origin: Point3, normal: Vector3) -> Option<Self> {
        let normal = normal.try_normalize()?;
        let reference = if normal.x.abs() < 0.9 {
            Vector3::X
        } else {
            Vector3::Y
        };
        Self::with_x_axis(origin, normal, reference)
    }

    pub fn with_x_axis(origin: Point3, normal: Vector3, x_direction: Vector3) -> Option<Self> {
        let normal = normal.try_normalize()?;
        let in_plane = x_direction.reject_from_normalized(normal);
        if in_plane.length() <= FRAME_TOLERANCE * x_direction.length() {
            return None;
        }
        let x_axis = in_plane.try_normalize()?;
        Some(Self {
            origin,
            normal,
            x_axis,
        })
    }

    pub fn from_frame(origin: Point3, normal: Vector3, x_axis: Vector3) -> Option<Self> {
        let finite = origin.is_finite() && normal.is_finite() && x_axis.is_finite();
        let orthonormal = (normal.length() - 1.0).abs() <= FRAME_TOLERANCE
            && (x_axis.length() - 1.0).abs() <= FRAME_TOLERANCE
            && normal.dot(x_axis).abs() <= FRAME_TOLERANCE;
        match (finite, orthonormal) {
            (false, _) => None,
            (true, true) => Some(Self {
                origin,
                normal,
                x_axis,
            }),
            (true, false) => Self::with_x_axis(origin, normal, x_axis),
        }
    }

    pub fn origin(&self) -> Point3 {
        self.origin
    }

    pub fn normal(&self) -> Vector3 {
        self.normal
    }

    pub fn x_axis(&self) -> Vector3 {
        self.x_axis
    }

    pub fn y_axis(&self) -> Vector3 {
        self.normal.cross(self.x_axis)
    }

    pub fn to_world(&self, point: Point2) -> Point3 {
        self.origin + self.x_axis * point.x + self.y_axis() * point.y
    }

    pub fn to_local(&self, point: Point3) -> Point2 {
        let offset = point - self.origin;
        Point2::new(offset.dot(self.x_axis), offset.dot(self.y_axis()))
    }

    pub fn signed_distance(&self, point: Point3) -> f64 {
        (point - self.origin).dot(self.normal)
    }

    #[must_use]
    pub fn flipped(&self) -> Self {
        Self {
            origin: self.origin,
            normal: -self.normal,
            x_axis: self.x_axis,
        }
    }

    #[must_use]
    pub fn transformed(&self, transform: &RigidTransform) -> Self {
        Self {
            origin: transform.apply_point(self.origin),
            normal: transform.apply_vector(self.normal),
            x_axis: transform.apply_vector(self.x_axis),
        }
    }

    #[must_use]
    pub fn mapped(&self, similarity: &Similarity) -> Self {
        Self {
            origin: similarity.apply_point(self.origin),
            normal: similarity.apply_direction(self.normal),
            x_axis: similarity.apply_direction(self.x_axis),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn principal_planes_are_right_handed_through_origin() {
        for plane in [Plane::XY, Plane::XZ, Plane::YZ] {
            assert_eq!(plane.origin(), Point3::ZERO);
            assert!(plane.normal().is_normalized());
            assert!(plane.x_axis().is_normalized());
            assert_eq!(plane.x_axis().cross(plane.y_axis()), plane.normal());
        }
        assert_eq!(Plane::XY.y_axis(), Vector3::Y);
        assert_eq!(Plane::XZ.y_axis(), Vector3::Z);
        assert_eq!(Plane::YZ.y_axis(), Vector3::Z);
    }

    #[test]
    fn new_normalizes_and_rejects_degenerate_normals() {
        let plane = Plane::new(Point3::ONE, Vector3::new(0.0, 0.0, 5.0)).unwrap();
        assert_eq!(plane.normal(), Vector3::Z);
        assert!(plane.x_axis().dot(plane.normal()).abs() < 1e-12);
        assert!(Plane::new(Point3::ONE, Vector3::ZERO).is_none());
        assert!(Plane::with_x_axis(Point3::ONE, Vector3::Z, Vector3::Z).is_none());
    }

    #[test]
    fn an_x_direction_along_a_slanted_normal_is_refused_rather_than_rounded() {
        let normal = Vector3::new(-1.0, -1.0, -1.0);
        let along = Vector3::new(1.0, 1.0, 1.0);

        assert!(Plane::with_x_axis(Point3::ZERO, normal, along).is_none());
        assert!(Plane::from_frame(Point3::ZERO, normal, along).is_none());
        assert!(Plane::with_x_axis(Point3::ZERO, normal, along + Vector3::X * 1e-6).is_some());
    }

    #[test]
    fn from_frame_keeps_an_orthonormal_frame_exactly_and_repairs_others() {
        let tilted = Plane::new(Point3::ONE, Vector3::new(1.0, 2.0, 3.0)).unwrap();
        let restored = Plane::from_frame(tilted.origin(), tilted.normal(), tilted.x_axis());
        assert_eq!(restored, Some(tilted));

        let repaired =
            Plane::from_frame(Point3::ZERO, Vector3::Z * 2.0, Vector3::new(1.0, 0.0, 1.0));
        assert_eq!(repaired, Some(Plane::XY));
        assert_eq!(
            Plane::from_frame(Point3::ZERO, Vector3::Z, Vector3::Z),
            None
        );
        assert_eq!(
            Plane::from_frame(Point3::new(f64::NAN, 0.0, 0.0), Vector3::Z, Vector3::X),
            None
        );
    }

    #[test]
    fn local_and_world_coordinates_round_trip() {
        let plane = Plane::new(Point3::new(3.0, -2.0, 7.0), Vector3::new(1.0, 1.0, 1.0)).unwrap();
        let local = Point2::new(4.5, -1.25);
        let world = plane.to_world(local);

        assert!(plane.signed_distance(world).abs() < 1e-12);
        assert!(plane.to_local(world).distance(local) < 1e-12);
    }

    #[test]
    fn flipping_keeps_the_x_axis_and_mirrors_the_y_axis() {
        let flipped = Plane::XY.flipped();
        assert_eq!(flipped.normal(), Vector3::NEG_Z);
        assert_eq!(flipped.x_axis(), Vector3::X);
        assert_eq!(flipped.y_axis(), Vector3::NEG_Y);
    }

    #[test]
    fn transforming_moves_the_whole_frame() {
        let turn =
            RigidTransform::rotation_about(Point3::ZERO, Vector3::X, std::f64::consts::FRAC_PI_2)
                .unwrap()
                .then(&RigidTransform::translation(Vector3::new(0.0, 0.0, 5.0)).unwrap());
        let moved = Plane::XY.transformed(&turn);
        assert!(moved.origin().distance(Point3::new(0.0, 0.0, 5.0)) < 1e-12);
        assert!(moved.normal().distance(Vector3::NEG_Y) < 1e-12);
        assert!(moved.x_axis().distance(Vector3::X) < 1e-12);
    }

    #[test]
    fn mapping_by_a_reflection_keeps_the_normal_and_mirrors_the_y_axis() {
        let mirror = Similarity::reflection(&Plane::YZ).unwrap();
        let frame = Plane::with_x_axis(Point3::new(2.0, 1.0, 0.0), Vector3::X, Vector3::Y).unwrap();
        let mapped = frame.mapped(&mirror);

        assert!(mapped.origin().distance(Point3::new(-2.0, 1.0, 0.0)) < 1e-12);
        assert!(mapped.normal().distance(Vector3::NEG_X) < 1e-12);
        assert!(mapped.x_axis().distance(Vector3::Y) < 1e-12);
        assert!(mapped.y_axis().distance(-frame.y_axis()) < 1e-12);
    }
}
