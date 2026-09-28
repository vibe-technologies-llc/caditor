use caditor_kernel::{EdgeId, FaceId, FaceOrigin, Solid};
use caditor_sketch::EntityId;

use crate::document::{Document, FeatureId};

pub fn origin_feature(origin: FaceOrigin) -> FeatureId {
    FeatureId::from_raw(origin.feature())
}

pub fn describe_origin(document: &Document, origin: Option<FaceOrigin>) -> String {
    let Some(origin) = origin else {
        return "Face".to_owned();
    };
    let Some(feature) = document.feature(origin_feature(origin)) else {
        return "Face of a deleted feature".to_owned();
    };
    let name = &feature.name;
    match origin {
        FaceOrigin::Side { entity, .. } => {
            let curve = feature
                .kind
                .solid()
                .and_then(|solid| document.feature(solid.sketch()))
                .and_then(|sketch| sketch.kind.sketch())
                .map_or_else(
                    || "a sketch curve".to_owned(),
                    |sketch| sketch.entity_label(EntityId::from_raw(entity)),
                );
            format!("{name} side from {curve}")
        }
        FaceOrigin::StartCap { .. } => format!("{name} start face"),
        FaceOrigin::EndCap { .. } => format!("{name} end face"),
        FaceOrigin::Fillet { .. } | FaceOrigin::Chamfer { .. } => format!("{name} face"),
        FaceOrigin::Shell { .. } => format!("{name} inner face"),
    }
}

pub fn edge_faces(solid: &Solid, edge: EdgeId) -> Vec<FaceId> {
    let Some(edge) = solid.edge(edge) else {
        return Vec::new();
    };
    let mut faces: Vec<FaceId> = edge
        .coedges()
        .iter()
        .filter_map(|coedge| solid.coedge_face(*coedge))
        .collect();
    faces.dedup();
    faces
}

pub fn describe_edge(document: &Document, solid: &Solid, edge: EdgeId) -> String {
    let origin = |face: FaceId| solid.face(face).and_then(|face| face.origin());
    match edge_faces(solid, edge).as_slice() {
        [first, second] => format!(
            "the edge between {} and {}",
            describe_origin(document, origin(*first)),
            describe_origin(document, origin(*second))
        ),
        _ => "an edge".to_owned(),
    }
}
