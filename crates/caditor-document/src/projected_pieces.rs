use caditor_kernel::EdgeReference;
use caditor_sketch::EntityId;

use crate::{
    document::{Document, FeatureId, FeatureKind},
    edit::{Edit, Transaction},
    projection::{Outline, ProjectionSource},
};

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedPieces {
    feature: FeatureId,
    entity: EntityId,
    label: String,
    body: FeatureId,
    followed: EdgeReference,
    first: EdgeReference,
    rest: Vec<(EdgeReference, Outline)>,
}

pub(crate) struct PiecesFound {
    pub(crate) body: FeatureId,
    pub(crate) followed: EdgeReference,
    pub(crate) first: EdgeReference,
    pub(crate) rest: Vec<(EdgeReference, Outline)>,
}

impl ProjectedPieces {
    pub(crate) fn new(
        feature: FeatureId,
        entity: EntityId,
        label: String,
        found: PiecesFound,
    ) -> Option<Self> {
        (!found.rest.is_empty()).then_some(Self {
            feature,
            entity,
            label,
            body: found.body,
            followed: found.followed,
            first: found.first,
            rest: found.rest,
        })
    }

    pub fn count(&self) -> usize {
        self.rest.len() + 1
    }

    pub fn action(&self) -> String {
        match self.count() {
            2 => "Project both pieces".to_owned(),
            count => format!("Project the {count} pieces"),
        }
    }

    pub fn update(&self, document: &Document) -> Option<Transaction> {
        let FeatureKind::Sketch(sketch) = &document.feature(self.feature)?.kind else {
            return None;
        };
        let followed = self.source(self.followed);

        if sketch.projections.get(&self.entity) != Some(&followed) {
            return None;
        }

        let mut transaction = document.transaction(format!("{} of {}", self.action(), self.label));

        transaction.edit(Edit::SetSketchProjection {
            feature: self.feature,
            id: self.entity,
            source: Some(self.source(self.first)),
        });
        for (edge, outline) in &self.rest {
            transaction.add_projection(self.feature, self.source(*edge), outline);
        }
        Some(transaction.finish())
    }

    fn source(&self, edge: EdgeReference) -> ProjectionSource {
        ProjectionSource::Edge {
            body: self.body,
            edge,
        }
    }
}
