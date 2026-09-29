use caditor_geometry::{Plane, Ray, Vector3};
use caditor_kernel::LINEAR_RESOLUTION;

pub(crate) const DIRECTION_TOLERANCE: f64 = 1e-6;
pub(crate) const POSITION_TOLERANCE: f64 = LINEAR_RESOLUTION;

pub(crate) fn parallel(first: Vector3, second: Vector3) -> bool {
    first
        .normalize_or_zero()
        .cross(second.normalize_or_zero())
        .length()
        <= DIRECTION_TOLERANCE
}

pub(crate) fn perpendicular(first: Vector3, second: Vector3) -> bool {
    first
        .normalize_or_zero()
        .dot(second.normalize_or_zero())
        .abs()
        <= DIRECTION_TOLERANCE
}

pub(crate) fn on_line(ray: Ray, point: caditor_geometry::Point3) -> bool {
    (point - ray.origin())
        .cross(ray.direction().normalize_or_zero())
        .length()
        <= POSITION_TOLERANCE
}

pub(crate) fn same_line(first: Ray, second: Ray) -> bool {
    parallel(first.direction(), second.direction()) && on_line(first, second.origin())
}

pub(crate) fn along_plane(ray: Ray, plane: &Plane) -> bool {
    perpendicular(ray.direction(), plane.normal())
        && plane.signed_distance(ray.origin()).abs() <= POSITION_TOLERANCE
}

pub(crate) fn same_plane(first: &Plane, second: &Plane) -> bool {
    parallel(first.normal(), second.normal())
        && first.normal().dot(second.normal()) > 0.0
        && first.signed_distance(second.origin()).abs() <= POSITION_TOLERANCE
}

#[cfg(test)]
mod tests {
    use caditor_geometry::Point3;

    use super::*;

    #[test]
    fn slightly_noisy_geometry_counts_as_lying_on_a_plane_or_line() {
        let noise = 0.5 * POSITION_TOLERANCE;
        let plane = Plane::XY;
        let edge = Ray::new(Point3::new(3.0, 1.0, noise), Vector3::new(10.0, 0.0, noise)).unwrap();
        let lifted = Ray::new(Point3::new(3.0, 1.0, 0.1), Vector3::X).unwrap();
        let tilted = Ray::new(Point3::new(3.0, 1.0, 0.0), Vector3::new(1.0, 0.0, 0.01)).unwrap();
        let shifted =
            Plane::from_frame(Point3::new(5.0, 2.0, noise), Vector3::Z, Vector3::X).unwrap();
        let flipped = Plane::from_frame(Point3::ZERO, -Vector3::Z, Vector3::X).unwrap();

        assert!(along_plane(edge, &plane));
        assert!(!along_plane(lifted, &plane));
        assert!(!along_plane(tilted, &plane));
        assert!(same_line(
            edge,
            Ray::new(Point3::new(7.0, 1.0, 0.0), -Vector3::X).unwrap()
        ));
        assert!(same_plane(&plane, &shifted));
        assert!(!same_plane(&plane, &flipped));
    }
}
