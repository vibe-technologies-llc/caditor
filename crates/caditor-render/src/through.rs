use ahash::AHashMap;
use caditor_geometry::{Aabb, Point3, Ray};
use glam::DVec2;

use crate::{
    camera::View,
    culling::placed_corners,
    mesh::MeshInstance,
    picking::PICK_RADIUS_POINTS,
    scene::{Layer, PickHit, PickId, Scene},
};

const PARALLEL: f64 = 1e-12;
const NEAR_MARGIN: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Found {
    hit: PickHit,
    depth: f64,
    rank: Rank,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Rank {
    Front,
    Model,
    BehindEverything,
}

impl Rank {
    fn of(layer: Layer) -> Self {
        if layer.draws_in_front() {
            Self::Front
        } else {
            Self::Model
        }
    }

    fn of_fill(layer: Layer) -> Self {
        match layer {
            Layer::Reference => Self::BehindEverything,
            Layer::Model | Layer::Hidden | Layer::Front => Self::of(layer),
        }
    }
}

impl Found {
    fn is_better_than(&self, other: &Self) -> bool {
        self.hit
            .offset_points
            .total_cmp(&other.hit.offset_points)
            .then(self.depth.total_cmp(&other.depth))
            .is_lt()
    }
}

struct Probe<'a> {
    view: &'a View,
    cursor: DVec2,
    ray: Ray,
    pixels_per_point: f64,
    found: AHashMap<PickId, Found>,
}

impl Scene {
    pub fn hits_through(&self, view: &View, cursor: DVec2, pixels_per_point: f32) -> Vec<PickHit> {
        let Some(ray) = view.ray_through(cursor) else {
            return Vec::new();
        };
        let pixels_per_point = f64::from(pixels_per_point);
        let mut probe = Probe {
            view,
            cursor,
            ray,
            pixels_per_point: if pixels_per_point.is_finite() && pixels_per_point > 0.0 {
                pixels_per_point
            } else {
                1.0
            },
            found: AHashMap::new(),
        };
        for instance in self
            .meshes
            .iter()
            .chain(&self.flat_meshes)
            .chain(&self.translucent_meshes)
        {
            probe.mesh(instance);
        }
        for line in self.lines().filter(|line| line.layer != Layer::Hidden) {
            if let Some(id) = line.pick {
                probe.segment(id, [line.start, line.end], line.width, line.layer);
            }
        }
        for marker in self
            .markers()
            .filter(|marker| marker.layer != Layer::Hidden)
        {
            if let Some(id) = marker.pick {
                probe.point(id, marker.position, marker.diameter, marker.layer);
            }
        }
        for fill in self.fills().filter(|fill| fill.layer != Layer::Hidden) {
            if let Some(id) = fill.pick {
                for triangle in &fill.triangles {
                    probe.triangle(id, *triangle, Rank::of_fill(fill.layer));
                }
            }
        }
        probe.sorted()
    }
}

