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

const MAX_SPLIT_FACES: usize = 12;

pub(super) type Spoke = (EdgeId, VertexId);

struct Around {
    faces: Vec<FaceId>,
    spokes: Vec<Vec<Spoke>>,
}

pub(super) struct Cluster<'a> {
    pub members: &'a BTreeSet<VertexId>,
    pub vanishing: &'a BTreeSet<EdgeId>,
    pub dropped: &'a BTreeSet<FaceId>,
}

impl Around {
    fn of(solid: &Solid, cluster: &Cluster<'_>) -> Option<Self> {
        let mut leaving: BTreeMap<Spoke, (FaceId, Spoke)> = BTreeMap::new();
        for (_, face_loop) in solid.loops() {
            let coedges = face_loop.coedges();
            let count = coedges.len();
            let mut ends = Vec::with_capacity(count);
            for coedge in coedges {
                let (start, end) = solid.coedge_vertices(*coedge)?;
                ends.push((solid.coedge(*coedge)?.edge(), start, end));
            }
            for (index, (arriving, _, end)) in ends.iter().enumerate() {
                if !cluster.members.contains(end) || cluster.vanishing.contains(arriving) {
                    continue;
                }
                let (departing, start, _) = (1..=count)
                    .filter_map(|step| ends.get((index + step) % count))
                    .find(|(edge, _, _)| !cluster.vanishing.contains(edge))?;
                if !cluster.members.contains(start)
                    || leaving
                        .insert((*arriving, *end), (face_loop.face(), (*departing, *start)))
                        .is_some()
                {
                    return None;
                }
            }
        }
        let (first, _) = leaving.first_key_value()?;
        let mut cycle: Vec<(FaceId, Spoke)> = Vec::new();
        let mut arriving = *first;
        loop {
            let (face, departing) = leaving.get(&arriving)?;
            cycle.push((*face, *departing));
            arriving = *departing;
            if arriving == *first || cycle.len() > leaving.len() {
                break;
            }
        }
        if arriving != *first || cycle.len() != leaving.len() {
            return None;
        }
        let kept = cycle
            .iter()
            .position(|(face, _)| !cluster.dropped.contains(face))?;
        cycle.rotate_left(kept);
        let mut around = Self {
            faces: Vec::new(),
            spokes: Vec::new(),
        };
        for (face, departing) in cycle {
            if cluster.dropped.contains(&face) {
                around.spokes.last_mut()?.push(departing);
            } else {
                around.faces.push(face);
                around.spokes.push(vec![departing]);
            }
        }
        Some(around)
    }

    fn count(&self) -> usize {
        self.faces.len()
    }

    fn face(&self, index: usize) -> Option<FaceId> {
        self.faces.get(index % self.count()).copied()
    }

    fn arrival(&self, index: usize) -> Option<VertexId> {
        let (_, vertex) = self.spokes.get(index % self.count())?.last()?;
        Some(*vertex)
    }

