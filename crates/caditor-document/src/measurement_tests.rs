use caditor_expression::{Dimension, Expression, ParameterId, Quantity, Unit};
use caditor_geometry::{Plane, Point2, Point3};
use caditor_kernel::{FaceForm, FaceReference, Solid, VertexName, face_form, vertex_names};
use caditor_sketch::Sketch;

use crate::*;

const CLOSE: f64 = 1e-9;

fn millimetres(value: f64) -> Expression {
    Expression::Measure(value, Unit::Millimetre)
}

fn evaluate(document: &Document) -> Evaluation {
    Recompute::default().run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
}

fn failure(evaluation: &Evaluation, feature: FeatureId) -> FeatureError {
    match &evaluation.feature(feature).unwrap().state {
        FeatureState::Failed(error) => error.clone(),
        other => panic!("expected a failure, found {other:?}"),
    }
}

fn point(evaluation: &Evaluation, feature: FeatureId) -> Point3 {
    let status = evaluation.feature(feature).unwrap();
    assert_eq!(status.state, FeatureState::UpToDate);
    status
        .result
        .as_deref()
        .unwrap()
        .datum()
        .unwrap()
        .point()
        .unwrap()
}

fn reading(evaluation: &Evaluation, feature: FeatureId) -> MeasurementResult {
    let status = evaluation.feature(feature).unwrap();
    assert_eq!(status.state, FeatureState::UpToDate, "{status:?}");
    *status.result.as_deref().unwrap().measurement().unwrap()
}

struct Model {
    document: Document,
    height: ParameterId,
    base: FeatureId,
}

fn model() -> Model {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let height = transaction.add_parameter("height", transaction.parse("4 mm").unwrap());
    let mut outline = Sketch::new(Plane::XY);
    let corners = [
        Point2::new(0.0, 0.0),
        Point2::new(10.0, 0.0),
        Point2::new(10.0, 8.0),
        Point2::new(0.0, 8.0),
    ];
    for index in 0..4 {
        outline.add_line(corners[index], corners[(index + 1) % 4]);
    }
    let outline = transaction.add_feature("Outline", FeatureKind::from(outline));
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
    Model {
        document,
        height,
        base,
    }
}

fn corner(solid: &Solid, point: Point3) -> VertexName {
    let names = vertex_names(solid);
    let (id, _) = solid
        .vertices()
        .find(|(_, vertex)| vertex.point().distance(point) < CLOSE)
        .unwrap();
    names
        .iter()
        .find(|(vertex, _)| **vertex == id)
        .map(|(_, name)| *name)
        .unwrap()
}

fn top_face(solid: &Solid) -> FaceReference {
    let (top, _) = solid
        .faces()
        .find(|(face, _)| {
            matches!(
                face_form(solid, *face),
                Ok(FaceForm::Plane { origin, normal }) if normal.z > 0.5 && origin.z > 1.0
            )
        })
        .unwrap();
    FaceReference::capture(solid, top).unwrap()
}

fn keep(model: &mut Model, name: &str, reading: Reading) -> (FeatureId, ParameterId) {
    let mut transaction = model.document.transaction("Keep");
    let parameter = transaction.add_parameter(name, millimetres(0.0));
    let measurement = transaction.add_feature(
        "Measurement 1",
        FeatureKind::from(Measurement {
            reading,
            parameter: Some(parameter),
        }),
    );
    model.document.apply(transaction.finish()).unwrap();
    (measurement, parameter)
}

fn height_of_corner(model: &Model) -> Reading {
    let evaluation = evaluate(&model.document);
    let solid = evaluation.body(model.base).unwrap();
    Reading::Between {
        quantity: Between::Distance,
        first: MeasuredItem::Point(PointReference::Vertex {
            body: model.base,
            vertex: corner(solid, Point3::new(10.0, 8.0, 4.0)),
        }),
        second: MeasuredItem::Plane(PlaneReference::Principal(PrincipalPlane::Xy)),
    }
}

fn reader(model: &mut Model, gap: ParameterId) -> FeatureId {
    let mut transaction = model.document.transaction("Add");
    let feature = transaction.add_feature(
        "Reader",
        FeatureKind::Datum(Datum::Point(DatumPoint {
            base: PointReference::Origin,
            offset: [
                Expression::Parameter(gap),
                millimetres(0.0),
                millimetres(0.0),
            ],
        })),
    );
    model.document.apply(transaction.finish()).unwrap();
    feature
}

fn set_height(model: &mut Model, value: f64) {
    model
        .document
        .apply(Transaction::single(
            "Height",
            Edit::SetParameterExpression {
                id: model.height,
                expression: millimetres(value),
            },
        ))
        .unwrap();
}

