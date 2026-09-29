use std::{
    collections::{BTreeMap, BTreeSet},
    f64::consts::TAU,
};

use caditor_geometry::{Plane, Point2, Point3, Vector3};

use super::{EDGE_SAMPLES, OFFSET_TOLERANCE, Offsets, ShellError};
use crate::{
    build::plan::{Plan, PlanCoedge, PlanError, PlanFace},
    curve::{Circle, Curve, Line},
    interrupt,
    interval::Interval,
    naming::{EdgeName, FaceName, FaceOrigin},
    sense::Sense,
    surface::Surface,
    tolerance::LINEAR_RESOLUTION,
    topology::{EdgeId, FaceId, Solid, VertexId},
};

const VERTEX_ITERATIONS: usize = 40;
const INDEPENDENT_ROW: f64 = 1e-6;
const PARALLEL: f64 = 1e-6;
const MAX_SPLIT_FACES: usize = 8;

#[derive(Debug, Clone, Copy)]
enum Collapse {
    Axis(Vector3),
    Centre,
    Spine(Vector3),
}

impl Collapse {
    fn vanishes(self, curve: &Curve) -> Option<bool> {
        let parallel = |a: Vector3, b: Vector3| a.cross(b).length() <= PARALLEL;
        match (self, curve) {
            (Self::Centre, _) => Some(true),
            (Self::Axis(axis), Curve::Circle(circle))
                if parallel(circle.frame().normal(), axis) =>
            {
                Some(true)
            }
            (Self::Axis(axis), Curve::Line(line)) if parallel(line.direction(), axis) => {
                Some(false)
            }
            (Self::Spine(axis), Curve::Circle(circle))
                if circle.frame().normal().dot(axis).abs() <= PARALLEL =>
            {
                Some(true)
            }
            (Self::Spine(axis), Curve::Circle(circle))
                if parallel(circle.frame().normal(), axis) =>
            {
                Some(false)
            }
            _ => None,
        }
    }
}

fn collapse(offsets: &Offsets<'_>, face: FaceId) -> Option<Collapse> {
    let definition = offsets.solid.face(face)?;
    let along = -offsets.distance(face) * definition.sense().sign();
    let gone = |radius: f64| radius + along <= OFFSET_TOLERANCE;
    match definition.surface() {
        Surface::Cylinder(cylinder) if gone(cylinder.radius()) => {
            Some(Collapse::Axis(cylinder.frame().normal()))
        }
        Surface::Sphere(sphere) if gone(sphere.radius()) => Some(Collapse::Centre),
        Surface::Torus(torus) if gone(torus.minor_radius()) => {
            Some(Collapse::Spine(torus.frame().normal()))
        }
        _ => None,
    }
}

fn collapsed_faces(offsets: &Offsets<'_>) -> BTreeMap<FaceId, Collapse> {
    offsets
        .solid
        .faces()
        .filter_map(|(id, _)| Some((id, collapse(offsets, id)?)))
        .collect()
}

pub(super) fn collapses(offsets: &Offsets<'_>, face: FaceId) -> bool {
    collapse(offsets, face).is_some()
}

fn coedge_faces(solid: &Solid, edge: EdgeId) -> Vec<FaceId> {
    solid
        .edge(edge)
        .into_iter()
        .flat_map(|edge| edge.coedges())
        .filter_map(|coedge| solid.coedge_face(*coedge))
        .collect()
}

fn vanishing_edges(
    solid: &Solid,
    collapsed: &BTreeMap<FaceId, Collapse>,
) -> Result<BTreeSet<EdgeId>, ShellError> {
    let mut vanishing = BTreeSet::new();
    for (id, edge) in solid.edges() {
        for face in coedge_faces(solid, id) {
            let Some(collapse) = collapsed.get(&face) else {
                continue;
            };
            if collapse
                .vanishes(edge.curve())
                .ok_or(ShellError::TooCurved(face))?
            {
                vanishing.insert(id);
            }
        }
    }
    Ok(vanishing)
}

struct Clusters {
    root: BTreeMap<VertexId, VertexId>,
}

impl Clusters {
    fn new(solid: &Solid, vanishing: &BTreeSet<EdgeId>) -> Self {
        let mut clusters = Self {
            root: solid.vertices().map(|(id, _)| (id, id)).collect(),
        };
        for edge in vanishing.iter().filter_map(|id| solid.edge(*id)) {
            let (a, b) = (clusters.find(edge.start()), clusters.find(edge.end()));
            let (low, high) = (a.min(b), a.max(b));
            clusters.root.insert(high, low);
        }
        clusters
    }

