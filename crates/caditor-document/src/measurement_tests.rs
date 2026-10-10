use std::sync::Arc;

use caditor_expression::{Dimension, Expression, ParameterId, Quantity, Unit};
use caditor_geometry::{Plane, Point2, Point3};
use caditor_kernel::{
    FaceForm, FaceReference, MeasureError, Solid, VertexName, face_form, interruptible,
    vertex_names,
};
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
    let earlier = model
        .document
        .features()
        .filter(|feature| feature.kind.measurement().is_some())
        .count();
    let title = format!("Measurement {}", earlier + 1);
    let measurement = transaction.add_feature(
        title,
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
fn reading_a_measurement_above_it_is_refused_naming_the_cycle() {
    let mut model = model();
    let kept = height_of_corner(&model);
    let (measurement, gap) = keep(&mut model, "clearance", kept);
    let reader = reader(&mut model, gap);

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
    let height_reads_it = model.document.apply(Transaction::single(
        "Height",
        Edit::SetParameterExpression {
            id: model.height,
            expression: model.document.parse("clearance / 2").unwrap(),
        },
    ));

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
    let refusal = height_reads_it.unwrap_err().to_string();
    assert!(
        refusal.contains("Base → height → clearance → Measurement 1 → Base"),
        "{refusal}"
    );
    assert_eq!(model.document.dependents_of(&[measurement]), vec![reader]);
}

#[test]
fn a_parameter_reading_a_measurement_feeds_features_below_it() {
    let mut model = model();
    let kept = height_of_corner(&model);
    let (measurement, _) = keep(&mut model, "clearance", kept);
    let mut transaction = model.document.transaction("Half");
    let half = transaction.add_parameter("half", transaction.parse("clearance / 2").unwrap());
    model.document.apply(transaction.finish()).unwrap();
    let reader = reader(&mut model, half);

    let first = evaluate(&model.document);
    set_height(&mut model, 7.0);
    let second = evaluate(&model.document);
    let values = ParameterValues::evaluate(&model.document);
    let base = model.document.feature(model.base).unwrap();
    let FeatureKind::Solid(SolidFeature::Extrude(mut extrude)) = base.kind.clone() else {
        panic!("the base is an extrusion");
    };
    extrude.extent = ExtrudeExtent::one_side(Expression::Parameter(half), false);
    let above = model.document.apply(Transaction::single(
        "Base",
        Edit::SetFeatureKind {
            id: model.base,
            kind: FeatureKind::Solid(SolidFeature::Extrude(extrude)),
        },
    ));
    let moved_down = model
        .document
        .move_row(TreeRow::Feature(measurement), usize::MAX, "Move");

    assert!((point(&first, reader).x - 2.0).abs() < CLOSE);
    assert!((point(&second, reader).x - 3.5).abs() < CLOSE);
    assert_eq!(second.parameters.value(half), Ok(Quantity::length(3.5)));
    assert!(matches!(
        values.get(half),
        Some(Err(ParameterError::Unmeasured { .. }))
    ));
    let refusal = above.unwrap_err().to_string();
    assert!(
        refusal.contains("Base → half → clearance → Measurement 1 → Base"),
        "{refusal}"
    );
    assert!(moved_down.is_err());
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

#[test]
fn deleting_a_used_measurement_leaves_its_parameter_at_the_last_reading() {
    let mut model = model();
    let kept = height_of_corner(&model);
    let (measurement, gap) = keep(&mut model, "clearance", kept);
    let reader = reader(&mut model, gap);
    let mut editor = Editor::new(model.document.clone());

    let first = evaluate(editor.document());
    let followed = editor.follow_readings(&first);
    let again = editor.follow_readings(&first);
    editor
        .apply(Transaction::single(
            "Height",
            Edit::SetParameterExpression {
                id: model.height,
                expression: millimetres(6.5),
            },
        ))
        .unwrap();
    let second = evaluate(editor.document());
    editor.follow_readings(&second);
    let followed_value = editor.document().parameter(gap).unwrap().expression.clone();
    let deletion = editor.document().deletion(&[measurement], "Delete");
    editor.apply(deletion.clone()).unwrap();
    let after = evaluate(editor.document());

    assert!(followed);
    assert!(!again, "a reading already followed is no change");
    assert_eq!(editor.undo_labels().count(), 2, "following is no undo step");
    assert_eq!(followed_value, millimetres(6.5));
    assert!(deletion.edits().iter().any(|edit| matches!(
        edit,
        Edit::SetParameterExpression { id, expression }
            if *id == gap && *expression == millimetres(6.5)
    )));
    assert_eq!(
        editor.document().parameter(gap).unwrap().expression,
        millimetres(6.5)
    );
    assert!((point(&after, reader).x - 6.5).abs() < CLOSE);
}

#[test]
fn a_followed_area_is_kept_in_square_millimetres() {
    let area = Quantity::new(80.0, Dimension::new(2, 0));

    let literal = reading_literal(area).unwrap();

    assert_eq!(
        ParameterValues::default().evaluate_expression(&literal),
        Ok(area)
    );
}

#[test]
fn a_measurement_reads_an_offset_along_an_axis_a_perimeter_and_a_sweep() {
    let mut model = model();
    let evaluation = evaluate(&model.document);
    let solid = evaluation.body(model.base).unwrap();
    let face = top_face(solid);
    let body = model.base;
    let corner = MeasuredItem::Point(PointReference::Vertex {
        body,
        vertex: corner(solid, Point3::new(10.0, 8.0, 4.0)),
    });
    let mut arcs = Sketch::new(Plane::XY);
    let arc = arcs.add_arc(
        Point2::new(0.0, 0.0),
        Point2::new(5.0, 0.0),
        Point2::new(0.0, 5.0),
    );
    let mut transaction = model.document.transaction("Arc");
    let arcs = transaction.add_feature("Arcs", FeatureKind::from(arcs));
    model.document.apply(transaction.finish()).unwrap();
    let along = |axis| Reading::Along {
        first: MeasuredItem::Point(PointReference::Origin),
        second: corner.clone(),
        axis: MeasuredItem::Axis(AxisReference::Principal(axis)),
    };
    let (along_x, _) = keep(&mut model, "along_x", along(PrincipalAxis::X));
    let (along_z, _) = keep(&mut model, "along_z", along(PrincipalAxis::Z));
    let (perimeter, _) = keep(
        &mut model,
        "perimeter",
        Reading::Of {
            quantity: Of::Perimeter,
            item: MeasuredItem::Face {
                body,
                face: face.clone(),
            },
        },
    );
    let (sweep, _) = keep(
        &mut model,
        "sweep",
        Reading::Of {
            quantity: Of::Sweep,
            item: MeasuredItem::Sketch {
                sketch: arcs,
                entity: arc,
            },
        },
    );
    let (not_an_arc, _) = keep(
        &mut model,
        "flat",
        Reading::Of {
            quantity: Of::Sweep,
            item: MeasuredItem::Face { body, face },
        },
    );

    let evaluation = evaluate(&model.document);

    assert!((reading_value(&evaluation, along_x) - 10.0).abs() < CLOSE);
    assert!((reading_value(&evaluation, along_z) - 4.0).abs() < CLOSE);
    assert!((reading_value(&evaluation, perimeter) - 36.0).abs() < 1e-6);
    let swept = reading(&evaluation, sweep).value;
    assert_eq!(swept.dimension, Dimension::ANGLE);
    assert!((swept.value - 90.0).abs() < 1e-9);
    assert!(
        failure(&evaluation, not_an_arc)
            .reason
            .contains("is not an arc")
    );
}

fn set_density(model: &mut Model, density: f64) {
    let appearance = BodyAppearance {
        density: Some(Expression::number(density)),
        ..BodyAppearance::default()
    };
    model
        .document
        .apply(Transaction::single(
            "Density",
            Edit::SetBodyAppearance {
                id: model.base,
                appearance,
            },
        ))
        .unwrap();
}

#[test]
fn a_measurement_reads_a_point_position_along_an_axis() {
    let mut model = model();
    let evaluation = evaluate(&model.document);
    let solid = evaluation.body(model.base).unwrap();
    let top = MeasuredItem::Point(PointReference::Vertex {
        body: model.base,
        vertex: corner(solid, Point3::new(10.0, 8.0, 4.0)),
    });
    let along = |axis| MeasuredItem::Axis(AxisReference::Principal(axis));
    let (across, _) = keep(
        &mut model,
        "across",
        Reading::Position {
            item: top.clone(),
            axis: along(PrincipalAxis::Y),
        },
    );
    let (up, _) = keep(
        &mut model,
        "up",
        Reading::Position {
            item: top,
            axis: along(PrincipalAxis::Z),
        },
    );

    let first = evaluate(&model.document);
    set_height(&mut model, 6.0);
    let second = evaluate(&model.document);

    assert!((reading_value(&first, across) - 8.0).abs() < CLOSE);
    assert!((reading_value(&first, up) - 4.0).abs() < CLOSE);
    assert!((reading_value(&second, up) - 6.0).abs() < CLOSE);
    assert_eq!(
        reading(&second, up).line,
        Some((Point3::ZERO, Point3::new(0.0, 0.0, 6.0)))
    );
}

#[test]
fn a_measurement_reads_a_body_volume_area_and_centre_of_mass() {
    let mut model = model();
    let body = MeasuredItem::Body(model.base);
    let (volume, volume_parameter) = keep(
        &mut model,
        "volume",
        Reading::Of {
            quantity: Of::Volume,
            item: body.clone(),
        },
    );
    let (area, _) = keep(
        &mut model,
        "area",
        Reading::Of {
            quantity: Of::Area,
            item: body.clone(),
        },
    );
    let (centre, _) = keep(
        &mut model,
        "centre",
        Reading::Position {
            item: body,
            axis: MeasuredItem::Axis(AxisReference::Principal(PrincipalAxis::Z)),
        },
    );

    let evaluation = evaluate(&model.document);
    let followed = model.document.following_readings(&evaluation);
    model.document.apply(followed).unwrap();

    assert!((reading_value(&evaluation, volume) - 320.0).abs() < 1e-6);
    assert_eq!(
        reading(&evaluation, volume).value.dimension,
        Dimension::VOLUME
    );
    assert!((reading_value(&evaluation, area) - 304.0).abs() < 1e-6);
    assert!((reading_value(&evaluation, centre) - 2.0).abs() < 1e-6);
    assert_eq!(
        model
            .document
            .parameter(volume_parameter)
            .unwrap()
            .expression,
        Expression::WithUnit(
            Box::new(Expression::number(reading_value(&evaluation, volume))),
            Unit::Millimetre,
            3
        )
    );
}

#[test]
fn a_mass_reading_needs_a_density_and_follows_it_without_integrating_the_body_again() {
    let mut model = model();
    let body = MeasuredItem::Body(model.base);

    let (mass, _) = keep(
        &mut model,
        "mass",
        Reading::Of {
            quantity: Of::Mass,
            item: body,
        },
    );
    let mut recompute = Recompute::default();
    let mut run = |document: &Document| {
        recompute.run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
    };

    let without = run(&model.document);
    set_density(&mut model, 2.0);
    let dense = run(&model.document);
    set_density(&mut model, 3.0);
    let denser = run(&model.document);

    let error = failure(&without, mass);
    assert!(error.reason.contains("has no density"), "{}", error.reason);
    assert_eq!(error.fix, Some(FixTarget::Feature(model.base)));
    assert!((reading_value(&dense, mass) - 0.64).abs() < 1e-9);
    assert!((reading_value(&denser, mass) - 0.96).abs() < 1e-9);
    assert_eq!(reading(&denser, mass).value.dimension, MASS);
    let body = denser.body_result(model.base).unwrap();
    assert!(Arc::ptr_eq(body, without.body_result(model.base).unwrap()));
    assert!(body.solid().unwrap().is_mass_known());
}

#[test]
fn working_out_a_body_mass_stops_at_the_kernel_interrupt_and_is_not_kept() {
    let model = model();
    let evaluation = evaluate(&model.document);
    let body = evaluation.body_result(model.base).unwrap().solid().unwrap();

    let stopped = interruptible(Arc::new(|| true), || body.exact_mass());

    assert!(matches!(stopped, Err(MeasureError::Cancelled(_))));
    assert!(!body.is_mass_known());
    assert!((body.exact_mass().unwrap().properties.volume - 320.0).abs() < 1e-6);
    assert!(body.is_mass_known());
}

#[test]
fn reading_a_body_quantity_of_anything_else_fails_in_words() {
    let mut model = model();
    let (volume, _) = keep(
        &mut model,
        "volume",
        Reading::Of {
            quantity: Of::Volume,
            item: MeasuredItem::Point(PointReference::Origin),
        },
    );
    let (position, _) = keep(
        &mut model,
        "position",
        Reading::Position {
            item: MeasuredItem::Plane(PlaneReference::Principal(PrincipalPlane::Xy)),
            axis: MeasuredItem::Axis(AxisReference::Principal(PrincipalAxis::X)),
        },
    );

    let evaluation = evaluate(&model.document);

    assert!(
        failure(&evaluation, volume)
            .reason
            .contains("is not a body")
    );
    assert!(
        failure(&evaluation, position)
            .reason
            .contains("has no position")
    );
}