#[test]
fn a_feature_below_a_measurement_reads_it_and_follows_upstream_changes() {
    let mut model = model();
    let kept = height_of_corner(&model);
    let (measurement, gap) = keep(&mut model, "clearance", kept);
    let reader = reader(&mut model, gap);

    let first = evaluate(&model.document);
    set_height(&mut model, 6.5);
    let second = evaluate(&model.document);

    assert!((reading_value(&first, measurement) - 4.0).abs() < CLOSE);
    assert!((point(&first, reader).x - 4.0).abs() < CLOSE);
    assert!((point(&second, reader).x - 6.5).abs() < CLOSE);
    assert_eq!(
        second.parameters.value(gap),
        Ok(Quantity::length(6.5)),
        "the parameter shows what was measured"
    );
    let line = reading(&second, measurement).line.unwrap();
    assert!((line.0.z - line.1.z).abs() > 6.0);
}

fn reading_value(evaluation: &Evaluation, measurement: FeatureId) -> f64 {
    reading(evaluation, measurement).value.value
}

#[test]
fn a_measurement_reads_the_area_of_a_face() {
    let mut model = model();
    let evaluation = evaluate(&model.document);
    let face = top_face(evaluation.body(model.base).unwrap());
    let body = model.base;
    let (measurement, _) = keep(
        &mut model,
        "top",
        Reading::Of {
            quantity: Of::Area,
            item: MeasuredItem::Face { body, face },
        },
    );

    let measured = reading(&evaluate(&model.document), measurement);

    assert!((measured.value.value - 80.0).abs() < 1e-6);
    assert_eq!(measured.value.dimension, Dimension::new(2, 0));
    assert!((measured.anchor.z - 4.0).abs() < CLOSE);
}

#[test]
fn reading_a_measurement_above_it_or_in_a_parameter_is_refused_naming_the_cycle() {
    let mut model = model();
    let kept = height_of_corner(&model);
    let (measurement, gap) = keep(&mut model, "clearance", kept);
    let reader = reader(&mut model, gap);

    let mut transaction = model.document.transaction("Twice");
    transaction.add_parameter("twice", transaction.parse("clearance * 2").unwrap());
    let in_parameter = model.document.apply(transaction.finish());
    let base = model.document.feature(model.base).unwrap();
    let FeatureKind::Solid(SolidFeature::Extrude(mut extrude)) = base.kind.clone() else {
        panic!("the base is an extrusion");
    };
    extrude.extent = ExtrudeExtent::one_side(Expression::Parameter(gap), false);
    let above = model.document.apply(Transaction::single(
        "Base",
        Edit::SetFeatureKind {
            id: model.base,
            kind: FeatureKind::Solid(SolidFeature::Extrude(extrude)),
        },
    ));
    let moved_up = model.document.move_row(TreeRow::Feature(reader), 0, "Move");
    let inlined = model.document.inline_parameter(gap);

    match in_parameter {
        Err(EditError::ParameterReadsMeasurement {
            name,
            parameter,
            measurement,
        }) => {
            assert_eq!(name, "twice");
            assert_eq!(parameter, "clearance");
            assert_eq!(measurement, "Measurement 1");
        }
        other => panic!("expected a refusal, found {other:?}"),
    }
    let refusal = above.unwrap_err().to_string();
    assert!(
        refusal.contains("Base → clearance → Measurement 1 → Base"),
        "{refusal}"
    );
    assert!(moved_up.is_err());
    assert!(matches!(
        inlined,
        Err(EditError::MeasuredParameterInlined { .. })
    ));
    assert_eq!(model.document.dependents_of(&[measurement]), vec![reader]);
}

#[test]
fn a_failed_measurement_fails_the_features_reading_it_alone() {
    let mut model = model();
    let evaluation = evaluate(&model.document);
    let face = top_face(evaluation.body(model.base).unwrap());
    let body = model.base;
    let (measurement, gap) = keep(
        &mut model,
        "radius",
        Reading::Of {
            quantity: Of::Radius,
            item: MeasuredItem::Face { body, face },
        },
    );
    let reader = reader(&mut model, gap);

    let evaluation = evaluate(&model.document);

    assert!(
        failure(&evaluation, measurement)
            .reason
            .contains("has no radius")
    );
    let follows = failure(&evaluation, reader);
    assert_eq!(follows.fix, Some(FixTarget::Feature(measurement)));
    assert_eq!(
        evaluation.feature(model.base).unwrap().state,
        FeatureState::UpToDate
    );
    assert!(matches!(
        evaluation.parameters.get(gap),
        Some(Err(ParameterError::Unmeasured { .. }))
    ));
}
