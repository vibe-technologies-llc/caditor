use caditor_kernel::{EdgeId, FaceId, FaceOrigin, Solid};
use caditor_sketch::EntityId;

use crate::{
    document::{Document, FeatureId},
    hole::Hole,
    pattern::PatternKind,
    primitive::Cap,
};

pub fn origin_feature(origin: FaceOrigin) -> FeatureId {
    FeatureId::from_raw(origin.copy().map_or(origin.feature(), |copy| copy.pattern))
}

pub fn describe_origin(document: &Document, origin: Option<FaceOrigin>) -> String {
    let Some(origin) = origin else {
        return "Face".to_owned();
    };
    let original = describe_made(document, origin.original());
    let Some(copy) = origin.copy() else {
        return original;
    };
    let copier = document.feature(FeatureId::from_raw(copy.pattern));
    let pattern = copier.map_or("a deleted pattern", |feature| feature.name.as_str());
    if copier.is_some_and(|feature| feature.kind.mirror().is_some()) {
        return format!("{pattern} image of {}", lowercase_first(&original));
    }
    if let Some(at_points) = copier
        .and_then(|feature| feature.kind.pattern())
        .filter(|found| matches!(found.kind, PatternKind::Points(_)))
    {
        let copy = at_points.instance_words(copy.index);
        let copy = copy.strip_prefix("the ").unwrap_or(&copy);
        return format!("{pattern} {copy} of {}", lowercase_first(&original));
    }
    let steps = match copy.index {
        [step, 0] => step.to_string(),
        [first, second] => format!("({first}, {second})"),
    };
    format!("{pattern} copy {steps} of {}", lowercase_first(&original))
}

pub(crate) fn lowercase_first(text: &str) -> String {
    let mut characters = text.chars();
    match characters.next() {
        Some(first) if text.starts_with("Face") => first.to_lowercase().chain(characters).collect(),
        _ => text.to_owned(),
    }
}

fn describe_made(document: &Document, origin: FaceOrigin) -> String {
    let Some(feature) = document.feature(FeatureId::from_raw(origin.feature())) else {
        return "Face of a deleted feature".to_owned();
    };
    let name = &feature.name;
    match origin {
        FaceOrigin::Side { entity, .. } if let Some(primitive) = feature.kind.primitive() => {
            format!("{name} {}", primitive.shape.side_name(entity))
        }
        FaceOrigin::StartCap { .. } if let Some(primitive) = feature.kind.primitive() => {
            format!("{name} {}", primitive.shape.cap_name(Cap::Start))
        }
        FaceOrigin::EndCap { .. } if let Some(primitive) = feature.kind.primitive() => {
            format!("{name} {}", primitive.shape.cap_name(Cap::End))
        }
        FaceOrigin::Side { entity, .. } if feature.kind.hole().is_some() => {
            let part = Hole::part_name(entity);
            format!("{name} {part}")
        }
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
        FaceOrigin::Fillet { .. } | FaceOrigin::Chamfer { .. } | FaceOrigin::Copy { .. } => {
            format!("{name} face")
        }
        FaceOrigin::Shell { .. } => format!("{name} inner face"),
        FaceOrigin::Imported { face, .. } => format!("{name} face {}", u64::from(face) + 1),
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
