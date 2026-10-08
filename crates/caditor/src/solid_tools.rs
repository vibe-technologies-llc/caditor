use std::collections::BTreeSet;

use caditor_document::{
    AxisReference, BodyOperation, Document, Edit, Extrude, ExtrudeExtent, FeatureId, FeatureKind,
    FeatureResult, RegionChoice, Revolve, RevolveAxis, RevolveExtent, SketchAttachment,
    SketchFeature, SolidFeature, SolidStart, Transaction, describe_axis,
};
use caditor_expression::{Expression, Unit};
use caditor_geometry::Point2;
use caditor_kernel::RegionKey;
use caditor_sketch::{Entity, EntityId, Reference, Sketch};

use crate::{
    bodies,
    editing::{self, EditingCommand, SketchEditing},
    model::{Action, Model, Notice},
    projecting, scene,
    selection::{self, Pickable, Selection},
    sketch_placement::{self, FaceChoice},
    units::LengthUnit,
    visibility,
};

pub const DEFAULT_DISTANCE: f64 = 10.0;
pub const DEFAULT_PARTIAL_ANGLE: f64 = 180.0;
pub const DEFAULT_BACKWARD_ANGLE: f64 = 30.0;
const OUTLINE_SEGMENT_ANGLE: f64 = 0.02;
const OUTLINE_GAP: f64 = 1e-6;
pub const NOT_FLAT_TO_EXTRUDE: &str =
    "The selected face is curved; only a flat face can be extruded";
pub const NOTHING_TO_EXTRUDE: &str =
    "Select one flat face of a body to extrude it, or the curves of a sketch";
pub const NOTHING_TO_REVOLVE: &str =
    "Select the curves of a sketch to revolve, with the axis to turn them about";
const NO_OUTLINE: &str = "The selected face has no edges to extrude it by";

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
    pub regions: RegionChoice,
}

pub fn sweep_source(
    document: &Document,
    selection: &Selection,
    editing: &SketchEditing,
    may_guess: bool,
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
    let last_sketch = || {
        document
            .active_features()
            .rev()
            .find(|feature| feature.kind.sketch().is_some())
            .map(|feature| feature.id())
    };
    let guessed = may_guess
        .then(|| opened_sketch.or_else(last_sketch))
        .flatten();
    let sketch = editing.feature().or(selected_sketch).or(guessed)?;
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
    Some(SweepSource {
        sketch,
        axis,
        regions: RegionChoice::All,
    })
}

pub fn may_guess_sketch(sweep: Sweep, selection: &Selection, lone_axis: bool) -> bool {
    selection.is_empty() || (sweep == Sweep::Revolve && lone_axis && selection.len() == 1)
}

pub fn face_to_extrude(
    model: &Model,
    selection: &Selection,
    editing: &SketchEditing,
) -> Option<Result<FaceChoice, &'static str>> {
    let sketch_chosen = selection
        .iter()
        .any(|pickable| matches!(pickable, Pickable::SketchEntity { .. }));
    if editing.feature().is_some() || sketch_chosen {
        return None;
    }
    let face = sketch_placement::selected_face(selection)?;
    Some(if sketch_placement::is_flat(model, face) {
        Ok(face)
    } else {
        Err(NOT_FLAT_TO_EXTRUDE)
    })
}

pub fn create_on_face(
    model: &Model,
    face: FaceChoice,
) -> Result<(Transaction, FeatureId, String), &'static str> {
    let document = model.document();
    let (attachment, plane) = sketch_placement::attachment_at(model, face, document.bar_index())?;
    let shown = bodies::shown(model.evaluation(), face.body).ok_or(NO_OUTLINE)?;
    let id = bodies::find_face(shown, face.face).ok_or(NO_OUTLINE)?;
    let outline = projecting::face_projections(shown, face.body, id, &plane);
    if outline.is_empty() {
        return Err(NO_OUTLINE);
    }
    let sketch_name = editing::next_sketch_name(document);
    let name = editing::next_feature_name(document, Sweep::Extrude.label());
    let mut transaction = document.transaction(format!("Create {name}"));
    let sketch = transaction.add_feature(
        sketch_name.clone(),
        FeatureKind::Sketch(SketchFeature::on_face(Sketch::new(plane), attachment)),
    );
    transaction.edit(Edit::SetFeatureHidden {
        id: sketch,
        hidden: true,
    });
    for (source, edges) in &outline {
        transaction.add_projection(sketch, source.clone(), edges);
    }
    let feature = transaction.add_feature(
        name.clone(),
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(
                model.length_unit().default_length(DEFAULT_DISTANCE),
                false,
            ),
            operation: BodyOperation::Add(face.body),
            start: None,
            other_bodies: Vec::new(),
        })),
    );
    let told = format!(
        "{name} extrudes the selected face out of its body, following its edges through \
         {sketch_name}. Edit {sketch_name} to change the outline."
    );
    Ok((transaction.finish(), feature, told))
}

