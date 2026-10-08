use std::collections::BTreeSet;

use caditor_document::{
    AxisReference, BodyOperation, Document, Edit, Evaluation, Extrude, ExtrudeExtent, Feature,
    FeatureId, FeatureKind, FeatureResult, RegionChoice, Revolve, RevolveAxis, RevolveExtent,
    SketchAttachment, SketchFeature, SolidFeature, SolidStart, Transaction, describe_axis,
};
use caditor_expression::{Expression, Unit};
use caditor_geometry::Point2;
use caditor_kernel::RegionKey;
use caditor_sketch::{Entity, EntityId, Reference, Sketch};

use crate::{
    bodies, body_selection,
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
pub const SEVERAL_SKETCHES: &str =
    "Curves of several sketches are selected; select the curves of one sketch only";
pub const SEVERAL_BODIES: &str =
    "Faces of several bodies are selected; select faces of the one body to add to";
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
    pub body: Option<FeatureId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Guess {
    Never,
    Sweep,
    Drill,
}

impl Guess {
    pub fn sweep_when(may_guess: bool) -> Self {
        if may_guess { Self::Sweep } else { Self::Never }
    }
}

pub fn sweep_source(
    document: &Document,
    evaluation: &Evaluation,
    selection: &Selection,
    editing: &SketchEditing,
    guess: Guess,
) -> Result<Option<SweepSource>, &'static str> {
    let picked = selection.in_pick_order();
    let mut selected: Vec<FeatureId> = Vec::new();
    for pickable in &picked {
        if let Pickable::SketchEntity { feature, .. } = *pickable
            && !selected.contains(&feature)
        {
            selected.push(feature);
        }
    }
    let sketch = match (editing.feature(), selected.as_slice()) {
        (Some(edited), _) => edited,
        (None, [sketch]) => *sketch,
        (None, [_, _, ..]) => return Err(SEVERAL_SKETCHES),
        (None, []) => match guessed_sketch(document, evaluation, editing, guess) {
            Some(guessed) => guessed,
            None => return Ok(None),
        },
    };
    let Some(definition) = editing::edited_sketch(document, sketch) else {
        return Ok(None);
    };
    let axis = picked.iter().rev().find_map(|pickable| match *pickable {
        Pickable::SketchEntity { feature, entity } if feature == sketch => {
            let is_axis = match entity.reference() {
                Some(Reference::HorizontalAxis | Reference::VerticalAxis) => true,
                Some(Reference::Origin) => false,
                None => matches!(definition.entity(entity), Some(Entity::Line { .. })),
            };
            is_axis.then_some(RevolveAxis::Sketch(entity))
        }
        _ => None,
    });
    let body = match body_selection::bodies_in(selection).as_slice() {
        [] => None,
        [body] => Some(*body),
        [_, _, ..] => return Err(SEVERAL_BODIES),
    };
    Ok(Some(SweepSource {
        sketch,
        axis,
        regions: RegionChoice::All,
        body,
    }))
}

fn guessed_sketch(
    document: &Document,
    evaluation: &Evaluation,
    editing: &SketchEditing,
    guess: Guess,
) -> Option<FeatureId> {
    let opened = editing
        .solid()
        .and_then(|solid| document.feature(solid))
        .and_then(|feature| feature.kind.solid())
        .map(SolidFeature::sketch);
    let last = || {
        document
            .active_features()
            .rev()
            .filter(|feature| feature.kind.sketch().is_some())
            .map(|feature| feature.id())
            .find(|sketch| visibility::is_shown(document, *sketch))
    };
    let unused = |sketch: &FeatureId| match guess {
        Guess::Never => false,
        Guess::Sweep => has_unswept_regions(document, evaluation, *sketch),
        Guess::Drill => !is_drilled(document, *sketch),
    };
    opened.or_else(last).filter(unused)
}

fn is_drilled(document: &Document, sketch: FeatureId) -> bool {
    document
        .active_features()
        .any(|feature| matches!(&feature.kind, FeatureKind::Hole(hole) if hole.sketch == sketch))
}

