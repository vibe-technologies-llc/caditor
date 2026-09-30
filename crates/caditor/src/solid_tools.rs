use caditor_document::{
    AxisReference, BodyOperation, Document, Edit, Extrude, ExtrudeExtent, FeatureId, FeatureKind,
    RegionChoice, Revolve, RevolveAxis, RevolveExtent, SolidFeature, Transaction, describe_axis,
};
use caditor_expression::{Expression, Unit};
use caditor_kernel::RegionKey;
use caditor_sketch::{Entity, EntityId, Reference};

use crate::{
    editing::{self, EditingCommand, SketchEditing},
    model::{Action, Model},
    scene,
    selection::{self, Pickable, Selection},
    units::LengthUnit,
    visibility,
};

pub const DEFAULT_DISTANCE: f64 = 10.0;
pub const DEFAULT_PARTIAL_ANGLE: f64 = 180.0;
pub const DEFAULT_BACKWARD_ANGLE: f64 = 30.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sweep {
    Extrude,
    Revolve,
}

impl Sweep {
    pub const ALL: [Self; 2] = [Self::Extrude, Self::Revolve];

    pub fn label(self) -> &'static str {
        match self {
            Self::Extrude => "Extrude",
            Self::Revolve => "Revolve",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SweepSource {
    pub sketch: FeatureId,
    pub axis: Option<RevolveAxis>,
}

pub fn sweep_source(
    document: &Document,
    selection: &Selection,
    editing: &SketchEditing,
) -> Option<SweepSource> {
    let selected_sketch = selection.iter().find_map(|pickable| match pickable {
        Pickable::SketchEntity { feature, .. } => Some(feature),
        Pickable::Origin
        | Pickable::Axis(_)
        | Pickable::Plane(_)
        | Pickable::SketchConstraint { .. }
        | Pickable::Face { .. }
        | Pickable::Edge { .. }
        | Pickable::Vertex { .. }
        | Pickable::Region { .. }
        | Pickable::BlendEdge { .. }
        | Pickable::ShellFace { .. }
        | Pickable::Datum(_) => None,
    });
    let opened_sketch = editing
        .solid()
        .and_then(|solid| document.feature(solid))
        .and_then(|feature| feature.kind.solid())
        .map(SolidFeature::sketch);
    let last_sketch = document
        .features()
        .rev()
        .find(|feature| feature.kind.sketch().is_some())
        .map(|feature| feature.id());
    let sketch = editing
        .feature()
        .or(selected_sketch)
        .or(opened_sketch)
        .or(last_sketch)?;
    let definition = editing::edited_sketch(document, sketch)?;
    let axis = selection.iter().find_map(|pickable| match pickable {
        Pickable::SketchEntity { feature, entity } if feature == sketch => {
            let is_axis = match entity.reference() {
                Some(Reference::HorizontalAxis | Reference::VerticalAxis) => true,
                Some(Reference::Origin) => false,
                None => matches!(definition.entity(entity), Some(Entity::Line { .. })),
            };
            is_axis.then_some(RevolveAxis::Sketch(entity))
        }
        Pickable::Origin
        | Pickable::Axis(_)
        | Pickable::Plane(_)
        | Pickable::SketchEntity { .. }
        | Pickable::SketchConstraint { .. }
        | Pickable::Face { .. }
        | Pickable::Edge { .. }
        | Pickable::Vertex { .. }
        | Pickable::Region { .. }
        | Pickable::BlendEdge { .. }
        | Pickable::ShellFace { .. }
        | Pickable::Datum(_) => None,
    });
    Some(SweepSource { sketch, axis })
}

pub fn with_model_axis(source: SweepSource, axis: Option<&AxisReference>) -> SweepSource {
    if source.axis.is_some() {
        return source;
    }
    SweepSource {
        axis: axis.cloned().map(RevolveAxis::Model),
        ..source
    }
}

pub fn axis_name(document: &Document, sketch: FeatureId, axis: &RevolveAxis) -> String {
    match axis {
        RevolveAxis::Sketch(line) => editing::edited_sketch(document, sketch).map_or_else(
            || "the chosen line".to_owned(),
            |sketch| sketch.entity_label(*line),
        ),
        RevolveAxis::Model(axis) => describe_axis(document, axis),
    }
}

fn last_body(document: &Document, before: Option<FeatureId>) -> Option<FeatureId> {
    let end = before
        .and_then(|feature| document.feature_index(feature))
        .unwrap_or(document.features().len());
    document
        .features()
        .take(end)
        .rev()
        .find(|feature| feature.makes_body())
        .map(|feature| feature.id())
}

pub fn bodies_before(document: &Document, feature: FeatureId) -> Vec<FeatureId> {
    let end = document.feature_index(feature).unwrap_or(0);
    document
        .features()
        .take(end)
        .filter(|candidate| candidate.makes_body())
        .map(|candidate| candidate.id())
        .collect()
}

pub fn default_operation(document: &Document, before: Option<FeatureId>) -> BodyOperation {
    last_body(document, before).map_or(BodyOperation::NewBody, BodyOperation::Add)
}

pub fn degrees(value: f64) -> Expression {
    Expression::Measure(value, Unit::Degree)
}

pub fn create(
    document: &Document,
    sweep: Sweep,
    source: SweepSource,
    unit: LengthUnit,
) -> (Transaction, FeatureId) {
    let name = editing::next_feature_name(document, sweep.label());
    let operation = default_operation(document, None);
    let solid = match sweep {
        Sweep::Extrude => SolidFeature::Extrude(Extrude {
            sketch: source.sketch,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(unit.default_length(DEFAULT_DISTANCE), false),
            operation,
        }),
        Sweep::Revolve => SolidFeature::Revolve(Revolve {
            sketch: source.sketch,
            regions: RegionChoice::All,
            axis: source
                .axis
                .unwrap_or(RevolveAxis::Sketch(EntityId::VERTICAL_AXIS)),
            extent: RevolveExtent::Full,
            operation,
        }),
    };
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(name, FeatureKind::Solid(solid));
    if visibility::is_shown(document, source.sketch) {
        transaction.edit(Edit::SetFeatureHidden {
            id: source.sketch,
            hidden: true,
        });
    }
    (transaction.finish(), feature)
}

pub fn create_actions(
    document: &Document,
    sweep: Sweep,
    source: SweepSource,
    unit: LengthUnit,
) -> Vec<Action> {
    let (transaction, feature) = create(document, sweep, source, unit);
    vec![
        Action::Apply(transaction),
        Action::Editing(EditingCommand::OpenSolid(feature)),
    ]
}

pub fn edit(document: &Document, feature: FeatureId, solid: SolidFeature) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    Some(Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Solid(solid),
        },
    ))
}

