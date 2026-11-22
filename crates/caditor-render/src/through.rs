use ahash::AHashMap;
use caditor_geometry::{Aabb, Point3, Ray, larger};
use glam::DVec2;

use crate::{
    camera::View,
    culling::placed_corners,
    mesh::MeshInstance,
    picking::PICK_RADIUS_POINTS,
    scene::{Batch, Layer, PickHit, PickId, Scene, SectionPlane, is_cut_away, section_slack},
};

const PARALLEL: f64 = 1e-12;
const NEAR_MARGIN: f64 = 1e-9;
const CAP_TOLERANCE: f64 = 1e-6;
const BOX_SLACK: f64 = 1e-9;

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Work {
    lines: usize,
    triangles: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Found {
    hit: PickHit,
    depth: f64,
    rank: Rank,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
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
            Layer::Model | Layer::Front => Self::of(layer),
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
    section: &'a [SectionPlane],
    slack: f64,
    cap_depth: f64,
    bounded: bool,
    found: AHashMap<PickId, Found>,
    #[cfg(test)]
    work: Work,
}

impl Scene {
    pub fn hits_through(&self, view: &View, cursor: DVec2, pixels_per_point: f32) -> Vec<PickHit> {
        self.probed(view, cursor, pixels_per_point, true)
            .map(Probe::sorted)
            .unwrap_or_default()
    }

    fn probed<'a>(
        &'a self,
        view: &'a View,
        cursor: DVec2,
        pixels_per_point: f32,
        bounded: bool,
    ) -> Option<Probe<'a>> {
        let ray = view.ray_through(cursor)?;
        let pixels_per_point = f64::from(pixels_per_point);
        let pixels_per_point = if pixels_per_point.is_finite() && pixels_per_point > 0.0 {
            pixels_per_point
        } else {
            1.0
        };

        let mut probe = Probe {
            view,
            cursor,
            ray,
            pixels_per_point,
            section: &self.section,
            slack: section_slack(view.viewpoint().distance),
            cap_depth: f64::INFINITY,
            bounded,
            found: AHashMap::new(),
            #[cfg(test)]
            work: Work::default(),
        };

        for instance in self
            .meshes
            .iter()
            .chain(&self.flat_meshes)
            .chain(&self.reflective_meshes)
        {
            probe.cap(instance);
        }
        for instance in self
            .meshes
            .iter()
            .chain(&self.flat_meshes)
            .chain(&self.reflective_meshes)
            .chain(&self.translucent_meshes)
        {
            probe.mesh(instance);
        }

        let reachable = self.reachable_batches(&probe);

        for line in reachable.iter().flat_map(|batch| batch.all_lines()) {
            if let Some(id) = line.pick {
                probe.segment(id, [line.start, line.end], line.width, line.layer);
            }
        }
        for marker in reachable.iter().flat_map(|batch| &batch.markers) {
            if let Some(id) = marker.pick {
                probe.point(id, marker.position, marker.diameter, marker.layer);
            }
        }
        for fill in reachable.iter().flat_map(|batch| &batch.fills) {
            if let Some(id) = fill.pick {
                for triangle in &fill.triangles {
                    probe.triangle(
                        id,
                        *triangle,
                        (Rank::of_fill(fill.layer), fill.layer.is_sectioned()),
                    );
                }
            }
        }
        Some(probe)
    }

    fn reachable_batches(&self, probe: &Probe<'_>) -> Vec<&Batch> {
        if !probe.bounded {
            return self.batches.iter().map(|batch| &**batch).collect();
        }

        self.batch_bounds
            .of(&self.batches)
            .into_iter()
            .zip(&self.batches)
            .filter(|(bounds, _)| {
                bounds.as_ref().is_some_and(|bounds| {
                    !bounds.is_out_of_reach(probe.view, probe.cursor, probe.pixels_per_point)
                })
            })
            .map(|(_, batch)| &**batch)
            .collect()
    }
}

