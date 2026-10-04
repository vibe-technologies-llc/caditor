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
    pub fn heap_size(&self) -> usize {
        self.neighbours.len() * size_of::<FaceName>()
    }

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
    origins: [Option<FaceOrigin>; 2],
    ends: [VertexName; 2],
}

impl EdgeReference {
    pub fn new(name: EdgeName, faces: [FaceName; 2], ends: [VertexName; 2]) -> Self {
        Self {
            name,
            faces: sorted(faces),
            origins: [None, None],
            ends: sorted(ends),
        }
    }

    pub fn with_origins(self, origins: [Option<FaceOrigin>; 2]) -> Self {
        Self { origins, ..self }
    }

    pub fn capture(solid: &Solid, edge: EdgeId) -> Option<Self> {
        Self::capture_in(&EdgeNaming::new(solid), edge)
    }

    pub fn capture_in(naming: &EdgeNaming, edge: EdgeId) -> Option<Self> {
        let entry = naming.edges.get(&edge)?;
        let sides = entry.sides?;
        Some(
            Self::new(entry.name, sides.map(|side| side.name), entry.ends)
                .with_origins(sides.map(|side| side.origin)),
        )
    }

    pub fn name(&self) -> EdgeName {
        self.name
    }

    pub fn faces(&self) -> [FaceName; 2] {
        self.faces
    }

    pub fn origins(&self) -> [Option<FaceOrigin>; 2] {
        self.origins
    }

    pub fn ends(&self) -> [VertexName; 2] {
        self.ends
    }

    pub fn resolve(&self, solid: &Solid) -> Result<EdgeId, ReferenceError<EdgeId>> {
        self.resolve_in(&EdgeNaming::new(solid))
    }

    pub fn resolve_in(&self, naming: &EdgeNaming) -> Result<EdgeId, ReferenceError<EdgeId>> {
        self.resolve_with(naming, Fallback::Origins)
    }

    pub fn resolve_by_names_in(
        &self,
        naming: &EdgeNaming,
    ) -> Result<EdgeId, ReferenceError<EdgeId>> {
        self.resolve_with(naming, Fallback::None)
    }

    fn resolve_with(
        &self,
        naming: &EdgeNaming,
        fallback: Fallback,
    ) -> Result<EdgeId, ReferenceError<EdgeId>> {
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
        if candidates.is_empty() {
            return match fallback {
                Fallback::Origins => self.resolve_by_origins(naming),
                Fallback::None => Err(ReferenceError::Missing),
            };
        }
        let scored: Vec<(EdgeId, usize)> = candidates
            .iter()
            .filter_map(|id| Some((*id, self.matching_ends(naming.edges.get(id)?))))
            .filter(|(_, matching)| *matching > 0)
            .collect();
        best_of(scored)
    }

    fn resolve_by_origins(&self, naming: &EdgeNaming) -> Result<EdgeId, ReferenceError<EdgeId>> {
        let scored: Vec<(EdgeId, (usize, usize))> = naming
            .edges
            .iter()
            .filter_map(|(id, edge)| {
                let kept = self.kept_sides(edge.sides?)?;
                Some((*id, (kept, self.matching_ends(edge))))
            })
            .collect();
        best_of(scored)
    }

    fn kept_sides(&self, sides: [NamedSide; 2]) -> Option<usize> {
        let [first, second] = sides;
        let [expected_first, expected_second] = self.expected_sides();
        let paired = |a: &ExpectedSide, b: &ExpectedSide| Some(a.kept(&first)? + b.kept(&second)?);
        paired(&expected_first, &expected_second)
            .into_iter()
            .chain(paired(&expected_second, &expected_first))
            .max()
    }

    fn expected_sides(&self) -> [ExpectedSide; 2] {
        let [first_name, second_name] = self.faces;
        let [first_origin, second_origin] = self.origins;
        [
            ExpectedSide {
                name: first_name,
                origin: first_origin,
            },
            ExpectedSide {
                name: second_name,
                origin: second_origin,
            },
        ]
    }

    fn matching_ends(&self, edge: &NamedEdge) -> usize {
        edge.ends
            .iter()
            .filter(|end| self.ends.contains(end))
            .count()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fallback {
    Origins,
    None,
}

struct ExpectedSide {
    name: FaceName,
    origin: Option<FaceOrigin>,
}

impl ExpectedSide {
    fn kept(&self, side: &NamedSide) -> Option<usize> {
        if side.name == self.name {
            Some(1)
        } else if self.origin.is_some() && side.origin == self.origin {
            Some(0)
        } else {
            None
        }
    }
}

fn best_of<Score: Ord + Copy>(
    scored: Vec<(EdgeId, Score)>,
) -> Result<EdgeId, ReferenceError<EdgeId>> {
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

#[derive(Debug, Clone, Copy)]
struct NamedEdge {
    name: EdgeName,
    sides: Option<[NamedSide; 2]>,
    ends: [VertexName; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct NamedSide {
    name: FaceName,
    origin: Option<FaceOrigin>,
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
            let sides = edge_sides(solid, id);
            naming.edges.insert(
                id,
                NamedEdge {
                    name: edge.name(),
                    sides,
                    ends: [vertex_name(edge.start()), vertex_name(edge.end())],
                },
            );
            naming.by_name.entry(edge.name()).or_default().push(id);
            if let Some(sides) = sides {
                naming
                    .by_faces
                    .entry(sides.map(|side| side.name))
                    .or_default()
                    .push(id);
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

fn edge_sides(solid: &Solid, edge: EdgeId) -> Option<[NamedSide; 2]> {
    let sides: Vec<NamedSide> = edge_faces(solid, edge)
        .into_iter()
        .filter_map(|face| solid.face(face))
        .map(|face| NamedSide {
            name: face.name(),
            origin: face.origin(),
        })
        .collect();
    match sides.as_slice() {
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
