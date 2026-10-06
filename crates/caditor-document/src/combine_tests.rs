use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2};
use caditor_kernel::SamplingTolerance;
use caditor_sketch::Sketch;

use crate::*;

pub(crate) fn rectangle(min: (f64, f64), max: (f64, f64)) -> Sketch {
    let mut sketch = Sketch::new(Plane::XY);
    let corners = [
        Point2::new(min.0, min.1),
        Point2::new(max.0, min.1),
        Point2::new(max.0, max.1),
        Point2::new(min.0, max.1),
    ];
    for index in 0..4 {
        sketch.add_line(corners[index], corners[(index + 1) % 4]);
    }
    sketch
}

pub(crate) fn block(
    transaction: &mut TransactionBuilder<'_>,
    name: &str,
    min: (f64, f64),
    max: (f64, f64),
    height: &str,
) -> FeatureId {
    let outline = transaction.add_feature(
        format!("{name} outline"),
        FeatureKind::from(rectangle(min, max)),
    );
    transaction.add_feature(
        name,
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::parse_stored(height).unwrap(), false),
            operation: BodyOperation::NewBody,
            start: None,
        })),
    )
}

pub(crate) struct Pair {
    pub(crate) document: Document,
    pub(crate) plate: FeatureId,
    pub(crate) peg: FeatureId,
}

pub(crate) fn pair() -> Pair {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let plate = block(&mut transaction, "Plate", (0.0, 0.0), (20.0, 10.0), "4 mm");
    let peg = block(&mut transaction, "Peg", (15.0, 2.0), (25.0, 8.0), "6 mm");
    document.apply(transaction.finish()).unwrap();
    Pair {
        document,
        plate,
        peg,
    }
}

fn combine(pair: &mut Pair, operation: CombineOperation) -> FeatureId {
    let (plate, peg) = (pair.plate, pair.peg);
    let mut transaction = pair.document.transaction("Combine");
    let combined = transaction.add_feature(
        "Combine 1",
        FeatureKind::Combine(Combine {
            body: plate,
            tool: peg,
            operation,
        }),
    );
    pair.document.apply(transaction.finish()).unwrap();
    combined
}

pub(crate) fn evaluate(document: &Document, engine: &mut Recompute) -> Evaluation {
    engine.run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
}

pub(crate) fn volume(evaluation: &Evaluation, body: FeatureId) -> f64 {
    evaluation
        .body(body)
        .unwrap()
        .tessellate(&SamplingTolerance::new(1e-3, 0.1).unwrap())
        .unwrap()
        .mass_properties()
        .volume
}

const PLATE: f64 = 20.0 * 10.0 * 4.0;
const PEG: f64 = 10.0 * 6.0 * 6.0;
const OVERLAP: f64 = 5.0 * 6.0 * 4.0;

#[test]
fn two_bodies_are_two_bodies_until_they_are_combined() {
    let pair = pair();
    let mut engine = Recompute::default();

    let evaluation = evaluate(&pair.document, &mut engine);

    assert_eq!(
        evaluation
            .bodies()
            .map(|(body, _)| body)
            .collect::<Vec<_>>(),
        vec![pair.plate, pair.peg]
    );
}

#[test]
fn joining_cutting_and_intersecting_make_one_body_of_the_target_and_consume_the_tool() {
    for (operation, expected) in [
        (CombineOperation::Join, PLATE + PEG - OVERLAP),
        (CombineOperation::Cut, PLATE - OVERLAP),
        (CombineOperation::Intersect, OVERLAP),
    ] {
        let mut pair = pair();
        let combined = combine(&mut pair, operation);
        let mut engine = Recompute::default();

        let evaluation = evaluate(&pair.document, &mut engine);

        assert_eq!(evaluation.failed_count(), 0, "{operation:?}");
        let bodies: Vec<FeatureId> = evaluation.bodies().map(|(body, _)| body).collect();
        assert_eq!(bodies, vec![pair.plate], "{operation:?}");
        assert!(evaluation.body(pair.peg).is_none());
        let found = volume(&evaluation, pair.plate);
        assert!(
            (found - expected).abs() < 0.01 * expected,
            "{operation:?}: {found} instead of {expected}"
        );
        assert_eq!(
            evaluation.bodies().find(|(body, _)| *body == pair.plate),
            Some((pair.plate, combined))
        );
    }
}

#[test]
fn a_combine_that_fails_leaves_both_bodies_as_they_were() {
    let mut pair = pair();
    let mut transaction = pair.document.transaction("Far away");
    let far = block(&mut transaction, "Far", (100.0, 0.0), (110.0, 10.0), "4 mm");
    pair.document.apply(transaction.finish()).unwrap();
    let mut transaction = pair.document.transaction("Combine");
    let combined = transaction.add_feature(
        "Combine 1",
        FeatureKind::Combine(Combine {
            body: pair.plate,
            tool: far,
            operation: CombineOperation::Intersect,
        }),
    );
    pair.document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();

    let evaluation = evaluate(&pair.document, &mut engine);

    let FeatureState::Failed(error) = &evaluation.feature(combined).unwrap().state else {
        panic!("the combine should fail");
    };
    assert!(error.reason.contains("do not overlap"), "{}", error.reason);
    assert_eq!(evaluation.failed_count(), 1);
    let bodies: Vec<FeatureId> = evaluation.bodies().map(|(body, _)| body).collect();
    assert_eq!(bodies, vec![pair.plate, pair.peg, far]);
}

