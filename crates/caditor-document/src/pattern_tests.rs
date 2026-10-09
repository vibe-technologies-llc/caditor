use std::collections::BTreeSet;

use caditor_expression::{Expression, ParameterId};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::{
    FaceId, FaceName, FaceOrigin, FaceReference, SamplingTolerance, Solid, Surface,
};
use caditor_sketch::Sketch;

use crate::*;

fn rectangle(min: (f64, f64), max: (f64, f64)) -> Sketch {
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

fn evaluate(document: &Document, engine: &mut Recompute) -> Evaluation {
    engine.run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
}

fn volume(evaluation: &Evaluation, body: FeatureId) -> f64 {
    evaluation
        .body(body)
        .unwrap()
        .tessellate(&SamplingTolerance::new(1e-3, 0.05).unwrap())
        .unwrap()
        .mass_properties()
        .volume
}

fn assert_volume(evaluation: &Evaluation, body: FeatureId, expected: f64) {
    let found = volume(evaluation, body);
    assert!(
        (found - expected).abs() < 0.01 * expected,
        "volume {found} instead of {expected}"
    );
}

pub(crate) fn failure(evaluation: &Evaluation, feature: FeatureId) -> FeatureError {
    match &evaluation.feature(feature).unwrap().state {
        FeatureState::Failed(error) => error.clone(),
        other => panic!("expected a failure, found {other:?}"),
    }
}

fn along(axis: PrincipalAxis, count: Expression, spacing: &str) -> LinearDirection {
    LinearDirection {
        axis: AxisReference::Principal(axis),
        count,
        spacing: Expression::parse(spacing, &|_| None).unwrap(),
        measured: LinearSpacing::BetweenCopies,
        reversed: false,
    }
}

struct Model {
    document: Document,
    engine: Recompute,
    height: ParameterId,
    count: ParameterId,
    base: FeatureId,
    pattern: FeatureId,
}

fn model(kind: impl FnOnce(ParameterId) -> PatternKind) -> Model {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let height = transaction.add_parameter("height", transaction.parse("4 mm").unwrap());
    let count = transaction.add_parameter("count", transaction.parse("3").unwrap());
    let outline = transaction.add_feature(
        "Outline",
        FeatureKind::from(rectangle((10.0, 0.0), (20.0, 8.0))),
    );
    let base = transaction.add_feature(
        "Base",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::Parameter(height), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
        })),
    );
    let pattern = transaction.add_feature(
        "Pattern 1",
        FeatureKind::from(Pattern::new(base, kind(count))),
    );
    document.apply(transaction.finish()).unwrap();
    Model {
        document,
        engine: Recompute::default(),
        height,
        count,
        base,
        pattern,
    }
}

fn linear(count: ParameterId) -> PatternKind {
    PatternKind::Linear {
        first: along(PrincipalAxis::X, Expression::Parameter(count), "20 mm"),
        second: None,
    }
}

fn set(model: &mut Model, parameter: ParameterId, text: &str) {
    let expression = model.document.parse(text).unwrap();
    model
        .document
        .apply(Transaction::single(
            "Edit",
            Edit::SetParameterExpression {
                id: parameter,
                expression,
            },
        ))
        .unwrap();
}

fn set_kind(model: &mut Model, kind: PatternKind) -> Result<(), EditError> {
    let body = model.base;
    set_pattern(model, Pattern::new(body, kind))
}

fn set_pattern(model: &mut Model, pattern: Pattern) -> Result<(), EditError> {
    model
        .document
        .apply(Transaction::single(
            "Edit",
            Edit::SetFeatureKind {
                id: model.pattern,
                kind: FeatureKind::from(pattern),
            },
        ))
        .map(|_| ())
}