    fn convexities(&self, solid: &Solid) -> Option<Vec<bool>> {
        let mut convex = Vec::with_capacity(self.spokes.len());
        for (index, spokes) in self.spokes.iter().enumerate() {
            let (edge_id, vertex) = spokes.first()?;
            let edge = solid.edge(*edge_id)?;
            if edge.is_closed() {
                return None;
            }
            let at = solid.vertex(*vertex)?.point();
            let interval = edge.interval();
            let leaving = if edge.start() == *vertex {
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

    fn shape(&self, solid: &Solid) -> Option<Shape> {
        let convex = self.convexities(solid)?;
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
    pub points: Vec<Point3>,
    pub ends: BTreeMap<Spoke, usize>,
    pub detours: BTreeMap<(FaceId, VertexId), Vec<PlanCoedge>>,
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

struct Corners<'a> {
    offsets: &'a Offsets<'a>,
    around: &'a Around,
    shape: Shape,
    start: Point3,
    solved: BTreeMap<Triangle, Option<Point3>>,
}

impl Corners<'_> {
    fn solve(&self, triangle: &Triangle) -> Option<Point3> {
        let faces: BTreeSet<FaceId> = triangle
            .iter()
            .filter_map(|index| self.around.face(*index))
            .collect();
        if faces.len() != triangle.len() {
            return None;
        }
        let point = offset_vertex(self.offsets, &faces, &[], self.start)?;
        let mut others = BTreeMap::new();
        for index in (0..self.around.count()).filter(|index| !triangle.contains(index)) {
            let (residual, _) = self
                .offsets
                .residual(self.around.face(index)?, point, None)?;
            others.insert(index, residual);
        }
        self.shape.keeps(triangle, &others).then_some(point)
    }

    fn point(&mut self, triangle: &Triangle) -> Option<Point3> {
        if let Some(known) = self.solved.get(triangle) {
            return *known;
        }
        let point = self.solve(triangle);
        self.solved.insert(*triangle, point);
        point
    }
}

fn candidate(corners: &mut Corners<'_>, triangles: Vec<Triangle>) -> Option<Candidate> {
    let (offsets, around) = (corners.offsets, corners.around);
    let mut points = Vec::with_capacity(triangles.len());
    for triangle in &triangles {
        points.push(corners.point(triangle)?);
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
            if coincide(*from, *to) {
                return true;
            }
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

fn coincide(first: Point3, second: Point3) -> bool {
    first.distance(second) <= OFFSET_TOLERANCE
}

pub(super) fn split_vertex(
    offsets: &Offsets<'_>,
    cluster: &Cluster<'_>,
    plan: &mut Plan,
) -> Option<Split> {
    let solid = offsets.solid;
    let points: Vec<Point3> = cluster
        .members
        .iter()
        .filter_map(|vertex| Some(solid.vertex(*vertex)?.point()))
        .collect();
    let start =
        points.iter().fold(Point3::ZERO, |sum, point| sum + *point) / points.len().max(1) as f64;
    let around = Around::of(solid, cluster)?;
    let count = around.count();
    if !(4..=MAX_SPLIT_FACES).contains(&count) {
        return None;
    }
    let mut corners = Corners {
        offsets,
        around: &around,
        shape: around.shape(solid)?,
        start,
        solved: BTreeMap::new(),
    };
    let polygon: Vec<usize> = (0..count).collect();
    let chosen = triangulations(&polygon)
        .into_iter()
        .find_map(|triangles| candidate(&mut corners, triangles))?;
    let mut merged: Vec<usize> = (0..chosen.points.len()).collect();
    for (_, _, first, second) in chosen.diagonals(count) {
        let (Some(from), Some(to)) = (chosen.points.get(first), chosen.points.get(second)) else {
            return None;
        };
        if coincide(*from, *to) {
            let (keep, other) = (*merged.get(first)?, *merged.get(second)?);
            for slot in &mut merged {
                if *slot == other {
                    *slot = keep;
                }
            }
        }
    }
    let mut placed: BTreeMap<usize, usize> = BTreeMap::new();
    let mut corners = Vec::with_capacity(merged.len());
    for keep in &merged {
        let vertex = match placed.get(keep) {
            Some(vertex) => *vertex,
            None => {
                let vertex = plan.vertex(*chosen.points.get(*keep)?);
                placed.insert(*keep, vertex);
                vertex
            }
        };
        corners.push(vertex);
    }
    let side = |index: usize| chosen.holding(index % count, (index + 1) % count).next();
    let mut ends = BTreeMap::new();
    for (index, spokes) in around.spokes.iter().enumerate() {
        let corner = *corners.get(side(index)?)?;
        for spoke in spokes {
            ends.insert(*spoke, corner);
        }
    }
    let name = |index: usize| {
        around
            .face(index)
            .and_then(|face| solid.face(face))
            .map_or(FaceName::NONE, |face| face.name())
    };
    let mut diagonals: BTreeMap<(usize, usize), Option<(usize, usize)>> = BTreeMap::new();
    for (a, b, first, second) in chosen.diagonals(count) {
        if corners.get(first)? == corners.get(second)? {
            diagonals.insert((a, b), None);
            continue;
        }
        let edge = plan
            .line(
                *corners.get(first)?,
                *corners.get(second)?,
                EdgeName::between(name(a), name(b)),
            )
            .ok()?;
        diagonals.insert((a, b), Some((edge, first)));
    }
    let mut detours = BTreeMap::new();
    for corner in 0..count {
        let mut detour = Vec::new();
        let mut triangle = side(corner + count - 1)?;
        let mut came_from = (corner + count - 1) % count;
        let mut steps = 0;
        while triangle != side(corner)? {
            let next_corner = chosen
                .triangles
                .get(triangle)?
                .iter()
                .copied()
                .find(|index| *index != corner && *index != came_from)?;
            let key = (corner.min(next_corner), corner.max(next_corner));
            if let Some((edge, starts_at)) = diagonals.get(&key)? {
                let sense = if *starts_at == triangle {
                    Sense::Same
                } else {
                    Sense::Reversed
                };
                detour.push(PlanCoedge::new(*edge, sense));
            }
            triangle = chosen
                .holding(corner, next_corner)
                .find(|index| *index != triangle)?;
            came_from = next_corner;
            steps += 1;
            if steps > count {
                return None;
            }
        }
        let arrival = around.arrival(corner + count - 1)?;
        detours.insert((around.face(corner)?, arrival), detour);
    }
    Some(Split {
        points: chosen.points,
        ends,
        detours,
    })
}
