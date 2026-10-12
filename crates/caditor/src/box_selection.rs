use std::collections::BTreeMap;

use caditor_document::FeatureId;
use caditor_geometry::{Aabb, Point3, Vector2};
use caditor_kernel::Mesh;

use crate::{
    bodies::{BodyMesh, EdgePoints, face_keys},
    selection::{Pickable, SelectionFilter},
    sketch_drag::{BoxMode, ScreenArea},
};

const SMALLEST_BOX: f64 = 3.0;
const MAX_CELLS: f64 = 512.0;
const SLACK_CELLS: f64 = 4.0;
const SAMPLE_POINTS: f64 = 4.0;
const MAX_SEGMENT_SAMPLES: f64 = 64.0;
const BOUNDS_SLACK: f64 = 8.0;

pub struct Seen {
    pub at: Vector2,
    pub depth: f64,
    pub units_per_point: f64,
}

#[derive(Debug, Clone, Copy)]
struct Grid {
    origin: Vector2,
    cell: f64,
    columns: usize,
    rows: usize,
}

impl Grid {
    fn cells_inside(self, [a, b, c]: [Vector2; 3]) -> impl Iterator<Item = (usize, [f64; 3])> {
        let area = (b - a).perp_dot(c - a);
        let flat = area.abs() <= f64::EPSILON;
        let low = a.min(b).min(c);
        let high = a.max(b).max(c);
        let first_column = ((low.x - self.origin.x) / self.cell).floor().max(0.0) as usize;
        let first_row = ((low.y - self.origin.y) / self.cell).floor().max(0.0) as usize;
        let last_column = ((high.x - self.origin.x) / self.cell)
            .ceil()
            .max(0.0)
            .min(self.columns as f64) as usize;
        let last_row = ((high.y - self.origin.y) / self.cell)
            .ceil()
            .max(0.0)
            .min(self.rows as f64) as usize;
        let rows = if flat { 0..0 } else { first_row..last_row };
        rows.flat_map(move |row| {
            (first_column..last_column).filter_map(move |column| {
                let centre =
                    self.origin + Vector2::new(column as f64 + 0.5, row as f64 + 0.5) * self.cell;
                let weight_a = (b - centre).perp_dot(c - centre) / area;
                let weight_b = (c - centre).perp_dot(a - centre) / area;
                let weight_c = 1.0 - weight_a - weight_b;
                let inside = weight_a >= 0.0 && weight_b >= 0.0 && weight_c >= 0.0;
                inside.then_some((row * self.columns + column, [weight_a, weight_b, weight_c]))
            })
        })
    }

    fn index_of(self, at: Vector2) -> Option<usize> {
        let local = (at - self.origin) / self.cell;
        if local.x < 0.0 || local.y < 0.0 {
            return None;
        }
        let (column, row) = (local.x as usize, local.y as usize);
        (column < self.columns && row < self.rows).then_some(row * self.columns + column)
    }
}

fn depth_at(depths: [f64; 3], weights: [f64; 3]) -> f64 {
    depths
        .iter()
        .zip(weights)
        .map(|(depth, weight)| depth * weight)
        .sum()
}

pub struct Occlusion {
    grid: Grid,
    depths: Vec<f64>,
}

