use std::{f64::consts::PI, time::Duration};

use caditor_geometry::{Aabb, Plane, Point3, Ray, Rotation3, Vector3};
use glam::{DMat3, DMat4, DVec2, DVec3, dcamera::rh::proj::directx};

const FIELD_OF_VIEW_Y: f64 = 30.0 * PI / 180.0;
const NEAR_PLANE_FRACTION: f64 = 1e-3;
const ORBIT_RADIANS_PER_VIEWPORT_HEIGHT: f64 = PI;
const MIN_DISTANCE: f64 = 1e-4;
const MAX_DISTANCE: f64 = 1e8;
const MIN_FIT_RADIUS: f64 = 1.0;
const FIT_MARGIN: f64 = 1.15;
const TRANSITION_DURATION: Duration = Duration::from_millis(350);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewpoint {
    pub target: Point3,
    pub orientation: Rotation3,
    pub distance: f64,
}

impl Viewpoint {
    pub fn looking_from(direction: Vector3, target: Point3, distance: f64) -> Option<Self> {
        let toward_eye = direction.try_normalize()?;
        let forward = -toward_eye;
        let up_hint = if toward_eye.cross(Vector3::Z).length_squared() < 1e-12 {
            Vector3::Y * toward_eye.z.signum()
        } else {
            Vector3::Z
        };
        let right = forward.cross(up_hint).try_normalize()?;
        let up = right.cross(forward);
        let orientation =
            Rotation3::from_mat3(&DMat3::from_cols(right, up, toward_eye)).normalize();
        Some(Self {
            target,
            orientation,
            distance: distance.clamp(MIN_DISTANCE, MAX_DISTANCE),
        })
    }

    pub fn facing(plane: &Plane, target: Point3, distance: f64) -> Self {
        let orientation = Rotation3::from_mat3(&DMat3::from_cols(
            plane.x_axis(),
            plane.y_axis(),
            plane.normal(),
        ))
        .normalize();
        Self {
            target,
            orientation,
            distance: distance.clamp(MIN_DISTANCE, MAX_DISTANCE),
        }
    }

    pub fn forward(&self) -> Vector3 {
        self.orientation * Vector3::NEG_Z
    }

    pub fn up(&self) -> Vector3 {
        self.orientation * Vector3::Y
    }

    pub fn right(&self) -> Vector3 {
        self.orientation * Vector3::X
    }

    pub fn eye(&self) -> Point3 {
        self.target - self.forward() * self.distance
    }

    fn rotated_about(self, pivot: Point3, rotation: Rotation3) -> Self {
        Self {
            target: pivot + rotation * (self.target - pivot),
            orientation: (rotation * self.orientation).normalize(),
            distance: self.distance,
        }
    }

    fn scaled_about(self, anchor: Point3, factor: f64) -> Self {
        let distance = (self.distance * factor).clamp(MIN_DISTANCE, MAX_DISTANCE);
        let factor = distance / self.distance;
        Self {
            target: anchor + (self.target - anchor) * factor,
            orientation: self.orientation,
            distance,
        }
    }

    fn translated(self, offset: Vector3) -> Self {
        Self {
            target: self.target + offset,
            ..self
        }
    }

