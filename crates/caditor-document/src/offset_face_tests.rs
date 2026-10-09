use caditor_expression::{Expression, ParameterId};
use caditor_geometry::{Plane, Point2, Vector3};
use caditor_kernel::{FaceName, FaceReference, SamplingTolerance, Solid, Surface};
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

fn top_face(solid: &Solid) -> FaceReference {
    let (id, _) = solid
        .faces()
        .find(|(_, face)| match face.surface() {
            Surface::Plane(plane) => {
                plane.frame().normal() * face.sense().sign() == Vector3::Z
                    && plane.frame().origin().z > 0.0
            }
            _ => false,
        })
        .unwrap();
    FaceReference::capture(solid, id).unwrap()
}

fn failure(evaluation: &Evaluation, feature: FeatureId) -> FeatureError {
    match &evaluation.feature(feature).unwrap().state {
        FeatureState::Failed(error) => error.clone(),
        other => panic!("expected a failure, found {other:?}"),
    }
}

struct Model {
    document: Document,
    engine: Recompute,
    height: ParameterId,
    lift: ParameterId,
    base: FeatureId,
    offset: FeatureId,
}

fn model() -> Model {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let height = transaction.add_parameter("height", transaction.parse("4 mm").unwrap());
    let lift = transaction.add_parameter("lift", transaction.parse("1 mm").unwrap());
    let outline = transaction.add_feature(
        "Outline",
        FeatureKind::from(rectangle((0.0, 0.0), (10.0, 8.0))),
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
            taper: None,
            wall: None,
            direction: None,
        })),
    );
    document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();
    let evaluation = evaluate(&document, &mut engine);
    let faces = vec![top_face(evaluation.body(base).unwrap())];
    let mut transaction = document.transaction("Offset face");
    let offset = transaction.add_feature(
        "Offset face 1",
        FeatureKind::OffsetFace(OffsetFace {
            body: base,
            faces,
            distance: Expression::Parameter(lift),
            tangent: false,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    Model {
        document,
        engine,
        height,
        lift,
        base,
        offset,
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

fn set_offset(model: &mut Model, change: impl FnOnce(&mut OffsetFace)) {
    let mut definition = model
        .document
        .feature(model.offset)
        .unwrap()
        .kind
        .offset_face()
        .unwrap()
        .clone();
    change(&mut definition);
    model
        .document
        .apply(Transaction::single(
            "Edit",
            Edit::SetFeatureKind {
                id: model.offset,
                kind: FeatureKind::OffsetFace(definition),
            },
        ))
        .unwrap();
}

fn assert_volume(evaluation: &Evaluation, body: FeatureId, expected: f64) {
    let found = volume(evaluation, body);
    assert!(
        (found - expected).abs() < 0.01 * expected,
        "volume {found} instead of {expected}"
    );
}

#[test]
fn a_moved_face_follows_its_distance_and_upstream_edits() {
    let mut model = model();
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_eq!(evaluation.failed_count(), 0);
    assert_eq!(
        evaluation.bodies().collect::<Vec<_>>(),
        vec![(model.base, model.offset)]
    );
    assert_volume(&evaluation, model.base, 80.0 * 5.0);
    assert!(evaluation.body_before(model.offset).is_some());

    let id = model.lift;
    set(&mut model, id, "-1.5 mm");
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_volume(&evaluation, model.base, 80.0 * 2.5);

    let id = model.height;
    set(&mut model, id, "6 mm");
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_eq!(evaluation.failed_count(), 0);
    assert_volume(&evaluation, model.base, 80.0 * 4.5);
}

#[test]
fn offset_face_errors_name_the_problem_and_the_fix() {
    let mut model = model();
    let id = model.lift;
    set(&mut model, id, "0 mm");
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.offset);
    assert_eq!(error.reason, "The distance must be more than 0.000001 mm.");
    assert_eq!(error.fix, Some(FixTarget::Feature(model.offset)));

    set(&mut model, id, "30 deg");
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.offset);
    assert_eq!(
        error.remedy,
        "Edit the distance so it gives a length, such as 2 mm."
    );

    set(&mut model, id, "-5 mm");
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.offset);
    assert_eq!(error.remedy, "Enter a smaller distance.");
    assert!(evaluation.body(model.base).is_some());

    set(&mut model, id, "1 mm");
    set_offset(&mut model, |offset| {
        offset
            .faces
            .push(FaceReference::new(FaceName::from_digest(7), None, []));
    });
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.offset);
    assert_eq!(
        error.reason,
        "A face to move is no longer part of the body of Base."
    );

    set_offset(&mut model, |offset| offset.faces.clear());
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.offset);
    assert_eq!(error.reason, "No face is chosen to move.");
}

#[test]
fn a_moved_face_stays_that_kind_and_keeps_its_body_in_use() {
    let model = model();
    let sketch = FeatureKind::from(rectangle((0.0, 0.0), (1.0, 1.0)));
    assert!(matches!(
        model.document.clone().apply(Transaction::single(
            "Change",
            Edit::SetFeatureKind {
                id: model.offset,
                kind: sketch
            }
        )),
        Err(EditError::KindChange(_))
    ));
    assert!(
        model
            .document
            .dependents_of(&[model.base])
            .contains(&model.offset)
    );
}