    fn find(&self, mut vertex: VertexId) -> VertexId {
        while let Some(parent) = self.root.get(&vertex).copied() {
            if parent == vertex {
                break;
            }
            vertex = parent;
        }
        vertex
    }

    fn members(&self) -> BTreeMap<VertexId, Vec<VertexId>> {
        let mut members: BTreeMap<VertexId, Vec<VertexId>> = BTreeMap::new();
        for vertex in self.root.keys() {
            members.entry(self.find(*vertex)).or_default().push(*vertex);
        }
        members
    }
}

pub(super) fn faces_at(solid: &Solid) -> BTreeMap<VertexId, BTreeSet<FaceId>> {
    let mut around: BTreeMap<VertexId, BTreeSet<FaceId>> = BTreeMap::new();
    for (id, edge) in solid.edges() {
        let faces = coedge_faces(solid, id);
        for vertex in [edge.start(), edge.end()] {
            around
                .entry(vertex)
                .or_default()
                .extend(faces.iter().copied());
        }
    }
    around
}

fn independent(rows: &[(f64, Vector3)]) -> Vec<(f64, Vector3)> {
    let mut chosen: Vec<(f64, Vector3)> = Vec::with_capacity(3);
    for (residual, normal) in rows {
        let spans_new = match chosen.as_slice() {
            [] => true,
            [(_, first)] => first.cross(*normal).length() > INDEPENDENT_ROW,
            [(_, first), (_, second)] => first.cross(*second).dot(*normal).abs() > INDEPENDENT_ROW,
            _ => false,
        };
        if spans_new {
            chosen.push((*residual, *normal));
        }
    }
    chosen
}

fn minimal_step(rows: &[(f64, Vector3)]) -> Option<Vector3> {
    match independent(rows).as_slice() {
        [] => Some(Vector3::ZERO),
        [(r, n)] => Some(-*n * (*r / n.length_squared())),
        [(ra, a), (rb, b)] => {
            let (aa, ab, bb) = (a.dot(*a), a.dot(*b), b.dot(*b));
            let determinant = aa * bb - ab * ab;
            if determinant.abs() <= f64::EPSILON {
                return None;
            }
            let x = (-ra * bb + rb * ab) / determinant;
            let y = (-rb * aa + ra * ab) / determinant;
            Some(*a * x + *b * y)
        }
        [(ra, a), (rb, b), (rc, c)] => {
            let determinant = a.dot(b.cross(*c));
            if determinant.abs() <= f64::EPSILON {
                return None;
            }
            Some((b.cross(*c) * -ra + c.cross(*a) * -rb + a.cross(*b) * -rc) / determinant)
        }
        _ => None,
    }
}

fn offset_vertex(offsets: &Offsets<'_>, faces: &BTreeSet<FaceId>, start: Point3) -> Option<Point3> {
    let hints: Vec<(FaceId, Point2)> = faces
        .iter()
        .filter_map(|face| {
            let surface = offsets.solid.face(*face)?.surface();
            Some((*face, surface.project(start, None)))
        })
        .collect();
    let mut point = start;
    for _ in 0..VERTEX_ITERATIONS {
        let rows: Vec<(f64, Vector3)> = hints
            .iter()
            .filter_map(|(face, hint)| offsets.residual(*face, point, Some(*hint)))
            .collect();
        if rows.len() != hints.len() {
            return None;
        }
        if rows
            .iter()
            .all(|(residual, _)| residual.abs() <= 0.01 * LINEAR_RESOLUTION)
        {
            return Some(point);
        }
        point += minimal_step(&rows)?;
    }
    let settled = hints.iter().all(|(face, hint)| {
        offsets
            .residual(*face, point, Some(*hint))
            .is_some_and(|(residual, _)| residual.abs() <= OFFSET_TOLERANCE)
    });
    settled.then_some(point)
}

