use std::{collections::BTreeMap, f64::consts::PI, sync::Arc, thread};

use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::Solid;
use caditor_sketch::Sketch;

use crate::{
    combine_tests::{evaluate, rectangle, volume},
    history::result_buffers,
    pattern_tests::failure,
    recompute::Inputs,
    solid::SolidResult,
    values::ParameterValues,
    *,
};

const BASE: f64 = 40.0 * 10.0 * 10.0;
const STEP: f64 = 20.0 * 10.0 * 10.0;
const PEG_SECTION: f64 = 4.0 * 4.0;
const CHAIN_LINKS: usize = 20_000;
const SMALL_STACK: usize = 256 * 1024;

fn length(text: &str) -> Expression {
    Expression::parse(text, &|_| None).unwrap()
}

fn extrude(sketch: FeatureId, extent: ExtrudeExtent, operation: BodyOperation) -> FeatureKind {
    FeatureKind::Solid(SolidFeature::Extrude(Extrude {
        sketch,
        regions: RegionChoice::All,
        extent,
        operation,
        start: None,
        other_bodies: Vec::new(),
        taper: None,
        wall: None,
        direction: None,
    }))
}

fn on_plane(mut sketch: Sketch, height: f64) -> Sketch {
    sketch.set_plane(
        Plane::from_frame(Point3::new(0.0, 0.0, height), Vector3::Z, Vector3::X).unwrap(),
    );
    sketch
}

struct Stepped {
    document: Document,
    base: FeatureId,
    peg: FeatureId,
}

fn stepped() -> Stepped {
    let mut document = Document::default();

    let mut transaction = document.transaction("Build");

    let outline = transaction.add_feature(
        "Base outline",
        FeatureKind::from(rectangle((-20.0, 0.0), (20.0, 10.0))),
    );
    let base = transaction.add_feature(
        "Base",
        extrude(
            outline,
            ExtrudeExtent::one_side(length("10 mm"), false),
            BodyOperation::NewBody,
        ),
    );
    let step_outline = transaction.add_feature(
        "Step outline",
        FeatureKind::from(on_plane(rectangle((0.0, 0.0), (20.0, 10.0)), 10.0)),
    );

    transaction.add_feature(
        "Step",
        extrude(
            step_outline,
            ExtrudeExtent::one_side(length("10 mm"), false),
            BodyOperation::Add(base),
        ),
    );

    let peg_outline = transaction.add_feature(
        "Peg outline",
        FeatureKind::from(on_plane(rectangle((4.0, 3.0), (8.0, 7.0)), 30.0)),
    );
    let peg = transaction.add_feature(
        "Peg",
        extrude(
            peg_outline,
            ExtrudeExtent::OneSide {
                end: ExtrudeEnd::up_to_next(),
                reversed: true,
            },
            BodyOperation::Add(base),
        ),
    );

    document.apply(transaction.finish()).unwrap();
    Stepped {
        document,
        base,
        peg,
    }
}

fn add(document: &mut Document, name: &str, kind: FeatureKind) -> FeatureId {
    let mut transaction = document.transaction(name);

    let feature = transaction.add_feature(name, kind);

    document.apply(transaction.finish()).unwrap();
    feature
}

fn backwards(count: &str, spacing: &str) -> PatternKind {
    PatternKind::Linear {
        first: LinearDirection {
            axis: AxisReference::Principal(PrincipalAxis::X),
            count: length(count),
            spacing: length(spacing),
            measured: LinearSpacing::BetweenCopies,
            reversed: true,
        },
        second: None,
    }
}

fn assert_volume(found: f64, expected: f64) {
    assert!(
        (found - expected).abs() < 0.001 * expected,
        "volume {found} instead of {expected}"
    );
}