#[test]
fn a_body_cannot_be_combined_with_itself() {
    let mut pair = pair();
    let mut transaction = pair.document.transaction("Combine");
    let combined = transaction.add_feature(
        "Combine 1",
        FeatureKind::Combine(Combine {
            body: pair.plate,
            tool: pair.plate,
            operation: CombineOperation::Join,
        }),
    );
    pair.document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();

    let evaluation = evaluate(&pair.document, &mut engine);

    let FeatureState::Failed(error) = &evaluation.feature(combined).unwrap().state else {
        panic!("the combine should fail");
    };
    assert_eq!(error.reason, "A body cannot be combined with itself.");
}

#[test]
fn suppressing_a_combine_brings_the_tool_body_back() {
    let mut pair = pair();
    let combined = combine(&mut pair, CombineOperation::Join);
    let mut engine = Recompute::default();
    assert_eq!(evaluate(&pair.document, &mut engine).bodies().count(), 1);

    let mut transaction = pair.document.transaction("Suppress");
    transaction.edit(Edit::SetFeatureSuppressed {
        id: combined,
        suppressed: true,
    });
    pair.document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&pair.document, &mut engine);
    assert_eq!(evaluation.bodies().count(), 2);
    assert!((volume(&evaluation, pair.peg) - PEG).abs() < 0.01 * PEG);
}

#[test]
fn deleting_the_tool_body_is_a_deletion_the_combine_depends_on() {
    let mut pair = pair();
    let combined = combine(&mut pair, CombineOperation::Join);

    assert_eq!(pair.document.dependents_of(&[pair.peg]), vec![combined]);
    assert_eq!(pair.document.dependents_of(&[pair.plate]), vec![combined]);
}

#[test]
fn a_combine_depends_on_both_bodies_and_follows_the_tree_order() {
    let mut pair = pair();
    let combined = combine(&mut pair, CombineOperation::Cut);
    let feature = pair.document.feature(combined).unwrap();

    assert!(feature.kind.dependencies().contains(&pair.plate));
    assert!(feature.kind.dependencies().contains(&pair.peg));
    assert_eq!(feature.kind.consumed_bodies(), vec![pair.peg]);
    assert_eq!(feature.body(), Some(pair.plate));
    assert!(!feature.makes_body());
    assert!(feature.kind.modifies_body());
}

#[test]
fn the_bodies_before_a_combine_leave_out_those_an_earlier_combine_consumed() {
    let mut pair = pair();
    let mut transaction = pair.document.transaction("Third");
    let third = block(&mut transaction, "Third", (0.0, 20.0), (5.0, 25.0), "2 mm");
    pair.document.apply(transaction.finish()).unwrap();
    let first = combine(&mut pair, CombineOperation::Join);
    let mut transaction = pair.document.transaction("Second combine");
    let second = transaction.add_feature(
        "Combine 2",
        FeatureKind::Combine(Combine {
            body: pair.plate,
            tool: third,
            operation: CombineOperation::Join,
        }),
    );
    pair.document.apply(transaction.finish()).unwrap();

    assert_eq!(
        pair.document.bodies_before(first),
        vec![pair.plate, pair.peg, third]
    );
    assert_eq!(pair.document.bodies_before(second), vec![pair.plate, third]);
}

#[test]
fn a_peg_a_few_micrometres_past_the_edge_of_a_plate_joins_it() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let plate = block(&mut transaction, "Plate", (0.0, 0.0), (20.0, 10.0), "4 mm");
    let peg = block(
        &mut transaction,
        "Peg",
        (10.0, 2.0),
        (20.000005, 8.0),
        "4 mm",
    );
    let combined = transaction.add_feature(
        "Combine 1",
        FeatureKind::Combine(Combine {
            body: plate,
            tool: peg,
            operation: CombineOperation::Join,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();

    let evaluation = evaluate(&document, &mut engine);

    assert_eq!(
        evaluation.feature(combined).unwrap().state,
        FeatureState::UpToDate
    );
}

#[test]
fn a_combine_failing_where_faces_nearly_touch_names_them_and_where() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let mut holed = rectangle((0.0, 0.0), (20.0, 10.0));
    holed.add_circle(Point2::new(10.0, 5.0), 2.5);
    let outline = transaction.add_feature("Plate outline", FeatureKind::from(holed));
    let plate = transaction.add_feature(
        "Plate",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::parse_stored("4 mm").unwrap(), false),
            operation: BodyOperation::NewBody,
            start: None,
        })),
    );
    let mut round = Sketch::new(Plane::XY);
    round.add_circle(Point2::new(10.0000015, 5.0), 2.5);
    let outline = transaction.add_feature("Peg outline", FeatureKind::from(round));
    let peg = transaction.add_feature(
        "Peg",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::parse_stored("4 mm").unwrap(), false),
            operation: BodyOperation::NewBody,
            start: None,
        })),
    );
    let combined = transaction.add_feature(
        "Combine 1",
        FeatureKind::Combine(Combine {
            body: plate,
            tool: peg,
            operation: CombineOperation::Join,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();

    let evaluation = evaluate(&document, &mut engine);

    let FeatureState::Failed(error) = &evaluation.feature(combined).unwrap().state else {
        panic!("the near contact should not combine");
    };
    let place = error.place.unwrap();
    assert_eq!(
        error.reason,
        "The bodies of Plate and Peg could not be combined. The faces of the result would not \
         close up at Plate side from Circle 13, Plate start face and Peg start face."
    );
    assert!(
        error
            .remedy
            .contains("line up exactly or stay clearly apart"),
        "{}",
        error.remedy
    );
    assert!(!error.remedy.contains("exactly touch"), "{}", error.remedy);
    let from_axis = (place.x - 10.0).hypot(place.y - 5.0);
    assert!((from_axis - 2.5).abs() < 1e-4, "{place:?}");
    assert!(place.z.abs() < 1e-6, "{place:?}");
}
