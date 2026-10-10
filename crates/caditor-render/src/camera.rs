use std::{
    f64::consts::{FRAC_PI_2, PI},
    time::Duration,
};

use caditor_geometry::{Aabb, Plane, Point3, Ray, Rotation3, Vector3};
use glam::{DMat3, DMat4, DVec2, DVec3, DVec4, dcamera::rh::proj::directx};

const FIELD_OF_VIEW_Y: f64 = 30.0 * PI / 180.0;
const NEAR_PLANE_FRACTION: f64 = 1e-3;
const ORBIT_RADIANS_PER_VIEWPORT_HEIGHT: f64 = PI;
const MIN_DISTANCE: f64 = 1e-4;
const MAX_DISTANCE: f64 = 1e8;
const LEVELLING_RATE: f64 = 2.0;
const VERTICAL_TOLERANCE: f64 = 1e-9;
const FIT_MARGIN: f64 = 1.35;
const MIN_FIT_DEPTH_FRACTION: f64 = 0.1;
const TRANSITION_DURATION: Duration = Duration::from_millis(350);
const GLIDE_DURATION: Duration = Duration::from_millis(150);
const ORTHOGRAPHIC_REACH_PER_DISTANCE: f64 = 40.0;
const ORTHOGRAPHIC_SCENE_MARGIN: f64 = 1.1;
const SQUARE_TOLERANCE_DEGREES: f64 = 0.1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Projection {
    #[default]
    Perspective,
    Orthographic,
}

impl Projection {
    pub const ALL: [Self; 2] = [Self::Perspective, Self::Orthographic];
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ProjectionMode {
    #[default]
    Perspective,
    Orthographic,
    Automatic,
}

impl ProjectionMode {
    pub const ALL: [Self; 3] = [Self::Perspective, Self::Orthographic, Self::Automatic];

    pub fn other(self) -> Self {
        match self {
            Self::Perspective => Self::Orthographic,
            Self::Orthographic | Self::Automatic => Self::Perspective,
        }
    }

    pub fn automatic_toggled(self) -> Self {
        match self {
            Self::Automatic => Self::Perspective,
            Self::Perspective | Self::Orthographic => Self::Automatic,
        }
    }

    pub fn resolved(self, viewpoint: &Viewpoint) -> Projection {
        match self {
            Self::Perspective => Projection::Perspective,
            Self::Orthographic => Projection::Orthographic,
            Self::Automatic if viewpoint.is_square_to_an_axis() => Projection::Orthographic,
            Self::Automatic => Projection::Perspective,
        }
    }
}

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

    pub fn orbited(self, pivot: Point3, drag: DVec2, viewport_height: f64) -> Option<Self> {
        if !pivot.is_finite() || !drag.is_finite() || !viewport_height.is_finite() {
            return None;
        }
        let radians_per_pixel = ORBIT_RADIANS_PER_VIEWPORT_HEIGHT / viewport_height.max(1.0);
        let forward = self.forward();
        let levelling_limit = drag.length() * radians_per_pixel * LEVELLING_RATE;
        let level = Rotation3::from_axis_angle(
            forward,
            roll(&self).clamp(-levelling_limit, levelling_limit),
        );

        let elevation = forward.z.atan2(forward.truncate().length());
        let tilt =
            (-drag.y * radians_per_pixel).clamp(-FRAC_PI_2 - elevation, FRAC_PI_2 - elevation);
        let tilt_axis = level_right(forward).unwrap_or_else(|| level * self.right());
        let pitch = Rotation3::from_axis_angle(tilt_axis, tilt);
        let yaw = Rotation3::from_rotation_z(-drag.x * radians_per_pixel);

        Some(self.rotated_about(pivot, yaw * pitch * level))
    }

    pub fn panned(self, drag: DVec2, units_per_pixel: f64) -> Option<Self> {
        if !drag.is_finite() || !units_per_pixel.is_finite() {
            return None;
        }
        let offset = (self.up() * drag.y - self.right() * drag.x) * units_per_pixel;
        Some(self.translated(offset))
    }

    pub fn zoomed(self, anchor: Point3, factor: f64) -> Option<Self> {
        if !factor.is_finite() || factor <= 0.0 || !anchor.is_finite() {
            return None;
        }
        Some(self.scaled_about(anchor, factor))
    }

