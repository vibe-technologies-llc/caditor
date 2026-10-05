use std::collections::{BTreeMap, BTreeSet};

use caditor_geometry::{Plane, Point2, Point3, Vector3};

use super::{
    OFFSET_TOLERANCE, Offsets, ShellError,
    collapse::{Collapses, coedge_faces},
    edge::{offset_edge, reverses},
    split::{Split, split_vertex},
};
use crate::{
    build::plan::{Plan, PlanCoedge, PlanError, PlanFace},
    curve::Curve,
    interrupt,
    naming::{EdgeName, FaceName, FaceOrigin},
    surface::Surface,
    tolerance::LINEAR_RESOLUTION,
    topology::{EdgeId, FaceId, Solid, VertexId},
};

const VERTEX_ITERATIONS: usize = 40;
const INDEPENDENT_ROW: f64 = 1e-6;
const PARALLEL: f64 = 1e-6;

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

fn faces_at(solid: &Solid) -> BTreeMap<VertexId, BTreeSet<FaceId>> {
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

fn edges_at(solid: &Solid) -> BTreeMap<VertexId, Vec<EdgeId>> {
    let mut at: BTreeMap<VertexId, Vec<EdgeId>> = BTreeMap::new();
    for (id, edge) in solid.edges() {
        at.entry(edge.start()).or_default().push(id);
        if edge.end() != edge.start() {
            at.entry(edge.end()).or_default().push(id);
        }
    }
    at
}

#[derive(Debug, Clone, Copy)]
pub(super) struct SeamPlane {
    origin: Point3,
    normal: Vector3,
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

fn rows_at(
    offsets: &Offsets<'_>,
    hints: &[(FaceId, Point2)],
    seams: &[SeamPlane],
    point: Point3,
) -> Option<Vec<(f64, Vector3)>> {
    let mut rows = Vec::with_capacity(hints.len() + seams.len());
    for (face, hint) in hints {
        rows.push(offsets.residual(*face, point, Some(*hint))?);
    }
    rows.extend(
        seams
            .iter()
            .map(|seam| ((point - seam.origin).dot(seam.normal), seam.normal)),
    );
    Some(rows)
}

pub(super) fn offset_vertex(
    offsets: &Offsets<'_>,
    faces: &BTreeSet<FaceId>,
    seams: &[SeamPlane],
    start: Point3,
) -> Option<Point3> {
    let hints: Vec<(FaceId, Point2)> = faces
        .iter()
        .filter_map(|face| {
            let surface = offsets.solid.face(*face)?.surface();
            Some((*face, surface.project(start, None)))
        })
        .collect();
    if hints.len() != faces.len() || hints.is_empty() {
        return None;
    }
    let mut point = start;
    for _ in 0..VERTEX_ITERATIONS {
        let rows = rows_at(offsets, &hints, seams, point)?;
        if rows
            .iter()
            .all(|(residual, _)| residual.abs() <= 0.01 * LINEAR_RESOLUTION)
        {
            return Some(point);
        }
        point += minimal_step(&rows)?;
    }
    let settled = rows_at(offsets, &hints, seams, point)?
        .iter()
        .all(|(residual, _)| residual.abs() <= OFFSET_TOLERANCE);
    settled.then_some(point)
}

pub(super) fn outward_normal(solid: &Solid, face: FaceId, point: Point3) -> Option<Vector3> {
    let face = solid.face(face)?;
    let surface = face.surface();
    let uv = surface.project(point, None);
    Some(surface.normal(uv.x, uv.y)? * face.sense().sign())
}

fn rotational_frame(surface: &Surface) -> Option<&Plane> {
    match surface {
        Surface::Cylinder(cylinder) => Some(cylinder.frame()),
        Surface::Cone(cone) => Some(cone.frame()),
        Surface::Sphere(sphere) => Some(sphere.frame()),
        Surface::Torus(torus) => Some(torus.frame()),
        _ => None,
    }
}

fn seam_plane(
    solid: &Solid,
    live: &BTreeSet<FaceId>,
    edge: EdgeId,
    point: Point3,
) -> Option<SeamPlane> {
    let [first, second] = coedge_faces(solid, edge)[..] else {
        return None;
    };
    if first != second || !live.contains(&first) {
        return None;
    }
    let frame = rotational_frame(solid.face(first)?.surface())?;
    let axis = frame.normal();
    let along_axis = match solid.edge(edge)?.curve() {
        Curve::Line(_) => true,
        Curve::Circle(circle) => circle.frame().normal().dot(axis).abs() <= PARALLEL,
        _ => false,
    };
    if !along_axis {
        return None;
    }
    let radial = (point - frame.origin()).reject_from(axis).try_normalize()?;
    Some(SeamPlane {
        origin: frame.origin(),
        normal: axis.cross(radial),
    })
}

fn underdetermined(offsets: &Offsets<'_>, faces: &BTreeSet<FaceId>, point: Point3) -> bool {
    let rows: Vec<(f64, Vector3)> = faces
        .iter()
        .filter_map(|face| offsets.residual(*face, point, None))
        .collect();
    independent(&rows).len() < 3
}

fn pole_face(
    solid: &Solid,
    collapses: &Collapses,
    members: &[VertexId],
    around: &BTreeMap<VertexId, BTreeSet<FaceId>>,
    live: &BTreeSet<FaceId>,
) -> Option<FaceId> {
    let faces_of = |vertex: &VertexId| around.get(vertex).into_iter().flatten().copied();
    let apex = members
        .iter()
        .flat_map(faces_of)
        .find_map(|face| collapses.apex_of(face));
    apex.or_else(|| {
        members.iter().find_map(|vertex| {
            let point = solid.vertex(*vertex)?.point();
            faces_of(vertex)
                .filter(|face| live.contains(face))
                .find(|face| {
                    solid.face(*face).is_some_and(|definition| {
                        let surface = definition.surface();
                        surface.pole_at(surface.project(point, None)).is_some()
                    })
                })
        })
    })
}

fn at_pole(
    offsets: &Offsets<'_>,
    face: FaceId,
    live: &BTreeSet<FaceId>,
    start: Point3,
) -> Result<Option<Point3>, ShellError> {
    let surface = offsets.surface(face)?;
    let Some(pole) = surface
        .poles()
        .into_iter()
        .map(|pole| pole.point)
        .min_by(|a, b| a.distance(start).total_cmp(&b.distance(start)))
    else {
        return Ok(None);
    };
    Ok(offsets
        .on_both(&live.iter().copied().collect::<Vec<_>>(), pole)
        .then_some(pole))
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
    collapses: &Collapses,
    clusters: &Clusters,
    plan: &mut Plan,
) -> Result<Corners, ShellError> {
    let solid = offsets.solid;
    let around = faces_at(solid);
    let edges = edges_at(solid);
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
            .filter(|face| !collapses.contains(**face))
            .copied()
            .collect();
        let points: Vec<Point3> = members
            .iter()
            .filter_map(|vertex| Some(solid.vertex(*vertex)?.point()))
            .collect();
        let start = points.iter().fold(Point3::ZERO, |sum, point| sum + *point)
            / points.len().max(1) as f64;
        let placed = match pole_face(solid, collapses, &members, &around, &live) {
            Some(face) => at_pole(offsets, face, &live, start)?,
            None => {
                let seams: Vec<SeamPlane> = if underdetermined(offsets, &live, start) {
                    members
                        .iter()
                        .flat_map(|vertex| {
                            let point = solid.vertex(*vertex).map(|vertex| vertex.point());
                            let live = &live;
                            edges
                                .get(vertex)
                                .into_iter()
                                .flatten()
                                .filter_map(move |edge| seam_plane(solid, live, *edge, point?))
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                offset_vertex(offsets, &live, &seams, start)
            }
        };
        if let Some(point) = placed {
            collapses.closes_over(offsets, &every, point)?;
            corners.points.insert(root, plan.vertex(point));
            continue;
        }
        if let Some(refusal) = every
            .iter()
            .find_map(|face| collapses.refusal(solid, *face))
        {
            return Err(refusal);
        }
        let split = split_vertex(offsets, root, plan).ok_or(ShellError::Corner(root))?;
        corners.splits.insert(root, split);
    }
    Ok(corners)
}

struct Layout {
    vanishing: BTreeSet<EdgeId>,
    clusters: Clusters,
    corners: Corners,
    plan: Plan,
}

impl Layout {
    fn new(offsets: &Offsets<'_>, collapses: &Collapses) -> Result<Self, ShellError> {
        let vanishing = collapses.vanishing_edges(offsets.solid);
        let clusters = Clusters::new(offsets.solid, &vanishing);
        let mut plan = Plan::default();
        let corners = place_corners(offsets, collapses, &clusters, &mut plan)?;
        Ok(Self {
            vanishing,
            clusters,
            corners,
            plan,
        })
    }

    fn ends(&self, solid: &Solid, edge: EdgeId) -> Option<(usize, usize)> {
        let definition = solid.edge(edge)?;
        Some((
            self.corners.end(&self.clusters, definition.start(), edge)?,
            self.corners.end(&self.clusters, definition.end(), edge)?,
        ))
    }

    fn reversed_edges(&self, solid: &Solid) -> BTreeSet<EdgeId> {
        solid
            .edges()
            .filter(|(id, edge)| !self.vanishing.contains(id) && !edge.is_closed())
            .filter(|(id, edge)| {
                let Some((start, end)) = self.ends(solid, *id) else {
                    return false;
                };
                match (self.plan.point(start), self.plan.point(end)) {
                    (Some(from), Some(to)) => reverses(edge, from, to),
                    _ => false,
                }
            })
            .map(|(id, _)| id)
            .collect()
    }

    fn shrinking_faces(
        &self,
        solid: &Solid,
        collapses: &Collapses,
    ) -> BTreeMap<FaceId, BTreeSet<EdgeId>> {
        let reversed = self.reversed_edges(solid);
        let mut shrinking = BTreeMap::new();
        if reversed.is_empty() {
            return shrinking;
        }
        for (id, face) in solid.faces() {
            if collapses.contains(id) {
                continue;
            }
            let [only] = face.loops() else {
                continue;
            };
            let Some(face_loop) = solid.face_loop(*only) else {
                continue;
            };
            let edges: Vec<EdgeId> = face_loop
                .coedges()
                .iter()
                .filter_map(|coedge| Some(solid.coedge(*coedge)?.edge()))
                .filter(|edge| !self.vanishing.contains(edge))
                .collect();
            if let Some(across) = shrinks_across(&edges, &reversed) {
                shrinking.insert(id, across);
            }
        }
        shrinking
    }
}

fn shrinks_across(edges: &[EdgeId], reversed: &BTreeSet<EdgeId>) -> Option<BTreeSet<EdgeId>> {
    let count = edges.len();
    let across: BTreeSet<EdgeId> = edges
        .iter()
        .filter(|edge| reversed.contains(edge))
        .copied()
        .collect();
    let kept: Vec<usize> = (0..count)
        .filter(|index| {
            edges
                .get(*index)
                .is_some_and(|edge| !reversed.contains(edge))
        })
        .collect();
    let closes = match kept[..] {
        [] => !across.is_empty(),
        [first, second] => {
            let apart = second - first > 1 && first + count - second > 1;
            apart && edges.get(first) != edges.get(second)
        }
        _ => false,
    };
    closes.then_some(across)
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
    collapses: &Collapses,
    layout: &mut Layout,
) -> Result<BTreeMap<EdgeId, Placed>, ShellError> {
    let solid = offsets.solid;
    let mut placed = BTreeMap::new();
    let mut halves: BTreeMap<(usize, usize), Vec<Half>> = BTreeMap::new();
    for (id, edge) in solid.edges() {
        interrupt::check()?;
        if layout.vanishing.contains(&id) {
            continue;
        }
        let ends = layout
            .ends(solid, id)
            .ok_or(ShellError::UnsupportedEdge(id))?;
        let faces = coedge_faces(solid, id);
        let live: Vec<FaceId> = faces
            .iter()
            .filter(|face| !collapses.contains(**face))
            .copied()
            .collect();
        if live.len() == faces.len() {
            let mut distinct = live.clone();
            distinct.dedup();
            let index = new_edge(offsets, id, &distinct, ends, edge.name(), &mut layout.plan)?;
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
            .find(|face| collapses.contains(**face))
            .copied()
            .ok_or(ShellError::UnsupportedEdge(id))?;
        let [face] = live.as_slice() else {
            return Err(collapses
                .refusal(solid, dead)
                .unwrap_or(ShellError::TooCurved(dead)));
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
            let lone = group.first().map(|half| half.edge);
            let dead = lone.and_then(|id| {
                coedge_faces(solid, id)
                    .into_iter()
                    .find(|face| collapses.contains(*face))
            });
            return Err(dead
                .and_then(|face| collapses.refusal(solid, face))
                .unwrap_or(ShellError::Walls {
                    faces: Vec::new(),
                    edge: lone,
                }));
        };
        let name = |face: FaceId| solid.face(face).map_or(FaceName::NONE, |face| face.name());
        let index = new_edge(
            offsets,
            *first,
            &[*first_face, *second_face],
            *ends,
            EdgeName::between(name(*first_face), name(*second_face)),
            &mut layout.plan,
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
    let (curve, interval) = offset_edge(offsets, source, faces, (from, to))?;
    Ok(plan.edge(curve, interval, (start, end), name))
}

fn loops(
    solid: &Solid,
    face: FaceId,
    placed: &BTreeMap<EdgeId, Placed>,
    layout: &Layout,
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
            if layout.vanishing.contains(&coedge.edge()) {
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
                .and_then(|(_, end)| layout.corners.splits.get(&end))
                .and_then(|split| split.detours.get(&face));
            if let Some(detour) = detour {
                coedges.extend(detour.iter().cloned());
            }
        }
        if coedges.is_empty() {
            return Err(ShellError::walls_at([face]));
        }
        loops.push(coedges);
    }
    Ok(loops)
}

pub(super) struct Inner {
    pub solid: Solid,
    pub dropped: BTreeSet<FaceId>,
}

fn settled_layout(offsets: &Offsets<'_>, collapses: &mut Collapses) -> Result<Layout, ShellError> {
    let solid = offsets.solid;
    let mut unsettled = Vec::new();
    for _ in 0..=solid.faces().count() {
        interrupt::check()?;
        let layout = Layout::new(offsets, collapses)?;
        let shrinking = layout.shrinking_faces(solid, collapses);
        if shrinking.is_empty() {
            return Ok(layout);
        }
        unsettled = shrinking.keys().copied().collect();
        collapses.add_shrinking(shrinking);
    }
    Err(ShellError::walls_at(unsettled))
}

pub(super) fn inner_solid(offsets: &Offsets<'_>, feature: u64) -> Result<Inner, ShellError> {
    let solid = offsets.solid;
    let mut collapses = Collapses::of(offsets);
    for (id, _) in solid.faces() {
        if !collapses.contains(id) {
            offsets.surface(id)?;
        }
    }
    let mut layout = settled_layout(offsets, &mut collapses)?;
    let placed = place_edges(offsets, &collapses, &mut layout)?;
    let mut faces = Vec::new();
    for (id, face) in solid.faces() {
        if collapses.contains(id) {
            continue;
        }
        faces.push((
            id,
            PlanFace {
                surface: offsets.surface(id)?,
                sense: face.sense(),
                name: FaceName::shell(feature, face.name()),
                origin: Some(FaceOrigin::Shell { feature }),
                loops: loops(solid, id, &placed, &layout)?,
            },
        ));
    }
    let mut plan = layout.plan;
    for (id, face) in faces {
        plan.label(vec![id.index() as u64]);
        plan.face(face);
    }
    let built = plan.build().map_err(|error| match error {
        PlanError::Build(error) => match error.interrupted() {
            Some(interrupted) => ShellError::Cancelled(interrupted),
            None => ShellError::walls_at([]),
        },
        PlanError::Labelled { error, labels } => match error.interrupted() {
            Some(interrupted) => ShellError::Cancelled(interrupted),
            None => ShellError::walls_at(
                labels
                    .into_iter()
                    .filter_map(|label| FaceId::from_index(label as usize)),
            ),
        },
        PlanError::Unassembled => ShellError::walls_at([]),
    })?;
    Ok(Inner {
        solid: built.renamed(|name, origin| (name, origin)),
        dropped: collapses.faces(),
    })
}