fn top_of_copy(model: &Model, solid: &Solid, copy: [u32; 2]) -> Option<FaceReference> {
    let cap = FaceOrigin::EndCap {
        feature: model.base.raw(),
    };
    let tops: Vec<(FaceId, FaceName)> = solid
        .faces()
        .filter(|(_, face)| face.origin().map(FaceOrigin::original) == Some(cap))
        .map(|(id, face)| (id, face.name()))
        .collect();
    let (id, _) = tops.iter().find(|(_, name)| {
        tops.iter()
            .any(|(_, original)| FaceName::pattern(model.pattern.raw(), copy, *original) == *name)
    })?;
    FaceReference::capture(solid, *id)
}

fn plane_origin(solid: &Solid, reference: &FaceReference) -> Point3 {
    let face = reference.resolve(solid).unwrap();
    match solid.face(face).unwrap().surface() {
        Surface::Plane(plane) => plane.frame().origin(),
        other => panic!("the top is not flat: {other:?}"),
    }
}

#[test]
fn a_linear_pattern_repeats_its_body_and_follows_its_count() {
    let mut model = model(linear);
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_eq!(evaluation.failed_count(), 0);
    assert_eq!(
        evaluation.bodies().collect::<Vec<_>>(),
        vec![(model.base, model.pattern)]
    );
    assert_volume(&evaluation, model.base, 3.0 * 320.0);
    assert_eq!(evaluation.body(model.base).unwrap().faces().count(), 18);
    assert!(evaluation.body_before(model.pattern).is_none());

    let id = model.count;
    set(&mut model, id, "5");
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_eq!(evaluation.failed_count(), 0);
    assert_volume(&evaluation, model.base, 5.0 * 320.0);

    set(&mut model, id, "1");
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_eq!(evaluation.failed_count(), 0);
    assert_volume(&evaluation, model.base, 320.0);
}

#[test]
fn a_reference_to_a_copied_face_survives_upstream_edits_and_a_new_count() {
    let mut model = model(linear);
    let evaluation = evaluate(&model.document, &mut model.engine);
    let solid = evaluation.body(model.base).unwrap();
    let reference = top_of_copy(&model, solid, [2, 0]).unwrap();
    let origin = plane_origin(solid, &reference);
    assert!((origin.x - 40.0).abs() < 1e-9 && (origin.z - 4.0).abs() < 1e-9);

    let id = model.height;
    set(&mut model, id, "6 mm");
    let id = model.count;
    set(&mut model, id, "4");
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_eq!(evaluation.failed_count(), 0);
    let origin = plane_origin(evaluation.body(model.base).unwrap(), &reference);
    assert!((origin.x - 40.0).abs() < 1e-9 && (origin.z - 6.0).abs() < 1e-9);
}

fn opening(model: &mut Model, name: &str, top: FaceReference) -> FeatureId {
    let mut transaction = model.document.transaction("Shell");
    let shell = transaction.add_feature(
        name,
        FeatureKind::Shell(Shell {
            body: model.base,
            open: vec![top],
            thickness: Expression::parse("1 mm", &|_| None).unwrap(),
        }),
    );
    model.document.apply(transaction.finish()).unwrap();
    shell
}

#[test]
fn a_feature_holding_a_copied_face_depends_on_the_pattern_that_made_it() {
    let mut model = model(linear);
    let evaluation = evaluate(&model.document, &mut model.engine);
    let solid = evaluation.body(model.base).unwrap();
    let copied = top_of_copy(&model, solid, [2, 0]).unwrap();
    let cap = FaceOrigin::EndCap {
        feature: model.base.raw(),
    };
    let original = solid
        .faces()
        .find(|(_, face)| face.origin() == Some(cap))
        .and_then(|(id, _)| FaceReference::capture(solid, id))
        .unwrap();

    let on_copy = opening(&mut model, "Shell 1", copied.clone());
    let on_original = opening(&mut model, "Shell 2", original);
    let dependents = model.document.dependents_of(&[model.pattern]);

    assert!(dependents.contains(&on_copy));
    assert!(!dependents.contains(&on_original));
    assert!(
        model
            .document
            .dependents_of(&[model.base])
            .contains(&on_copy)
    );
    assert_eq!(
        describe_origin(&model.document, copied.origin()),
        "Pattern 1 copy 2 of Base end face"
    );
    assert_eq!(origin_feature(copied.origin().unwrap()), model.pattern);
}