pub fn has_unswept_regions(
    document: &Document,
    evaluation: &Evaluation,
    sketch: FeatureId,
) -> bool {
    let users: Vec<&Feature> = document
        .active_features()
        .filter(|feature| match &feature.kind {
            FeatureKind::Solid(solid) => solid.sketch() == sketch,
            FeatureKind::Hole(hole) => hole.sketch == sketch,
            _ => false,
        })
        .collect();
    let regions = evaluation
        .feature(sketch)
        .and_then(|evaluated| evaluated.result.as_deref())
        .and_then(FeatureResult::sketch)
        .and_then(|result| result.regions());
    let regions = match regions {
        None => return users.is_empty(),
        Some(Err(_)) => return false,
        Some(Ok(regions)) => regions,
    };
    let mut swept: BTreeSet<RegionKey> = BTreeSet::new();
    for user in &users {
        match &user.kind {
            FeatureKind::Solid(solid) => {
                swept.extend(scene::chosen_regions(solid.regions(), regions));
            }
            _ => return false,
        }
    }
    regions
        .iter()
        .filter(|region| region.even_depth)
        .any(|region| !swept.contains(&region.region.key()))
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

pub fn with_model_axis(
    source: SweepSource,
    selection: &Selection,
    axes: &[(Pickable, AxisReference)],
) -> SweepSource {
    let sketch_line = |pickable: Pickable| match (pickable, &source.axis) {
        (Pickable::SketchEntity { feature, entity }, Some(RevolveAxis::Sketch(line))) => {
            feature == source.sketch && entity == *line
        }
        _ => false,
    };
    let latest = selection
        .in_pick_order()
        .into_iter()
        .rev()
        .find_map(|pickable| {
            if sketch_line(pickable) {
                return source.axis.clone();
            }
            let in_sketch = matches!(
                pickable,
                Pickable::SketchEntity { feature, .. } if feature == source.sketch
            );
            axes.iter()
                .find(|(candidate, _)| *candidate == pickable && !in_sketch)
                .map(|(_, axis)| RevolveAxis::Model(axis.clone()))
        });
    SweepSource {
        axis: latest.or(source.axis),
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
    let operation = source
        .body
        .map_or_else(|| default_operation(document, None), BodyOperation::Add);
    let solid = match sweep {
        Sweep::Extrude => {
            let (operation, into_the_body) = match face_body(document, source.sketch) {
                Some(body) if source.body.is_none_or(|selected| selected == body) => {
                    (BodyOperation::Remove(body), true)
                }
                Some(_) | None => (operation, false),
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

    fn source(
        document: &Document,
        selection: &Selection,
        editing: &SketchEditing,
        may_guess: bool,
    ) -> Option<SweepSource> {
        sweep_source(
            document,
            &Evaluation::default(),
            selection,
            editing,
            Guess::sweep_when(may_guess),
        )
        .unwrap()
    }

    #[test]
    fn the_source_prefers_the_edited_sketch_then_the_selection_then_the_last_sketch() {
        let (document, base, side, line) = document_with_sketches();
        let mut selection = Selection::default();
        let idle = SketchEditing::default();

        assert_eq!(
            source(&document, &selection, &idle, true),
            Some(SweepSource {
                sketch: side,
                axis: None,
                regions: RegionChoice::All,
                body: None,
            })
        );
        assert_eq!(
            source(&document, &selection, &SketchEditing::editing(base), false).map(|s| s.sketch),
            Some(base)
        );
        selection.replace_with(Pickable::SketchEntity {
            feature: side,
            entity: line,
        });
        assert_eq!(
            source(&document, &selection, &SketchEditing::editing(side), false),
            Some(SweepSource {
                sketch: side,
                axis: Some(RevolveAxis::Sketch(line)),
                regions: RegionChoice::All,
                body: None,
            })
        );
        assert_eq!(
            source(&Document::default(), &Selection::default(), &idle, true),
            None
        );
    }

    #[test]
    fn no_sketch_is_guessed_for_a_selection_that_holds_none() {
        let (document, _, side, _) = document_with_sketches();
        let idle = SketchEditing::default();
        let mut selection = Selection::default();
        selection.replace_with(Pickable::Axis(crate::selection::Axis::X));

        assert_eq!(source(&document, &selection, &idle, false), None);
        assert_eq!(
            source(&document, &selection, &idle, true).map(|source| source.sketch),
            Some(side)
        );
    }

    #[test]
    fn curves_of_two_sketches_are_refused_and_the_last_picked_line_is_the_axis() {
        let (mut document, base, side, line) = document_with_sketches();
        let mut lines = Sketch::new(Plane::XZ);
        let first = lines.add_line(Point2::ZERO, Point2::new(0.0, 10.0));
        let second = lines.add_line(Point2::new(5.0, 0.0), Point2::new(5.0, 10.0));
        let mut transaction = document.transaction("Lines");
        let both = transaction.add_feature("Lines", FeatureKind::from(lines));
        document.apply(transaction.finish()).unwrap();
        let idle = SketchEditing::default();
        let evaluation = Evaluation::default();
        let mut selection = Selection::default();
        selection.toggle(Pickable::SketchEntity {
            feature: side,
            entity: line,
        });
        selection.toggle(Pickable::SketchEntity {
            feature: base,
            entity: EntityId::HORIZONTAL_AXIS,
        });

        assert_eq!(
            sweep_source(&document, &evaluation, &selection, &idle, Guess::Never),
            Err(SEVERAL_SKETCHES)
        );

        selection.clear();
        selection.toggle(Pickable::SketchEntity {
            feature: both,
            entity: second,
        });
        selection.toggle(Pickable::SketchEntity {
            feature: both,
            entity: first,
        });

        assert_eq!(
            source(&document, &selection, &idle, false).and_then(|source| source.axis),
            Some(RevolveAxis::Sketch(first))
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
                body: None,
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
                body: None,
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