    pub fn is_square_to_an_axis(&self) -> bool {
        let forward = self.forward();
        let squareness = forward.abs().max_element();
        squareness >= SQUARE_TOLERANCE_DEGREES.to_radians().cos()
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
    projection: Projection,
    scene_reach: f64,
}

impl View {
    pub fn new(viewpoint: Viewpoint, width: f64, height: f64) -> Self {
        Self {
            viewpoint,
            size: DVec2::new(width.max(1.0), height.max(1.0)),
            projection: Projection::Perspective,
            scene_reach: 0.0,
        }
    }

    #[must_use]
    pub fn with_projection(self, projection: Projection) -> Self {
        Self { projection, ..self }
    }

    #[must_use]
    pub fn reaching(self, scene: Aabb) -> Self {
        let target = self.viewpoint.target;
        let reach = scene
            .corners()
            .into_iter()
            .map(|corner| corner.distance(target))
            .fold(self.scene_reach, f64::max);
        Self {
            scene_reach: if reach.is_finite() {
                reach
            } else {
                self.scene_reach
            },
            ..self
        }
    }

    pub fn projection(&self) -> Projection {
        self.projection
    }

    pub fn is_orthographic(&self) -> bool {
        self.projection == Projection::Orthographic
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
        match self.projection {
            Projection::Perspective => self.viewpoint.distance * NEAR_PLANE_FRACTION,
            Projection::Orthographic => self.viewpoint.distance - self.orthographic_reach(),
        }
    }

    pub fn far_plane(&self) -> f64 {
        match self.projection {
            Projection::Perspective => f64::INFINITY,
            Projection::Orthographic => self.viewpoint.distance + self.orthographic_reach(),
        }
    }

    fn orthographic_reach(&self) -> f64 {
        (self.scene_reach * ORTHOGRAPHIC_SCENE_MARGIN)
            .max(self.viewpoint.distance * ORTHOGRAPHIC_REACH_PER_DISTANCE)
    }

    fn half_height_at(&self, depth: f64) -> f64 {
        match self.projection {
            Projection::Perspective => depth * tan_half_fov_y(),
            Projection::Orthographic => self.viewpoint.distance * tan_half_fov_y(),
        }
    }

    fn ndc_of(&self, pixel: DVec2) -> DVec2 {
        DVec2::new(
            pixel.x / self.size.x * 2.0 - 1.0,
            1.0 - pixel.y / self.size.y * 2.0,
        )
    }

    fn orthographic_offset(&self, pixel: DVec2) -> Vector3 {
        let ndc = self.ndc_of(pixel);
        let half_height = self.half_height_at(self.viewpoint.distance);
        self.viewpoint.orientation
            * DVec3::new(
                ndc.x * half_height * self.aspect(),
                ndc.y * half_height,
                0.0,
            )
    }

    pub fn aspect(&self) -> f64 {
        self.size.x / self.size.y
    }

    pub fn view_depth(&self, point: Point3) -> f64 {
        (point - self.eye()).dot(self.forward())
    }

    pub fn units_per_pixel_at(&self, depth: f64) -> f64 {
        2.0 * self.half_height_at(depth) / self.size.y
    }

    pub fn ray_through(&self, pixel: DVec2) -> Option<Ray> {
        match self.projection {
            Projection::Perspective => {
                let ndc = self.ndc_of(pixel);
                let direction = DVec3::new(
                    ndc.x * tan_half_fov_y() * self.aspect(),
                    ndc.y * tan_half_fov_y(),
                    -1.0,
                );
                Ray::new(self.eye(), self.viewpoint.orientation * direction)
            }
            Projection::Orthographic => Ray::new(
                self.eye() + self.orthographic_offset(pixel) + self.forward() * self.near_plane(),
                self.forward(),
            ),
        }
    }

    pub fn unproject(&self, pixel: DVec2, depth: f64) -> Option<Point3> {
        if !depth.is_finite() || !pixel.is_finite() {
            return None;
        }
        let point = match self.projection {
            Projection::Perspective => {
                let ray = self.ray_through(pixel)?;
                let facing = ray.direction().dot(self.forward());
                (facing > 0.0).then(|| ray.at(depth / facing))?
            }
            Projection::Orthographic => {
                self.eye() + self.orthographic_offset(pixel) + self.forward() * depth
            }
        };
        point.is_finite().then_some(point)
    }

    pub fn project(&self, point: Point3) -> Option<DVec2> {
        let local = self.viewpoint.orientation.inverse() * (point - self.eye());
        let depth = -local.z;
        if depth <= self.near_plane() {
            return None;
        }
        let half_height = self.half_height_at(depth);
        let ndc = DVec2::new(
            local.x / (half_height * self.aspect()),
            local.y / half_height,
        );
        Some(DVec2::new(
            (ndc.x + 1.0) * 0.5 * self.size.x,
            (1.0 - ndc.y) * 0.5 * self.size.y,
        ))
    }