#[test]
fn overlapping_copies_and_a_second_direction_join_into_one_body() {
    let mut model = model(linear);
    let count = Expression::Parameter(model.count);
    set_kind(
        &mut model,
        PatternKind::Linear {
            first: along(PrincipalAxis::X, count.clone(), "5 mm"),
            second: Some(along(PrincipalAxis::Y, Expression::Number(2.0), "20 mm")),
        },
    )
    .unwrap();
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_eq!(evaluation.failed_count(), 0);
    assert_volume(&evaluation, model.base, 2.0 * 20.0 * 8.0 * 4.0);
    assert_eq!(evaluation.body(model.base).unwrap().shells().count(), 2);
}

#[test]
fn a_circular_pattern_spaces_copies_evenly_around_a_full_turn_or_over_its_angle() {
    let mut model = model(|count| {
        PatternKind::Circular(CircularPattern {
            axis: AxisReference::Principal(PrincipalAxis::Z),
            count: Expression::Parameter(count),
            angle: Expression::parse("360 deg", &|_| None).unwrap(),
            reversed: false,
        })
    });
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_eq!(evaluation.failed_count(), 0);
    assert_volume(&evaluation, model.base, 3.0 * 320.0);
    assert_eq!(evaluation.body(model.base).unwrap().shells().count(), 3);

    let count = Expression::Parameter(model.count);
    set_kind(
        &mut model,
        PatternKind::Circular(CircularPattern {
            axis: AxisReference::Principal(PrincipalAxis::Z),
            count,
            angle: Expression::parse("90 deg", &|_| None).unwrap(),
            reversed: true,
        }),
    )
    .unwrap();
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_eq!(evaluation.failed_count(), 0);
    let bounds = evaluation.body(model.base).unwrap().bounding_box().unwrap();
    assert!((bounds.max().x - 20.0).abs() < 1e-6);
    assert!((bounds.min().y + 20.0).abs() < 1e-6);
    assert!(bounds.max().y < 8.0 + 1e-6);
}

#[test]
fn pattern_errors_name_the_problem_and_the_fix() {
    let mut model = model(linear);
    let id = model.count;
    let cases = [
        ("0", "The count must be at least 1."),
        ("2.5", "The count must be a whole number, and 2.5 is not."),
        (
            "101",
            "The pattern would make 101 instances of the body of Base, and at most 100 are \
             allowed.",
        ),
    ];
    for (text, reason) in cases {
        set(&mut model, id, text);
        let evaluation = evaluate(&model.document, &mut model.engine);
        let error = failure(&evaluation, model.pattern);
        assert_eq!(error.reason, reason);
        assert_eq!(error.fix, Some(FixTarget::Feature(model.pattern)));
        assert_volume(&evaluation, model.base, 320.0);
    }

    set(&mut model, id, "3 mm");
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_eq!(
        failure(&evaluation, model.pattern).remedy,
        "Edit the count so it gives a plain number, such as 4."
    );

    set(&mut model, id, "11");
    let count = Expression::Parameter(model.count);
    set_kind(
        &mut model,
        PatternKind::Linear {
            first: along(PrincipalAxis::X, count.clone(), "20 mm"),
            second: Some(along(PrincipalAxis::Y, Expression::Number(10.0), "20 mm")),
        },
    )
    .unwrap();
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_eq!(
        failure(&evaluation, model.pattern).reason,
        "The pattern would make 110 instances of the body of Base, and at most 100 are allowed."
    );

    set(&mut model, id, "3");
    set_kind(
        &mut model,
        PatternKind::Linear {
            first: along(PrincipalAxis::X, count.clone(), "0 mm"),
            second: None,
        },
    )
    .unwrap();
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_eq!(
        failure(&evaluation, model.pattern).reason,
        "The spacing must be more than zero."
    );

    set_kind(
        &mut model,
        PatternKind::Linear {
            first: along(PrincipalAxis::X, count.clone(), "20 mm"),
            second: Some(along(PrincipalAxis::X, Expression::Number(2.0), "5 mm")),
        },
    )
    .unwrap();
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_eq!(
        failure(&evaluation, model.pattern).reason,
        "The two directions are parallel, so the copies would fall on one line."
    );

    set_kind(
        &mut model,
        PatternKind::Linear {
            first: along(PrincipalAxis::X, count.clone(), "600 m"),
            second: None,
        },
    )
    .unwrap();
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_eq!(
        failure(&evaluation, model.pattern).reason,
        "The copies would reach farther than a kilometre, the largest size caditor models."
    );

    set_kind(
        &mut model,
        PatternKind::Circular(CircularPattern {
            axis: AxisReference::Principal(PrincipalAxis::Z),
            count,
            angle: Expression::parse("400 deg", &|_| None).unwrap(),
            reversed: false,
        }),
    )
    .unwrap();
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_eq!(
        failure(&evaluation, model.pattern).reason,
        "The angle must be more than 0° and at most 360°."
    );
}

