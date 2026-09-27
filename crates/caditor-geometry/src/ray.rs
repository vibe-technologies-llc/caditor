use crate::{Plane, Point3, Vector3};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    origin: Point3,
    direction: Vector3,
}

impl Ray {
    pub fn new(origin: Point3, direction: Vector3) -> Option<Self> {
        let direction = direction.try_normalize()?;
        Some(Self { origin, direction })
    }

    pub fn origin(&self) -> Point3 {
        self.origin
    }

    pub fn direction(&self) -> Vector3 {
        self.direction
    }

    pub fn at(&self, distance: f64) -> Point3 {
        self.origin + self.direction * distance
    }

    pub fn intersect_plane(&self, plane: &Plane) -> Option<f64> {
        let facing = self.direction.dot(plane.normal());
        if facing.abs() < 1e-12 {
            return None;
        }
        let distance = -plane.signed_distance(self.origin) / facing;
        (distance >= 0.0).then_some(distance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hits_a_plane_in_front_and_misses_one_behind_or_parallel() {
        let ray = Ray::new(Point3::new(1.0, 2.0, 10.0), Vector3::NEG_Z).unwrap();

        let distance = ray.intersect_plane(&Plane::XY).unwrap();
        assert_eq!(distance, 10.0);
        assert_eq!(ray.at(distance), Point3::new(1.0, 2.0, 0.0));

        let behind = Plane::new(Point3::new(0.0, 0.0, 20.0), Vector3::Z).unwrap();
        assert_eq!(ray.intersect_plane(&behind), None);
        assert_eq!(ray.intersect_plane(&Plane::XZ), None);
    }

    #[test]
    fn rejects_a_zero_direction() {
        assert!(Ray::new(Point3::ZERO, Vector3::ZERO).is_none());
    }
}