pub fn with_regions(solid: &SolidFeature, regions: RegionChoice) -> SolidFeature {
    let mut changed = solid.clone();
    match &mut changed {
        SolidFeature::Extrude(extrude) => extrude.regions = regions,
        SolidFeature::Revolve(revolve) => revolve.regions = regions,
    }
    changed
}

pub fn with_operation(solid: &SolidFeature, operation: BodyOperation) -> SolidFeature {
    let mut changed = solid.clone();
    match &mut changed {
        SolidFeature::Extrude(extrude) => extrude.operation = operation,
        SolidFeature::Revolve(revolve) => revolve.operation = operation,
    }
    changed
}

pub fn toggle_region(model: &Model, feature: FeatureId, region: RegionKey) -> Option<Transaction> {
    let document = model.document();
    let owner = document.feature(feature)?;
    let solid = owner.kind.solid()?;
    let (_, regions) = selection::swept_regions(document, model.evaluation(), feature)?;
    let mut chosen = scene::chosen_regions(solid.regions(), regions);
    if !chosen.remove(&region) {
        chosen.insert(region);
    }
    let choice = RegionChoice::Chosen(chosen.into_iter().collect());
    Some(Transaction::single(
        format!("Choose regions of {}", owner.name),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Solid(with_regions(solid, choice)),
        },
    ))
}

#[cfg(test)]
mod tests {
    use caditor_geometry::{Plane, Point2};
    use caditor_sketch::Sketch;

    use super::*;

    fn document_with_sketches() -> (Document, FeatureId, FeatureId, EntityId) {
        let mut document = Document::default();
        let mut first = Sketch::new(Plane::XY);
        first.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
        let mut second = Sketch::new(Plane::XZ);
        let line = second.add_line(Point2::ZERO, Point2::new(0.0, 10.0));
        let mut transaction = document.transaction("Sketches");
        let base = transaction.add_feature("Base", FeatureKind::from(first));
        let side = transaction.add_feature("Side", FeatureKind::from(second));
        document.apply(transaction.finish()).unwrap();
        (document, base, side, line)
    }

    #[test]
    fn the_source_prefers_the_edited_sketch_then_the_selection_then_the_last_sketch() {
        let (document, base, side, line) = document_with_sketches();
        let mut selection = Selection::default();
        let idle = SketchEditing::default();

        assert_eq!(
            sweep_source(&document, &selection, &idle),
            Some(SweepSource {
                sketch: side,
                axis: None
            })
        );
        assert_eq!(
            sweep_source(&document, &selection, &SketchEditing::editing(base)).map(|s| s.sketch),
            Some(base)
        );
        selection.replace_with(Pickable::SketchEntity {
            feature: side,
            entity: line,
        });
        assert_eq!(
            sweep_source(&document, &selection, &SketchEditing::editing(side)),
            Some(SweepSource {
                sketch: side,
                axis: Some(RevolveAxis::Sketch(line))
            })
        );
        assert_eq!(
            sweep_source(&Document::default(), &Selection::default(), &idle),
            None
        );
    }

    #[test]
    fn new_features_add_to_the_last_body_once_there_is_one() {
        let (mut document, base, side, line) = document_with_sketches();
        let (transaction, first) = create(
            &document,
            Sweep::Extrude,
            SweepSource {
                sketch: base,
                axis: None,
            },
            LengthUnit::Millimetre,
        );
        document.apply(transaction).unwrap();
        let (transaction, second) = create(
            &document,
            Sweep::Revolve,
            SweepSource {
                sketch: side,
                axis: Some(RevolveAxis::Sketch(line)),
            },
            LengthUnit::Millimetre,
        );
        assert_eq!(transaction.label(), "Create Revolve 1");
        document.apply(transaction).unwrap();

        let first = document.feature(first).unwrap();
        assert_eq!(first.name, "Extrude 1");
        assert_eq!(
            first.kind.solid().unwrap().operation(),
            BodyOperation::NewBody
        );
        let second = document.feature(second).unwrap().kind.solid().unwrap();
        assert_eq!(second.operation(), BodyOperation::Add(first.id()));
        assert_eq!(second.axis_line(), Some(line));
    }
}
