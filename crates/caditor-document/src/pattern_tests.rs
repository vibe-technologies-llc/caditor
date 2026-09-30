use caditor_expression::{Expression, ParameterId};
use caditor_geometry::{Plane, Point2, Point3};
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

fn failure(evaluation: &Evaluation, feature: FeatureId) -> FeatureError {
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
            extent: ExtrudeExtent::OneSide {
                distance: Expression::Parameter(height),
                reversed: false,
            },
            operation: BodyOperation::NewBody,
        })),
    );
    let pattern = transaction.add_feature(
        "Pattern 1",
        FeatureKind::from(Pattern {
            body: base,
            kind: kind(count),
        }),
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
    model
        .document
        .apply(Transaction::single(
            "Edit",
            Edit::SetFeatureKind {
                id: model.pattern,
                kind: FeatureKind::from(Pattern { body, kind }),
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
        .filter(|(_, face)| face.origin() == Some(cap))
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
