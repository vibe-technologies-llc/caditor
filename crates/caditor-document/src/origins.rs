use std::collections::BTreeSet;

use caditor_kernel::{EdgeReference, FaceReference};

use crate::document::FeatureId;

pub(crate) fn of_face(face: &FaceReference) -> BTreeSet<FeatureId> {
    face.origin()
        .into_iter()
        .flat_map(|origin| origin.features())
        .map(FeatureId::from_raw)
        .collect()
}

pub(crate) fn of_edge(edge: &EdgeReference) -> BTreeSet<FeatureId> {
    edge.origins()
        .into_iter()
        .flatten()
        .flat_map(|origin| origin.features())
        .map(FeatureId::from_raw)
        .collect()
}