    pub fn plane_projection(&self, plane: &Plane, pixels_per_unit: f64) -> PlaneProjection {
        let inverse = self.viewpoint.orientation.inverse();
        let scale = self.size * 0.5 / pixels_per_unit;
        let half_height = tan_half_fov_y();
        PlaneProjection {
            origin: inverse * (plane.origin() - self.eye()),
            x_axis: inverse * plane.x_axis(),
            y_axis: inverse * plane.y_axis(),
            near: self.near_plane(),
            spread: match self.projection {
                Projection::Perspective => Spread::Perspective(half_height),
                Projection::Orthographic => {
                    Spread::Orthographic(self.viewpoint.distance * half_height)
                }
            },
            aspect: self.aspect(),
            scale,
        }
    }

    pub fn focal_point_under(&self, pixel: DVec2) -> Option<Point3> {
        self.unproject(pixel, self.viewpoint.distance)
    }

    pub fn rotation_projection(&self) -> DMat4 {
        let projection = match self.projection {
            Projection::Perspective => directx::perspective_infinite_reverse(
                FIELD_OF_VIEW_Y,
                self.aspect(),
                self.near_plane(),
            ),
            Projection::Orthographic => {
                let half_height = self.half_height_at(self.viewpoint.distance);
                let near = self.near_plane();
                let far = self.far_plane();
                let depth = far - near;
                DMat4::from_cols(
                    DVec4::new(1.0 / (half_height * self.aspect()), 0.0, 0.0, 0.0),
                    DVec4::new(0.0, 1.0 / half_height, 0.0, 0.0),
                    DVec4::new(0.0, 0.0, 1.0 / depth, 0.0),
                    DVec4::new(0.0, 0.0, far / depth, 1.0),
                )
            }
        };
        projection * DMat4::from_quat(self.viewpoint.orientation.inverse())
    }

    pub fn fitted(&self, bounds: Aabb) -> Viewpoint {
        let radius = bounds.bounding_radius();
        let distance = if radius > 0.0 {
            self.fitting_distance(bounds, radius)
        } else {
            self.viewpoint.distance
        };
        Viewpoint {
            target: bounds.center(),
            orientation: self.viewpoint.orientation,
            distance: distance.clamp(MIN_DISTANCE, MAX_DISTANCE),
        }
    }

