use std::{
    collections::{BTreeMap, BTreeSet},
    f64::consts::TAU,
};

use super::edge_faces;
use crate::{
    build::plan::{Plan, PlanCoedge, PlanError, PlanFace},
    curve::{Curve, Line},
    interrupt::{self, Interrupted},
    interval::Interval,
    naming::EdgeName,
    sense::Sense,
    surface::Surface,
    tolerance::LINEAR_RESOLUTION,
    topology::{EdgeId, FaceId, Pcurve, Solid, VertexId},
};

const AREA_SAMPLES: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MergedFaces {
    pub keeper: FaceId,
    pub faces: BTreeSet<FaceId>,
}

impl MergedFaces {
    fn absorbs(&self, face: FaceId) -> bool {
        face != self.keeper && self.faces.contains(&face)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MergeFailure {
    Unmergeable,
    Cancelled(Interrupted),
}

impl From<Interrupted> for MergeFailure {
    fn from(interrupted: Interrupted) -> Self {
        Self::Cancelled(interrupted)
    }
}

#[derive(Clone)]
struct Drawn {
    curve: Curve,
    interval: Interval,
    start: VertexId,
    end: VertexId,
    name: EdgeName,
    borders_keeper: bool,
}

#[derive(Clone)]
struct Walked {
    edge: EdgeId,
    sense: Sense,
    pcurve: Option<Pcurve>,
}

fn walked_from(solid: &Solid, face: FaceId, keeps_pcurve: bool) -> Option<Vec<Vec<Walked>>> {
    solid
        .face(face)?
        .loops()
        .iter()
        .map(|id| {
            solid
                .face_loop(*id)?
                .coedges()
                .iter()
                .map(|coedge| {
                    let coedge = solid.coedge(*coedge)?;
                    Some(Walked {
                        edge: coedge.edge(),
                        sense: coedge.sense(),
                        pcurve: keeps_pcurve.then(|| coedge.pcurve().clone()),
                    })
                })
                .collect()
        })
        .collect()
}

fn ends(solid: &Solid, walked: &Walked) -> Option<(VertexId, VertexId)> {
    let edge = solid.edge(walked.edge)?;
    Some(if walked.sense.is_same() {
        (edge.start(), edge.end())
    } else {
        (edge.end(), edge.start())
    })
}

fn internal_edges(solid: &Solid, group: &MergedFaces) -> BTreeSet<EdgeId> {
    group
        .faces
        .iter()
        .filter_map(|face| walked_from(solid, *face, false))
        .flatten()
        .flatten()
        .map(|walked| walked.edge)
        .filter(|edge| {
            let faces = edge_faces(solid, *edge);
            faces.len() >= 2 && faces.iter().all(|face| group.faces.contains(face))
        })
        .collect()
}

fn chained(solid: &Solid, boundary: Vec<Walked>) -> Option<Vec<Vec<Walked>>> {
    let mut leaving: BTreeMap<VertexId, Walked> = BTreeMap::new();
    for walked in boundary {
        let (start, _) = ends(solid, &walked)?;
        if leaving.insert(start, walked).is_some() {
            return None;
        }
    }
    let mut chains = Vec::new();
    while let Some((first, walked)) = leaving.pop_first() {
        let (_, mut at) = ends(solid, &walked)?;
        let mut chain = vec![walked];
        while at != first {
            let next = leaving.remove(&at)?;
            (_, at) = ends(solid, &next)?;
            chain.push(next);
        }
        chains.push(chain);
    }
    Some(chains)
}

fn enclosed_area(solid: &Solid, surface: &Surface, sense: Sense, chain: &[Walked]) -> Option<f64> {
    let mut outline = Vec::new();
    for walked in chain {
        let edge = solid.edge(walked.edge)?;
        let mut stations: Vec<f64> = edge.interval().split(AREA_SAMPLES).collect();
        if !walked.sense.is_same() {
            stations.reverse();
        }
        outline.extend(
            stations
                .into_iter()
                .map(|parameter| surface.project(edge.curve().point(parameter), None)),
        );
    }
    let turning: f64 = outline
        .iter()
        .zip(outline.iter().cycle().skip(1))
        .map(|(from, to)| from.x * to.y - to.x * from.y)
        .sum();
    Some(turning / 2.0 * sense.sign())
}

fn merged_loops(solid: &Solid, group: &MergedFaces) -> Option<Vec<Vec<Walked>>> {
    let keeper = solid.face(group.keeper)?;
    if !matches!(keeper.surface(), Surface::Plane(_)) {
        return None;
    }
    let internal = internal_edges(solid, group);
    let mut boundary = Vec::new();
    for face in &group.faces {
        let walked = walked_from(solid, *face, *face == group.keeper)?;
        boundary.extend(
            walked
                .into_iter()
                .flatten()
                .filter(|walked| !internal.contains(&walked.edge)),
        );
    }
    let mut outer = Vec::new();
    let mut inner = Vec::new();
    for chain in chained(solid, boundary)? {
        let area = enclosed_area(solid, keeper.surface(), keeper.sense(), &chain)?;
        if area > 0.0 {
            outer.push(chain);
        } else {
            inner.push(chain);
        }
    }
    let [only] = <[Vec<Walked>; 1]>::try_from(outer).ok()?;
    let mut loops = vec![only];
    loops.extend(inner);
    Some(loops)
}

struct Merging {
    drawn: BTreeMap<EdgeId, Drawn>,
    loops: BTreeMap<FaceId, Vec<Vec<Walked>>>,
    joined: BTreeSet<EdgeId>,
}

impl Merging {
    fn edges_at(&self, vertex: VertexId) -> Vec<EdgeId> {
        self.drawn
            .iter()
            .filter(|(_, edge)| edge.start == vertex || edge.end == vertex)
            .map(|(id, _)| *id)
            .collect()
    }

    fn users(&self, edge: EdgeId) -> BTreeSet<FaceId> {
        self.loops
            .iter()
            .filter(|(_, loops)| {
                loops
                    .iter()
                    .flatten()
                    .any(|walked: &Walked| walked.edge == edge)
            })
            .map(|(face, _)| *face)
            .collect()
    }

    fn join_at(&mut self, solid: &Solid, vertex: VertexId) -> Option<()> {
        let [first, second] = self.edges_at(vertex)[..] else {
            return None;
        };
        if self.users(first) != self.users(second) {
            return None;
        }
        let (keep, gone) = (self.drawn.get(&first)?, self.drawn.get(&second)?);
        let joined = joined_edge(solid, keep, gone, vertex)?;
        self.drawn.insert(first, joined);
        self.drawn.remove(&second);
        self.joined.insert(first);
        for chain in self.loops.values_mut().flatten() {
            chain.retain(|walked| walked.edge != second);
        }
        Some(())
    }
}

fn far_end(edge: &Drawn, vertex: VertexId) -> VertexId {
    if edge.start == vertex {
        edge.end
    } else {
        edge.start
    }
}

fn joined_edge(solid: &Solid, keep: &Drawn, gone: &Drawn, vertex: VertexId) -> Option<Drawn> {
    if keep.start == keep.end || gone.start == gone.end {
        return None;
    }
    let point = |id: VertexId| Some(solid.vertex(id)?.point());
    let far = far_end(gone, vertex);
    let (at, reached) = (point(vertex)?, point(far)?);
    let keep_runs_into = keep.end == vertex;
    let (start, end) = if keep_runs_into {
        (keep.start, far)
    } else {
        (far, keep.end)
    };
    let (curve, interval) = match (&keep.curve, &gone.curve) {
        (Curve::Line(_), Curve::Line(_)) => {
            let near = point(far_end(keep, vertex))?;
            if (near - at).dot(reached - at) >= 0.0 {
                return None;
            }
            let (from, to) = (point(start)?, point(end)?);
            let line = Line::through(from, to).ok()?;
            let off = line.point(line.parameter_of(at)).distance(at);
            if off > LINEAR_RESOLUTION {
                return None;
            }
            (Curve::from(line), Interval::new(0.0, from.distance(to))?)
        }
        (Curve::Circle(circle), Curve::Circle(other)) => {
            let same_circle = circle.center().distance(other.center()) <= LINEAR_RESOLUTION
                && (circle.radius() - other.radius()).abs() <= LINEAR_RESOLUTION
                && circle
                    .frame()
                    .normal()
                    .cross(other.frame().normal())
                    .length()
                    <= LINEAR_RESOLUTION;
            if !same_circle {
                return None;
            }
            let sweep = gone.interval.length();
            let interval = if keep_runs_into {
                Interval::new(keep.interval.start(), keep.interval.end() + sweep)?
            } else {
                Interval::new(keep.interval.start() - sweep, keep.interval.end())?
            };
            let along = if keep_runs_into {
                interval.end()
            } else {
                interval.start()
            };
            let lands = keep.curve.point(along).distance(reached) <= LINEAR_RESOLUTION;
            if !lands || interval.length() > TAU {
                return None;
            }
            (keep.curve.clone(), interval)
        }
        _ => return None,
    };
    let name = if keep.borders_keeper || !gone.borders_keeper {
        keep.name
    } else {
        gone.name
    };
    Some(Drawn {
        curve,
        interval,
        start,
        end,
        name,
        borders_keeper: keep.borders_keeper || gone.borders_keeper,
    })
}

fn drawn_edges(
    solid: &Solid,
    groups: &[MergedFaces],
    internal: &BTreeSet<EdgeId>,
) -> BTreeMap<EdgeId, Drawn> {
    solid
        .edges()
        .filter(|(id, _)| !internal.contains(id))
        .map(|(id, edge)| {
            let borders_keeper = edge_faces(solid, id)
                .iter()
                .any(|face| groups.iter().any(|group| group.keeper == *face));
            (
                id,
                Drawn {
                    curve: edge.curve().clone(),
                    interval: edge.interval(),
                    start: edge.start(),
                    end: edge.end(),
                    name: edge.name(),
                    borders_keeper,
                },
            )
        })
        .collect()
}

fn into_plan(solid: &Solid, merging: &Merging) -> Option<Plan> {
    let mut plan = Plan::default();
    let mut points: BTreeMap<VertexId, usize> = BTreeMap::new();
    let mut edges: BTreeMap<EdgeId, usize> = BTreeMap::new();
    for (id, edge) in &merging.drawn {
        let mut placed = |vertex: VertexId| -> Option<usize> {
            if let Some(known) = points.get(&vertex) {
                return Some(*known);
            }
            let made = plan.vertex(solid.vertex(vertex)?.point());
            points.insert(vertex, made);
            Some(made)
        };
        let ends = (placed(edge.start)?, placed(edge.end)?);
        edges.insert(
            *id,
            plan.edge(edge.curve.clone(), edge.interval, ends, edge.name),
        );
    }
    for (id, loops) in &merging.loops {
        let face = solid.face(*id)?;
        let planned = loops
            .iter()
            .map(|chain| {
                let carried = chain.iter().all(|walked| {
                    walked.pcurve.is_some() && !merging.joined.contains(&walked.edge)
                });
                chain
                    .iter()
                    .map(|walked| {
                        let edge = *edges.get(&walked.edge)?;
                        Some(match (&walked.pcurve, carried) {
                            (Some(pcurve), true) => {
                                PlanCoedge::settled(edge, walked.sense, pcurve.clone())
                            }
                            _ => PlanCoedge::new(edge, walked.sense),
                        })
                    })
                    .collect::<Option<Vec<_>>>()
            })
            .collect::<Option<Vec<_>>>()?;
        plan.face(PlanFace {
            surface: face.surface().clone(),
            sense: face.sense(),
            name: face.name(),
            origin: face.origin(),
            loops: planned,
        });
    }
    Some(plan)
}

pub(super) fn merged_faces(solid: &Solid, groups: &[MergedFaces]) -> Result<Solid, MergeFailure> {
    let mut loops: BTreeMap<FaceId, Vec<Vec<Walked>>> = BTreeMap::new();
    let mut internal = BTreeSet::new();
    for (id, _) in solid.faces() {
        interrupt::check()?;
        if groups.iter().any(|group| group.absorbs(id)) {
            continue;
        }
        let walked = match groups.iter().find(|group| group.keeper == id) {
            Some(group) => {
                internal.extend(internal_edges(solid, group));
                merged_loops(solid, group)
            }
            None => walked_from(solid, id, true),
        };
        loops.insert(id, walked.ok_or(MergeFailure::Unmergeable)?);
    }
    let touched: BTreeSet<VertexId> = internal
        .iter()
        .filter_map(|edge| solid.edge(*edge))
        .flat_map(|edge| [edge.start(), edge.end()])
        .collect();
    let mut merging = Merging {
        drawn: drawn_edges(solid, groups, &internal),
        loops,
        joined: BTreeSet::new(),
    };
    for vertex in touched {
        merging.join_at(solid, vertex);
    }
    let plan = into_plan(solid, &merging).ok_or(MergeFailure::Unmergeable)?;
    plan.build().map_err(|error| {
        let built = match error {
            PlanError::Build(error) | PlanError::Labelled { error, .. } => error.interrupted(),
            PlanError::Unassembled => None,
        };
        built.map_or(MergeFailure::Unmergeable, MergeFailure::Cancelled)
    })
}
