use std::collections::BTreeMap;

use caditor_document::FeatureId;
use caditor_geometry::{Point3, Vector2};
use caditor_kernel::Mesh;

use crate::{
    bodies::{BodyMesh, face_keys},
    selection::{Pickable, SelectionFilter},
    sketch_drag::{BoxMode, ScreenArea},
};

const SMALLEST_BOX: f64 = 3.0;
const MAX_CELLS: f64 = 512.0;
const SLACK_CELLS: f64 = 4.0;
const SAMPLE_POINTS: f64 = 4.0;
const MAX_SEGMENT_SAMPLES: f64 = 64.0;

pub struct Seen {
    pub at: Vector2,
    pub depth: f64,
    pub units_per_point: f64,
}

pub struct Occlusion {
    origin: Vector2,
    cell: f64,
    columns: usize,
    rows: usize,
    depths: Vec<f64>,
}

impl Occlusion {
    pub fn open() -> Self {
        Self {
            origin: Vector2::ZERO,
            cell: 1.0,
            columns: 0,
            rows: 0,
            depths: Vec::new(),
        }
    }

    pub fn of<'a>(
        meshes: impl IntoIterator<Item = &'a Mesh>,
        seen: &impl Fn(Point3) -> Option<Seen>,
        area: &ScreenArea,
    ) -> Self {
        let (low, high) = area.bounds();
        let longest = (high - low).max_element().max(1.0);
        let cell = (longest / MAX_CELLS).max(1.0);
        let origin = low - Vector2::splat(cell);
        let columns = ((high.x - origin.x) / cell).ceil() as usize + 2;
        let rows = ((high.y - origin.y) / cell).ceil() as usize + 2;
        let mut occlusion = Self {
            origin,
            cell,
            columns,
            rows,
            depths: vec![f64::INFINITY; columns * rows],
        };
        for mesh in meshes {
            let projected: Vec<Option<(Vector2, f64)>> = mesh
                .positions()
                .iter()
                .map(|position| seen(*position).map(|seen| (seen.at, seen.depth)))
                .collect();
            let corner = |vertex: u32| {
                let position = mesh.vertices().get(vertex as usize)?.position;
                projected.get(position as usize).copied().flatten()
            };
            for triangle in mesh.triangles() {
                if let [Some(a), Some(b), Some(c)] = triangle.map(corner) {
                    occlusion.fill([a, b, c]);
                }
            }
        }
        occlusion
    }

    fn fill(&mut self, corners: [(Vector2, f64); 3]) {
        let [(a, depth_a), (b, depth_b), (c, depth_c)] = corners;
        let area = (b - a).perp_dot(c - a);
        if area.abs() <= f64::EPSILON {
            return;
        }
        let low = a.min(b).min(c);
        let high = a.max(b).max(c);
        let first_column = ((low.x - self.origin.x) / self.cell).floor().max(0.0) as usize;
        let first_row = ((low.y - self.origin.y) / self.cell).floor().max(0.0) as usize;
        let last_column = ((high.x - self.origin.x) / self.cell).ceil().max(0.0) as usize;
        let last_row = ((high.y - self.origin.y) / self.cell).ceil().max(0.0) as usize;
        for row in first_row..last_row.min(self.rows) {
            for column in first_column..last_column.min(self.columns) {
                let centre =
                    self.origin + Vector2::new(column as f64 + 0.5, row as f64 + 0.5) * self.cell;
                let weight_a = (b - centre).perp_dot(c - centre) / area;
                let weight_b = (c - centre).perp_dot(a - centre) / area;
                let weight_c = 1.0 - weight_a - weight_b;
                if weight_a < 0.0 || weight_b < 0.0 || weight_c < 0.0 {
                    continue;
                }
                let depth = weight_a * depth_a + weight_b * depth_b + weight_c * depth_c;
                if let Some(nearest) = self.depths.get_mut(row * self.columns + column) {
                    *nearest = nearest.min(depth);
                }
            }
        }
    }

    pub fn shows(&self, seen: &Seen) -> bool {
        let local = (seen.at - self.origin) / self.cell;
        if local.x < 0.0 || local.y < 0.0 {
            return true;
        }
        let (column, row) = (local.x as usize, local.y as usize);
        if column >= self.columns || row >= self.rows {
            return true;
        }
        let nearest = self
            .depths
            .get(row * self.columns + column)
            .copied()
            .unwrap_or(f64::INFINITY);
        let slack = SLACK_CELLS * self.cell * seen.units_per_point;
        seen.depth <= nearest + slack
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Catch {
    Faces,
    Edges,
    Vertices,
    SketchGeometry,
}

impl Catch {
    pub fn of(filter: SelectionFilter) -> Self {
        match filter {
            SelectionFilter::Everything | SelectionFilter::Bodies | SelectionFilter::Faces => {
                Self::Faces
            }
            SelectionFilter::Edges => Self::Edges,
            SelectionFilter::Vertices => Self::Vertices,
            SelectionFilter::SketchGeometry => Self::SketchGeometry,
        }
    }
}

pub fn is_a_box(area: &ScreenArea) -> bool {
    let (low, high) = area.bounds();
    let size = high - low;
    size.x >= SMALLEST_BOX || size.y >= SMALLEST_BOX
}

pub struct Looking<'a, S> {
    pub seen: &'a S,
    pub occlusion: &'a Occlusion,
}

