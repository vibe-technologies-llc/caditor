use crate::{Plane, Point3, Vector3};

const PARALLEL_TOLERANCE: f64 = 1e-12;

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

    pub fn closest_along_line(&self, through: Point3, along: Vector3) -> Option<f64> {
        let along = along.try_normalize()?;
        let across = along.dot(self.direction);
        let apart = 1.0 - across * across;
        if apart < PARALLEL_TOLERANCE {
            return None;
        }
        let offset = through - self.origin;
        Some((across * offset.dot(self.direction) - offset.dot(along)) / apart)
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

    #[test]
    fn finds_the_point_of_a_line_nearest_a_ray_and_none_along_it() {
        let ray = Ray::new(Point3::new(3.0, 0.0, 10.0), Vector3::NEG_Z).unwrap();

        let along = ray.closest_along_line(Point3::new(-1.0, 0.0, 0.0), Vector3::X * 2.0);
        assert!((along.unwrap() - 4.0).abs() < 1e-12);
        let skew = ray.closest_along_line(Point3::new(0.0, 5.0, 1.0), Vector3::Y);
        assert!((skew.unwrap() + 5.0).abs() < 1e-12);
        assert_eq!(ray.closest_along_line(Point3::ZERO, Vector3::Z), None);
    }
}
