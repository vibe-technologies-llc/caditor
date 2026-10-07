use std::collections::BTreeMap;

use caditor_document::FeatureId;
use caditor_geometry::{Point3, Vector2};

use crate::{
    bodies::{BodyMesh, face_keys},
    selection::{Pickable, SelectionFilter},
    sketch_drag::{BoxMode, ScreenBox},
};

const SMALLEST_BOX: f64 = 3.0;

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
            SelectionFilter::Everything | SelectionFilter::Faces => Self::Faces,
            SelectionFilter::Edges => Self::Edges,
            SelectionFilter::Vertices => Self::Vertices,
            SelectionFilter::SketchGeometry => Self::SketchGeometry,
        }
    }
}

pub fn is_a_box(area: ScreenBox) -> bool {
    let size = (area.to - area.from).abs();
    size.x >= SMALLEST_BOX || size.y >= SMALLEST_BOX
}

pub fn within_body(
    body: FeatureId,
    mesh: &BodyMesh,
    screen: &impl Fn(Point3) -> Option<Vector2>,
    area: ScreenBox,
    catch: Catch,
) -> Vec<Pickable> {
    match catch {
        Catch::Faces => faces_within(body, mesh, screen, area),
        Catch::Edges => mesh
            .edges
            .iter()
            .filter(|edge| polyline_caught(&edge.points, screen, area))
            .map(|edge| Pickable::Edge {
                body,
                edge: edge.name,
            })
            .collect(),
        Catch::Vertices => mesh
            .vertices
            .iter()
            .filter(|vertex| screen(vertex.position).is_some_and(|point| area.contains(point)))
            .map(|vertex| Pickable::Vertex {
                body,
                vertex: vertex.key,
            })
            .collect(),
        Catch::SketchGeometry => Vec::new(),
    }
}

fn polyline_caught(
    points: &[Point3],
    screen: &impl Fn(Point3) -> Option<Vector2>,
    area: ScreenBox,
) -> bool {
    let projected: Vec<Option<Vector2>> = points.iter().map(|point| screen(*point)).collect();
    match area.mode() {
        BoxMode::Window => {
            !projected.is_empty()
                && projected
                    .iter()
                    .all(|point| point.is_some_and(|point| area.contains(point)))
        }
        BoxMode::Crossing => {
            projected.windows(2).any(|pair| match pair {
                [Some(from), Some(to)] => area.crosses(*from, *to),
                _ => false,
            }) || matches!(projected.as_slice(), [Some(only)] if area.contains(*only))
        }
    }
}

fn faces_within(
    body: FeatureId,
    mesh: &BodyMesh,
    screen: &impl Fn(Point3) -> Option<Vector2>,
    area: ScreenBox,
) -> Vec<Pickable> {
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
    let centre = (area.from + area.to) * 0.5;
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
            let facing: Vec<[Vector2; 3]> = triangles
                .iter()
                .filter_map(|triangle| match triangle.map(corner) {
                    [Some(a), Some(b), Some(c)] => Some([a, b, c]),
                    _ => None,
                })
                .filter(|[a, b, c]| (*b - *a).perp_dot(*c - *a) < 0.0)
                .collect();
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

fn inside_triangle(point: Vector2, [a, b, c]: [Vector2; 3]) -> bool {
    let side = |p: Vector2, q: Vector2| (q - p).perp_dot(point - p);
    let (first, second, third) = (side(a, b), side(b, c), side(c, a));
    (first >= 0.0 && second >= 0.0 && third >= 0.0)
        || (first <= 0.0 && second <= 0.0 && third <= 0.0)
}
