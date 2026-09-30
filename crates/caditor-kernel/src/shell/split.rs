use std::collections::{BTreeMap, BTreeSet};

use caditor_geometry::Point3;

use super::{
    OFFSET_TOLERANCE, Offsets,
    edge::follows,
    inner::{offset_vertex, outward_normal},
};
use crate::{
    build::plan::{Plan, PlanCoedge},
    curve::Line,
    interval::Interval,
    naming::{EdgeName, FaceName},
    sense::Sense,
    topology::{EdgeId, FaceId, Solid, VertexId},
};

const MAX_SPLIT_FACES: usize = 8;

struct Around {
    faces: Vec<FaceId>,
    edges: Vec<EdgeId>,
}

impl Around {
    fn of(solid: &Solid, vertex: VertexId) -> Option<Self> {
        let mut leaving: BTreeMap<EdgeId, (FaceId, EdgeId)> = BTreeMap::new();
        for (_, face_loop) in solid.loops() {
            let coedges = face_loop.coedges();
            for (index, coedge) in coedges.iter().enumerate() {
                let (_, end) = solid.coedge_vertices(*coedge)?;
                if end != vertex {
                    continue;
                }
                let next = coedges.get((index + 1) % coedges.len())?;
                let arriving = solid.coedge(*coedge)?.edge();
                let departing = solid.coedge(*next)?.edge();
                if leaving
                    .insert(arriving, (face_loop.face(), departing))
                    .is_some()
                {
                    return None;
                }
            }
        }
        let (first, _) = leaving.first_key_value()?;
        let mut around = Self {
            faces: Vec::new(),
            edges: Vec::new(),
        };
        let mut arriving = *first;
        loop {
            let (face, departing) = leaving.get(&arriving)?;
            around.faces.push(*face);
            around.edges.push(*departing);
            arriving = *departing;
            if arriving == *first || around.faces.len() > leaving.len() {
                break;
            }
        }
        let distinct: BTreeSet<FaceId> = around.faces.iter().copied().collect();
        let complete = arriving == *first && around.faces.len() == leaving.len();
        (complete && distinct.len() == around.faces.len()).then_some(around)
    }

    fn count(&self) -> usize {
        self.faces.len()
    }

    fn face(&self, index: usize) -> Option<FaceId> {
        self.faces.get(index % self.count()).copied()
    }

    fn convexities(&self, solid: &Solid, vertex: VertexId) -> Option<Vec<bool>> {
        let at = solid.vertex(vertex)?.point();
        let mut convex = Vec::with_capacity(self.edges.len());
        for (index, edge_id) in self.edges.iter().enumerate() {
            let edge = solid.edge(*edge_id)?;
            if edge.is_closed() {
                return None;
            }
            let interval = edge.interval();
            let leaving = if edge.start() == vertex {
                edge.curve().evaluate(interval.start()).first
            } else {
                -edge.curve().evaluate(interval.end()).first
            };
            let left = outward_normal(solid, self.face(index)?, at)?;
            let right = outward_normal(solid, self.face(index + 1)?, at)?;
            convex.push(left.cross(right).dot(leaving) > 0.0);
        }
        Some(convex)
    }

    fn shape(&self, solid: &Solid, vertex: VertexId) -> Option<Shape> {
        let convex = self.convexities(solid, vertex)?;
        let folds: Vec<usize> = (0..convex.len())
            .filter(|index| convex.get(*index) == Some(&false))
            .collect();
        match folds[..] {
            [] => Some(Shape::Convex),
            [fold] => Some(Shape::ConvexButFold([fold, (fold + 1) % self.count()])),
            _ if folds.len() == convex.len() => Some(Shape::Concave),
            _ => None,
        }
    }
}

type Triangle = [usize; 3];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    Convex,
    Concave,
    ConvexButFold([usize; 2]),
}

impl Shape {
    fn keeps(self, corner: &Triangle, others: &BTreeMap<usize, f64>) -> bool {
        let inside = |residual: f64| residual <= OFFSET_TOLERANCE;
        let outside = |residual: f64| residual >= -OFFSET_TOLERANCE;
        match self {
            Self::Convex => others.values().all(|residual| inside(*residual)),
            Self::Concave => others.values().all(|residual| outside(*residual)),
            Self::ConvexButFold(fold) => {
                let folded = |index: &usize| fold.contains(index);
                let rest = others
                    .iter()
                    .filter(|(index, _)| !folded(index))
                    .all(|(_, residual)| inside(*residual));
                let mut across_fold = others
                    .iter()
                    .filter(|(index, _)| folded(index))
                    .map(|(_, residual)| *residual);
                let exposed = if corner.iter().any(folded) {
                    across_fold.all(outside)
                } else {
                    across_fold.any(|residual| residual < -OFFSET_TOLERANCE)
                };
                rest && exposed
            }
        }
    }
}

fn triangulations(polygon: &[usize]) -> Vec<Vec<Triangle>> {
    let (Some(first), Some(last)) = (polygon.first(), polygon.last()) else {
        return vec![Vec::new()];
    };
    if polygon.len() < 3 {
        return vec![Vec::new()];
    }
    let mut all = Vec::new();
    for apex in 1..polygon.len() - 1 {
        let (Some(left), Some(right), Some(tip)) =
            (polygon.get(..=apex), polygon.get(apex..), polygon.get(apex))
        else {
            continue;
        };
        for before in triangulations(left) {
            for after in triangulations(right) {
                let mut triangles = before.clone();
                triangles.extend(after.iter().copied());
                triangles.push([*first, *tip, *last]);
                all.push(triangles);
            }
        }
    }
    all
}