impl Probe<'_> {
    fn record(&mut self, id: PickId, found: Found) {
        self.found
            .entry(id)
            .and_modify(|best| {
                if found.is_better_than(best) {
                    *best = found;
                }
            })
            .or_insert(found);
    }

    fn sorted(self) -> Vec<PickHit> {
        let mut found: Vec<Found> = self.found.into_values().collect();
        found.sort_by(|a, b| {
            a.rank
                .cmp(&b.rank)
                .then(a.depth.total_cmp(&b.depth))
                .then(a.hit.offset_points.total_cmp(&b.hit.offset_points))
                .then(a.hit.id.cmp(&b.hit.id))
        });
        found.into_iter().map(|found| found.hit).collect()
    }

    fn in_view(&self, point: Point3) -> Option<f64> {
        let depth = self.view.view_depth(point);
        (depth.is_finite() && depth > self.view.near_plane()).then_some(depth)
    }

    fn mesh(&mut self, instance: &MeshInstance) {
        let picks = |face: usize| instance.faces.get(face).and_then(|style| style.pick);
        if instance.faces.iter().all(|style| style.pick.is_none()) {
            return;
        }
        let Some(bounds) = instance.mesh.bounds() else {
            return;
        };
        let placed = Aabb::from_points(placed_corners(bounds, instance.placement));
        if !placed.is_some_and(|placed| crosses_box(&self.ray, &placed)) {
            return;
        }
        for (face, corners) in instance.mesh.face_triangles() {
            let Some(id) = picks(face) else {
                continue;
            };
            let corners = match instance.placement {
                Some(placement) => corners.map(|corner| placement.apply_point(corner)),
                None => corners,
            };
            self.triangle(id, corners, Rank::Model);
        }
    }

    fn triangle(&mut self, id: PickId, corners: [Point3; 3], rank: Rank) {
        let Some(distance) = ray_meets_triangle(&self.ray, corners) else {
            return;
        };
        let position = self.ray.at(distance);
        let Some(depth) = self.in_view(position) else {
            return;
        };
        self.record(
            id,
            Found {
                hit: PickHit {
                    id,
                    offset_points: 0.0,
                    position,
                },
                depth,
                rank,
            },
        );
    }

    fn segment(&mut self, id: PickId, ends: [Point3; 2], width: f32, layer: Layer) {
        let Some([start, end]) = self.clipped(ends) else {
            return;
        };
        let (Some(from), Some(to)) = (self.view.project(start), self.view.project(end)) else {
            return;
        };
        let along = to - from;
        let length_squared = along.length_squared();
        let on_screen = if length_squared > 0.0 {
            ((self.cursor - from).dot(along) / length_squared).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let distance = self.cursor.distance(from + along * on_screen);
        let offset = distance / self.pixels_per_point - f64::from(width) / 2.0;
        if offset > PICK_RADIUS_POINTS {
            return;
        }
        let in_space = if self.view.is_orthographic() {
            on_screen
        } else {
            let near = self.view.view_depth(start);
            let far = self.view.view_depth(end);
            let weight = on_screen / far;
            weight / ((1.0 - on_screen) / near + weight)
        };
        let position = start.lerp(end, in_space);
        let Some(depth) = self.in_view(position) else {
            return;
        };
        self.record(
            id,
            Found {
                hit: PickHit {
                    id,
                    offset_points: offset.max(0.0) as f32,
                    position,
                },
                depth,
                rank: Rank::of(layer),
            },
        );
    }

    fn clipped(&self, [start, end]: [Point3; 2]) -> Option<[Point3; 2]> {
        let near = self.view.near_plane() * (1.0 + NEAR_MARGIN) + NEAR_MARGIN;
        let (start_depth, end_depth) = (self.view.view_depth(start), self.view.view_depth(end));
        let inside = |depth: f64| depth >= near;
        let cut = || start.lerp(end, (near - start_depth) / (end_depth - start_depth));
        match (inside(start_depth), inside(end_depth)) {
            (true, true) => Some([start, end]),
            (false, false) => None,
            (false, true) => Some([cut(), end]),
            (true, false) => Some([start, cut()]),
        }
    }

    fn point(&mut self, id: PickId, position: Point3, diameter: f32, layer: Layer) {
        let Some(depth) = self.in_view(position) else {
            return;
        };
        let Some(at) = self.view.project(position) else {
            return;
        };
        let offset = self.cursor.distance(at) / self.pixels_per_point - f64::from(diameter) / 2.0;
        if offset > PICK_RADIUS_POINTS {
            return;
        }
        self.record(
            id,
            Found {
                hit: PickHit {
                    id,
                    offset_points: offset.max(0.0) as f32,
                    position,
                },
                depth,
                rank: Rank::of(layer),
            },
        );
    }
}

fn ray_meets_triangle(ray: &Ray, [a, b, c]: [Point3; 3]) -> Option<f64> {
    let (first, second) = (b - a, c - a);
    let direction = ray.direction();
    let across = direction.cross(second);
    let determinant = first.dot(across);
    if determinant.abs() <= PARALLEL * first.length() * second.length() {
        return None;
    }
    let inverse = 1.0 / determinant;
    let from_corner = ray.origin() - a;
    let u = from_corner.dot(across) * inverse;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let up = from_corner.cross(first);
    let v = direction.dot(up) * inverse;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let distance = second.dot(up) * inverse;
    (distance.is_finite() && distance >= 0.0).then_some(distance)
}

