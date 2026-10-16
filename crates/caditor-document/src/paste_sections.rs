use caditor_geometry::Plane;
use caditor_kernel::EdgeReference;

use crate::{
    attachment::SketchFeature,
    document::{Document, Feature, FeatureId, FeatureKind},
    origins,
    paste::{Copying, PasteRefusal},
    paste_picks::{Index, Unmatched, choose, refusal},
    projection::ProjectionSource,
    recompute::Evaluation,
    section::{section_reference, section_solid},
};

const TRIAL_LABEL: &str = "Copied sketch";
const SECTION_EDGE: &str = "edge of the cut it projects";

pub(crate) struct Trial {
    pub(crate) document: Document,
    pub(crate) sketch: FeatureId,
}

impl Trial {
    pub(crate) fn of(shown: &Document, feature: &Feature, kind: &FeatureKind) -> Option<Self> {
        let mut stripped = kind.clone();
        if let FeatureKind::Sketch(definition) = &mut stripped {
            definition.projections.clear();
        }
        let mut document = shown.clone();
        let mut builder = document.transaction(TRIAL_LABEL);
        let sketch =
            builder.add_copied_feature(Feature::new(feature.id(), unused_name(shown), stripped));
        document.apply(builder.finish()).ok()?;
        Some(Self { document, sketch })
    }
}

fn unused_name(document: &Document) -> String {
    (0..)
        .map(|number| format!("{TRIAL_LABEL} {number}"))
        .find(|name| document.features().all(|feature| feature.name != *name))
        .unwrap_or_default()
}

pub(crate) fn follows_sections(definition: &SketchFeature, copying: &Copying<'_>) -> bool {
    definition.projections.values().any(|source| match source {
        ProjectionSource::Section { body, edge } => cut_by_copies(*body, edge, copying),
        _ => false,
    })
}

fn cut_by_copies(body: FeatureId, edge: &EdgeReference, copying: &Copying<'_>) -> bool {
    copying.original_of(body) != body
        || origins::of_edge(edge)
            .iter()
            .any(|feature| copying.names.contains_key(feature))
}

pub(crate) struct Sections<'a> {
    pub(crate) originals: &'a Evaluation,
    pub(crate) trial: &'a Evaluation,
    pub(crate) trial_sketch: FeatureId,
    pub(crate) original_sketch: FeatureId,
    pub(crate) copying: &'a Copying<'a>,
}

impl Sections<'_> {
    pub(crate) fn follow(&self, kind: &mut FeatureKind) -> Result<(), PasteRefusal> {
        let FeatureKind::Sketch(definition) = kind else {
            return Ok(());
        };
        for source in definition.projections.values_mut() {
            let ProjectionSource::Section { body, edge } = source else {
                continue;
            };
            if cut_by_copies(*body, edge, self.copying) {
                *edge = self.follow_edge(*body, edge)?;
            }
        }
        Ok(())
    }

    fn follow_edge(
        &self,
        body: FeatureId,
        edge: &EdgeReference,
    ) -> Result<EdgeReference, PasteRefusal> {
        let original_body = self.copying.original_of(body);
        let name = match self.copying.name_of(original_body) {
            named if named.is_empty() => self.copying.first_deep_name(),
            named => named,
        };
        let unreadable = || PasteRefusal::PicksUnreadable { name: name.clone() };
        let uncomputed = || PasteRefusal::CopyUncomputed { name: name.clone() };
        let missing = || refusal(Unmatched::Missing, SECTION_EDGE, &name);

        let original_solid = self
            .originals
            .body_seen_by(self.original_sketch, original_body)
            .ok_or_else(unreadable)?;
        let original_plane =
            plane_of(self.originals, self.original_sketch).ok_or_else(unreadable)?;
        let copy_solid = self.trial.body(body).ok_or_else(uncomputed)?;
        let copy_plane = plane_of(self.trial, self.trial_sketch).ok_or_else(uncomputed)?;

        let original_section = section_solid(original_solid, &original_plane, self.original_sketch)
            .map_err(|_| unreadable())?;
        let copy_section =
            section_solid(copy_solid, &copy_plane, self.trial_sketch).map_err(|_| missing())?;
        let original_edge = edge.resolve(&original_section).map_err(|_| unreadable())?;

        let renamed = self.copying.renamed;
        let rename = |raw: u64| {
            if raw == self.original_sketch.raw() {
                return self.trial_sketch.raw();
            }
            renamed
                .get(&FeatureId::from_raw(raw))
                .map_or(raw, |copy| copy.raw())
        };
        let originals = Index::of(&original_section, &rename);
        let copies = Index::of(&copy_section, &|raw| raw);

        let copy_edge = choose(original_edge, &originals.edges, &copies.edges)
            .map_err(|unmatched| refusal(unmatched, SECTION_EDGE, &name))?;
        let captured =
            section_reference(&copy_section, copy_edge, self.trial_sketch).ok_or_else(missing)?;
        if captured.resolve(&copy_section) != Ok(copy_edge) {
            return Err(refusal(Unmatched::Ambiguous, SECTION_EDGE, &name));
        }
        Ok(captured)
    }
}

fn plane_of(evaluation: &Evaluation, sketch: FeatureId) -> Option<Plane> {
    evaluation
        .feature(sketch)?
        .result
        .as_deref()?
        .sketch()
        .map(|result| result.geometry.plane())
}