pub(super) struct Split {
    pub ends: BTreeMap<EdgeId, usize>,
    pub detours: BTreeMap<FaceId, Vec<PlanCoedge>>,
}

struct Candidate {
    triangles: Vec<Triangle>,
    points: Vec<Point3>,
}

impl Candidate {
    fn holding(&self, corner: usize, other: usize) -> impl Iterator<Item = usize> + '_ {
        self.triangles
            .iter()
            .enumerate()
            .filter(move |(_, triangle)| triangle.contains(&corner) && triangle.contains(&other))
            .map(|(index, _)| index)
    }

    fn diagonals(&self, count: usize) -> Vec<(usize, usize, usize, usize)> {
        let mut diagonals = Vec::new();
        for a in 0..count {
            for b in a + 2..count {
                if a == 0 && b == count - 1 {
                    continue;
                }
                let triangles: Vec<usize> = self.holding(a, b).collect();
                if let [first, second] = triangles.as_slice() {
                    diagonals.push((a, b, *first, *second));
                }
            }
        }
        diagonals
    }
}

fn candidate(
    offsets: &Offsets<'_>,
    around: &Around,
    shape: Shape,
    start: Point3,
    triangles: Vec<Triangle>,
) -> Option<Candidate> {
    let mut points = Vec::with_capacity(triangles.len());
    for triangle in &triangles {
        let faces: BTreeSet<FaceId> = triangle
            .iter()
            .filter_map(|index| around.face(*index))
            .collect();
        let point = offset_vertex(offsets, &faces, &[], start)?;
        let mut others = BTreeMap::new();
        for index in (0..around.count()).filter(|index| !triangle.contains(index)) {
            let (residual, _) = offsets.residual(around.face(index)?, point, None)?;
            others.insert(index, residual);
        }
        if !shape.keeps(triangle, &others) {
            return None;
        }
        points.push(point);
    }
    let candidate = Candidate { triangles, points };
    let straight = candidate
        .diagonals(around.count())
        .into_iter()
        .all(|(a, b, first, second)| {
            let (Some(from), Some(to)) =
                (candidate.points.get(first), candidate.points.get(second))
            else {
                return false;
            };
            let Ok(line) = Line::through(*from, *to) else {
                return false;
            };
            let Some(interval) = Interval::new(0.0, from.distance(*to)) else {
                return false;
            };
            let faces: Vec<FaceId> = [a, b]
                .iter()
                .filter_map(|index| around.face(*index))
                .collect();
            follows(offsets, &faces, &line.into(), interval)
        });
    straight.then_some(candidate)
}

pub(super) fn split_vertex(
    offsets: &Offsets<'_>,
    vertex: VertexId,
    plan: &mut Plan,
) -> Option<Split> {
    let solid = offsets.solid;
    let start = solid.vertex(vertex)?.point();
    let around = Around::of(solid, vertex)?;
    let count = around.count();
    if !(4..=MAX_SPLIT_FACES).contains(&count) {
        return None;
    }
    let shape = around.shape(solid, vertex)?;
    let polygon: Vec<usize> = (0..count).collect();
    let chosen = triangulations(&polygon)
        .into_iter()
        .find_map(|triangles| candidate(offsets, &around, shape, start, triangles))?;
    let corners: Vec<usize> = chosen
        .points
        .iter()
        .map(|point| plan.vertex(*point))
        .collect();
    let side = |index: usize| chosen.holding(index % count, (index + 1) % count).next();
    let mut ends = BTreeMap::new();
    for (index, edge) in around.edges.iter().enumerate() {
        ends.insert(*edge, *corners.get(side(index)?)?);
    }
    let name = |index: usize| {
        around
            .face(index)
            .and_then(|face| solid.face(face))
            .map_or(FaceName::NONE, |face| face.name())
    };
    let mut diagonals: BTreeMap<(usize, usize), (usize, usize)> = BTreeMap::new();
    for (a, b, first, second) in chosen.diagonals(count) {
        let edge = plan
            .line(
                *corners.get(first)?,
                *corners.get(second)?,
                EdgeName::between(name(a), name(b)),
            )
            .ok()?;
        diagonals.insert((a, b), (edge, first));
    }
    let mut detours = BTreeMap::new();
    for corner in 0..count {
        let mut detour = Vec::new();
        let mut triangle = side(corner + count - 1)?;
        let mut came_from = (corner + count - 1) % count;
        while triangle != side(corner)? {
            let next_corner = chosen
                .triangles
                .get(triangle)?
                .iter()
                .copied()
                .find(|index| *index != corner && *index != came_from)?;
            let key = (corner.min(next_corner), corner.max(next_corner));
            let (edge, starts_at) = diagonals.get(&key)?;
            let sense = if *starts_at == triangle {
                Sense::Same
            } else {
                Sense::Reversed
            };
            detour.push(PlanCoedge::new(*edge, sense));
            triangle = chosen
                .holding(corner, next_corner)
                .find(|index| *index != triangle)?;
            came_from = next_corner;
            if detour.len() > count {
                return None;
            }
        }
        detours.insert(around.face(corner)?, detour);
    }
    Some(Split { ends, detours })
}