fn crosses_box(ray: &Ray, bounds: &Aabb) -> bool {
    let (low, high) = (bounds.min(), bounds.max());
    let origin = ray.origin();
    let direction = ray.direction();
    let mut entry = 0.0_f64;
    let mut exit = f64::INFINITY;
    for axis in 0..3 {
        let (start, toward, min, max) = (origin[axis], direction[axis], low[axis], high[axis]);
        if toward.abs() <= PARALLEL {
            if start < min || start > max {
                return false;
            }
            continue;
        }
        let (near, far) = ((min - start) / toward, (max - start) / toward);
        entry = entry.max(near.min(far));
        exit = exit.min(near.max(far));
        if entry > exit {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use caditor_geometry::{RigidTransform, Vector3};

    use super::*;
    use crate::{
        camera::Viewpoint,
        mesh::{FaceStyle, MeshFace, MeshPoint, ShadedMesh},
        scene::{Batch, Color, Fill, Line, Marker, Stroke},
    };

    const GREY: Color = Color::from_rgb8(128, 128, 128);

    fn view() -> View {
        let viewpoint = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 100.0).unwrap();
        View::new(viewpoint, 400.0, 300.0)
    }

    fn id(raw: u32) -> PickId {
        PickId::from_raw(raw).unwrap()
    }

    fn square(height: f64) -> MeshFace {
        let corner = |x: f64, y: f64| MeshPoint {
            position: Point3::new(x, y, height),
            normal: Vector3::Z,
        };
        MeshFace {
            points: vec![
                corner(-10.0, -10.0),
                corner(10.0, -10.0),
                corner(10.0, 10.0),
                corner(-10.0, 10.0),
            ],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
        }
    }

    fn style(pick: Option<PickId>) -> FaceStyle {
        FaceStyle { color: GREY, pick }
    }

    fn stacked() -> Scene {
        let mesh = Arc::new(ShadedMesh::new([square(0.0), square(5.0), square(-5.0)]));
        let line = Line {
            start: Point3::new(-20.0, 0.0, -2.0),
            end: Point3::new(20.0, 0.0, -2.0),
            color: GREY.with_alpha(0.0),
            width: 1.0,
            layer: Layer::Model,
            pick: Some(id(4)),
            stroke: Stroke::Solid,
        };
        let marker = Marker {
            position: Point3::new(0.0, 0.0, 8.0),
            color: GREY,
            diameter: 4.0,
            layer: Layer::Model,
            pick: Some(id(5)),
        };
        let far_line = Line {
            start: Point3::new(-20.0, 30.0, 0.0),
            end: Point3::new(20.0, 30.0, 0.0),
            pick: Some(id(6)),
            ..line.clone()
        };
        Scene {
            meshes: vec![MeshInstance {
                mesh,
                faces: vec![style(Some(id(1))), style(Some(id(2))), style(None)],
                placement: None,
            }],
            batches: vec![Arc::new(Batch {
                lines: vec![line, far_line],
                markers: vec![marker],
                fills: vec![Fill::convex(
                    &[
                        Point3::new(-10.0, -10.0, 20.0),
                        Point3::new(10.0, -10.0, 20.0),
                        Point3::new(10.0, 10.0, 20.0),
                        Point3::new(-10.0, 10.0, 20.0),
                    ],
                    GREY,
                    Layer::Reference,
                    Some(id(7)),
                )],
            })],
            ..Scene::default()
        }
    }

    #[test]
    fn every_pickable_under_the_cursor_is_listed_nearest_first_and_reference_fills_last() {
        let view = view();
        let centre = DVec2::new(200.0, 150.0);

        let hits = stacked().hits_through(&view, centre, 1.0);
        let order: Vec<PickId> = hits.iter().map(|hit| hit.id).collect();

        assert_eq!(order, vec![id(5), id(2), id(1), id(4), id(7)]);
        assert!(hits[1].position.distance(Point3::new(0.0, 0.0, 5.0)) < 1e-6);
        assert!(hits[3].position.distance(Point3::new(0.0, 0.0, -2.0)) < 1e-6);
        assert!(hits.iter().all(|hit| hit.offset_points <= 0.0));
    }

    #[test]
    fn beside_the_faces_only_nearby_lines_and_points_are_listed_with_their_distance() {
        let view = view();
        let on_line = view.project(Point3::new(15.0, 0.0, -2.0)).unwrap();
        let near = on_line + DVec2::new(0.0, 3.0);
        let far = on_line + DVec2::new(0.0, 10.0);

        let at_line = stacked().hits_through(&view, on_line, 1.0);
        let beside = stacked().hits_through(&view, near, 1.0);
        let away = stacked().hits_through(&view, far, 1.0);

        assert_eq!(at_line.len(), 1);
        assert_eq!(at_line[0].id, id(4));
        assert_eq!(at_line[0].offset_points, 0.0);
        assert!(at_line[0].position.distance(Point3::new(15.0, 0.0, -2.0)) < 1e-6);
        assert_eq!(beside.len(), 1);
        assert!((beside[0].offset_points - 2.5).abs() < 1e-3);
        assert!(away.is_empty());
    }

    #[test]
    fn a_placed_mesh_is_hit_where_it_is_drawn_and_front_geometry_comes_first() {
        let view = view();
        let centre = DVec2::new(200.0, 150.0);
        let moved = RigidTransform::translation(Vector3::new(40.0, 0.0, 0.0)).unwrap();
        let mut scene = stacked();
        scene.meshes[0].placement = Some(moved);
        Arc::make_mut(&mut scene.batches[0]).markers[0].layer = Layer::Front;
        Arc::make_mut(&mut scene.batches[0]).markers[0].position = Point3::new(0.0, 0.0, -50.0);
        let placed = view.project(Point3::new(40.0, 0.0, 0.0)).unwrap();

        let at_centre: Vec<PickId> = scene
            .hits_through(&view, centre, 1.0)
            .iter()
            .map(|hit| hit.id)
            .collect();
        let at_placed: Vec<PickId> = scene
            .hits_through(&view, placed, 1.0)
            .iter()
            .map(|hit| hit.id)
            .collect();

        assert_eq!(at_centre, vec![id(5), id(4), id(7)]);
        assert_eq!(at_placed, vec![id(2), id(1)]);
    }
}
