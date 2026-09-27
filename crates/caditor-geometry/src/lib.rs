pub use glam::{DVec2 as Point2, DVec2 as Vector2, DVec3 as Point3, DVec3 as Vector3};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plane {
    pub origin: Point3,
    pub normal: Vector3,
}

impl Plane {
    pub const XY: Self = Self::through_origin(Vector3::Z);
    pub const XZ: Self = Self::through_origin(Vector3::Y);
    pub const YZ: Self = Self::through_origin(Vector3::X);

    const fn through_origin(normal: Vector3) -> Self {
        Self {
            origin: Point3::ZERO,
            normal,
        }
    }

    pub fn new(origin: Point3, normal: Vector3) -> Option<Self> {
        let normal = normal.try_normalize()?;
        Some(Self { origin, normal })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn principal_planes_have_unit_normals_through_origin() {
        for plane in [Plane::XY, Plane::XZ, Plane::YZ] {
            assert_eq!(plane.origin, Point3::ZERO);
            assert!(plane.normal.is_normalized());
        }
        assert_eq!(Plane::XY.normal, Vector3::Z);
    }

    #[test]
    fn new_normalizes_and_rejects_degenerate_normals() {
        let plane = Plane::new(Point3::ONE, Vector3::new(0.0, 0.0, 5.0)).unwrap();
        assert_eq!(plane.normal, Vector3::Z);
        assert!(Plane::new(Point3::ONE, Vector3::ZERO).is_none());
    }
}