impl<S: Fn(Point3) -> Option<Seen>> Looking<'_, S> {
    fn visible(&self, point: Point3) -> Option<Vector2> {
        let seen = (self.seen)(point)?;
        self.occlusion.shows(&seen).then_some(seen.at)
    }
}

pub fn within_body<S: Fn(Point3) -> Option<Seen>>(
    body: FeatureId,
    mesh: &BodyMesh,
    looking: &Looking<'_, S>,
    area: &ScreenArea,
    catch: Catch,
) -> Vec<Pickable> {
    match catch {
        Catch::Faces => faces_within(body, mesh, looking, area),
        Catch::Edges => mesh
            .edges
            .iter()
            .filter(|edge| polyline_caught(&edge.points, looking, area))
            .map(|edge| Pickable::Edge {
                body,
                edge: edge.name,
            })
            .collect(),
        Catch::Vertices => mesh
            .vertices
            .iter()
            .filter(|vertex| {
                looking
                    .visible(vertex.position)
                    .is_some_and(|point| area.contains(point))
            })
            .map(|vertex| Pickable::Vertex {
                body,
                vertex: vertex.key,
            })
            .collect(),
        Catch::SketchGeometry => Vec::new(),
    }
}

fn samples<S: Fn(Point3) -> Option<Seen>>(
    points: &[Point3],
    looking: &Looking<'_, S>,
) -> Vec<Point3> {
    let screen = |point: Point3| (looking.seen)(point).map(|seen| seen.at);
    let mut sampled = Vec::with_capacity(points.len());
    for pair in points.windows(2) {
        let [from, to] = [pair.first(), pair.get(1)].map(|point| point.copied());
        let (Some(from), Some(to)) = (from, to) else {
            continue;
        };
        let span = screen(from)
            .zip(screen(to))
            .map_or(1.0, |(start, end)| start.distance(end));
        let steps = (span / SAMPLE_POINTS)
            .ceil()
            .clamp(1.0, MAX_SEGMENT_SAMPLES) as usize;
        sampled.extend((0..steps).map(|step| from.lerp(to, step as f64 / steps as f64)));
    }
    sampled.extend(points.last().copied());
    sampled
}

fn polyline_caught<S: Fn(Point3) -> Option<Seen>>(
    points: &[Point3],
    looking: &Looking<'_, S>,
    area: &ScreenArea,
) -> bool {
    let projected: Vec<Option<Vector2>> = samples(points, looking)
        .into_iter()
        .map(|point| looking.visible(point))
        .collect();
    let visible = projected.iter().flatten().count();
    if visible * 2 < projected.len() {
        return false;
    }
    match area.mode() {
        BoxMode::Window => {
            projected.iter().any(Option::is_some)
                && projected
                    .iter()
                    .flatten()
                    .all(|point| area.contains(*point))
        }
        BoxMode::Crossing => {
            projected.windows(2).any(|pair| match pair {
                [Some(from), Some(to)] => area.crosses(*from, *to),
                _ => false,
            }) || projected
                .iter()
                .flatten()
                .any(|point| area.contains(*point))
        }
    }
}