    fn fitting_distance(&self, bounds: Aabb, radius: f64) -> f64 {
        let to_view = self.viewpoint.orientation.inverse();
        let tan_y = tan_half_fov_y();
        let tan_x = tan_y * self.aspect();
        let center = bounds.center();
        let corners = bounds.corners().map(|corner| to_view * (corner - center));

        let lateral = corners.iter().fold(0.0_f64, |distance, corner| {
            let needed = match self.projection {
                Projection::Perspective => {
                    (corner.x.abs() / tan_x).max(corner.y.abs() / tan_y) * FIT_MARGIN + corner.z
                }
                Projection::Orthographic => {
                    (corner.x.abs() / tan_x).max(corner.y.abs() / tan_y) * FIT_MARGIN
                }
            };
            distance.max(needed)
        });
        let in_front = corners
            .iter()
            .map(|corner| corner.z)
            .fold(0.0_f64, f64::max)
            + radius * MIN_FIT_DEPTH_FRACTION;

        match self.projection {
            Projection::Perspective => lateral.max(in_front),
            Projection::Orthographic => lateral,
        }
    }
}

fn roll(viewpoint: &Viewpoint) -> f64 {
    let forward = viewpoint.forward();
    let up = viewpoint.up();
    let Some(level_up) = level_right(forward).map(|right| right.cross(forward)) else {
        return 0.0;
    };
    forward.dot(up.cross(level_up)).atan2(up.dot(level_up))
}

fn level_right(forward: Vector3) -> Option<Vector3> {
    let right = forward.cross(Vector3::Z);
    (right.length() > VERTICAL_TOLERANCE).then(|| right.normalize())
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Spread {
    Perspective(f64),
    Orthographic(f64),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaneProjection {
    origin: DVec3,
    x_axis: DVec3,
    y_axis: DVec3,
    near: f64,
    spread: Spread,
    aspect: f64,
    scale: DVec2,
}

impl PlaneProjection {
    pub fn project(&self, point: DVec2) -> Option<DVec2> {
        let local = self.origin + self.x_axis * point.x + self.y_axis * point.y;
        let depth = -local.z;
        if depth <= self.near {
            return None;
        }
        let half_height = match self.spread {
            Spread::Perspective(per_depth) => depth * per_depth,
            Spread::Orthographic(fixed) => fixed,
        };
        Some(DVec2::new(
            (local.x / (half_height * self.aspect) + 1.0) * self.scale.x,
            (1.0 - local.y / half_height) * self.scale.y,
        ))
    }
}

fn tan_half_fov_y() -> f64 {
    (FIELD_OF_VIEW_Y * 0.5).tan()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Easing {
    InAndOut,
    Out,
}

impl Easing {
    fn duration(self) -> Duration {
        match self {
            Self::InAndOut => TRANSITION_DURATION,
            Self::Out => GLIDE_DURATION,
        }
    }

    fn eased(self, progress: f64) -> f64 {
        match self {
            Self::InAndOut => progress * progress * (3.0 - 2.0 * progress),
            Self::Out => 1.0 - (1.0 - progress) * (1.0 - progress),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Transition {
    from: Viewpoint,
    to: Viewpoint,
    elapsed: Duration,
    easing: Easing,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Camera {
    viewpoint: Viewpoint,
    transition: Option<Transition>,
    projection: ProjectionMode,
}

impl Camera {
    pub fn new(viewpoint: Viewpoint) -> Self {
        Self {
            viewpoint,
            transition: None,
            projection: ProjectionMode::Perspective,
        }
    }

    pub fn projection(&self) -> ProjectionMode {
        self.projection
    }

    pub fn set_projection(&mut self, projection: ProjectionMode) {
        self.projection = projection;
    }

    pub fn viewpoint(&self) -> Viewpoint {
        self.viewpoint
    }

    pub fn destination(&self) -> Viewpoint {
        self.transition
            .map_or(self.viewpoint, |transition| transition.to)
    }

    pub fn view(&self, width: f64, height: f64) -> View {
        self.view_from(self.viewpoint, width, height)
    }

    pub fn destination_view(&self, width: f64, height: f64) -> View {
        self.view_from(self.destination(), width, height)
    }

    pub fn view_from(&self, viewpoint: Viewpoint, width: f64, height: f64) -> View {
        View::new(viewpoint, width, height).with_projection(self.projection.resolved(&viewpoint))
    }

    pub fn is_animating(&self) -> bool {
        self.transition.is_some()
    }

    pub fn animate_to(&mut self, to: Viewpoint) {
        self.start_transition(to, Easing::InAndOut);
    }

    pub fn glide_to(&mut self, to: Viewpoint) {
        self.start_transition(to, Easing::Out);
    }

    fn start_transition(&mut self, to: Viewpoint, easing: Easing) {
        self.transition = Some(Transition {
            from: self.viewpoint,
            to,
            elapsed: Duration::ZERO,
            easing,
        });
    }

    pub fn advance(&mut self, elapsed: Duration) {
        let Some(transition) = self.transition.as_mut() else {
            return;
        };
        transition.elapsed += elapsed;
        let progress =
            transition.elapsed.as_secs_f64() / transition.easing.duration().as_secs_f64();
        if progress >= 1.0 {
            self.viewpoint = transition.to;
            self.transition = None;
        } else {
            let eased = transition.easing.eased(progress);
            self.viewpoint = transition.from.interpolated(transition.to, eased);
        }
    }

    pub fn orbit(&mut self, pivot: Point3, drag: DVec2, viewport_height: f64) {
        self.move_to(self.viewpoint.orbited(pivot, drag, viewport_height));
    }

    pub fn pan(&mut self, drag: DVec2, units_per_pixel: f64) {
        self.move_to(self.viewpoint.panned(drag, units_per_pixel));
    }

    pub fn zoom(&mut self, anchor: Point3, factor: f64) {
        self.move_to(self.viewpoint.zoomed(anchor, factor));
    }

    fn move_to(&mut self, moved: Option<Viewpoint>) {
        if let Some(moved) = moved {
            self.transition = None;
            self.viewpoint = moved;
        }
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
    fn a_plane_projection_puts_plane_points_where_the_view_projects_them() {
        let plane = Plane::new(
            Point3::new(3.0, -2.0, 4.0),
            Vector3::new(0.2, 0.3, 1.0).normalize(),
        )
        .unwrap();
        let points = [DVec2::ZERO, DVec2::new(14.0, -3.0), DVec2::new(-40.0, 25.0)];
        for projection in Projection::ALL {
            let view = View::new(isometric(), WIDTH, HEIGHT).with_projection(projection);
            let projected = view.plane_projection(&plane, 2.0);

            for point in points {
                let expected = view.project(plane.to_world(point)).unwrap() / 2.0;
                let actual = projected.project(point).unwrap();
                assert!(
                    actual.distance(expected) < 1e-9,
                    "{actual} is not {expected}"
                );
            }
        }

        let behind = View::new(isometric(), WIDTH, HEIGHT).plane_projection(&Plane::XY, 1.0);

        assert_eq!(behind.project(DVec2::new(1e6, -1e6)), None);
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
    fn orbit_stops_at_the_poles_and_keeps_horizontal_drag_the_same_way_round() {
        for vertical in [-5000.0, 5000.0] {
            let mut camera = Camera::new(isometric());

            camera.orbit(Point3::ZERO, DVec2::new(0.0, vertical), HEIGHT);
            let viewpoint = camera.viewpoint();
            assert!((viewpoint.forward().z.abs() - 1.0).abs() < 1e-9);
            assert!(viewpoint.up().z.abs() < 1e-9);

            camera.orbit(Point3::ZERO, DVec2::new(0.0, vertical), HEIGHT);
            assert_close(camera.viewpoint().forward(), viewpoint.forward(), 1e-9);

            camera.orbit(
                Point3::ZERO,
                DVec2::new(0.0, -vertical.signum() * 60.0),
                HEIGHT,
            );
            let viewpoint = camera.viewpoint();
            assert!(viewpoint.forward().z.abs() < 0.999);
            assert!(viewpoint.up().z > 0.0);

            let marker = viewpoint.target + viewpoint.right() * -1.0;
            let before = camera.view(WIDTH, HEIGHT).project(marker).unwrap();
            camera.orbit(viewpoint.target, DVec2::new(30.0, 0.0), HEIGHT);
            let after = camera.view(WIDTH, HEIGHT).project(marker).unwrap();
            assert!(after.x > before.x);
        }
    }

    #[test]
    fn orbiting_levels_a_rolled_view_while_keeping_the_pivot_on_screen() {
        let plane = Plane::with_x_axis(
            Point3::new(4.0, -2.0, 7.0),
            Vector3::new(1.0, -1.0, 0.5),
            Vector3::new(1.0, 1.0, 1.0),
        )
        .unwrap();
        let mut camera = Camera::new(Viewpoint::facing(&plane, plane.origin(), 50.0));
        let pivot = plane.origin() + plane.x_axis() * 3.0;
        assert!(camera.viewpoint().right().z.abs() > 0.1);

        for _ in 0..40 {
            let before = camera.view(WIDTH, HEIGHT).project(pivot).unwrap();
            camera.orbit(pivot, DVec2::new(8.0, 0.0), HEIGHT);
            let after = camera.view(WIDTH, HEIGHT).project(pivot).unwrap();
            assert_close(after.extend(0.0), before.extend(0.0), 1e-6);
        }

        let viewpoint = camera.viewpoint();
        assert!(viewpoint.right().z.abs() < 1e-9);
        assert!(viewpoint.up().z > 0.0);
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
    fn a_wide_flat_part_fills_the_view_in_both_projections() {
        let plate = Aabb::from_points([
            Point3::new(-100.0, -75.0, 0.0),
            Point3::new(100.0, 75.0, 2.0),
        ])
        .unwrap();
        let from_above = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 50.0).unwrap();

        for projection in Projection::ALL {
            let view = View::new(from_above, WIDTH, HEIGHT).with_projection(projection);
            let fitted = View::new(view.fitted(plate), WIDTH, HEIGHT).with_projection(projection);

            let pixels = plate
                .corners()
                .map(|corner| fitted.project(corner).unwrap());
            let widest = pixels
                .iter()
                .map(|pixel| (pixel.x - WIDTH / 2.0).abs())
                .fold(0.0, f64::max);
            let tallest = pixels
                .iter()
                .map(|pixel| (pixel.y - HEIGHT / 2.0).abs())
                .fold(0.0, f64::max);

            assert!(widest <= WIDTH / 2.0 && tallest <= HEIGHT / 2.0);
            assert!(widest.max(tallest * WIDTH / HEIGHT) > WIDTH / 2.0 / (FIT_MARGIN * 1.05));
        }
    }

    #[test]
    fn a_tiny_part_fills_the_view_and_a_single_point_keeps_the_distance() {
        let view = View::new(isometric(), WIDTH, HEIGHT);
        let tiny = Aabb::from_points([Point3::new(1.0, 1.0, 1.0), Point3::new(1.01, 1.02, 1.005)])
            .unwrap();
        let fitted = View::new(view.fitted(tiny), WIDTH, HEIGHT);

        let pixels = tiny.corners().map(|corner| fitted.project(corner).unwrap());
        let spread = pixels
            .iter()
            .map(|pixel| pixel.y)
            .fold(f64::NEG_INFINITY, f64::max)
            - pixels
                .iter()
                .map(|pixel| pixel.y)
                .fold(f64::INFINITY, f64::min);
        assert!(spread > HEIGHT / 4.0, "{spread}");

        let point = view.fitted(Aabb::from_point(Point3::new(3.0, 0.0, 0.0)));
        assert_eq!(point.distance, view.viewpoint().distance);
        assert_eq!(point.target, Point3::new(3.0, 0.0, 0.0));
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

    #[test]
    fn a_glide_moves_at_once_and_settles_sooner_than_a_transition() {
        let start = isometric();
        let end = start.panned(DVec2::new(100.0, 0.0), 1.0).unwrap();
        let mut glide = Camera::new(start);
        let mut transition = Camera::new(start);

        glide.glide_to(end);
        transition.animate_to(end);
        glide.advance(Duration::from_millis(30));
        transition.advance(Duration::from_millis(30));
        let glided = glide.viewpoint().target.distance(start.target);
        let eased = transition.viewpoint().target.distance(start.target);

        assert!(glided > 3.0 * eased, "{glided} {eased}");
        assert_eq!(glide.destination(), end);
        glide.advance(GLIDE_DURATION);
        assert!(!glide.is_animating());
        assert_eq!(glide.viewpoint(), end);
        assert!(transition.is_animating());
    }

    #[test]
    fn moving_a_viewpoint_matches_moving_the_camera() {
        let pivot = Point3::new(8.0, 0.0, 4.0);
        let mut camera = Camera::new(isometric());

        camera.orbit(pivot, DVec2::new(40.0, -12.0), HEIGHT);
        let orbited = isometric().orbited(pivot, DVec2::new(40.0, -12.0), HEIGHT);
        assert_eq!(Some(camera.viewpoint()), orbited);

        camera.pan(DVec2::new(-5.0, 3.0), 0.2);
        let panned = orbited.and_then(|viewpoint| viewpoint.panned(DVec2::new(-5.0, 3.0), 0.2));
        assert_eq!(Some(camera.viewpoint()), panned);

        camera.zoom(pivot, 0.5);
        let zoomed = panned.and_then(|viewpoint| viewpoint.zoomed(pivot, 0.5));
        assert_eq!(Some(camera.viewpoint()), zoomed);

        assert!(isometric().zoomed(pivot, f64::NAN).is_none());
        assert!(isometric().panned(DVec2::NAN, 1.0).is_none());
    }

    #[test]
    fn the_destination_view_resolves_the_automatic_projection_where_the_camera_is_heading() {
        let front = Viewpoint::looking_from(Vector3::NEG_Y, Point3::ZERO, 80.0).unwrap();
        let mut camera = Camera::new(isometric());
        camera.set_projection(ProjectionMode::Automatic);

        camera.animate_to(front);
        camera.advance(TRANSITION_DURATION / 4);

        assert!(!camera.view(WIDTH, HEIGHT).is_orthographic());
        let heading = camera.destination_view(WIDTH, HEIGHT);
        assert!(heading.is_orthographic());
        assert_eq!(*heading.viewpoint(), front);
    }

    fn orthographic(viewpoint: Viewpoint) -> View {
        View::new(viewpoint, WIDTH, HEIGHT).with_projection(Projection::Orthographic)
    }

    fn orthographic_camera() -> Camera {
        let mut camera = Camera::new(isometric());
        camera.set_projection(ProjectionMode::Orthographic);
        camera
    }

    fn clip_to_pixel(clip: DVec4) -> DVec3 {
        let ndc = clip.truncate() / clip.w;
        DVec3::new(
            (ndc.x + 1.0) * 0.5 * WIDTH,
            (1.0 - ndc.y) * 0.5 * HEIGHT,
            ndc.z,
        )
    }

    #[test]
    fn orthographic_rays_are_parallel_and_agree_with_projection_and_unprojection() {
        let view = orthographic(isometric());
        let point = Point3::new(12.0, -7.0, 3.0);
        let behind_the_eye = view.eye() - view.forward() * 30.0 + view.viewpoint().up() * 4.0;

        let pixel = view.project(point).unwrap();
        let ray = view.ray_through(pixel).unwrap();
        let along = (point - ray.origin()).dot(ray.direction());
        let corner_ray = view.ray_through(DVec2::ZERO).unwrap();
        let behind = view.project(behind_the_eye).unwrap();

        assert_close(ray.direction(), view.forward(), 1e-12);
        assert_close(corner_ray.direction(), view.forward(), 1e-12);
        assert_close(ray.at(along), point, 1e-9);
        assert!(ray.origin().distance(point) > view.viewpoint().distance);
        assert_close(
            view.unproject(pixel, view.view_depth(point)).unwrap(),
            point,
            1e-9,
        );
        assert_close(
            view.unproject(behind, view.view_depth(behind_the_eye))
                .unwrap(),
            behind_the_eye,
            1e-9,
        );
        assert_close(
            view.project(view.viewpoint().target).unwrap().extend(0.0),
            DVec3::new(WIDTH / 2.0, HEIGHT / 2.0, 0.0),
            1e-9,
        );
    }

    #[test]
    fn unprojecting_a_depth_finds_the_point_in_both_projections() {
        for view in [
            View::new(isometric(), WIDTH, HEIGHT),
            orthographic(isometric()),
        ] {
            let point = Point3::new(-3.0, 8.0, 14.0);
            let pixel = view.project(point).unwrap();

            let found = view.unproject(pixel, view.view_depth(point)).unwrap();

            assert_close(found, point, 1e-9);
        }
    }

    #[test]
    fn an_orthographic_view_draws_things_the_same_size_at_every_depth() {
        let view = orthographic(isometric());
        let near = view.viewpoint().target - view.forward() * 50.0;
        let far = view.viewpoint().target + view.forward() * 500.0;
        let offset = view.viewpoint().right() * 10.0;

        let near_width = view.project(near + offset).unwrap().x - view.project(near).unwrap().x;
        let far_width = view.project(far + offset).unwrap().x - view.project(far).unwrap().x;
        let perspective = View::new(isometric(), WIDTH, HEIGHT);
        let target = perspective.viewpoint().target;
        let target_width = perspective.project(target + offset).unwrap().x
            - perspective.project(target).unwrap().x;

        assert!((near_width - far_width).abs() < 1e-9);
        assert!((near_width - target_width).abs() < 1e-9);
        assert!(
            (view.units_per_pixel_at(1.0) - perspective.units_per_pixel_at(120.0)).abs() < 1e-12
        );
    }

    #[test]
    fn the_orthographic_matrix_matches_projection_and_maps_its_depth_range_in_reverse() {
        let bounds = Aabb::from_points([
            Point3::new(-4000.0, -10.0, 0.0),
            Point3::new(10.0, 10.0, 5.0),
        ])
        .unwrap();
        let view = orthographic(isometric()).reaching(bounds);
        let matrix = view.rotation_projection();
        let clip = |point: Point3| matrix * (point - view.eye()).extend(1.0);
        let point = Point3::new(12.0, -7.0, 3.0);
        let at_near = view.eye() + view.forward() * view.near_plane();
        let at_far = view.eye() + view.forward() * view.far_plane();

        let pixel = clip_to_pixel(clip(point));

        assert_close(
            pixel.truncate().extend(0.0),
            view.project(point).unwrap().extend(0.0),
            1e-9,
        );
        assert!(pixel.z > 0.0 && pixel.z < 1.0);
        assert!((clip(at_near).z - 1.0).abs() < 1e-9);
        assert!(clip(at_far).z.abs() < 1e-9);
        assert!(view.near_plane() < 0.0);
        for corner in bounds.corners() {
            let depth = view.view_depth(corner);
            assert!(depth > view.near_plane() && depth < view.far_plane());
        }
    }

    #[test]
    fn orthographic_navigation_keeps_the_anchor_pivot_and_grab_under_the_cursor() {
        let mut camera = orthographic_camera();
        let cursor = DVec2::new(610.0, 145.0);
        let anchor = camera.view(WIDTH, HEIGHT).unproject(cursor, 95.0).unwrap();

        camera.zoom(anchor, 0.4);
        let zoomed = camera.view(WIDTH, HEIGHT);
        let pivot = Point3::new(8.0, 0.0, 4.0);
        let pivot_pixel = zoomed.project(pivot).unwrap();
        camera.orbit(pivot, DVec2::new(140.0, -60.0), HEIGHT);
        let orbited = camera.view(WIDTH, HEIGHT);
        let grabbed = Point3::new(3.0, 1.0, 9.0);
        let grab_pixel = orbited.project(grabbed).unwrap();
        let drag = DVec2::new(-35.0, 20.0);
        camera.pan(
            drag,
            orbited.units_per_pixel_at(orbited.view_depth(grabbed)),
        );
        let panned = camera.view(WIDTH, HEIGHT);

        assert!(zoomed.is_orthographic());
        assert_close(
            zoomed.project(anchor).unwrap().extend(0.0),
            cursor.extend(0.0),
            1e-6,
        );
        assert_close(
            orbited.project(pivot).unwrap().extend(0.0),
            pivot_pixel.extend(0.0),
            1e-6,
        );
        assert_close(
            panned.project(grabbed).unwrap().extend(0.0),
            (grab_pixel + drag).extend(0.0),
            1e-6,
        );
    }

    #[test]
    fn a_non_finite_depth_unprojects_to_nothing_in_both_projections() {
        let pixel = DVec2::new(400.0, 300.0);

        for view in [
            View::new(isometric(), WIDTH, HEIGHT),
            orthographic(isometric()),
        ] {
            assert!(view.unproject(pixel, f64::NAN).is_none());
            assert!(view.unproject(pixel, f64::INFINITY).is_none());
            assert!(view.unproject(DVec2::NAN, 10.0).is_none());
            assert!(view.unproject(pixel, 10.0).is_some());
        }
    }

    #[test]
    fn a_non_finite_anchor_or_pivot_leaves_the_camera_where_it_was() {
        let mut camera = orthographic_camera();
        let before = camera.viewpoint();
        let poisoned = Point3::new(f64::NAN, 0.0, 1.0);

        camera.zoom(poisoned, 0.5);
        camera.orbit(poisoned, DVec2::new(40.0, 10.0), HEIGHT);
        camera.orbit(Point3::ZERO, DVec2::new(f64::NAN, 10.0), HEIGHT);
        camera.pan(DVec2::new(5.0, 5.0), f64::NAN);

        assert_eq!(camera.viewpoint(), before);
    }

    #[test]
    fn an_orthographic_fit_contains_every_corner_and_fills_more_of_the_view() {
        let bounds =
            Aabb::from_points([Point3::new(-40.0, 10.0, 0.0), Point3::new(60.0, 30.0, 25.0)])
                .unwrap();
        let view = orthographic(isometric());
        let fitted = orthographic(view.fitted(bounds));
        let perspective = View::new(isometric(), WIDTH, HEIGHT).fitted(bounds);

        for corner in bounds.corners() {
            let pixel = fitted.project(corner).unwrap();
            assert!((0.0..=WIDTH).contains(&pixel.x) && (0.0..=HEIGHT).contains(&pixel.y));
        }
        assert!(fitted.viewpoint().distance < perspective.distance);
        assert_eq!(
            ProjectionMode::Perspective.other(),
            ProjectionMode::Orthographic
        );
        assert_eq!(
            ProjectionMode::Orthographic.other(),
            ProjectionMode::Perspective
        );
        assert_eq!(
            ProjectionMode::Automatic.other(),
            ProjectionMode::Perspective
        );
        assert_eq!(
            ProjectionMode::Perspective.automatic_toggled(),
            ProjectionMode::Automatic
        );
        assert_eq!(
            ProjectionMode::Automatic.automatic_toggled(),
            ProjectionMode::Perspective
        );
    }

    #[test]
    fn the_automatic_projection_is_orthographic_only_while_square_to_an_axis() {
        let square = Viewpoint::looking_from(Vector3::NEG_Y, Point3::ZERO, 80.0).unwrap();
        let top = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 80.0).unwrap();
        let nudged =
            Viewpoint::looking_from(Vector3::new(0.0, -1.0, 0.05), Point3::ZERO, 80.0).unwrap();
        let mut camera = Camera::new(isometric());
        camera.set_projection(ProjectionMode::Automatic);

        assert!(!camera.view(WIDTH, HEIGHT).is_orthographic());
        camera.animate_to(square);
        camera.advance(TRANSITION_DURATION);
        assert!(camera.view(WIDTH, HEIGHT).is_orthographic());
        camera.animate_to(top);
        camera.advance(TRANSITION_DURATION);
        assert!(camera.view(WIDTH, HEIGHT).is_orthographic());
        camera.animate_to(nudged);
        camera.advance(TRANSITION_DURATION);
        assert!(!camera.view(WIDTH, HEIGHT).is_orthographic());
        assert_eq!(camera.projection(), ProjectionMode::Automatic);
    }
}
