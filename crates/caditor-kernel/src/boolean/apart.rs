use std::collections::BTreeSet;

use caditor_geometry::Aabb;

use crate::{
    boolean::{BooleanError, BooleanOperation, TOLERANCE, face_box},
    intersect::boxes_overlap,
    topology::{Edge, FaceId, Solid, doubly_curved},
};

const APART: f64 = 10.0 * TOLERANCE;

fn enclosure(solid: &Solid) -> Option<Aabb> {
    let bulges = solid
        .faces()
        .filter(|(_, face)| doubly_curved(face.surface()))
        .map(|(id, _)| face_box(solid, id));
    solid
        .outline_box()
        .into_iter()
        .map(Some)
        .chain(bulges)
        .collect::<Option<Vec<Aabb>>>()?
        .into_iter()
        .reduce(Aabb::union)
}

fn edge_faces(solid: &Solid, edge: &Edge) -> [Option<FaceId>; 2] {
    let mut faces = edge
        .coedges()
        .iter()
        .map(|coedge| solid.coedge_face(*coedge));
    let (first, second) = (faces.next().flatten(), faces.next().flatten());
    if faces.next().is_some() {
        return [None; 2];
    }
    [first.min(second), first.max(second)]
}

fn mergeable_faces(solid: &Solid) -> bool {
    solid.edges().any(|(_, edge)| {
        let [Some(first), Some(second)] = edge_faces(solid, edge) else {
            return false;
        };
        if first == second {
            return false;
        }
        let (Some(first), Some(second)) = (solid.face(first), solid.face(second)) else {
            return false;
        };
        first
            .surface()
            .same_surface(second.surface())
            .is_some_and(|relation| relation.combined(first.sense()) == second.sense())
    })
}

#[derive(Debug, Clone, Copy, Default)]
enum Around {
    #[default]
    Unused,
    One([Option<FaceId>; 2]),
    Two([Option<FaceId>; 2], [Option<FaceId>; 2]),
    More,
}

impl Around {
    fn with(self, faces: [Option<FaceId>; 2]) -> Self {
        match self {
            Self::Unused => Self::One(faces),
            Self::One(first) => Self::Two(first, faces),
            Self::Two(..) | Self::More => Self::More,
        }
    }
}

fn joinable_edges(solid: &Solid) -> bool {
    let mut around = vec![Around::Unused; solid.vertices().count()];
    for (_, edge) in solid.edges() {
        let faces = edge_faces(solid, edge);
        let open = edge.start() != edge.end();
        for vertex in [Some(edge.start()), open.then_some(edge.end())]
            .into_iter()
            .flatten()
        {
            if let Some(slot) = around.get_mut(vertex.index()) {
                *slot = slot.with(faces);
            }
        }
    }
    around
        .iter()
        .any(|around| matches!(around, Around::Two(first, second) if first == second))
}

fn settled(solids: &[&Solid]) -> bool {
    let mut names = BTreeSet::new();
    let unique = solids
        .iter()
        .flat_map(|solid| solid.edges())
        .all(|(_, edge)| names.insert(edge.name()));
    unique
        && solids
            .iter()
            .all(|solid| !mergeable_faces(solid) && !joinable_edges(solid))
}

pub(super) fn combine_apart(
    first: &Solid,
    second: &Solid,
    operation: BooleanOperation,
) -> Option<Result<Solid, BooleanError>> {
    let (first_box, second_box) = (enclosure(first)?, enclosure(second)?);
    if boxes_overlap(&first_box, &second_box, APART) {
        return None;
    }
    match operation {
        BooleanOperation::Intersection => Some(Err(BooleanError::Empty)),
        BooleanOperation::Difference => settled(&[first]).then(|| Ok(first.clone())),
        BooleanOperation::Union => settled(&[first, second])
            .then(|| first.beside(second))
            .flatten()
            .map(Ok),
    }
}