#[test]
fn a_pattern_keeps_its_body_and_datum_axis_in_use_and_undoes_as_one_step() {
    let mut model = model(linear);
    let mut transaction = model.document.transaction("Axis");
    let axis = transaction.add_feature(
        "Axis 1",
        FeatureKind::Datum(Datum::Axis(DatumAxis::Along(AxisReference::Principal(
            PrincipalAxis::Y,
        )))),
    );
    model.document.apply(transaction.finish()).unwrap();
    let moved = model.document.apply(Transaction::single(
        "Move",
        Edit::MoveFeature { id: axis, index: 0 },
    ));
    assert!(moved.is_ok());
    let count = Expression::Parameter(model.count);
    set_kind(
        &mut model,
        PatternKind::Circular(CircularPattern {
            axis: AxisReference::Datum(axis),
            count: count.clone(),
            angle: Expression::parse("360 deg", &|_| None).unwrap(),
            reversed: false,
        }),
    )
    .unwrap();
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_eq!(evaluation.failed_count(), 0);
    assert_volume(&evaluation, model.base, 3.0 * 320.0);
    for used in [model.base, axis] {
        assert!(
            model
                .document
                .dependents_of(&[used])
                .contains(&model.pattern)
        );
    }
    assert!(matches!(
        model.document.apply(Transaction::single(
            "Change",
            Edit::SetFeatureKind {
                id: model.pattern,
                kind: FeatureKind::from(rectangle((0.0, 0.0), (1.0, 1.0))),
            }
        )),
        Err(EditError::KindChange(_))
    ));
    let base = model.base;
    assert!(matches!(
        set_kind(
            &mut model,
            PatternKind::Circular(CircularPattern {
                axis: AxisReference::Datum(base),
                count,
                angle: Expression::Number(360.0),
                reversed: false,
            }),
        ),
        Err(EditError::NotAnAxis(_))
    ));

    let before = model.document.clone();
    let mut editor = Editor::new(model.document.clone());
    editor
        .apply(Transaction::single(
            "Delete Pattern 1",
            Edit::RemoveFeature { id: model.pattern },
        ))
        .unwrap();
    assert!(editor.document().feature(model.pattern).is_none());
    editor.undo().unwrap();
    assert!(editor.document().same_content(&before));
    editor.redo().unwrap();
    assert!(editor.document().feature(model.pattern).is_none());
}

