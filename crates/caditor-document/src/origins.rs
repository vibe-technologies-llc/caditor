use std::collections::BTreeSet;

use caditor_kernel::{EdgeReference, FaceReference};

use crate::document::FeatureId;

pub(crate) fn of_face(face: &FaceReference) -> Option<FeatureId> {
    face.origin()
        .map(|origin| FeatureId::from_raw(origin.feature()))
}

pub(crate) fn of_edge(edge: &EdgeReference) -> BTreeSet<FeatureId> {
    edge.origins()
        .into_iter()
        .flatten()
        .map(|origin| FeatureId::from_raw(origin.feature()))
        .collect()
}
