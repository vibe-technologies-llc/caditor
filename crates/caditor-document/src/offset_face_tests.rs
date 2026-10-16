use caditor_expression::{Expression, ParameterId, Unit};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::{EdgeReference, FaceName, FaceReference, SamplingTolerance, Solid, Surface};
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
    assert_eq!(
        error.reason(),
        "The distance must be more than 0.000001 mm."
    );
    assert_eq!(error.fix, Some(FixTarget::Feature(model.offset)));

    set(&mut model, id, "30 deg");
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.offset);
    assert_eq!(
        error.remedy(),
        "Edit the distance so it gives a length, such as 2 mm."
    );

    set(&mut model, id, "-5 mm");
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.offset);
    assert_eq!(error.remedy(), "Enter a smaller distance.");
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
        error.reason(),
        "A face to move is no longer part of the body of Base."
    );

    set_offset(&mut model, |offset| offset.faces.clear());
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert!(matches!(
        evaluation.feature(model.offset).unwrap().state,
        FeatureState::Waiting(MissingInput::FacesToMove { .. })
    ));
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

fn plane_facing(solid: &Solid, normal: Vector3, level: f64) -> FaceReference {
    let (id, _) = solid
        .faces()
        .find(|(_, face)| match face.surface() {
            Surface::Plane(plane) => {
                plane.frame().normal() * face.sense().sign() == normal
                    && plane.frame().origin().dot(normal) == level
            }
            _ => false,
        })
        .unwrap();
    FaceReference::capture(solid, id).unwrap()
}

fn edge_at(solid: &Solid, point: Point3) -> EdgeReference {
    let (id, _) = solid
        .edges()
        .find(|(_, edge)| {
            let parameter = edge.curve().closest_parameter(point, edge.interval());
            edge.curve().point(parameter).distance(point) < 1e-6
        })
        .unwrap();
    EdgeReference::capture(solid, id).unwrap()
}

#[test]
fn a_fillet_on_one_rim_of_a_groove_holds_after_another_face_moves() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let outline = transaction.add_feature(
        "Outline",
        FeatureKind::from(rectangle((0.0, 0.0), (10.0, 8.0))),
    );
    let mut across = Sketch::new(
        Plane::from_frame(Point3::new(0.0, 9.0, 0.0), Vector3::NEG_Y, Vector3::X).unwrap(),
    );
    across.add_circle(Point2::new(5.0, 4.0), 1.5);
    let rod = transaction.add_feature("Rod", FeatureKind::from(across));
    let extrude = |sketch, height: f64, operation| {
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::Measure(height, Unit::Millimetre), false),
            operation,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        }))
    };
    let base = transaction.add_feature("Base", extrude(outline, 4.0, BodyOperation::NewBody));
    transaction.add_feature("Groove", extrude(rod, 10.0, BodyOperation::Remove(base)));
    document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();
    let evaluation = evaluate(&document, &mut engine);
    let right = plane_facing(evaluation.body(base).unwrap(), Vector3::X, 10.0);

    assert_eq!(evaluation.failed_count(), 0);

    let mut transaction = document.transaction("Offset face");
    transaction.add_feature(
        "Offset face 1",
        FeatureKind::OffsetFace(OffsetFace {
            body: base,
            faces: vec![right],
            distance: Expression::Measure(1.0, Unit::Millimetre),
            tangent: false,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document, &mut engine);
    let rim = edge_at(evaluation.body(base).unwrap(), Point3::new(3.5, 4.0, 4.0));
    let mut transaction = document.transaction("Fillet");
    let fillet = transaction.add_feature(
        "Fillet 1",
        FeatureKind::Blend(Blend {
            kind: BlendKind::Fillet,
            body: base,
            groups: Vec::new(),
            edges: vec![rim],
            size: Expression::Measure(0.5, Unit::Millimetre),
            form: ChamferForm::Equal,
            flipped: false,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document, &mut engine);

    assert_eq!(evaluation.failed_count(), 0);
    assert!(matches!(
        evaluation.feature(fillet).unwrap().state,
        FeatureState::UpToDate
    ));
}
