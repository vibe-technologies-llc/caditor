use std::collections::{BTreeMap, BTreeSet};

use caditor_geometry::{Aabb, Point3};
use thiserror::Error;

use super::{
    Offsets, ShellError,
    collapse::{coedge_faces, loop_edges},
    inner::healed_solid,
};
use crate::{
    interrupt::{self, Interrupted},
    surface::Surface,
    topology::{
        BuildError, CoedgeId, CrossingCheck, EdgeId, FaceId, LoopId, ShellId, Solid, SolidBuilder,
        VertexId,
    },
};

const MAX_TRIALS: usize = 16;
const PARALLEL: f64 = 1e-9;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum DeleteError {
    #[error("no face is to be deleted")]
    NothingToDelete,
    #[error("face {0:?} is not part of the solid")]
    MissingFace(FaceId),
    #[error("deleting every face of the solid would leave nothing")]
    Everything,
    #[error("the faces around face {0:?} cannot close the gap it leaves")]
    CannotClose(FaceId),
    #[error("face {0:?} would vanish as the gap closes")]
    Vanishes(FaceId),
    #[error("closing the gap would not leave a valid body")]
    Invalid { faces: Vec<FaceId> },
    #[error("faces {first:?} and {second:?} would cross")]
    Crosses { first: FaceId, second: FaceId },
    #[error(transparent)]
    Cancelled(#[from] Interrupted),
}

pub fn delete_faces(solid: &Solid, faces: &[FaceId]) -> Result<Solid, DeleteError> {
    deleted(solid, faces).or_else(|error| {
        interrupt::check()?;
        Err(error)
    })
}

fn deleted(solid: &Solid, faces: &[FaceId]) -> Result<Solid, DeleteError> {
    if faces.is_empty() {
        return Err(DeleteError::NothingToDelete);
    }
    for face in faces {
        solid.face(*face).ok_or(DeleteError::MissingFace(*face))?;
    }
    let chosen: BTreeSet<FaceId> = faces.iter().copied().collect();
    if chosen.len() == solid.faces().count() {
        return Err(DeleteError::Everything);
    }
    let mut capped = Capped::default();
    let mut banded = Vec::new();
    for component in components(solid, &chosen) {
        interrupt::check()?;
        match cap_loops(solid, &component) {
            Some(loops) => {
                capped.faces.extend(component);
                capped.loops.extend(loops);
            }
            None => banded.extend(component),
        }
    }
    let (opened, renumbered) = if capped.faces.is_empty() {
        let same = solid.faces().map(|(id, _)| (id, id)).collect();
        (solid.clone(), same)
    } else {
        capped.removed(solid)?
    };
    let original: BTreeMap<FaceId, FaceId> =
        renumbered.iter().map(|(old, new)| (*new, *old)).collect();
    let back = |face: FaceId| original.get(&face).copied().unwrap_or(face);
    let bands: Vec<FaceId> = banded
        .iter()
        .filter_map(|face| renumbered.get(face).copied())
        .collect();
    let healed = if bands.is_empty() {
        opened
    } else {
        heal(&opened, &bands).map_err(|error| error.renamed(back))?
    };
    match healed.find_crossing()? {
        CrossingCheck::Crossing(crossing) => {
            let [first, second] = crossing.faces;
            let named = |face: FaceId| {
                let name = healed.face(face).map(|face| face.name());
                solid
                    .faces()
                    .find(|(_, original)| Some(original.name()) == name)
                    .map_or(face, |(id, _)| id)
            };
            Err(DeleteError::Crosses {
                first: named(first),
                second: named(second),
            })
        }
        CrossingCheck::Clear | CrossingCheck::Inconclusive { .. } => Ok(healed),
    }
}

impl DeleteError {
    fn renamed(self, back: impl Fn(FaceId) -> FaceId) -> Self {
        match self {
            Self::MissingFace(face) => Self::MissingFace(back(face)),
            Self::CannotClose(face) => Self::CannotClose(back(face)),
            Self::Vanishes(face) => Self::Vanishes(back(face)),
            Self::Invalid { faces } => Self::Invalid {
                faces: faces.into_iter().map(back).collect(),
            },
            Self::Crosses { first, second } => Self::Crosses {
                first: back(first),
                second: back(second),
            },
            other => other,
        }
    }
}

fn other_faces(solid: &Solid, edge: EdgeId, face: FaceId) -> Vec<FaceId> {
    coedge_faces(solid, edge)
        .into_iter()
        .filter(|other| *other != face)
        .collect()
}

fn components(solid: &Solid, chosen: &BTreeSet<FaceId>) -> Vec<BTreeSet<FaceId>> {
    let mut left = chosen.clone();
    let mut found = Vec::new();
    while let Some(seed) = left.pop_first() {
        let mut component = BTreeSet::from([seed]);
        let mut frontier = vec![seed];
        while let Some(face) = frontier.pop() {
            for edge in loop_edges(solid, face) {
                for other in other_faces(solid, edge, face) {
                    if left.remove(&other) {
                        component.insert(other);
                        frontier.push(other);
                    }
                }
            }
        }
        found.push(component);
    }
    found
}

fn cap_loops(solid: &Solid, component: &BTreeSet<FaceId>) -> Option<BTreeSet<LoopId>> {
    let mut loops = BTreeSet::new();
    for face in component {
        for edge in loop_edges(solid, *face) {
            for coedge in solid.edge(edge)?.coedges() {
                let owner = solid.coedge(*coedge)?.owner();
                let kept = solid.face_loop(owner)?.face();
                if !component.contains(&kept) {
                    loops.insert(owner);
                }
            }
        }
    }
    for owner in &loops {
        let face_loop = solid.face_loop(*owner)?;
        let face = solid.face(face_loop.face())?;
        if face.loops().first() == Some(owner) {
            return None;
        }
        let borders_component = face_loop.coedges().iter().all(|coedge| {
            solid.coedge(*coedge).is_some_and(|coedge| {
                let others = other_faces(solid, coedge.edge(), face_loop.face());
                !others.is_empty() && others.iter().all(|other| component.contains(other))
            })
        });
        if !borders_component {
            return None;
        }
    }
    Some(loops)
}

#[derive(Default)]
struct Capped {
    faces: BTreeSet<FaceId>,
    loops: BTreeSet<LoopId>,
}

impl Capped {
    fn removed(&self, solid: &Solid) -> Result<(Solid, BTreeMap<FaceId, FaceId>), DeleteError> {
        let invalid = |_: BuildError| DeleteError::Invalid {
            faces: self.faces.iter().copied().collect(),
        };
        let mut builder = SolidBuilder::new();
        let mut shells: BTreeMap<ShellId, ShellId> = BTreeMap::new();
        let mut vertices: BTreeMap<VertexId, VertexId> = BTreeMap::new();
        let mut edges: BTreeMap<EdgeId, EdgeId> = BTreeMap::new();
        let mut renumbered = BTreeMap::new();
        for (id, face) in solid.faces() {
            interrupt::check()?;
            if self.faces.contains(&id) {
                continue;
            }
            let shell = match shells.get(&face.shell()) {
                Some(shell) => *shell,
                None => {
                    let made = builder.shell().map_err(invalid)?;
                    shells.insert(face.shell(), made);
                    made
                }
            };
            let made = builder
                .face(shell, face.surface().clone(), face.sense())
                .map_err(invalid)?;
            builder.set_face_name(made, face.name()).map_err(invalid)?;
            if let Some(origin) = face.origin() {
                builder.set_face_origin(made, origin).map_err(invalid)?;
            }
            renumbered.insert(id, made);
            for loop_id in face.loops() {
                if self.loops.contains(loop_id) {
                    continue;
                }
                let Some(face_loop) = solid.face_loop(*loop_id) else {
                    continue;
                };
                let mut coedges = Vec::with_capacity(face_loop.coedges().len());
                for coedge_id in face_loop.coedges() {
                    let coedge = solid
                        .coedge(*coedge_id)
                        .ok_or(BuildError::UnknownFace(id))
                        .map_err(invalid)?;
                    let edge = copied_edge(
                        solid,
                        coedge.edge(),
                        &mut builder,
                        &mut vertices,
                        &mut edges,
                    )
                    .map_err(invalid)?;
                    coedges.push((edge, coedge.sense(), coedge.pcurve().clone()));
                }
                let count = coedges.len();
                let made_loop = builder
                    .add_loop_with_pcurves(made, coedges)
                    .map_err(invalid)?;
                builder.settle(made_loop, &vec![true; count]);
            }
        }
        Ok((builder.build().map_err(invalid)?, renumbered))
    }
}

fn copied_edge(
    solid: &Solid,
    edge_id: EdgeId,
    builder: &mut SolidBuilder,
    vertices: &mut BTreeMap<VertexId, VertexId>,
    edges: &mut BTreeMap<EdgeId, EdgeId>,
) -> Result<EdgeId, BuildError> {
    if let Some(known) = edges.get(&edge_id) {
        return Ok(*known);
    }
    let edge = solid
        .edge(edge_id)
        .ok_or(BuildError::UnknownEdge(edge_id))?;
    let mut copied = |vertex: VertexId| -> Result<VertexId, BuildError> {
        if let Some(known) = vertices.get(&vertex) {
            return Ok(*known);
        }
        let point = solid
            .vertex(vertex)
            .map(|vertex| vertex.point())
            .ok_or(BuildError::UnknownVertex(vertex))?;
        let made = builder.vertex(point)?;
        vertices.insert(vertex, made);
        Ok(made)
    };
    let start = copied(edge.start())?;
    let end = copied(edge.end())?;
    let made = builder.edge(edge.curve().clone(), edge.interval(), start, end)?;
    builder.set_edge_name(made, edge.name())?;
    edges.insert(edge_id, made);
    Ok(made)
}

#[derive(Clone)]
struct Candidate {
    across: BTreeSet<EdgeId>,
    length: f64,
}

fn beyond(solid: &Solid, face: FaceId, coedges: &[(CoedgeId, EdgeId)]) -> Option<Vec<FaceId>> {
    let mut faces = Vec::with_capacity(coedges.len());
    for (_, edge) in coedges {
        let [other] = other_faces(solid, *edge, face)[..] else {
            return None;
        };
        faces.push(other);
    }
    Some(faces)
}

fn never_meet(solid: &Solid, first: &[FaceId], second: &[FaceId]) -> bool {
    let normal = |faces: &[FaceId]| match faces {
        [only] => match solid.face(*only)?.surface() {
            Surface::Plane(plane) => Some(plane.frame().normal()),
            _ => None,
        },
        _ => None,
    };
    match (normal(first), normal(second)) {
        (Some(a), Some(b)) => a.cross(b).length() <= PARALLEL,
        _ => false,
    }
}

fn candidates(solid: &Solid, face: FaceId, deleted: &BTreeSet<FaceId>) -> Vec<Candidate> {
    let Some([only]) = solid.face(face).map(|face| face.loops()) else {
        return Vec::new();
    };
    let Some(face_loop) = solid.face_loop(*only) else {
        return Vec::new();
    };
    let coedges: Vec<(CoedgeId, EdgeId)> = face_loop
        .coedges()
        .iter()
        .filter_map(|id| Some((*id, solid.coedge(*id)?.edge())))
        .collect();
    let kept = |faces: &[FaceId]| faces.iter().all(|other| !deleted.contains(other));
    let length = |edges: &BTreeSet<EdgeId>| {
        edges
            .iter()
            .filter_map(|edge| solid.edge(*edge))
            .map(|edge| edge.curve().length(edge.interval()))
            .sum::<f64>()
    };
    let count = coedges.len();
    let mut found = Vec::new();
    for first in 0..count {
        for second in first + 2..count {
            if first + count - second < 2 {
                continue;
            }
            let (Some(at_first), Some(at_second)) = (coedges.get(first), coedges.get(second))
            else {
                continue;
            };
            let across = BTreeSet::from([at_first.1, at_second.1]);
            let inside = coedges.get(first + 1..second).unwrap_or_default();
            let outside: Vec<(CoedgeId, EdgeId)> = coedges
                .get(second + 1..)
                .unwrap_or_default()
                .iter()
                .chain(coedges.get(..first).unwrap_or_default())
                .copied()
                .collect();
            if inside
                .iter()
                .chain(&outside)
                .any(|(_, edge)| across.contains(edge))
            {
                continue;
            }
            let (Some(near), Some(far)) =
                (beyond(solid, face, inside), beyond(solid, face, &outside))
            else {
                continue;
            };
            if !kept(&near) || !kept(&far) || never_meet(solid, &near, &far) {
                continue;
            }
            found.push(Candidate {
                length: length(&across),
                across,
            });
        }
    }
    found.sort_by(|a, b| a.length.total_cmp(&b.length));
    if count >= 3
        && let Some(around) = beyond(solid, face, &coedges)
        && !around.contains(&face)
    {
        let across: BTreeSet<EdgeId> = coedges.iter().map(|(_, edge)| *edge).collect();
        found.push(Candidate {
            length: length(&across),
            across,
        });
    }
    found
}

fn reach(solid: &Solid, faces: &[FaceId]) -> f64 {
    let points: Vec<Point3> = faces
        .iter()
        .flat_map(|face| loop_edges(solid, *face))
        .filter_map(|edge| solid.edge(edge))
        .flat_map(|edge| {
            let interval = edge.interval();
            [0.0, 0.5, 1.0].map(|fraction| edge.curve().point(interval.at(fraction)))
        })
        .collect();
    Aabb::from_points(points).map_or(0.0, |bounds| bounds.diagonal())
}

fn heal(solid: &Solid, bands: &[FaceId]) -> Result<Solid, DeleteError> {
    let deleted: BTreeSet<FaceId> = bands.iter().copied().collect();
    let offsets = Offsets::still(solid, reach(solid, bands));
    let mut choices = Vec::with_capacity(bands.len());
    for face in bands {
        let found = candidates(solid, *face, &deleted);
        if found.is_empty() {
            return Err(DeleteError::CannotClose(*face));
        }
        choices.push((*face, found));
    }
    let mut first_failure = None;
    for trial in trials(&choices) {
        interrupt::check()?;
        match attempt(&offsets, &deleted, trial) {
            Ok(healed) => return Ok(healed),
            Err(error @ DeleteError::Cancelled(_)) => return Err(error),
            Err(error) => {
                first_failure.get_or_insert(error);
            }
        }
    }
    let failure = first_failure.unwrap_or(DeleteError::Invalid { faces: Vec::new() });
    if bands.len() < 2 {
        return Err(failure);
    }
    for (face, found) in &choices {
        let alone = BTreeSet::from([*face]);
        let single = [(*face, found.clone())];
        let heals_alone = trials(&single)
            .into_iter()
            .any(|trial| attempt(&Offsets::still(solid, offsets.reach), &alone, trial).is_ok());
        interrupt::check()?;
        if !heals_alone {
            return Err(DeleteError::CannotClose(*face));
        }
    }
    Err(failure)
}

fn trials(choices: &[(FaceId, Vec<Candidate>)]) -> Vec<BTreeMap<FaceId, BTreeSet<EdgeId>>> {
    let mut found = Vec::new();
    let mut index = vec![0_usize; choices.len()];
    while found.len() < MAX_TRIALS {
        let trial: BTreeMap<FaceId, BTreeSet<EdgeId>> = choices
            .iter()
            .zip(&index)
            .filter_map(|((face, list), at)| Some((*face, list.get(*at)?.across.clone())))
            .collect();
        found.push(trial);
        let mut place = 0;
        loop {
            let (Some(at), Some((_, list))) = (index.get_mut(place), choices.get(place)) else {
                return found;
            };
            *at += 1;
            if *at < list.len() {
                break;
            }
            *at = 0;
            place += 1;
        }
    }
    found
}

fn attempt(
    offsets: &Offsets<'_>,
    deleted: &BTreeSet<FaceId>,
    trial: BTreeMap<FaceId, BTreeSet<EdgeId>>,
) -> Result<Solid, DeleteError> {
    let first = trial.keys().next().copied();
    let inner = healed_solid(offsets, trial).map_err(|error| match error {
        ShellError::Cancelled(interrupted) => DeleteError::Cancelled(interrupted),
        ShellError::Walls { faces, .. } if !faces.is_empty() => DeleteError::Invalid { faces },
        _ => first.map_or(
            DeleteError::Invalid { faces: Vec::new() },
            DeleteError::CannotClose,
        ),
    })?;
    if let Some(face) = inner.dropped.difference(deleted).next() {
        return Err(DeleteError::Vanishes(*face));
    }
    Ok(inner.solid)
}