#[test]
fn a_total_length_spreads_the_copies_from_the_first_to_the_last() {
    let mut model = model(linear);
    let count = Expression::Parameter(model.count);
    let mut first = along(PrincipalAxis::X, count, "60 mm");
    first.measured = LinearSpacing::Total;
    set_kind(
        &mut model,
        PatternKind::Linear {
            first,
            second: None,
        },
    )
    .unwrap();

    let evaluation = evaluate(&model.document, &mut model.engine);
    let solid = evaluation.body(model.base).unwrap();
    let last = top_of_copy(&model, solid, [2, 0]).unwrap();

    assert_eq!(evaluation.failed_count(), 0);
    assert_volume(&evaluation, model.base, 3.0 * 320.0);
    assert!((plane_origin(solid, &last).x - 60.0).abs() < 1e-9);

    let id = model.count;
    set(&mut model, id, "4");
    let evaluation = evaluate(&model.document, &mut model.engine);
    let solid = evaluation.body(model.base).unwrap();
    let last = top_of_copy(&model, solid, [3, 0]).unwrap();

    assert_eq!(evaluation.failed_count(), 0);
    assert_volume(&evaluation, model.base, 4.0 * 320.0);
    assert!((plane_origin(solid, &last).x - 60.0).abs() < 1e-9);
}

#[test]
fn a_zero_total_length_is_refused_in_its_own_words() {
    let mut model = model(linear);
    let mut first = along(PrincipalAxis::X, Expression::Number(3.0), "0 mm");
    first.measured = LinearSpacing::Total;
    set_kind(
        &mut model,
        PatternKind::Linear {
            first,
            second: None,
        },
    )
    .unwrap();

    let evaluation = evaluate(&model.document, &mut model.engine);

    assert_eq!(
        failure(&evaluation, model.pattern).reason,
        "The total length must be more than zero."
    );
}

#[test]
fn instances_left_out_are_not_made_and_the_others_keep_their_names() {
    let mut model = model(linear);
    let evaluation = evaluate(&model.document, &mut model.engine);
    let reference = top_of_copy(&model, evaluation.body(model.base).unwrap(), [2, 0]).unwrap();

    let skipping = Pattern::new(model.base, linear(model.count))
        .toggled([1, 0])
        .unwrap();
    set_pattern(&mut model, skipping.clone()).unwrap();
    let evaluation = evaluate(&model.document, &mut model.engine);
    let solid = evaluation.body(model.base).unwrap();

    assert_eq!(evaluation.failed_count(), 0);
    assert_volume(&evaluation, model.base, 2.0 * 320.0);
    assert_eq!(solid.shells().count(), 2);
    assert!(top_of_copy(&model, solid, [1, 0]).is_none());
    assert!((plane_origin(solid, &reference).x - 40.0).abs() < 1e-9);
    assert!(skipping.is_skipped([1, 0]));
    assert!(skipping.toggled(ORIGINAL_INSTANCE).is_none());

    let restored = skipping.toggled([1, 0]).unwrap();

    assert!(restored.skipped.is_empty());
}

#[test]
fn leaving_out_an_instance_past_the_count_changes_nothing() {
    let mut model = model(linear);
    let skipping = Pattern::new(model.base, linear(model.count))
        .toggled([7, 0])
        .unwrap();
    set_pattern(&mut model, skipping).unwrap();

    let evaluation = evaluate(&model.document, &mut model.engine);

    assert_eq!(evaluation.failed_count(), 0);
    assert_volume(&evaluation, model.base, 3.0 * 320.0);
}

struct Seeded {
    document: Document,
    plate: FeatureId,
    hole: FeatureId,
    boss: FeatureId,
    pattern: FeatureId,
}

pub(crate) fn top() -> Plane {
    Plane::from_frame(Point3::new(0.0, 0.0, 4.0), Vector3::Z, Vector3::X).unwrap()
}

pub(crate) fn on_top(min: (f64, f64), max: (f64, f64)) -> Sketch {
    let mut sketch = rectangle(min, max);
    sketch.set_plane(top());
    sketch
}

pub(crate) fn extrusion(sketch: FeatureId, height: &str, operation: BodyOperation) -> FeatureKind {
    FeatureKind::Solid(SolidFeature::Extrude(Extrude {
        sketch,
        regions: RegionChoice::All,
        extent: ExtrudeExtent::one_side(Expression::parse(height, &|_| None).unwrap(), false),
        operation,
        start: None,
        other_bodies: Vec::new(),
    }))
}