impl Probe<'_> {
    fn is_capped(&self, found: &Found) -> bool {
        match found.rank {
            Rank::Front => false,
            Rank::BehindEverything => self.cap_depth.is_finite(),
            Rank::Model => found.depth >= self.cap_depth * (1.0 - CAP_TOLERANCE),
        }
    }

    fn entry(&self) -> Option<f64> {
        let origin = self.eye_plane_origin();
        let direction = self.ray.direction();

        let toward = origin + direction;
        self.section
            .iter()
            .filter_map(|section| {
                let start = section.plane.signed_distance(origin) - self.slack;
                let approach =
                    section.plane.signed_distance(toward) - section.plane.signed_distance(origin);
                (approach < 0.0).then(|| start / -approach)
            })
            .filter(|entry| *entry > 0.0)
            .reduce(larger)
    }

    fn eye_plane_origin(&self) -> Point3 {
        if self.view.is_orthographic() {
            self.ray.origin() - self.view.forward() * self.view.near_plane()
        } else {
            self.ray.origin()
        }
    }

    fn cap(&mut self, instance: &MeshInstance) {
        if self.section.is_empty() || !instance.mesh.is_closed() {
            return;
        }
        if self.bounded && !self.crosses_placed(instance) {
            return;
        }

        let Some(entry) = self.entry() else {
            return;
        };
        let reach = (self.ray.origin() - self.eye_plane_origin()).length();
        let opaque = |face: usize| {
            instance
                .faces
                .get(face)
                .is_none_or(|style| style.color.alpha > 0.0)
        };
        for (face, corners, outward) in instance.mesh.oriented_triangles() {
            if !opaque(face) {
                continue;
            }

            let corners = match instance.placement {
                Some(placement) => corners.map(|corner| placement.apply_point(corner)),
                None => corners,
            };

            #[cfg(test)]
            {
                self.work.triangles += 1;
            }

            let Some(distance) = ray_meets_triangle(&self.ray, corners) else {
                continue;
            };
            let position = self.ray.at(distance);
            let Some(depth) = self.in_view(position) else {
                continue;
            };
            let [a, b, c] = corners;

            let flat = (b - a).cross(c - a) * outward;
            let back = flat.dot(self.ray.direction()) > 0.0;
            if back
                && distance + reach > entry
                && !self.is_cut_away(position, true)
                && depth < self.cap_depth
            {
                self.cap_depth = depth;
            }
        }
    }

    fn record(&mut self, id: PickId, found: Found) {
        if self.is_capped(&found) {
            return;
        }
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

        if !self.crosses_placed(instance) {
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

            self.triangle(id, corners, (Rank::Model, true));
        }
    }

    fn crosses_placed(&self, instance: &MeshInstance) -> bool {
        let Some(bounds) = instance.mesh.bounds() else {
            return false;
        };
        let placed = Aabb::from_points(placed_corners(bounds, instance.placement))
            .map(|placed| placed.expanded(BOX_SLACK * (1.0 + placed.diagonal())));

        placed.is_some_and(|placed| crosses_box(&self.ray, &placed))
    }

    fn is_cut_away(&self, position: Point3, layer_sectioned: bool) -> bool {
        layer_sectioned && is_cut_away(self.section, position, self.slack)
    }

    fn triangle(&mut self, id: PickId, corners: [Point3; 3], (rank, sectioned): (Rank, bool)) {
        #[cfg(test)]
        {
            self.work.triangles += 1;
        }

        let Some(distance) = ray_meets_triangle(&self.ray, corners) else {
            return;
        };
        let position = self.ray.at(distance);
        let Some(depth) = self.in_view(position) else {
            return;
        };

        if self.is_cut_away(position, sectioned) {
            return;
        }
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
        #[cfg(test)]
        {
            self.work.lines += 1;
        }

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

        if self.is_cut_away(position, layer.is_sectioned()) {
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

        if self.is_cut_away(position, layer.is_sectioned()) {
            return;
        }

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

    use caditor_geometry::{Plane, RigidTransform, Vector3};

    use super::*;
    use crate::{
        camera::{Projection, Viewpoint},
        mesh::{FaceStyle, MeshFace, MeshPoint, ShadedMesh},
        scene::{Batch, Color, CutFace, Fill, Line, Marker, Stroke},
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
        FaceStyle {
            color: GREY,
            pick,
            finish: None,
        }
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
                faces: vec![style(Some(id(1))), style(Some(id(2))), style(None)].into(),
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
                runs: Vec::new(),
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

    #[test]
    fn geometry_a_section_cuts_away_is_not_listed_and_reference_geometry_still_is() {
        let view = view();
        let centre = DVec2::new(200.0, 150.0);
        let mut scene = stacked();

        scene.section = vec![SectionPlane {
            plane: Plane::new(Point3::new(0.0, 0.0, 1.0), Vector3::Z).unwrap(),
            cut_face: CutFace::Hatched,
        }];

        let order: Vec<PickId> = scene
            .hits_through(&view, centre, 1.0)
            .iter()
            .map(|hit| hit.id)
            .collect();

        assert_eq!(order, vec![id(1), id(4), id(7)]);
    }

    fn cube(half: f64, centre: Point3) -> ShadedMesh {
        let face = |normal: Vector3, across: Vector3, up: Vector3| {
            let corner = |x: f64, y: f64| MeshPoint {
                position: centre + (normal + across * x + up * y) * half,
                normal,
            };
            MeshFace {
                points: vec![
                    corner(-1.0, -1.0),
                    corner(1.0, -1.0),
                    corner(1.0, 1.0),
                    corner(-1.0, 1.0),
                ],
                triangles: vec![[0, 1, 2], [0, 2, 3]],
            }
        };
        ShadedMesh::new([
            face(Vector3::Z, Vector3::X, Vector3::Y),
            face(-Vector3::Z, Vector3::Y, Vector3::X),
            face(Vector3::X, Vector3::Y, Vector3::Z),
            face(-Vector3::X, Vector3::Z, Vector3::Y),
            face(Vector3::Y, Vector3::Z, Vector3::X),
            face(-Vector3::Y, Vector3::X, Vector3::Z),
        ])
    }

    fn cube_styles(first: u32) -> Vec<FaceStyle> {
        (first..first + 6).map(|raw| style(Some(id(raw)))).collect()
    }

    fn halved_cubes(closed: bool) -> Scene {
        let near = Arc::new(cube(10.0, Point3::ZERO).with_closed(closed));
        let far = Arc::new(cube(10.0, Point3::new(0.0, 0.0, -40.0)).with_closed(closed));
        Scene {
            meshes: vec![
                MeshInstance {
                    mesh: near,
                    faces: cube_styles(1).into(),
                    placement: None,
                },
                MeshInstance {
                    mesh: far,
                    faces: cube_styles(11).into(),
                    placement: None,
                },
            ],
            section: vec![SectionPlane {
                plane: Plane::new(Point3::ZERO, Vector3::Z).unwrap(),
                cut_face: CutFace::Hatched,
            }],
            ..Scene::default()
        }
    }

    fn ids_at_centre(scene: &Scene) -> Vec<PickId> {
        scene
            .hits_through(&view(), DVec2::new(200.0, 150.0), 1.0)
            .iter()
            .map(|hit| hit.id)
            .collect()
    }

    #[test]
    fn a_section_cap_hides_the_far_wall_of_a_closed_body_and_what_lies_behind_it() {
        let scene = halved_cubes(true);

        assert_eq!(ids_at_centre(&scene), Vec::<PickId>::new());
    }

    #[test]
    fn a_section_cap_leaves_front_geometry_and_what_is_in_front_of_it() {
        let mut scene = halved_cubes(true);

        scene.batches = vec![Arc::new(Batch {
            lines: Vec::new(),
            markers: vec![Marker {
                position: Point3::new(0.0, 0.0, -2.0),
                color: GREY,
                diameter: 4.0,
                layer: Layer::Model,
                pick: Some(id(30)),
            }],
            fills: vec![Fill::convex(
                &[
                    Point3::new(-10.0, -10.0, -60.0),
                    Point3::new(10.0, -10.0, -60.0),
                    Point3::new(10.0, 10.0, -60.0),
                    Point3::new(-10.0, 10.0, -60.0),
                ],
                GREY,
                Layer::Reference,
                Some(id(31)),
            )],
            runs: Vec::new(),
        })];
        Arc::make_mut(&mut scene.batches[0]).markers.push(Marker {
            position: Point3::new(0.0, 0.0, -50.0),
            color: GREY,
            diameter: 4.0,
            layer: Layer::Front,
            pick: Some(id(32)),
        });

        assert_eq!(ids_at_centre(&scene), vec![id(32), id(30)]);
    }

    #[test]
    fn the_far_wall_of_an_open_mesh_under_a_section_is_still_listed() {
        let scene = halved_cubes(false);

        assert_eq!(ids_at_centre(&scene), vec![id(2), id(11), id(12)]);
    }

    #[test]
    fn nothing_is_a_cap_while_the_eye_is_on_the_kept_side() {
        let mut scene = halved_cubes(true);

        scene.section = vec![SectionPlane {
            plane: Plane::new(Point3::new(0.0, 0.0, 500.0), Vector3::Z).unwrap(),
            cut_face: CutFace::Hatched,
        }];

        assert_eq!(ids_at_centre(&scene), vec![id(1), id(2), id(11), id(12)]);
    }

    fn line_between(start: Point3, end: Point3, pick: u32) -> Line {
        Line {
            start,
            end,
            color: GREY,
            width: 2.0,
            layer: Layer::Model,
            pick: Some(id(pick)),
            stroke: Stroke::Solid,
        }
    }

    fn marker_at(position: Point3, layer: Layer, pick: u32) -> Marker {
        Marker {
            position,
            color: GREY,
            diameter: 6.0,
            layer,
            pick: Some(id(pick)),
        }
    }

    fn slab(centre: Point3, height: f64, layer: Layer, pick: u32) -> Fill {
        Fill::convex(
            &[
                centre + Vector3::new(-6.0, -6.0, height),
                centre + Vector3::new(6.0, -6.0, height),
                centre + Vector3::new(6.0, 6.0, height),
                centre + Vector3::new(-6.0, 6.0, height),
            ],
            GREY,
            layer,
            Some(id(pick)),
        )
    }

    fn cluster(centre: Point3, first: u32) -> Arc<Batch> {
        let spread = |x: f64, y: f64, z: f64| centre + Vector3::new(x, y, z);
        Arc::new(Batch {
            lines: vec![
                line_between(spread(-8.0, 0.0, 1.0), spread(8.0, 0.0, 1.0), first),
                line_between(spread(0.0, -8.0, -1.0), spread(0.0, 8.0, -1.0), first + 1),
                line_between(spread(-8.0, -8.0, 2.0), spread(8.0, 8.0, 2.0), first + 2),
            ],
            markers: vec![
                marker_at(spread(0.0, 0.0, 3.0), Layer::Model, first + 3),
                marker_at(spread(5.0, 5.0, -3.0), Layer::Front, first + 4),
            ],
            fills: vec![
                slab(centre, 0.0, Layer::Model, first + 5),
                slab(centre, -4.0, Layer::Reference, first + 6),
            ],
            runs: Vec::new(),
        })
    }

    fn placed_cube(centre: Point3, first: u32, placement: Option<RigidTransform>) -> MeshInstance {
        MeshInstance {
            mesh: Arc::new(cube(8.0, centre).with_closed(true)),
            faces: cube_styles(first).into(),
            placement,
        }
    }

    fn crowded() -> Scene {
        let moved = RigidTransform::translation(Vector3::new(30.0, -20.0, 0.0)).unwrap();
        let turned = RigidTransform::translation(Vector3::new(-60.0, 40.0, 10.0)).unwrap();
        Scene {
            meshes: vec![
                placed_cube(Point3::ZERO, 100, None),
                placed_cube(Point3::new(0.0, 0.0, -30.0), 110, Some(moved)),
            ],
            flat_meshes: vec![placed_cube(Point3::new(10.0, 10.0, 0.0), 120, Some(turned))],
            section: vec![SectionPlane {
                plane: Plane::new(Point3::new(0.0, 0.0, 4.0), Vector3::Z).unwrap(),
                cut_face: CutFace::Hatched,
            }],
            batches: vec![
                cluster(Point3::ZERO, 1),
                cluster(Point3::new(30.0, -20.0, 0.0), 11),
                cluster(Point3::new(-60.0, 40.0, 5.0), 21),
                cluster(Point3::new(500.0, 500.0, 0.0), 31),
                Arc::new(Batch::default()),
            ],
            ..Scene::default()
        }
    }

    fn views() -> Vec<View> {
        let looking =
            |direction: Vector3| Viewpoint::looking_from(direction, Point3::ZERO, 100.0).unwrap();
        vec![
            view(),
            View::new(looking(Vector3::new(1.0, 0.7, 0.5)), 400.0, 300.0),
            View::new(looking(Vector3::new(-0.3, 1.0, 0.8)), 400.0, 300.0)
                .with_projection(Projection::Orthographic),
            View::new(looking(Vector3::new(0.0, 0.0, -1.0)), 400.0, 300.0)
                .with_projection(Projection::Orthographic),
        ]
    }

    fn cursors(view: &View, scene: &Scene) -> Vec<DVec2> {
        let mut at: Vec<DVec2> = (0..=40)
            .flat_map(|column| {
                (0..=30).map(move |row| DVec2::new(f64::from(column) * 10.0, f64::from(row) * 10.0))
            })
            .collect();

        for batch in &scene.batches {
            for line in batch.lines.iter() {
                at.extend(view.project(line.start));
                at.extend(view.project(line.end));
            }
            for marker in &batch.markers {
                at.extend(view.project(marker.position));
            }
        }
        for instance in scene.meshes.iter().chain(&scene.flat_meshes) {
            let Some(bounds) = instance.mesh.bounds() else {
                continue;
            };

            at.extend(
                placed_corners(bounds, instance.placement)
                    .into_iter()
                    .filter_map(|corner| view.project(corner)),
            );
        }
        at
    }

    #[test]
    fn bounds_never_change_what_is_listed_or_its_order() {
        let scene = crowded();

        for view in views() {
            for cursor in cursors(&view, &scene) {
                let bounded = scene.probed(&view, cursor, 1.0, true).unwrap().sorted();
                let everything = scene.probed(&view, cursor, 1.0, false).unwrap().sorted();

                assert_eq!(bounded, everything, "{cursor:?}");
            }
        }
    }

    #[test]
    fn the_edge_of_the_widened_ray_lists_the_same_with_and_without_bounds() {
        let view = view();
        let scene = Scene {
            batches: vec![Arc::new(Batch {
                lines: vec![line_between(
                    Point3::new(-10.0, 0.0, 0.0),
                    Point3::new(10.0, 0.0, 0.0),
                    1,
                )],
                ..Batch::default()
            })],
            ..Scene::default()
        };
        let end = view.project(Point3::new(10.0, 0.0, 0.0)).unwrap();
        let mut listed = Vec::new();

        for step in 0..=160 {
            let beyond = f64::from(step) * 0.125;

            for cursor in [
                end + DVec2::new(beyond, 0.0),
                end + DVec2::new(0.0, beyond),
                end + DVec2::new(-beyond, -beyond),
            ] {
                let bounded = scene.probed(&view, cursor, 1.0, true).unwrap().sorted();
                let everything = scene.probed(&view, cursor, 1.0, false).unwrap().sorted();

                assert_eq!(bounded, everything, "{cursor:?}");

                listed.push(!bounded.is_empty());
            }
        }

        assert!(listed.iter().any(|listed| *listed));
        assert!(listed.iter().any(|listed| !*listed));
    }

    #[test]
    fn a_batch_the_widened_ray_misses_tests_none_of_its_lines_or_triangles() {
        let view = view();
        let scene = Scene {
            batches: vec![
                cluster(Point3::ZERO, 1),
                cluster(Point3::new(40.0, 0.0, 0.0), 11),
                cluster(Point3::new(0.0, 40.0, 0.0), 21),
            ],
            ..Scene::default()
        };
        let centre = DVec2::new(200.0, 150.0);

        let bounded = scene.probed(&view, centre, 1.0, true).unwrap();
        let everything = scene.probed(&view, centre, 1.0, false).unwrap();

        assert_eq!(
            bounded.work,
            Work {
                lines: 3,
                triangles: 4
            }
        );
        assert_eq!(
            everything.work,
            Work {
                lines: 9,
                triangles: 12
            }
        );
        assert_eq!(bounded.sorted(), everything.sorted());
    }

    #[test]
    fn a_batch_reaching_behind_the_eye_is_always_tested() {
        let view = view();
        let scene = Scene {
            batches: vec![Arc::new(Batch {
                lines: vec![line_between(
                    Point3::new(0.0, 0.0, 20.0),
                    Point3::new(0.0, 0.0, 1000.0),
                    1,
                )],
                ..Batch::default()
            })],
            ..Scene::default()
        };

        let bounded = scene
            .probed(&view, DVec2::new(10.0, 10.0), 1.0, true)
            .unwrap();

        assert_eq!(bounded.work.lines, 1);
    }

    #[test]
    fn a_closed_body_the_ray_misses_is_not_walked_for_a_cap_and_a_placed_one_is_found_where_it_is()
    {
        let view = view();
        let moved = RigidTransform::translation(Vector3::new(200.0, 0.0, 0.0)).unwrap();
        let mut scene = Scene {
            meshes: vec![
                placed_cube(Point3::ZERO, 1, None),
                placed_cube(Point3::ZERO, 11, Some(moved)),
            ],
            section: vec![SectionPlane {
                plane: Plane::new(Point3::new(0.0, 0.0, 4.0), Vector3::Z).unwrap(),
                cut_face: CutFace::Hatched,
            }],
            ..Scene::default()
        };
        let centre = DVec2::new(200.0, 150.0);

        let bounded = scene.probed(&view, centre, 1.0, true).unwrap();
        let everything = scene.probed(&view, centre, 1.0, false).unwrap();

        assert_eq!(everything.work.triangles - bounded.work.triangles, 12);
        assert_eq!(bounded.sorted(), everything.sorted());

        scene.meshes[1].placement = None;
        scene.meshes[0].placement = Some(moved);

        let swapped = scene.probed(&view, centre, 1.0, true).unwrap();
        let swapped_everything = scene.probed(&view, centre, 1.0, false).unwrap();

        assert_eq!(
            swapped_everything.work.triangles - swapped.work.triangles,
            12
        );
        assert_eq!(swapped.sorted(), swapped_everything.sorted());
    }

    #[test]
    fn a_batch_changed_after_it_was_listed_is_bounded_again() {
        let view = view();
        let mut scene = Scene {
            batches: vec![cluster(Point3::new(0.0, 0.0, 0.0), 1)],
            ..Scene::default()
        };
        let centre = DVec2::new(200.0, 150.0);

        let before = scene.hits_through(&view, centre, 1.0);

        Arc::make_mut(&mut scene.batches[0]).markers.clear();
        Arc::make_mut(&mut scene.batches[0]).fills.clear();
        Arc::make_mut(&mut scene.batches[0]).lines = vec![line_between(
            Point3::new(300.0, 300.0, 0.0),
            Point3::new(320.0, 300.0, 0.0),
            9,
        )];

        let after = scene.hits_through(&view, centre, 1.0);

        assert!(!before.is_empty());
        assert!(after.is_empty());
    }
}