#[test]
fn a_patterned_extrusion_up_to_next_stops_on_the_face_each_copy_meets() {
    let mut stepped = stepped();

    let pattern = Pattern::new(stepped.base, backwards("2", "12 mm")).repeating(vec![stepped.peg]);
    let pattern = add(
        &mut stepped.document,
        "Pattern 1",
        FeatureKind::from(pattern),
    );

    let evaluation = evaluate(&stepped.document, &mut Recompute::default());
    let solid = evaluation.body(stepped.base).unwrap();

    assert_eq!(evaluation.failed_count(), 0);

    assert_volume(
        volume(&evaluation, stepped.base),
        BASE + STEP + PEG_SECTION * 10.0 + PEG_SECTION * 20.0,
    );

    assert_eq!(solid.lumps().unwrap().len(), 1);
    assert!(
        solid
            .faces()
            .filter_map(|(_, face)| face.origin()?.copy())
            .any(|copy| copy.pattern == pattern.raw() && copy.index == [1, 0])
    );
}

#[test]
fn a_mirrored_extrusion_up_to_next_stops_on_the_face_its_image_meets() {
    let mut stepped = stepped();

    let mirror = Mirror::new(stepped.base, PlaneReference::Principal(PrincipalPlane::Yz))
        .mirroring(vec![stepped.peg]);

    add(
        &mut stepped.document,
        "Mirror 1",
        FeatureKind::Mirror(mirror),
    );

    let evaluation = evaluate(&stepped.document, &mut Recompute::default());

    assert_eq!(evaluation.failed_count(), 0);

    assert_volume(
        volume(&evaluation, stepped.base),
        BASE + STEP + PEG_SECTION * 10.0 + PEG_SECTION * 20.0,
    );

    assert_eq!(
        evaluation
            .body(stepped.base)
            .unwrap()
            .lumps()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn a_copy_whose_end_finds_no_face_fails_the_pattern_naming_the_copy() {
    let mut stepped = stepped();

    let pattern = Pattern::new(stepped.base, backwards("4", "12 mm")).repeating(vec![stepped.peg]);
    let pattern = add(
        &mut stepped.document,
        "Pattern 1",
        FeatureKind::from(pattern),
    );

    let evaluation = evaluate(&stepped.document, &mut Recompute::default());
    let error = failure(&evaluation, pattern);

    assert!(
        error
            .reason()
            .starts_with("Peg could not be repeated on copy 3, where its end is found again"),
        "{}",
        error.reason()
    );
    assert!(
        error.remedy().contains("leave copy 3 out"),
        "{}",
        error.remedy()
    );
    assert_eq!(error.fix, Some(FixTarget::Feature(pattern)));
}

struct Drilled {
    document: Document,
    plate: FeatureId,
    hole: FeatureId,
}

fn drilled(diameter: impl FnOnce(&mut TransactionBuilder<'_>) -> Expression) -> Drilled {
    let mut document = Document::default();

    let mut transaction = document.transaction("Build");

    let outline = transaction.add_feature(
        "Plate outline",
        FeatureKind::from(rectangle((-20.0, 0.0), (20.0, 10.0))),
    );
    let plate = transaction.add_feature(
        "Plate",
        extrude(
            outline,
            ExtrudeExtent::one_side(length("10 mm"), false),
            BodyOperation::NewBody,
        ),
    );
    let diameter = diameter(&mut transaction);
    let mut points = on_plane(Sketch::new(Plane::XY), 10.0);

    points.add_point(Point2::new(-10.0, 5.0));

    let points = transaction.add_feature("Hole sketch", FeatureKind::from(points));
    let hole = transaction.add_feature(
        "Hole 1",
        FeatureKind::Hole(Hole {
            sketch: points,
            body: plate,
            diameter,
            depth: HoleDepth::ThroughAll,
            style: HoleStyle::Plain,
            reversed: false,
            shape: HoleShape::Round,
            standard: None,
            sizing: HoleSizing::Typed,
            bottom: HoleBottom::Flat,
            thread: TappedThread::default(),
        }),
    );
    let block_outline = transaction.add_feature(
        "Block outline",
        FeatureKind::from(rectangle((0.0, 0.0), (20.0, 10.0))),
    );

    transaction.add_feature(
        "Block",
        extrude(
            block_outline,
            ExtrudeExtent::one_side(length("10 mm"), true),
            BodyOperation::Add(plate),
        ),
    );
    document.apply(transaction.finish()).unwrap();
    Drilled {
        document,
        plate,
        hole,
    }
}

fn across() -> PatternKind {
    PatternKind::Linear {
        first: LinearDirection {
            axis: AxisReference::Principal(PrincipalAxis::X),
            count: length("2"),
            spacing: length("20 mm"),
            measured: LinearSpacing::BetweenCopies,
            reversed: false,
        },
        second: None,
    }
}

fn measured_diameter(transaction: &mut TransactionBuilder<'_>) -> Expression {
    let gauge = transaction.add_parameter("gauge", length("0 mm"));

    transaction.add_parameter("bore", transaction.parse("gauge / 2").unwrap());

    let mark = transaction.add_feature(
        "Mark",
        FeatureKind::Datum(Datum::Point(DatumPoint {
            base: PointReference::Origin,
            offset: [length("40 mm"), length("0 mm"), length("0 mm")],
        })),
    );

    transaction.add_feature(
        "Measurement 1",
        FeatureKind::from(Measurement {
            reading: Reading::Between {
                quantity: Between::Distance,
                first: MeasuredItem::Point(PointReference::Datum(mark)),
                second: MeasuredItem::Point(PointReference::Origin),
            },
            parameter: Some(gauge),
        }),
    );
    transaction.parse("bore / 10").unwrap()
}

#[test]
fn a_patterned_hole_through_all_goes_through_what_was_added_below_it_since() {
    let mut drilled = drilled(|_| length("2 mm"));

    let pattern = Pattern::new(drilled.plate, across()).repeating(vec![drilled.hole]);

    add(
        &mut drilled.document,
        "Pattern 1",
        FeatureKind::from(pattern),
    );

    let evaluation = evaluate(&drilled.document, &mut Recompute::default());

    assert_eq!(evaluation.failed_count(), 0);

    assert_volume(
        volume(&evaluation, drilled.plate),
        BASE + STEP - PI * 10.0 - PI * 20.0,
    );
}

#[test]
fn a_patterned_hole_sized_by_a_measurement_lands_with_the_measured_diameter() {
    let mut drilled = drilled(measured_diameter);

    let pattern = Pattern::new(drilled.plate, across()).repeating(vec![drilled.hole]);

    add(
        &mut drilled.document,
        "Pattern 1",
        FeatureKind::from(pattern),
    );

    let evaluation = evaluate(&drilled.document, &mut Recompute::default());

    assert_eq!(evaluation.failed_count(), 0);

    assert_volume(
        volume(&evaluation, drilled.plate),
        BASE + STEP - PI * 10.0 - PI * 20.0,
    );
}

#[test]
fn a_mirrored_hole_sized_by_a_measurement_lands_with_the_measured_diameter() {
    let mut drilled = drilled(measured_diameter);

    let mirror = Mirror::new(drilled.plate, PlaneReference::Principal(PrincipalPlane::Yz))
        .mirroring(vec![drilled.hole]);

    add(
        &mut drilled.document,
        "Mirror 1",
        FeatureKind::Mirror(mirror),
    );

    let evaluation = evaluate(&drilled.document, &mut Recompute::default());

    assert_eq!(evaluation.failed_count(), 0);

    assert_volume(
        volume(&evaluation, drilled.plate),
        BASE + STEP - PI * 10.0 - PI * 20.0,
    );
}

fn keeps_landing(evaluation: &Evaluation, feature: FeatureId) -> bool {
    evaluation
        .feature(feature)
        .and_then(|status| status.result.as_deref())
        .and_then(FeatureResult::solid)
        .unwrap()
        .landing()
        .is_some()
}

fn assert_matches_a_fresh_recompute(
    document: &Document,
    incremental: &Evaluation,
    plate: FeatureId,
) {
    let fresh = evaluate(document, &mut Recompute::default());

    for feature in document.features() {
        let id = feature.id();
        let kept = incremental.feature(id).unwrap();
        let rebuilt = fresh.feature(id).unwrap();

        assert_eq!(kept.state, rebuilt.state, "{}", feature.name);
        assert_eq!(
            kept.result
                .as_deref()
                .and_then(FeatureResult::solid)
                .map(|solid| (&solid.solid, solid.landing().is_some())),
            rebuilt
                .result
                .as_deref()
                .and_then(FeatureResult::solid)
                .map(|solid| (&solid.solid, solid.landing().is_some())),
            "{}",
            feature.name
        );
    }
    assert_volume(volume(incremental, plate), volume(&fresh, plate));
}

#[test]
fn a_through_hole_keeps_its_landing_inputs_only_while_a_pattern_repeats_it() {
    let drilled = drilled(|_| length("2 mm"));
    let mut editor = Editor::new(drilled.document);
    let mut engine = Recompute::default();

    let alone = evaluate(editor.document(), &mut engine);

    assert!(!keeps_landing(&alone, drilled.hole));

    assert_volume(volume(&alone, drilled.plate), BASE + STEP - PI * 10.0);

    let mut transaction = editor.document().transaction("Pattern");

    transaction.add_feature(
        "Pattern 1",
        FeatureKind::from(Pattern::new(drilled.plate, across()).repeating(vec![drilled.hole])),
    );
    editor.apply(transaction.finish()).unwrap();

    let repeated = evaluate(editor.document(), &mut engine);

    assert_eq!(repeated.failed_count(), 0);
    assert!(keeps_landing(&repeated, drilled.hole));

    assert_volume(
        volume(&repeated, drilled.plate),
        BASE + STEP - PI * 10.0 - PI * 20.0,
    );
    assert_matches_a_fresh_recompute(editor.document(), &repeated, drilled.plate);

    editor.undo().unwrap();

    let undone = evaluate(editor.document(), &mut engine);

    assert!(!keeps_landing(&undone, drilled.hole));

    assert_volume(volume(&undone, drilled.plate), BASE + STEP - PI * 10.0);
    assert_matches_a_fresh_recompute(editor.document(), &undone, drilled.plate);

    editor.redo().unwrap();

    let redone = evaluate(editor.document(), &mut engine);

    assert!(keeps_landing(&redone, drilled.hole));

    assert_volume(
        volume(&redone, drilled.plate),
        BASE + STEP - PI * 10.0 - PI * 20.0,
    );
    assert_matches_a_fresh_recompute(editor.document(), &redone, drilled.plate);
}

#[test]
fn a_mirrored_through_hole_keeps_its_landing_inputs() {
    let mut drilled = drilled(|_| length("2 mm"));

    let mirror = Mirror::new(drilled.plate, PlaneReference::Principal(PrincipalPlane::Yz))
        .mirroring(vec![drilled.hole]);

    add(
        &mut drilled.document,
        "Mirror 1",
        FeatureKind::Mirror(mirror),
    );

    let evaluation = evaluate(&drilled.document, &mut Recompute::default());

    assert_eq!(evaluation.failed_count(), 0);
    assert!(keeps_landing(&evaluation, drilled.hole));
}

#[test]
fn a_long_chain_of_landing_inputs_is_measured_and_dropped_on_a_small_stack() {
    let drilled = drilled(|_| length("2 mm"));
    let hole = drilled.document.feature(drilled.hole).unwrap();
    let parameters = ParameterValues::evaluate(&drilled.document);
    let no_features = BTreeMap::new();

    let link = |before: Option<Arc<FeatureResult>>| {
        let bodies: BTreeMap<_, _> = before
            .map(|before| (drilled.plate, (drilled.hole, before)))
            .into_iter()
            .collect();
        let inputs = Inputs {
            document: &drilled.document,
            parameters: &parameters,
            features: &no_features,
            bodies: &bodies,
            previous: None,
        };
        let result = SolidResult::new(drilled.plate, Solid::default()).landing_on(hole, &inputs);

        Arc::new(FeatureResult::Solid(result))
    };

    thread::scope(|scope| {
        thread::Builder::new()
            .stack_size(SMALL_STACK)
            .spawn_scoped(scope, || {
                let chain = (0..CHAIN_LINKS).fold(None, |before, _| Some(link(before)));
                let mut results = 0;

                result_buffers(chain.as_ref().unwrap(), &mut |_| results += 1);

                assert_eq!(results, CHAIN_LINKS);

                drop(chain);
            })
            .unwrap()
            .join()
            .unwrap();
    });
}