fn offset_curve(
    solid: &Solid,
    edge: EdgeId,
    start: Point3,
    end: Point3,
) -> Result<(Curve, Interval), ShellError> {
    let unsupported = || ShellError::UnsupportedEdge(edge);
    let definition = solid.edge(edge).ok_or_else(unsupported)?;
    match definition.curve() {
        Curve::Line(original) => {
            if (end - start).dot(original.direction()) <= LINEAR_RESOLUTION {
                return Err(ShellError::EdgeCollapses(edge));
            }
            let line = Line::through(start, end).map_err(|_| ShellError::EdgeCollapses(edge))?;
            let interval =
                Interval::new(0.0, start.distance(end)).ok_or(ShellError::EdgeCollapses(edge))?;
            Ok((line.into(), interval))
        }
        Curve::Circle(circle) => {
            let axis = circle.frame().normal();
            let center = circle.center() + axis * (start - circle.center()).dot(axis);
            let radial = start - center;
            let radius = radial.length();
            if radius <= LINEAR_RESOLUTION {
                return Err(ShellError::EdgeCollapses(edge));
            }
            let x_axis = radial
                .try_normalize()
                .ok_or(ShellError::EdgeCollapses(edge))?;
            let frame =
                Plane::from_frame(center, axis, x_axis).ok_or(ShellError::EdgeCollapses(edge))?;
            let sweep = if definition.is_closed() {
                TAU
            } else {
                let local = end - center;
                let angle = axis
                    .cross(x_axis)
                    .dot(local)
                    .atan2(x_axis.dot(local))
                    .rem_euclid(TAU);
                if angle <= f64::EPSILON { TAU } else { angle }
            };
            let circle = Circle::new(frame, radius).map_err(|_| ShellError::EdgeCollapses(edge))?;
            let interval = Interval::new(0.0, sweep).ok_or(ShellError::EdgeCollapses(edge))?;
            Ok((circle.into(), interval))
        }
        _ => Err(unsupported()),
    }
}

fn follows(offsets: &Offsets<'_>, faces: &[FaceId], curve: &Curve, interval: Interval) -> bool {
    EDGE_SAMPLES
        .iter()
        .all(|fraction| offsets.on_both(faces, curve.point(interval.at(*fraction))))
}

fn outward_normal(solid: &Solid, face: FaceId, point: Point3) -> Option<Vector3> {
    let face = solid.face(face)?;
    let surface = face.surface();
    let uv = surface.project(point, None);
    Some(surface.normal(uv.x, uv.y)? * face.sense().sign())
}

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

    fn convexity(&self, solid: &Solid, vertex: VertexId) -> Option<bool> {
        let at = solid.vertex(vertex)?.point();
        let mut convex = BTreeSet::new();
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
            convex.insert(left.cross(right).dot(leaving) > 0.0);
        }
        match convex.into_iter().collect::<Vec<_>>().as_slice() {
            [only] => Some(*only),
            _ => None,
        }
    }
}

type Triangle = [usize; 3];

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

