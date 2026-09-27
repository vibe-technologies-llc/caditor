use crate::{Point3, Vector3};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    min: Point3,
    max: Point3,
}

impl Aabb {
    pub fn from_point(point: Point3) -> Self {
        Self {
            min: point,
            max: point,
        }
    }

    pub fn from_points(points: impl IntoIterator<Item = Point3>) -> Option<Self> {
        let mut points = points.into_iter();
        let first = Self::from_point(points.next()?);
        Some(points.fold(first, Self::including))
    }

    pub fn min(&self) -> Point3 {
        self.min
    }

    pub fn max(&self) -> Point3 {
        self.max
    }

    #[must_use]
    pub fn including(self, point: Point3) -> Self {
        Self {
            min: self.min.min(point),
            max: self.max.max(point),
        }
    }

    #[must_use]
    pub fn union(self, other: Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    pub fn center(&self) -> Point3 {
        (self.min + self.max) * 0.5
    }

    pub fn half_extent(&self) -> Vector3 {
        (self.max - self.min) * 0.5
    }

    pub fn bounding_radius(&self) -> f64 {
        self.half_extent().length()
    }

    pub fn corners(&self) -> [Point3; 8] {
        let Self { min, max } = *self;
        [
            Point3::new(min.x, min.y, min.z),
            Point3::new(max.x, min.y, min.z),
            Point3::new(min.x, max.y, min.z),
            Point3::new(max.x, max.y, min.z),
            Point3::new(min.x, min.y, max.z),
            Point3::new(max.x, min.y, max.z),
            Point3::new(min.x, max.y, max.z),
            Point3::new(max.x, max.y, max.z),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grows_to_contain_every_point() {
        let bounds = Aabb::from_points([
            Point3::new(1.0, -2.0, 0.0),
            Point3::new(-3.0, 4.0, 2.0),
            Point3::new(0.0, 0.0, -6.0),
        ])
        .unwrap();

        assert_eq!(bounds.min(), Point3::new(-3.0, -2.0, -6.0));
        assert_eq!(bounds.max(), Point3::new(1.0, 4.0, 2.0));
        assert_eq!(bounds.center(), Point3::new(-1.0, 1.0, -2.0));
        assert!(Aabb::from_points([]).is_none());
    }

    #[test]
    fn union_covers_both_boxes() {
        let a = Aabb::from_point(Point3::ZERO);
        let b = Aabb::from_point(Point3::new(2.0, 2.0, 1.0));
        let joined = a.union(b);

        assert_eq!(joined.half_extent(), Vector3::new(1.0, 1.0, 0.5));
        assert_eq!(joined.bounding_radius(), 1.5);
        assert!(joined.corners().contains(&Point3::new(2.0, 0.0, 1.0)));
    }
}