impl Occlusion {
    pub fn open() -> Self {
        Self {
            grid: Grid {
                origin: Vector2::ZERO,
                cell: 1.0,
                columns: 0,
                rows: 0,
            },
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
            grid: Grid {
                origin,
                cell,
                columns,
                rows,
            },
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
        let depths = corners.map(|(_, depth)| depth);
        for (index, weights) in self.grid.cells_inside(corners.map(|(at, _)| at)) {
            if let Some(nearest) = self.depths.get_mut(index) {
                *nearest = nearest.min(depth_at(depths, weights));
            }
        }
    }

    fn judges(&self, at: Vector2) -> bool {
        self.grid.index_of(at).is_some()
    }

    pub fn is_open(&self) -> bool {
        self.depths.is_empty()
    }

    fn slack(&self, units_per_point: f64) -> f64 {
        SLACK_CELLS * self.grid.cell * units_per_point
    }

    pub fn shows(&self, seen: &Seen) -> bool {
        let nearest = self
            .grid
            .index_of(seen.at)
            .and_then(|index| self.depths.get(index))
            .copied()
            .unwrap_or(f64::INFINITY);
        seen.depth <= nearest + self.slack(seen.units_per_point)
    }

    fn sees_triangle(&self, corners: &[Seen; 3]) -> Option<bool> {
        let depths = corners.each_ref().map(|seen| seen.depth);
        let units_per_point = corners
            .iter()
            .map(|seen| seen.units_per_point)
            .fold(0.0, f64::max);
        let slack = self.slack(units_per_point);
        let mut covered = false;
        for (index, weights) in self
            .grid
            .cells_inside(corners.each_ref().map(|seen| seen.at))
        {
            covered = true;
            let nearest = self.depths.get(index).copied().unwrap_or(f64::INFINITY);
            if depth_at(depths, weights) <= nearest + slack {
                return Some(true);
            }
        }
        covered.then_some(false)
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
    pub fn of(filter: SelectionFilter, faces_pick: bool) -> Self {
        match filter {
            SelectionFilter::Everything if !faces_pick => Self::Edges,
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

pub fn may_reach(
    bounds: Option<Aabb>,
    screen: &impl Fn(Point3) -> Option<Vector2>,
    area: &ScreenArea,
) -> bool {
    let Some(bounds) = bounds else {
        return true;
    };
    let Some(corners) = bounds
        .corners()
        .into_iter()
        .map(screen)
        .collect::<Option<Vec<Vector2>>>()
    else {
        return true;
    };
    let low = corners
        .iter()
        .fold(Vector2::splat(f64::INFINITY), |low, corner| {
            low.min(*corner)
        });
    let high = corners
        .iter()
        .fold(Vector2::splat(f64::NEG_INFINITY), |high, corner| {
            high.max(*corner)
        });
    let (area_low, area_high) = area.bounds();
    let slack = Vector2::splat(BOUNDS_SLACK);
    low.cmple(area_high + slack).all() && high.cmpge(area_low - slack).all()
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
            .filter(|edge| polyline_caught(mesh.points(edge), looking, area))
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
    points: EdgePoints<'_>,
    looking: &Looking<'_, S>,
) -> Vec<Point3> {
    let screen = |point: Point3| (looking.seen)(point).map(|seen| seen.at);
    let last = points.clone().last();
    let mut sampled = Vec::new();
    for (from, to) in points.segments() {
        let span = screen(from)
            .zip(screen(to))
            .map_or(1.0, |(start, end)| start.distance(end));
        let steps = (span / SAMPLE_POINTS)
            .ceil()
            .clamp(1.0, MAX_SEGMENT_SAMPLES) as usize;
        sampled.extend((0..steps).map(|step| from.lerp(to, step as f64 / steps as f64)));
    }
    sampled.extend(last);
    sampled
}

fn polyline_caught<S: Fn(Point3) -> Option<Seen>>(
    points: EdgePoints<'_>,
    looking: &Looking<'_, S>,
    area: &ScreenArea,
) -> bool {
    let judged: Vec<Option<(Vector2, bool, bool)>> = samples(points, looking)
        .into_iter()
        .map(|point| {
            let seen = (looking.seen)(point)?;
            let within = looking.occlusion.judges(seen.at);
            Some((seen.at, looking.occlusion.shows(&seen), within))
        })
        .collect();
    let any_within = judged.iter().flatten().any(|(_, _, within)| *within);
    let counted = |sample: &Option<(Vector2, bool, bool)>| {
        !any_within || sample.is_some_and(|(_, _, within)| within)
    };
    let counted_samples = judged.iter().filter(|sample| counted(sample)).count();
    let visible = judged
        .iter()
        .filter(|sample| counted(sample) && sample.is_some_and(|(_, shown, _)| shown))
        .count();
    if visible * 2 < counted_samples {
        return false;
    }
    let projected: Vec<Option<Vector2>> = judged
        .iter()
        .map(|sample| sample.and_then(|(at, shown, _)| shown.then_some(at)))
        .collect();
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
    let every_side = looking.occlusion.is_open();
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
                .filter(|([a, b, c], _)| every_side || (*b - *a).perp_dot(*c - *a) < 0.0)
                .collect();
            let seen = |corners: &[Point3; 3]| triangle_is_seen(looking, *corners);
            match mode {
                BoxMode::Window => {
                    !facing_in_space.is_empty()
                        && facing_in_space
                            .iter()
                            .flat_map(|(screen, _)| screen)
                            .all(|point| area.contains(*point))
                        && facing_in_space.iter().any(|(_, corners)| seen(corners))
                }
                BoxMode::Crossing => facing_in_space.iter().any(|([a, b, c], corners)| {
                    (area.crosses(*a, *b)
                        || area.crosses(*b, *c)
                        || area.crosses(*c, *a)
                        || inside_triangle(centre, [*a, *b, *c]))
                        && seen(corners)
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
    let projected = [a, b, c].map(looking.seen);
    if let [Some(first), Some(second), Some(third)] = projected
        && let Some(seen) = looking.occlusion.sees_triangle(&[first, second, third])
    {
        return seen;
    }
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

    #[test]
    fn a_large_triangle_hidden_inside_the_box_is_not_seen_through_its_corners_outside_it() {
        let area = ScreenArea::Box(crate::sketch_drag::ScreenBox {
            from: Vector2::new(40.0, 40.0),
            to: Vector2::new(60.0, 60.0),
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
            (Vector2::new(-100.0, -100.0), 5.0),
            (Vector2::new(300.0, -100.0), 5.0),
            (Vector2::new(-100.0, 300.0), 5.0),
        ]);
        let behind = [
            Point3::new(0.0, 0.0, 50.0),
            Point3::new(150.0, 0.0, 50.0),
            Point3::new(0.0, 150.0, 50.0),
        ];
        let in_front = behind.map(|corner| Point3::new(corner.x, corner.y, 1.0));

        let looking = Looking {
            seen: &project,
            occlusion: &covered,
        };

        assert!(!triangle_is_seen(&looking, behind));
        assert!(triangle_is_seen(&looking, in_front));
    }

    #[test]
    fn a_body_whose_bounds_fall_clear_of_the_box_is_passed_over_and_one_behind_the_eye_is_kept() {
        let area = ScreenArea::Box(crate::sketch_drag::ScreenBox {
            from: Vector2::new(100.0, 100.0),
            to: Vector2::new(200.0, 200.0),
        });
        let flat = |point: Point3| Some(Vector2::new(point.x, point.y));
        let behind_the_eye = |_: Point3| None;
        let bounds = |low: (f64, f64), high: (f64, f64)| {
            Aabb::from_points([
                Point3::new(low.0, low.1, 0.0),
                Point3::new(high.0, high.1, 10.0),
            ])
        };

        let inside = may_reach(bounds((120.0, 120.0), (140.0, 140.0)), &flat, &area);
        let across = may_reach(bounds((0.0, 150.0), (400.0, 160.0)), &flat, &area);
        let near_the_edge = may_reach(bounds((204.0, 150.0), (240.0, 160.0)), &flat, &area);
        let clear = may_reach(bounds((300.0, 300.0), (340.0, 340.0)), &flat, &area);
        let unprojected = may_reach(
            bounds((300.0, 300.0), (340.0, 340.0)),
            &behind_the_eye,
            &area,
        );
        let unbounded = may_reach(None, &flat, &area);

        assert!(inside);
        assert!(across);
        assert!(near_the_edge);
        assert!(!clear);
        assert!(unprojected);
        assert!(unbounded);
    }
}
