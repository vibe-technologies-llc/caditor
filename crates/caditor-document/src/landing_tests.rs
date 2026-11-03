use std::f64::consts::PI;

use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_sketch::Sketch;

use crate::{
    combine_tests::{evaluate, rectangle, volume},
    pattern_tests::failure,
    *,
};

const BASE: f64 = 40.0 * 10.0 * 10.0;
const STEP: f64 = 20.0 * 10.0 * 10.0;
const PEG_SECTION: f64 = 4.0 * 4.0;

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

#[test]
fn a_patterned_hole_through_all_goes_through_what_was_added_below_it_since() {
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
    let mut points = on_plane(Sketch::new(Plane::XY), 10.0);
    points.add_point(Point2::new(-10.0, 5.0));
    let points = transaction.add_feature("Hole sketch", FeatureKind::from(points));
    let hole = transaction.add_feature(
        "Hole 1",
        FeatureKind::Hole(Hole {
            sketch: points,
            body: plate,
            diameter: length("2 mm"),
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
    let pattern = Pattern::new(
        plate,
        PatternKind::Linear {
            first: LinearDirection {
                axis: AxisReference::Principal(PrincipalAxis::X),
                count: length("2"),
                spacing: length("20 mm"),
                measured: LinearSpacing::BetweenCopies,
                reversed: false,
            },
            second: None,
        },
    )
    .repeating(vec![hole]);
    add(&mut document, "Pattern 1", FeatureKind::from(pattern));

    let evaluation = evaluate(&document, &mut Recompute::default());

    assert_eq!(evaluation.failed_count(), 0);
    assert_volume(
        volume(&evaluation, plate),
        BASE + STEP - PI * 10.0 - PI * 20.0,
    );
}