struct Split {
    ends: BTreeMap<EdgeId, usize>,
    detours: BTreeMap<FaceId, Vec<PlanCoedge>>,
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
    convex: bool,
    start: Point3,
    triangles: Vec<Triangle>,
) -> Option<Candidate> {
    let mut points = Vec::with_capacity(triangles.len());
    for triangle in &triangles {
        let faces: BTreeSet<FaceId> = triangle
            .iter()
            .filter_map(|index| around.face(*index))
            .collect();
        let point = offset_vertex(offsets, &faces, start)?;
        let inside = around
            .faces
            .iter()
            .filter(|face| !faces.contains(face))
            .all(|face| {
                offsets
                    .residual(*face, point, None)
                    .is_some_and(|(residual, _)| {
                        if convex {
                            residual <= OFFSET_TOLERANCE
                        } else {
                            residual >= -OFFSET_TOLERANCE
                        }
                    })
            });
        if !inside {
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

fn split_vertex(offsets: &Offsets<'_>, vertex: VertexId, plan: &mut Plan) -> Option<Split> {
    let solid = offsets.solid;
    let start = solid.vertex(vertex)?.point();
    let around = Around::of(solid, vertex)?;
    let count = around.count();
    if !(4..=MAX_SPLIT_FACES).contains(&count) {
        return None;
    }
    let convex = around.convexity(solid, vertex)?;
    let polygon: Vec<usize> = (0..count).collect();
    let chosen = triangulations(&polygon)
        .into_iter()
        .find_map(|triangles| candidate(offsets, &around, convex, start, triangles))?;
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

#[derive(Default)]
struct Corners {
    points: BTreeMap<VertexId, usize>,
    splits: BTreeMap<VertexId, Split>,
}

impl Corners {
    fn end(&self, clusters: &Clusters, vertex: VertexId, edge: EdgeId) -> Option<usize> {
        let root = clusters.find(vertex);
        self.points
            .get(&root)
            .copied()
            .or_else(|| self.splits.get(&root)?.ends.get(&edge).copied())
    }
}

fn place_corners(
    offsets: &Offsets<'_>,
    collapsed: &BTreeMap<FaceId, Collapse>,
    clusters: &Clusters,
    plan: &mut Plan,
) -> Result<Corners, ShellError> {
    let solid = offsets.solid;
    let around = faces_at(solid);
    let mut corners = Corners::default();
    for (root, members) in clusters.members() {
        interrupt::check()?;
        let every: BTreeSet<FaceId> = members
            .iter()
            .filter_map(|vertex| around.get(vertex))
            .flatten()
            .copied()
            .collect();
        let live: BTreeSet<FaceId> = every
            .iter()
            .filter(|face| !collapsed.contains_key(face))
            .copied()
            .collect();
        let points: Vec<Point3> = members
            .iter()
            .filter_map(|vertex| Some(solid.vertex(*vertex)?.point()))
            .collect();
        let start = points.iter().fold(Point3::ZERO, |sum, point| sum + *point)
            / points.len().max(1) as f64;
        if let Some(point) = offset_vertex(offsets, &live, start) {
            corners.points.insert(root, plan.vertex(point));
            continue;
        }
        if let Some(face) = every.iter().find(|face| collapsed.contains_key(face)) {
            return Err(ShellError::TooCurved(*face));
        }
        let split = split_vertex(offsets, root, plan).ok_or(ShellError::Corner(root))?;
        corners.splits.insert(root, split);
    }
    Ok(corners)
}

struct Half {
    edge: EdgeId,
    face: FaceId,
    ends: (usize, usize),
}

struct Placed {
    edge: usize,
    flipped: bool,
}

fn merged_sense(solid: &Solid, first: EdgeId, second: EdgeId, same_ends: bool) -> Option<bool> {
    let (a, b) = (solid.edge(first)?, solid.edge(second)?);
    if !a.is_closed() {
        return Some(!same_ends);
    }
    let tangent = |edge: &crate::topology::Edge| {
        edge.curve()
            .evaluate(edge.interval().middle())
            .first
            .normalize_or_zero()
    };
    let (Curve::Circle(first_circle), Curve::Circle(second_circle)) = (a.curve(), b.curve()) else {
        return Some(tangent(a).dot(tangent(b)) < 0.0);
    };
    Some(
        first_circle
            .frame()
            .normal()
            .dot(second_circle.frame().normal())
            < 0.0,
    )
}

fn place_edges(
    offsets: &Offsets<'_>,
    collapsed: &BTreeMap<FaceId, Collapse>,
    vanishing: &BTreeSet<EdgeId>,
    clusters: &Clusters,
    corners: &Corners,
    plan: &mut Plan,
) -> Result<BTreeMap<EdgeId, Placed>, ShellError> {
    let solid = offsets.solid;
    let mut placed = BTreeMap::new();
    let mut halves: BTreeMap<(usize, usize), Vec<Half>> = BTreeMap::new();
    for (id, edge) in solid.edges() {
        interrupt::check()?;
        if vanishing.contains(&id) {
            continue;
        }
        let ends = (
            corners
                .end(clusters, edge.start(), id)
                .ok_or(ShellError::UnsupportedEdge(id))?,
            corners
                .end(clusters, edge.end(), id)
                .ok_or(ShellError::UnsupportedEdge(id))?,
        );
        let faces = coedge_faces(solid, id);
        let live: Vec<FaceId> = faces
            .iter()
            .filter(|face| !collapsed.contains_key(face))
            .copied()
            .collect();
        if live.len() == faces.len() {
            let mut distinct = live.clone();
            distinct.dedup();
            let index = new_edge(offsets, id, &distinct, ends, edge.name(), plan)?;
            placed.insert(
                id,
                Placed {
                    edge: index,
                    flipped: false,
                },
            );
            continue;
        }
        let dead = faces
            .iter()
            .find(|face| collapsed.contains_key(face))
            .copied()
            .ok_or(ShellError::UnsupportedEdge(id))?;
        let [face] = live.as_slice() else {
            return Err(ShellError::TooCurved(dead));
        };
        let key = (ends.0.min(ends.1), ends.0.max(ends.1));
        halves.entry(key).or_default().push(Half {
            edge: id,
            face: *face,
            ends,
        });
    }
    for group in halves.into_values() {
        let [
            Half {
                edge: first,
                face: first_face,
                ends,
            },
            Half {
                edge: second,
                face: second_face,
                ends: second_ends,
            },
        ] = group.as_slice()
        else {
            let dead = group.first().map(|half| half.edge).and_then(|id| {
                coedge_faces(solid, id)
                    .into_iter()
                    .find(|face| collapsed.contains_key(face))
            });
            return Err(dead.map_or(ShellError::Walls, ShellError::TooCurved));
        };
        let name = |face: FaceId| solid.face(face).map_or(FaceName::NONE, |face| face.name());
        let index = new_edge(
            offsets,
            *first,
            &[*first_face, *second_face],
            *ends,
            EdgeName::between(name(*first_face), name(*second_face)),
            plan,
        )?;
        let flipped = merged_sense(solid, *first, *second, ends == second_ends)
            .ok_or(ShellError::UnsupportedEdge(*second))?;
        placed.insert(
            *first,
            Placed {
                edge: index,
                flipped: false,
            },
        );
        placed.insert(
            *second,
            Placed {
                edge: index,
                flipped,
            },
        );
    }
    Ok(placed)
}

fn new_edge(
    offsets: &Offsets<'_>,
    source: EdgeId,
    faces: &[FaceId],
    (start, end): (usize, usize),
    name: EdgeName,
    plan: &mut Plan,
) -> Result<usize, ShellError> {
    let (Some(from), Some(to)) = (plan.point(start), plan.point(end)) else {
        return Err(ShellError::UnsupportedEdge(source));
    };
    let (curve, interval) = offset_curve(offsets.solid, source, from, to)?;
    if !follows(offsets, faces, &curve, interval) {
        return Err(ShellError::UnsupportedEdge(source));
    }
    Ok(plan.edge(curve, interval, (start, end), name))
}

fn loops(
    solid: &Solid,
    face: FaceId,
    placed: &BTreeMap<EdgeId, Placed>,
    vanishing: &BTreeSet<EdgeId>,
    corners: &Corners,
) -> Result<Vec<Vec<PlanCoedge>>, ShellError> {
    let definition = solid.face(face).ok_or(ShellError::MissingFace(face))?;
    let mut loops = Vec::new();
    for face_loop in definition
        .loops()
        .iter()
        .filter_map(|id| solid.face_loop(*id))
    {
        let mut coedges = Vec::new();
        for coedge_id in face_loop.coedges() {
            let Some(coedge) = solid.coedge(*coedge_id) else {
                continue;
            };
            if vanishing.contains(&coedge.edge()) {
                continue;
            }
            let target = placed
                .get(&coedge.edge())
                .ok_or(ShellError::UnsupportedEdge(coedge.edge()))?;
            let sense = if target.flipped {
                coedge.sense().reversed()
            } else {
                coedge.sense()
            };
            coedges.push(PlanCoedge::new(target.edge, sense));
            let detour = solid
                .coedge_vertices(*coedge_id)
                .and_then(|(_, end)| corners.splits.get(&end))
                .and_then(|split| split.detours.get(&face));
            if let Some(detour) = detour {
                coedges.extend(detour.iter().cloned());
            }
        }
        if coedges.is_empty() {
            return Err(ShellError::Walls);
        }
        loops.push(coedges);
    }
    Ok(loops)
}

pub(super) fn inner_solid(offsets: &Offsets<'_>, feature: u64) -> Result<Solid, ShellError> {
    let solid = offsets.solid;
    let collapsed = collapsed_faces(offsets);
    for (id, _) in solid.faces() {
        if !collapsed.contains_key(&id) {
            offsets.surface(id)?;
        }
    }
    let vanishing = vanishing_edges(solid, &collapsed)?;
    let clusters = Clusters::new(solid, &vanishing);
    let mut plan = Plan::default();
    let corners = place_corners(offsets, &collapsed, &clusters, &mut plan)?;
    let placed = place_edges(
        offsets, &collapsed, &vanishing, &clusters, &corners, &mut plan,
    )?;
    for (id, face) in solid.faces() {
        if collapsed.contains_key(&id) {
            continue;
        }
        plan.face(PlanFace {
            surface: offsets.surface(id)?,
            sense: face.sense(),
            name: FaceName::shell(feature, face.name()),
            origin: Some(FaceOrigin::Shell { feature }),
            loops: loops(solid, id, &placed, &vanishing, &corners)?,
        });
    }
    let built = plan.build().map_err(|error| match error {
        PlanError::Build(error) => match error.interrupted() {
            Some(interrupted) => ShellError::Cancelled(interrupted),
            None => ShellError::Walls,
        },
        PlanError::Unassembled => ShellError::Walls,
    })?;
    Ok(built.renamed(|name, origin| (name, origin)))
}