pub fn create_on_face_actions(model: &Model, face: FaceChoice) -> Vec<Action> {
    match create_on_face(model, face) {
        Ok((transaction, feature, told)) => vec![
            Action::Apply(transaction),
            Action::Editing(EditingCommand::OpenSolid(feature)),
            Action::Inform(Notice::info(told)),
        ],
        Err(reason) => vec![Action::Inform(Notice::info(format!(
            "{}: {reason}.",
            Sweep::Extrude.label()
        )))],
    }
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

pub fn with_selected_outline(
    model: &Model,
    sweep: Sweep,
    source: SweepSource,
    selection: &Selection,
) -> SweepSource {
    match enclosed_regions(model, sweep, &source, selection) {
        Some(regions) => SweepSource { regions, ..source },
        None => source,
    }
}

fn enclosed_regions(
    model: &Model,
    sweep: Sweep,
    source: &SweepSource,
    selection: &Selection,
) -> Option<RegionChoice> {
    let feature = model.document().feature(source.sketch)?;
    let displayed = model.displayed_sketch(feature)?;
    let axis = match (sweep, &source.axis) {
        (Sweep::Revolve, Some(RevolveAxis::Sketch(axis))) => Some(*axis),
        _ => None,
    };
    let outline: Vec<Vec<Point2>> = selection
        .iter()
        .filter_map(|pickable| match pickable {
            Pickable::SketchEntity { feature, entity }
                if feature == source.sketch && Some(entity) != axis =>
            {
                displayed.polyline(entity, OUTLINE_SEGMENT_ANGLE)
            }
            _ => None,
        })
        .collect();
    if outline.is_empty() || !closed(&outline) {
        return None;
    }
    let regions = model
        .evaluation()
        .feature(source.sketch)?
        .result
        .as_deref()
        .and_then(FeatureResult::sketch)?
        .regions()?
        .as_ref()
        .ok()?;
    let enclosed: BTreeSet<RegionKey> = regions
        .iter()
        .filter(|region| {
            region
                .reference()
                .anchor()
                .is_some_and(|anchor| encloses(&outline, anchor))
        })
        .map(|region| region.region.key())
        .collect();
    (!enclosed.is_empty())
        .then(|| RegionChoice::Chosen(scene::region_references(&enclosed, regions)))
}

fn closed(outline: &[Vec<Point2>]) -> bool {
    let ends: Vec<Point2> = outline
        .iter()
        .filter_map(|polyline| Some((*polyline.first()?, *polyline.last()?)))
        .filter(|(first, last)| first.distance(*last) > OUTLINE_GAP)
        .flat_map(|(first, last)| [first, last])
        .collect();
    ends.iter().enumerate().all(|(index, end)| {
        ends.iter()
            .enumerate()
            .any(|(other, point)| other != index && point.distance(*end) <= OUTLINE_GAP)
    })
}

fn encloses(outline: &[Vec<Point2>], point: Point2) -> bool {
    let crossings = outline
        .iter()
        .flat_map(|polyline| polyline.windows(2))
        .filter(|pair| match pair {
            [start, end] => {
                (start.y > point.y) != (end.y > point.y)
                    && point.x
                        < start.x + (point.y - start.y) / (end.y - start.y) * (end.x - start.x)
            }
            _ => false,
        })
        .count();
    crossings % 2 == 1
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
    let standing = match before.filter(|feature| document.feature_index(*feature).is_some()) {
        Some(feature) => document.bodies_before(feature),
        None => document.bodies_standing(),
    };
    standing.last().copied()
}

pub fn default_operation(document: &Document, before: Option<FeatureId>) -> BodyOperation {
    last_body(document, before).map_or(BodyOperation::NewBody, BodyOperation::Add)
}

pub fn face_body(document: &Document, sketch: FeatureId) -> Option<FeatureId> {
    let body = match document.feature(sketch)?.kind.attachment()? {
        SketchAttachment::Face(face) => face.body,
        SketchAttachment::Datum(_) => return None,
    };
    document.bodies_standing().contains(&body).then_some(body)
}

pub fn degrees(value: f64) -> Expression {
    Expression::measure(value, Unit::Degree)
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
        Sweep::Extrude => {
            let (operation, into_the_body) = match face_body(document, source.sketch) {
                Some(body) => (BodyOperation::Remove(body), true),
                None => (operation, false),
            };
            SolidFeature::Extrude(Extrude {
                sketch: source.sketch,
                regions: source.regions.clone(),
                extent: ExtrudeExtent::one_side(
                    unit.default_length(DEFAULT_DISTANCE),
                    into_the_body,
                ),
                operation,
                start: None,
                other_bodies: Vec::new(),
            })
        }
        Sweep::Revolve => SolidFeature::Revolve(Revolve {
            sketch: source.sketch,
            regions: source.regions.clone(),
            axis: source
                .axis
                .unwrap_or(RevolveAxis::Sketch(EntityId::VERTICAL_AXIS)),
            extent: RevolveExtent::Full,
            operation,
            start: None,
            other_bodies: Vec::new(),
            side: None,
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

pub fn with_start(solid: &SolidFeature, start: Option<SolidStart>) -> SolidFeature {
    let mut changed = solid.clone();
    match &mut changed {
        SolidFeature::Extrude(extrude) => extrude.start = start,
        SolidFeature::Revolve(revolve) => revolve.start = start,
    }
    changed
}

pub fn with_operation(solid: &SolidFeature, operation: BodyOperation) -> SolidFeature {
    let mut changed = solid.clone();
    match &mut changed {
        SolidFeature::Extrude(extrude) => extrude.operation = operation,
        SolidFeature::Revolve(revolve) => revolve.operation = operation,
    }
    let target = operation.target();
    let others = changed.other_bodies_mut();
    match operation {
        BodyOperation::Remove(_) => others.retain(|other| Some(*other) != target),
        BodyOperation::NewBody | BodyOperation::Add(_) | BodyOperation::Intersect(_) => {
            others.clear();
        }
    }
    changed
}

pub fn with_other_bodies(solid: &SolidFeature, other_bodies: Vec<FeatureId>) -> SolidFeature {
    let mut changed = solid.clone();
    *changed.other_bodies_mut() = other_bodies;
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
    let choice = RegionChoice::Chosen(scene::region_references(&chosen, regions));
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
            sweep_source(&document, &selection, &idle, true),
            Some(SweepSource {
                sketch: side,
                axis: None,
                regions: RegionChoice::All,
            })
        );
        assert_eq!(
            sweep_source(&document, &selection, &SketchEditing::editing(base), false)
                .map(|s| s.sketch),
            Some(base)
        );
        selection.replace_with(Pickable::SketchEntity {
            feature: side,
            entity: line,
        });
        assert_eq!(
            sweep_source(&document, &selection, &SketchEditing::editing(side), false),
            Some(SweepSource {
                sketch: side,
                axis: Some(RevolveAxis::Sketch(line)),
                regions: RegionChoice::All,
            })
        );
        assert_eq!(
            sweep_source(&Document::default(), &Selection::default(), &idle, true),
            None
        );
    }

    #[test]
    fn no_sketch_is_guessed_for_a_selection_that_holds_none() {
        let (document, _, side, _) = document_with_sketches();
        let idle = SketchEditing::default();
        let mut selection = Selection::default();
        selection.replace_with(Pickable::Axis(crate::selection::Axis::X));

        assert_eq!(sweep_source(&document, &selection, &idle, false), None);
        assert_eq!(
            sweep_source(&document, &selection, &idle, true).map(|source| source.sketch),
            Some(side)
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
                regions: RegionChoice::All,
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
                regions: RegionChoice::All,
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

    #[test]
    fn new_features_skip_a_body_a_combine_consumed() {
        let (mut document, base, side, _) = document_with_sketches();
        let mut transaction = document.transaction("Two bodies joined");
        let mut extrude = |name: &str, sketch| {
            transaction.add_feature(
                name,
                FeatureKind::Solid(SolidFeature::Extrude(Extrude {
                    sketch,
                    regions: RegionChoice::All,
                    extent: ExtrudeExtent::one_side(
                        Expression::measure(5.0, Unit::Millimetre),
                        false,
                    ),
                    operation: BodyOperation::NewBody,
                    start: None,
                    other_bodies: Vec::new(),
                })),
            )
        };
        let target = extrude("Block", base);
        let tool = extrude("Peg", side);
        let combine = transaction.add_feature(
            "Combine 1",
            FeatureKind::Combine(caditor_document::Combine {
                body: target,
                tool,
                operation: caditor_document::CombineOperation::Join,
            }),
        );
        document.apply(transaction.finish()).unwrap();

        assert_eq!(
            default_operation(&document, None),
            BodyOperation::Add(target)
        );
        assert_eq!(document.bodies_standing(), vec![target]);
        assert_eq!(
            default_operation(&document, Some(combine)),
            BodyOperation::Add(tool)
        );
    }
}
