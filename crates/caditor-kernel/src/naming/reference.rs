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
        Self::capture_in(&EdgeNaming::new(solid), edge)
    }

    pub fn capture_in(naming: &EdgeNaming, edge: EdgeId) -> Option<Self> {
        let entry = naming.edges.get(&edge)?;
        Some(Self::new(entry.name, entry.faces?, entry.ends))
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
        self.resolve_in(&EdgeNaming::new(solid))
    }

    pub fn resolve_in(&self, naming: &EdgeNaming) -> Result<EdgeId, ReferenceError<EdgeId>> {
        let named = naming.named(self.name);
        if let [only] = named {
            return Ok(*only);
        }
        let candidates = if named.is_empty() {
            naming.between(self.faces)
        } else {
            named
        };
        if let [only] = candidates {
            return Ok(*only);
        }
        let scored: Vec<(EdgeId, usize)> = candidates
            .iter()
            .filter_map(|id| {
                let ends = naming.edges.get(id)?.ends;
                let matching = ends.iter().filter(|end| self.ends.contains(end)).count();
                Some((*id, matching))
            })
            .collect();
        let Some(best) = scored
            .iter()
            .map(|(_, score)| *score)
            .max()
            .filter(|best| *best > 0)
        else {
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

#[derive(Debug, Clone, Copy)]
struct NamedEdge {
    name: EdgeName,
    faces: Option<[FaceName; 2]>,
    ends: [VertexName; 2],
}

#[derive(Debug, Clone, Default)]
pub struct EdgeNaming {
    edges: BTreeMap<EdgeId, NamedEdge>,
    by_name: BTreeMap<EdgeName, Vec<EdgeId>>,
    by_faces: BTreeMap<[FaceName; 2], Vec<EdgeId>>,
}

impl EdgeNaming {
    pub fn new(solid: &Solid) -> Self {
        let vertex_names = vertex_names(solid);
        let vertex_name = |vertex: VertexId| {
            vertex_names
                .get(&vertex)
                .copied()
                .unwrap_or_else(|| VertexName::of_faces([]))
        };
        let mut naming = Self::default();
        for (id, edge) in solid.edges() {
            let faces = edge_face_names(solid, id);
            naming.edges.insert(
                id,
                NamedEdge {
                    name: edge.name(),
                    faces,
                    ends: [vertex_name(edge.start()), vertex_name(edge.end())],
                },
            );
            naming.by_name.entry(edge.name()).or_default().push(id);
            if let Some(faces) = faces {
                naming.by_faces.entry(faces).or_default().push(id);
            }
        }
        naming
    }

    fn named(&self, name: EdgeName) -> &[EdgeId] {
        self.by_name.get(&name).map_or(&[], Vec::as_slice)
    }

    fn between(&self, faces: [FaceName; 2]) -> &[EdgeId] {
        self.by_faces.get(&faces).map_or(&[], Vec::as_slice)
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

pub fn vertex_names(solid: &Solid) -> BTreeMap<VertexId, VertexName> {
    faces_around_vertices(solid)
        .into_iter()
        .map(|(vertex, faces)| (vertex, VertexName::of_faces(faces)))
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
