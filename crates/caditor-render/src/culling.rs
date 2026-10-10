use caditor_geometry::{Aabb, Point3, RigidTransform};
use glam::{DMat4, DVec2, DVec4};

use crate::camera::View;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipWindow {
    eye: Point3,
    clip: DMat4,
    ndc_per_point: DVec2,
}

impl ClipWindow {
    pub fn new(view: &View, transform: [f32; 4], pixels_per_point: f32) -> Self {
        let [scale_x, scale_y, offset_x, offset_y] = transform.map(f64::from);
        let ndc_per_pixel = DVec2::new(scale_x, scale_y) * 2.0 / view.size().max(DVec2::ONE);
        let window = DMat4::from_cols(
            DVec4::new(scale_x, 0.0, 0.0, 0.0),
            DVec4::new(0.0, scale_y, 0.0, 0.0),
            DVec4::new(0.0, 0.0, 1.0, 0.0),
            DVec4::new(offset_x, offset_y, 0.0, 1.0),
        );
        Self {
            eye: view.eye(),
            clip: window * view.rotation_projection(),
            ndc_per_point: (ndc_per_pixel * f64::from(pixels_per_point)).abs(),
        }
    }

    pub fn sees_reaching(&self, corners: &[Point3; 8], reach_points: f64) -> bool {
        let reach = DVec2::ONE + self.ndc_per_point * reach_points.max(0.0);
        let clipped = corners.map(|corner| self.clip * (corner - self.eye).extend(1.0));
        let all = |outside: &dyn Fn(DVec4) -> bool| clipped.iter().all(|point| outside(*point));
        let beyond_a_side = all(&|point| point.x > point.w * reach.x)
            || all(&|point| point.x < -point.w * reach.x)
            || all(&|point| point.y > point.w * reach.y)
            || all(&|point| point.y < -point.w * reach.y);
        !beyond_a_side
    }

    pub fn sees(&self, corners: &[Point3; 8]) -> bool {
        let clipped = corners.map(|corner| self.clip * (corner - self.eye).extend(1.0));
        let all = |outside: fn(DVec4) -> bool| clipped.iter().all(|point| outside(*point));
        let beyond_any_side = all(|point| point.x > point.w)
            || all(|point| point.x < -point.w)
            || all(|point| point.y > point.w)
            || all(|point| point.y < -point.w)
            || all(|point| point.z > point.w)
            || all(|point| point.z < 0.0);
        !beyond_any_side
    }
}

pub fn placed_corners(bounds: Aabb, placement: Option<RigidTransform>) -> [Point3; 8] {
    let corners = bounds.corners();
    match placement {
        Some(placement) => corners.map(|corner| placement.apply_point(corner)),
        None => corners,
    }
}

#[cfg(test)]
mod tests {
    use caditor_geometry::Vector3;
    use glam::DVec2;

    use super::*;
    use crate::{
        SurfaceSize,
        camera::Viewpoint,
        image::{Tile, tile_transform},
    };

    const WHOLE: [f32; 4] = [1.0, 1.0, 0.0, 0.0];

    fn view_down_z() -> View {
        let viewpoint = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 100.0).unwrap();
        View::new(viewpoint, 800.0, 600.0)
    }

    fn cube_at(center: Point3) -> [Point3; 8] {
        placed_corners(
            Aabb::from_points([center - Vector3::ONE, center + Vector3::ONE]).unwrap(),
            None,
        )
    }

    #[test]
    fn a_box_in_view_is_seen_and_one_beside_or_behind_the_eye_is_not() {
        let window = ClipWindow::new(&view_down_z(), WHOLE, 1.0);

        assert!(window.sees(&cube_at(Point3::ZERO)));
        assert!(window.sees(&cube_at(Point3::new(20.0, 0.0, 0.0))));
        assert!(!window.sees(&cube_at(Point3::new(500.0, 0.0, 0.0))));
        assert!(!window.sees(&cube_at(Point3::new(0.0, -500.0, 0.0))));
        assert!(!window.sees(&cube_at(Point3::new(0.0, 0.0, 300.0))));
    }

    #[test]
    fn a_box_straddling_the_edge_or_the_eye_is_seen() {
        let window = ClipWindow::new(&view_down_z(), WHOLE, 1.0);
        let long = Aabb::from_points([
            Point3::new(-1000.0, -1.0, -1.0),
            Point3::new(1000.0, 1.0, 1.0),
        ])
        .unwrap();
        let through_eye =
            Aabb::from_points([Point3::new(-1.0, -1.0, -10.0), Point3::new(1.0, 1.0, 400.0)])
                .unwrap();

        assert!(window.sees(&placed_corners(long, None)));
        assert!(window.sees(&placed_corners(through_eye, None)));
    }

    #[test]
    fn a_placement_moves_the_box_before_the_test() {
        let window = ClipWindow::new(&view_down_z(), WHOLE, 1.0);
        let far = Aabb::from_points([Point3::new(499.0, -1.0, -1.0), Point3::new(501.0, 1.0, 1.0)])
            .unwrap();
        let back = RigidTransform::translation(Vector3::new(-500.0, 0.0, 0.0)).unwrap();

        assert!(!window.sees(&placed_corners(far, None)));
        assert!(window.sees(&placed_corners(far, Some(back))));
    }

    #[test]
    fn a_narrow_window_sees_only_what_lies_inside_it() {
        let view = view_down_z();
        let left_quarter = tile_transform(
            Tile {
                x: 0,
                y: 0,
                width: 200,
                height: 600,
            },
            SurfaceSize {
                width: 800,
                height: 600,
            },
        );
        let window = ClipWindow::new(&view, left_quarter, 1.0);
        let left = view.unproject(DVec2::new(100.0, 300.0), 100.0).unwrap();
        let right = view.unproject(DVec2::new(700.0, 300.0), 100.0).unwrap();

        assert!(window.sees(&cube_at(left)));
        assert!(!window.sees(&cube_at(right)));
    }
}
