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
    interrupt,
    interval::Interval,
    naming::{EdgeName, FaceName},
    sense::Sense,
    topology::{EdgeId, FaceId, Solid, VertexId},
};

const MAX_SPLIT_FACES: usize = 24;

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

fn straight(offsets: &Offsets<'_>, around: &Around, sides: [usize; 2], ends: [Point3; 2]) -> bool {
    let [from, to] = ends;
    let Ok(line) = Line::through(from, to) else {
        return false;
    };
    let Some(interval) = Interval::new(0.0, from.distance(to)) else {
        return false;
    };
    let faces: Vec<FaceId> = sides
        .iter()
        .filter_map(|index| around.face(*index))
        .collect();
    follows(offsets, &faces, &line.into(), interval)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Join {
    Edge,
    Merged,
}

#[derive(Debug, Clone, Copy)]
struct Under {
    apex: usize,
    merges: usize,
}

struct Search<'a, 'b> {
    corners: &'b mut Corners<'a>,
    fans: BTreeMap<Triangle, Option<usize>>,
    unders: BTreeMap<(usize, usize, Triangle), Option<Under>>,
    joins: BTreeMap<(Triangle, Triangle), Option<Join>>,
}

impl Search<'_, '_> {
    fn fan(&mut self, triangle: Triangle) -> Option<usize> {
        if let Some(known) = self.fans.get(&triangle) {
            return *known;
        }
        interrupt::check().ok()?;
        let [first, apex, last] = triangle;
        let merges = match self.corners.point(&triangle) {
            Some(_) => self
                .closes(first, apex, triangle)
                .zip(self.closes(apex, last, triangle))
                .map(|(before, after)| before + after),
            None => None,
        };
        self.fans.insert(triangle, merges);
        merges
    }

    fn closes(&mut self, first: usize, last: usize, above: Triangle) -> Option<usize> {
        if last == first + 1 {
            return Some(0);
        }
        self.under(first, last, above).map(|under| under.merges)
    }

    fn under(&mut self, first: usize, last: usize, above: Triangle) -> Option<Under> {
        if let Some(known) = self.unders.get(&(first, last, above)) {
            return *known;
        }
        let mut best: Option<Under> = None;
        for apex in first + 1..last {
            let below = [first, apex, last];
            let Some(inside) = self.fan(below) else {
                continue;
            };
            let Some(join) = self.join(below, above) else {
                continue;
            };
            let merges = inside + usize::from(join == Join::Merged);
            if best.is_none_or(|known| merges < known.merges) {
                best = Some(Under { apex, merges });
            }
        }
        self.unders.insert((first, last, above), best);
        best
    }

    fn join(&mut self, below: Triangle, above: Triangle) -> Option<Join> {
        if let Some(known) = self.joins.get(&(below, above)) {
            return *known;
        }
        let [first, _, last] = below;
        let ends = self.corners.point(&below).zip(self.corners.point(&above));
        let join = ends.and_then(|(from, to)| {
            if coincide(from, to) {
                return Some(Join::Merged);
            }
            straight(
                self.corners.offsets,
                self.corners.around,
                [first, last],
                [from, to],
            )
            .then_some(Join::Edge)
        });
        self.joins.insert((below, above), join);
        join
    }

    fn collect(
        &mut self,
        first: usize,
        last: usize,
        above: Triangle,
        triangles: &mut Vec<Triangle>,
    ) -> Option<()> {
        if last == first + 1 {
            return Some(());
        }
        let apex = self.under(first, last, above)?.apex;
        let below = [first, apex, last];
        self.collect(first, apex, below, triangles)?;
        self.collect(apex, last, below, triangles)?;
        triangles.push(below);
        Some(())
    }

    fn fewest_merges(&mut self, count: usize) -> Option<Candidate> {
        let last = count.checked_sub(1)?;
        let mut best: Option<Under> = None;
        for apex in 1..last {
            let Some(merges) = self.fan([0, apex, last]) else {
                continue;
            };
            if best.is_none_or(|known| merges < known.merges) {
                best = Some(Under { apex, merges });
            }
        }
        let top = [0, best?.apex, last];
        let mut triangles = Vec::with_capacity(count - 2);
        self.collect(0, top[1], top, &mut triangles)?;
        self.collect(top[1], last, top, &mut triangles)?;
        triangles.push(top);
        let points = triangles
            .iter()
            .map(|triangle| self.corners.point(triangle))
            .collect::<Option<Vec<Point3>>>()?;
        Some(Candidate { triangles, points })
    }
}

fn coincide(first: Point3, second: Point3) -> bool {
    first.distance(second) <= OFFSET_TOLERANCE
}

fn centre(solid: &Solid, cluster: &Cluster<'_>) -> Point3 {
    let points: Vec<Point3> = cluster
        .members
        .iter()
        .filter_map(|vertex| Some(solid.vertex(*vertex)?.point()))
        .collect();
    points.iter().fold(Point3::ZERO, |sum, point| sum + *point) / points.len().max(1) as f64
}

pub(super) fn stranded(offsets: &Offsets<'_>, cluster: &Cluster<'_>) -> BTreeSet<FaceId> {
    let solid = offsets.solid;
    let Some(around) = Around::of(solid, cluster) else {
        return BTreeSet::new();
    };
    let count = around.count();
    let Some(shape) = around.shape(solid) else {
        return BTreeSet::new();
    };
    if !(4..=MAX_SPLIT_FACES).contains(&count) {
        return BTreeSet::new();
    }
    let mut corners = Corners {
        offsets,
        around: &around,
        shape,
        start: centre(solid, cluster),
        solved: BTreeMap::new(),
    };
    let mut reached = BTreeSet::new();
    for first in 0..count {
        if interrupt::check().is_err() {
            return BTreeSet::new();
        }
        for second in first + 1..count {
            for third in second + 1..count {
                let triangle = [first, second, third];
                if corners.point(&triangle).is_some() {
                    reached.extend(triangle.iter().filter_map(|index| around.face(*index)));
                }
            }
        }
    }
    around
        .faces
        .iter()
        .filter(|face| !reached.contains(*face))
        .copied()
        .collect()
}

pub(super) fn split_vertex(
    offsets: &Offsets<'_>,
    cluster: &Cluster<'_>,
    plan: &mut Plan,
) -> Option<Split> {
    let solid = offsets.solid;
    let start = centre(solid, cluster);
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
    let chosen = Search {
        corners: &mut corners,
        fans: BTreeMap::new(),
        unders: BTreeMap::new(),
        joins: BTreeMap::new(),
    }
    .fewest_merges(count)?;
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