fn seeded(repeated: impl FnOnce(&Seeded) -> Vec<FeatureId>) -> Seeded {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let outline = transaction.add_feature(
        "Outline",
        FeatureKind::from(rectangle((0.0, 0.0), (60.0, 10.0))),
    );
    let plate =
        transaction.add_feature("Plate", extrusion(outline, "4 mm", BodyOperation::NewBody));
    let mut points = Sketch::new(top());
    points.add_point(Point2::new(5.0, 5.0));
    let points = transaction.add_feature("Hole sketch", FeatureKind::from(points));
    let hole = transaction.add_feature(
        "Hole 1",
        FeatureKind::Hole(Hole {
            sketch: points,
            body: plate,
            diameter: Expression::parse("2 mm", &|_| None).unwrap(),
            depth: HoleDepth::ThroughAll,
            style: HoleStyle::Plain,
            reversed: false,
            shape: HoleShape::Round,
            standard: None,
            sizing: HoleSizing::Typed,
            bottom: HoleBottom::Flat,
        }),
    );
    let boss_outline = transaction.add_feature(
        "Boss outline",
        FeatureKind::from(on_top((8.0, 2.0), (12.0, 8.0))),
    );
    let boss = transaction.add_feature(
        "Boss",
        extrusion(boss_outline, "2 mm", BodyOperation::Add(plate)),
    );
    document.apply(transaction.finish()).unwrap();
    let mut seeded = Seeded {
        document,
        plate,
        hole,
        boss,
        pattern: FeatureId::from_raw(0),
    };
    let pattern = Pattern::new(
        plate,
        PatternKind::Linear {
            first: along(PrincipalAxis::X, Expression::Number(3.0), "20 mm"),
            second: None,
        },
    )
    .repeating(repeated(&seeded));
    let mut transaction = seeded.document.transaction("Pattern");
    seeded.pattern = transaction.add_feature("Pattern 1", FeatureKind::from(pattern));
    seeded.document.apply(transaction.finish()).unwrap();
    seeded
}

#[test]
fn a_pattern_of_features_repeats_their_holes_and_bosses_on_the_body() {
    let seeded = seeded(|seeded| vec![seeded.hole, seeded.boss]);
    let evaluation = evaluate(&seeded.document, &mut Recompute::default());

    assert_eq!(evaluation.failed_count(), 0);
    let hole = std::f64::consts::PI * 4.0;
    assert_volume(
        &evaluation,
        seeded.plate,
        60.0 * 10.0 * 4.0 - 3.0 * hole + 3.0 * 4.0 * 6.0 * 2.0,
    );
    assert_eq!(evaluation.cuts(seeded.pattern).len(), 1);
    let solid = evaluation.body(seeded.plate).unwrap();
    let copied: BTreeSet<[u32; 2]> = solid
        .faces()
        .filter_map(|(_, face)| face.origin()?.copy())
        .filter(|copy| copy.pattern == seeded.pattern.raw())
        .map(|copy| copy.index)
        .collect();
    assert_eq!(copied, BTreeSet::from([[1, 0], [2, 0]]));
    assert!(
        seeded
            .document
            .feature(seeded.pattern)
            .unwrap()
            .kind
            .features()
            .contains(&seeded.hole)
    );
}

#[test]
fn a_feature_that_made_the_body_is_not_repeated_but_named() {
    let seeded = seeded(|seeded| vec![seeded.plate]);
    let evaluation = evaluate(&seeded.document, &mut Recompute::default());

    let error = failure(&evaluation, seeded.pattern);
    assert_eq!(
        error.reason,
        "Plate neither adds to nor removes from a body, so it cannot be repeated."
    );
    assert_eq!(error.fix, Some(FixTarget::Feature(seeded.plate)));
}