fn faces_within<S: Fn(Point3) -> Option<Seen>>(
    body: FeatureId,
    mesh: &BodyMesh,
    looking: &Looking<'_, S>,
    area: &ScreenArea,
) -> Vec<Pickable> {
    let screen = |point: Point3| (looking.seen)(point).map(|seen| seen.at);
    let Some(solid) = mesh.source().solid() else {
        return Vec::new();
    };
    let Some(triangulated) = solid.mesh() else {
        return Vec::new();
    };
    let keys: BTreeMap<_, _> = face_keys(&solid.solid).into_iter().collect();
    let projected: Vec<Option<Vector2>> = triangulated
        .positions()
        .iter()
        .map(|position| screen(*position))
        .collect();
    let corner = |vertex: u32| {
        let position = triangulated.vertices().get(vertex as usize)?.position;
        projected.get(position as usize).copied().flatten()
    };
    let world = |vertex: u32| {
        let position = triangulated.vertices().get(vertex as usize)?.position;
        triangulated.positions().get(position as usize).copied()
    };
    let centre = area.centre();
    let mode = area.mode();
    triangulated
        .faces()
        .iter()
        .filter(|face| {
            let triangles = triangulated
                .triangles()
                .get(face.triangles.clone())
                .unwrap_or_default();
            if triangles.is_empty() {
                return false;
            }
            let facing_in_space: Vec<([Vector2; 3], [Point3; 3])> = triangles
                .iter()
                .filter_map(
                    |triangle| match (triangle.map(corner), triangle.map(world)) {
                        ([Some(a), Some(b), Some(c)], [Some(x), Some(y), Some(z)]) => {
                            Some(([a, b, c], [x, y, z]))
                        }
                        _ => None,
                    },
                )
                .filter(|([a, b, c], _)| (*b - *a).perp_dot(*c - *a) < 0.0)
                .collect();
            if !facing_in_space
                .iter()
                .any(|(_, corners)| triangle_is_seen(looking, *corners))
            {
                return false;
            }
            let facing: Vec<[Vector2; 3]> =
                facing_in_space.iter().map(|(screen, _)| *screen).collect();
            match mode {
                BoxMode::Window => {
                    !facing.is_empty() && facing.iter().flatten().all(|point| area.contains(*point))
                }
                BoxMode::Crossing => facing.iter().any(|&[a, b, c]| {
                    area.crosses(a, b)
                        || area.crosses(b, c)
                        || area.crosses(c, a)
                        || inside_triangle(centre, [a, b, c])
                }),
            }
        })
        .filter_map(|face| keys.get(&face.face).copied())
        .map(|key| Pickable::Face { body, face: key })
        .collect()
}

fn triangle_is_seen<S: Fn(Point3) -> Option<Seen>>(
    looking: &Looking<'_, S>,
    [a, b, c]: [Point3; 3],
) -> bool {
    let middle = (a + b + c) / 3.0;
    [a, b, c, middle]
        .into_iter()
        .any(|point| looking.visible(point).is_some())
}

fn inside_triangle(point: Vector2, [a, b, c]: [Vector2; 3]) -> bool {
    let side = |p: Vector2, q: Vector2| (q - p).perp_dot(point - p);
    let (first, second, third) = (side(a, b), side(b, c), side(c, a));
    (first >= 0.0 && second >= 0.0 && third >= 0.0)
        || (first <= 0.0 && second <= 0.0 && third <= 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(x: f64, y: f64, depth: f64) -> Seen {
        Seen {
            at: Vector2::new(x, y),
            depth,
            units_per_point: 0.01,
        }
    }

    #[test]
    fn a_triangle_in_front_hides_what_lies_behind_it_but_not_what_lies_on_it() {
        let area = ScreenArea::Box(crate::sketch_drag::ScreenBox {
            from: Vector2::ZERO,
            to: Vector2::new(100.0, 100.0),
        });
        let mut occlusion = Occlusion::of(std::iter::empty(), &|_| None, &area);
        occlusion.fill([
            (Vector2::new(10.0, 10.0), 5.0),
            (Vector2::new(90.0, 10.0), 5.0),
            (Vector2::new(10.0, 90.0), 5.0),
        ]);

        assert!(!occlusion.shows(&seen(20.0, 20.0, 50.0)));
        assert!(occlusion.shows(&seen(20.0, 20.0, 5.0)));
        assert!(occlusion.shows(&seen(80.0, 80.0, 50.0)));
        assert!(occlusion.shows(&seen(150.0, 20.0, 50.0)));
    }

    #[test]
    fn a_triangle_is_seen_unless_a_nearer_one_covers_all_its_samples_and_select_through_opens_it() {
        let area = ScreenArea::Box(crate::sketch_drag::ScreenBox {
            from: Vector2::ZERO,
            to: Vector2::new(100.0, 100.0),
        });
        let project = |point: Point3| {
            Some(Seen {
                at: Vector2::new(point.x, point.y),
                depth: point.z,
                units_per_point: 0.01,
            })
        };
        let mut covered = Occlusion::of(std::iter::empty(), &|_| None, &area);
        covered.fill([
            (Vector2::new(0.0, 0.0), 5.0),
            (Vector2::new(100.0, 0.0), 5.0),
            (Vector2::new(0.0, 100.0), 5.0),
        ]);
        covered.fill([
            (Vector2::new(100.0, 0.0), 5.0),
            (Vector2::new(100.0, 100.0), 5.0),
            (Vector2::new(0.0, 100.0), 5.0),
        ]);
        let behind = [
            Point3::new(20.0, 20.0, 50.0),
            Point3::new(60.0, 20.0, 50.0),
            Point3::new(20.0, 60.0, 50.0),
        ];

        let hidden = Looking {
            seen: &project,
            occlusion: &covered,
        };
        let open = Occlusion::open();
        let through = Looking {
            seen: &project,
            occlusion: &open,
        };

        assert!(!triangle_is_seen(&hidden, behind));
        assert!(triangle_is_seen(&through, behind));
    }
}