    fn interpolated(self, to: Self, amount: f64) -> Self {
        Self {
            target: self.target.lerp(to.target, amount),
            orientation: self.orientation.slerp(to.orientation, amount).normalize(),
            distance: (self.distance.ln() * (1.0 - amount) + to.distance.ln() * amount).exp(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    viewpoint: Viewpoint,
    size: DVec2,
}

impl View {
    pub fn new(viewpoint: Viewpoint, width: f64, height: f64) -> Self {
        Self {
            viewpoint,
            size: DVec2::new(width.max(1.0), height.max(1.0)),
        }
    }

    pub fn viewpoint(&self) -> &Viewpoint {
        &self.viewpoint
    }

    pub fn size(&self) -> DVec2 {
        self.size
    }

    pub fn eye(&self) -> Point3 {
        self.viewpoint.eye()
    }

    pub fn forward(&self) -> Vector3 {
        self.viewpoint.forward()
    }

    pub fn near_plane(&self) -> f64 {
        self.viewpoint.distance * NEAR_PLANE_FRACTION
    }

    pub fn aspect(&self) -> f64 {
        self.size.x / self.size.y
    }

    pub fn view_depth(&self, point: Point3) -> f64 {
        (point - self.eye()).dot(self.forward())
    }

    pub fn units_per_pixel_at(&self, depth: f64) -> f64 {
        2.0 * depth * tan_half_fov_y() / self.size.y
    }

    pub fn ray_through(&self, pixel: DVec2) -> Option<Ray> {
        let ndc = DVec2::new(
            pixel.x / self.size.x * 2.0 - 1.0,
            1.0 - pixel.y / self.size.y * 2.0,
        );
        let direction = DVec3::new(
            ndc.x * tan_half_fov_y() * self.aspect(),
            ndc.y * tan_half_fov_y(),
            -1.0,
        );
        Ray::new(self.eye(), self.viewpoint.orientation * direction)
    }

    pub fn project(&self, point: Point3) -> Option<DVec2> {
        let local = self.viewpoint.orientation.inverse() * (point - self.eye());
        let depth = -local.z;
        if depth <= self.near_plane() {
            return None;
        }
        let ndc = DVec2::new(
            local.x / (depth * tan_half_fov_y() * self.aspect()),
            local.y / (depth * tan_half_fov_y()),
        );
        Some(DVec2::new(
            (ndc.x + 1.0) * 0.5 * self.size.x,
            (1.0 - ndc.y) * 0.5 * self.size.y,
        ))
    }

    pub fn focal_point_under(&self, pixel: DVec2) -> Option<Point3> {
        let ray = self.ray_through(pixel)?;
        let facing = ray.direction().dot(self.forward());
        (facing > 0.0).then(|| ray.at(self.viewpoint.distance / facing))
    }

    pub fn rotation_projection(&self) -> DMat4 {
        let projection = directx::perspective_infinite_reverse(
            FIELD_OF_VIEW_Y,
            self.aspect(),
            self.near_plane(),
        );
        projection * DMat4::from_quat(self.viewpoint.orientation.inverse())
    }

    pub fn fitted(&self, bounds: Aabb) -> Viewpoint {
        let radius = bounds.bounding_radius().max(MIN_FIT_RADIUS);
        let half_fov_x = (tan_half_fov_y() * self.aspect()).atan();
        let narrowest_half_fov = half_fov_x.min(FIELD_OF_VIEW_Y * 0.5);
        Viewpoint {
            target: bounds.center(),
            orientation: self.viewpoint.orientation,
            distance: (radius / narrowest_half_fov.sin() * FIT_MARGIN)
                .clamp(MIN_DISTANCE, MAX_DISTANCE),
        }
    }
}

fn tan_half_fov_y() -> f64 {
    (FIELD_OF_VIEW_Y * 0.5).tan()
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Transition {
    from: Viewpoint,
    to: Viewpoint,
    elapsed: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Camera {
    viewpoint: Viewpoint,
    transition: Option<Transition>,
}

impl Camera {
    pub fn new(viewpoint: Viewpoint) -> Self {
        Self {
            viewpoint,
            transition: None,
        }
    }

    pub fn viewpoint(&self) -> Viewpoint {
        self.viewpoint
    }

    pub fn destination(&self) -> Viewpoint {
        self.transition
            .map_or(self.viewpoint, |transition| transition.to)
    }

    pub fn view(&self, width: f64, height: f64) -> View {
        View::new(self.viewpoint, width, height)
    }

    pub fn is_animating(&self) -> bool {
        self.transition.is_some()
    }

    pub fn animate_to(&mut self, to: Viewpoint) {
        self.transition = Some(Transition {
            from: self.viewpoint,
            to,
            elapsed: Duration::ZERO,
        });
    }

    pub fn advance(&mut self, elapsed: Duration) {
        let Some(transition) = self.transition.as_mut() else {
            return;
        };
        transition.elapsed += elapsed;
        let progress = transition.elapsed.as_secs_f64() / TRANSITION_DURATION.as_secs_f64();
        if progress >= 1.0 {
            self.viewpoint = transition.to;
            self.transition = None;
        } else {
            let eased = progress * progress * (3.0 - 2.0 * progress);
            self.viewpoint = transition.from.interpolated(transition.to, eased);
        }
    }

    pub fn orbit(&mut self, pivot: Point3, drag: DVec2, viewport_height: f64) {
        self.transition = None;
        let radians_per_pixel = ORBIT_RADIANS_PER_VIEWPORT_HEIGHT / viewport_height.max(1.0);
        let yaw = Rotation3::from_rotation_z(-drag.x * radians_per_pixel);
        let pitch = Rotation3::from_axis_angle(self.viewpoint.right(), -drag.y * radians_per_pixel);
        self.viewpoint = self.viewpoint.rotated_about(pivot, yaw * pitch);
    }

    pub fn pan(&mut self, drag: DVec2, units_per_pixel: f64) {
        self.transition = None;
        let offset =
            (self.viewpoint.up() * drag.y - self.viewpoint.right() * drag.x) * units_per_pixel;
        self.viewpoint = self.viewpoint.translated(offset);
    }

    pub fn zoom(&mut self, anchor: Point3, factor: f64) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        self.transition = None;
        self.viewpoint = self.viewpoint.scaled_about(anchor, factor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTH: f64 = 800.0;
    const HEIGHT: f64 = 600.0;

    fn isometric() -> Viewpoint {
        Viewpoint::looking_from(
            Vector3::new(1.0, -1.0, 1.0),
            Point3::new(5.0, 2.0, -1.0),
            120.0,
        )
        .unwrap()
    }

    fn assert_close(a: DVec3, b: DVec3, tolerance: f64) {
        assert!(
            a.distance(b) < tolerance,
            "{a} is not within {tolerance} of {b}"
        );
    }

    #[test]
    fn standard_directions_put_world_z_up_and_keep_x_rightward_from_above() {
        let front = Viewpoint::looking_from(Vector3::NEG_Y, Point3::ZERO, 10.0).unwrap();
        assert_close(front.forward(), Vector3::Y, 1e-12);
        assert_close(front.up(), Vector3::Z, 1e-12);
        assert_close(front.right(), Vector3::X, 1e-12);
        assert_close(front.eye(), Point3::new(0.0, -10.0, 0.0), 1e-12);

        let top = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 10.0).unwrap();
        assert_close(top.up(), Vector3::Y, 1e-12);
        assert_close(top.right(), Vector3::X, 1e-12);

        let bottom = Viewpoint::looking_from(Vector3::NEG_Z, Point3::ZERO, 10.0).unwrap();
        assert_close(bottom.up(), Vector3::NEG_Y, 1e-12);
        assert_close(bottom.right(), Vector3::X, 1e-12);
    }

    #[test]
    fn facing_a_plane_looks_along_its_normal_with_its_axes_on_screen() {
        let tilted = Plane::with_x_axis(
            Point3::new(4.0, -2.0, 7.0),
            Vector3::new(1.0, 1.0, 2.0),
            Vector3::new(1.0, -1.0, 0.0),
        )
        .unwrap();
        for plane in [Plane::XY, Plane::XZ, Plane::YZ, tilted] {
            let viewpoint = Viewpoint::facing(&plane, plane.origin(), 50.0);
            assert_close(viewpoint.forward(), -plane.normal(), 1e-12);
            assert_close(viewpoint.right(), plane.x_axis(), 1e-12);
            assert_close(viewpoint.up(), plane.y_axis(), 1e-12);
            assert_close(
                viewpoint.eye(),
                plane.origin() + plane.normal() * 50.0,
                1e-9,
            );

            let view = View::new(viewpoint, WIDTH, HEIGHT);
            let center = view.project(plane.origin()).unwrap();
            let along_x = view.project(plane.to_world(DVec2::new(1.0, 0.0))).unwrap();
            let along_y = view.project(plane.to_world(DVec2::new(0.0, 1.0))).unwrap();
            assert!(along_x.x > center.x && (along_x.y - center.y).abs() < 1e-9);
            assert!(along_y.y < center.y && (along_y.x - center.x).abs() < 1e-9);
        }
    }

    #[test]
    fn project_and_ray_agree() {
        let view = View::new(isometric(), WIDTH, HEIGHT);
        let point = Point3::new(12.0, -7.0, 3.0);
        let pixel = view.project(point).unwrap();
        let ray = view.ray_through(pixel).unwrap();
        let along = (point - ray.origin()).dot(ray.direction());

        assert_close(ray.at(along), point, 1e-9);
        assert_close(
            view.project(view.viewpoint().target).unwrap().extend(0.0),
            DVec3::new(WIDTH / 2.0, HEIGHT / 2.0, 0.0),
            1e-9,
        );
    }

    #[test]
    fn orbit_keeps_the_pivot_on_screen_and_its_distance_from_the_eye() {
        let mut camera = Camera::new(isometric());
        let pivot = Point3::new(8.0, 0.0, 4.0);
        let before = camera.view(WIDTH, HEIGHT);
        let pivot_pixel = before.project(pivot).unwrap();
        let pivot_distance = before.eye().distance(pivot);

        camera.orbit(pivot, DVec2::new(140.0, -60.0), HEIGHT);
        let after = camera.view(WIDTH, HEIGHT);

        assert!((after.eye().distance(pivot) - pivot_distance).abs() < 1e-9);
        assert_close(
            after.project(pivot).unwrap().extend(0.0),
            pivot_pixel.extend(0.0),
            1e-6,
        );
        assert_ne!(after.forward(), before.forward());
    }

    #[test]
    fn orbiting_right_turns_the_model_counterclockwise_seen_from_above() {
        let mut camera =
            Camera::new(Viewpoint::looking_from(Vector3::NEG_Y, Point3::ZERO, 10.0).unwrap());
        let marker = Point3::new(0.0, -1.0, 0.0);
        let before = camera.view(WIDTH, HEIGHT).project(marker).unwrap();

        camera.orbit(Point3::ZERO, DVec2::new(30.0, 0.0), HEIGHT);
        let after = camera.view(WIDTH, HEIGHT).project(marker).unwrap();

        assert!(after.x > before.x);
    }

    #[test]
    fn zoom_keeps_the_anchor_under_the_cursor() {
        let mut camera = Camera::new(isometric());
        let before = camera.view(WIDTH, HEIGHT);
        let cursor = DVec2::new(610.0, 145.0);
        let anchor = before.ray_through(cursor).unwrap().at(95.0);

        camera.zoom(anchor, 0.4);
        let after = camera.view(WIDTH, HEIGHT);

        assert_close(
            after.project(anchor).unwrap().extend(0.0),
            cursor.extend(0.0),
            1e-6,
        );
        assert!((camera.viewpoint().distance - 48.0).abs() < 1e-9);
    }

    #[test]
    fn zoom_is_clamped_and_ignores_invalid_factors() {
        let mut camera = Camera::new(isometric());
        camera.zoom(Point3::ZERO, 0.0);
        camera.zoom(Point3::ZERO, f64::NAN);
        assert_eq!(camera.viewpoint(), isometric());

        camera.zoom(Point3::ZERO, 1e-30);
        assert_eq!(camera.viewpoint().distance, MIN_DISTANCE);
    }

    #[test]
    fn pan_moves_a_point_at_the_grab_depth_with_the_cursor() {
        let mut camera = Camera::new(isometric());
        let before = camera.view(WIDTH, HEIGHT);
        let grabbed = Point3::new(3.0, 1.0, 9.0);
        let pixel = before.project(grabbed).unwrap();
        let drag = DVec2::new(-35.0, 20.0);

        camera.pan(drag, before.units_per_pixel_at(before.view_depth(grabbed)));
        let after = camera.view(WIDTH, HEIGHT);

        assert_close(
            after.project(grabbed).unwrap().extend(0.0),
            (pixel + drag).extend(0.0),
            1e-6,
        );
    }

    #[test]
    fn fitted_view_contains_every_corner() {
        let view = View::new(isometric(), WIDTH, HEIGHT);
        let bounds =
            Aabb::from_points([Point3::new(-40.0, 10.0, 0.0), Point3::new(60.0, 30.0, 25.0)])
                .unwrap();
        let fitted = View::new(view.fitted(bounds), WIDTH, HEIGHT);

        for corner in bounds.corners() {
            let pixel = fitted.project(corner).unwrap();
            assert!((0.0..=WIDTH).contains(&pixel.x) && (0.0..=HEIGHT).contains(&pixel.y));
        }
        assert_close(fitted.viewpoint().target, bounds.center(), 1e-12);
    }

    #[test]
    fn transitions_ease_to_the_destination_and_navigation_interrupts_them() {
        let start = isometric();
        let end = Viewpoint::looking_from(Vector3::Z, Point3::new(100.0, 0.0, 0.0), 40.0).unwrap();
        let mut camera = Camera::new(start);

        camera.animate_to(end);
        assert_eq!(camera.destination(), end);
        camera.advance(TRANSITION_DURATION / 2);
        assert!(camera.is_animating());
        let halfway = camera.viewpoint();
        assert!(halfway.distance < start.distance && halfway.distance > end.distance);

        camera.advance(TRANSITION_DURATION);
        assert!(!camera.is_animating());
        assert_eq!(camera.viewpoint(), end);

        camera.animate_to(start);
        camera.advance(TRANSITION_DURATION / 4);
        camera.pan(DVec2::new(1.0, 0.0), 1.0);
        assert!(!camera.is_animating());
    }
}
