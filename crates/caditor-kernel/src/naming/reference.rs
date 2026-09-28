use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use super::{EdgeName, FaceName, FaceOrigin, VertexName};
use crate::topology::{EdgeId, FaceId, Solid, VertexId};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ReferenceError<Id> {
    #[error("the referenced topology no longer exists")]
    Missing,
    #[error("the reference matches several candidates equally well")]
    Ambiguous(Vec<Id>),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FaceReference {
    name: FaceName,
    origin: Option<FaceOrigin>,
    neighbours: BTreeSet<FaceName>,
}

impl FaceReference {
    pub fn new(
        name: FaceName,
        origin: Option<FaceOrigin>,
        neighbours: impl IntoIterator<Item = FaceName>,
    ) -> Self {
        Self {
            name,
            origin,
            neighbours: neighbours.into_iter().collect(),
        }
    }

    pub fn capture(solid: &Solid, face: FaceId) -> Option<Self> {
        let definition = solid.face(face)?;
        Some(Self {
            name: definition.name(),
            origin: definition.origin(),
            neighbours: neighbour_names(solid, face),
        })
    }

    pub fn name(&self) -> FaceName {
        self.name
    }

    pub fn origin(&self) -> Option<FaceOrigin> {
        self.origin
    }

    pub fn neighbours(&self) -> &BTreeSet<FaceName> {
        &self.neighbours
    }

    pub fn resolve(&self, solid: &Solid) -> Result<FaceId, ReferenceError<FaceId>> {
        let named: Vec<FaceId> = solid
            .faces()
            .filter(|(_, face)| face.name() == self.name)
            .map(|(id, _)| id)
            .collect();
        if let [only] = named.as_slice() {
            let found = neighbour_names(solid, *only);
            if self.neighbours.is_empty() || !self.neighbours.is_disjoint(&found) {
                return Ok(*only);
            }
        } else if !named.is_empty() {
            return self.best_by_neighbours(solid, &named, 0);
        }
        let Some(origin) = self.origin else {
            return Err(ReferenceError::Missing);
        };
        let same_origin: Vec<FaceId> = solid
            .faces()
            .filter(|(_, face)| face.origin() == Some(origin))
            .map(|(id, _)| id)
            .collect();
        self.best_by_neighbours(solid, &same_origin, 1)
    }

    fn best_by_neighbours(
        &self,
        solid: &Solid,
        candidates: &[FaceId],
        least_shared: usize,
    ) -> Result<FaceId, ReferenceError<FaceId>> {
        let scored: Vec<(FaceId, Match)> = candidates
            .iter()
            .map(|face| {
                let found = neighbour_names(solid, *face);
                (*face, Match::between(&self.neighbours, &found))
            })
            .filter(|(_, score)| score.shared >= least_shared)
            .collect();
        let Some(best) = scored.iter().map(|(_, score)| *score).max() else {
            return Err(ReferenceError::Missing);
        };
        match scored
            .iter()
            .filter(|(_, score)| *score == best)
            .map(|(face, _)| *face)
            .collect::<Vec<FaceId>>()
            .as_slice()
        {
            [only] => Ok(*only),
            tied => Err(ReferenceError::Ambiguous(tied.to_vec())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Match {
    shared: usize,
    fewer_differences: std::cmp::Reverse<usize>,
}

impl Match {
    fn between(expected: &BTreeSet<FaceName>, found: &BTreeSet<FaceName>) -> Self {
        let shared = expected.intersection(found).count();
        Self {
            shared,
            fewer_differences: std::cmp::Reverse(expected.len() + found.len() - 2 * shared),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EdgeReference {
    name: EdgeName,
    faces: [FaceName; 2],
    ends: [VertexName; 2],
}

impl EdgeReference {
    pub fn new(name: EdgeName, faces: [FaceName; 2], ends: [VertexName; 2]) -> Self {
        Self {
            name,
            faces: sorted(faces),
            ends: sorted(ends),
        }
    }

    pub fn capture(solid: &Solid, edge: EdgeId) -> Option<Self> {
        let definition = solid.edge(edge)?;
        let around = faces_around_vertices(solid);
        Some(Self::new(
            definition.name(),
            edge_face_names(solid, edge)?,
            [
                vertex_name(&around, definition.start()),
                vertex_name(&around, definition.end()),
            ],
        ))
    }

    pub fn name(&self) -> EdgeName {
        self.name
    }

    pub fn faces(&self) -> [FaceName; 2] {
        self.faces
    }

    pub fn ends(&self) -> [VertexName; 2] {
        self.ends
    }

    pub fn resolve(&self, solid: &Solid) -> Result<EdgeId, ReferenceError<EdgeId>> {
        let named: Vec<EdgeId> = solid
            .edges()
            .filter(|(_, edge)| edge.name() == self.name)
            .map(|(id, _)| id)
            .collect();
        if let [only] = named.as_slice() {
            return Ok(*only);
        }
        let between: Vec<EdgeId> = solid
            .edges()
            .map(|(id, _)| id)
            .filter(|id| edge_face_names(solid, *id) == Some(self.faces))
            .collect();
        let candidates = if named.is_empty() { between } else { named };
        if let [only] = candidates.as_slice() {
            return Ok(*only);
        }
        let around = faces_around_vertices(solid);
        let scored: Vec<(EdgeId, usize)> = candidates
            .iter()
            .filter_map(|id| {
                let edge = solid.edge(*id)?;
                let ends = [
                    vertex_name(&around, edge.start()),
                    vertex_name(&around, edge.end()),
                ];
                let matching = ends.iter().filter(|end| self.ends.contains(end)).count();
                Some((*id, matching))
            })
            .collect();
        let Some(best) = scored.iter().map(|(_, score)| *score).max() else {
            return Err(ReferenceError::Missing);
        };
        match scored
            .iter()
            .filter(|(_, score)| *score == best)
            .map(|(id, _)| *id)
            .collect::<Vec<EdgeId>>()
            .as_slice()
        {
            [only] => Ok(*only),
            tied => Err(ReferenceError::Ambiguous(tied.to_vec())),
        }
    }
}

fn sorted<T: Ord + Copy>([first, second]: [T; 2]) -> [T; 2] {
    if first <= second {
        [first, second]
    } else {
        [second, first]
    }
}

fn edge_faces(solid: &Solid, edge: EdgeId) -> Vec<FaceId> {
    solid
        .edge(edge)
        .into_iter()
        .flat_map(|edge| edge.coedges())
        .filter_map(|coedge| solid.coedge_face(*coedge))
        .collect()
}

fn edge_face_names(solid: &Solid, edge: EdgeId) -> Option<[FaceName; 2]> {
    let names: Vec<FaceName> = edge_faces(solid, edge)
        .into_iter()
        .filter_map(|face| solid.face(face).map(|face| face.name()))
        .collect();
    match names.as_slice() {
        [first, second] => Some(sorted([*first, *second])),
        _ => None,
    }
}

pub(crate) fn neighbour_names(solid: &Solid, face: FaceId) -> BTreeSet<FaceName> {
    let Some(definition) = solid.face(face) else {
        return BTreeSet::new();
    };
    definition
        .loops()
        .iter()
        .filter_map(|id| solid.face_loop(*id))
        .flat_map(|face_loop| face_loop.coedges())
        .filter_map(|coedge| solid.coedge(*coedge))
        .flat_map(|coedge| edge_faces(solid, coedge.edge()))
        .filter(|other| *other != face)
        .filter_map(|other| solid.face(other).map(|other| other.name()))
        .collect()
}

fn faces_around_vertices(solid: &Solid) -> BTreeMap<VertexId, BTreeSet<FaceName>> {
    let mut around: BTreeMap<VertexId, BTreeSet<FaceName>> = BTreeMap::new();
    for (id, edge) in solid.edges() {
        let names: Vec<FaceName> = edge_faces(solid, id)
            .into_iter()
            .filter_map(|face| solid.face(face).map(|face| face.name()))
            .collect();
        for vertex in [edge.start(), edge.end()] {
            around
                .entry(vertex)
                .or_default()
                .extend(names.iter().copied());
        }
    }
    around
}

fn vertex_name(around: &BTreeMap<VertexId, BTreeSet<FaceName>>, vertex: VertexId) -> VertexName {
    VertexName::of_faces(around.get(&vertex).into_iter().flatten().copied())
}
