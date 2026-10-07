use caditor_expression::{Expression, ParameterId, Unit};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::{Curve, EdgeReference, Solid, VertexName, vertex_names};
use caditor_sketch::{EntityId, Sketch};

use crate::*;

const CLOSE: f64 = 1e-9;

fn millimetres(value: f64) -> Expression {
    Expression::Measure(value, Unit::Millimetre)
}

fn evaluate(document: &Document) -> Evaluation {
    Recompute::default().run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
}

fn result(evaluation: &Evaluation, feature: FeatureId) -> DatumResult {
    match evaluation.feature(feature).unwrap() {
        FeatureStatus {
            state: FeatureState::UpToDate,
            result: Some(result),
            ..
        } => *result.datum().unwrap(),
        other => panic!("the datum failed: {other:?}"),
    }
}

fn failure(evaluation: &Evaluation, feature: FeatureId) -> FeatureError {
    match &evaluation.feature(feature).unwrap().state {
        FeatureState::Failed(error) => error.clone(),
        other => panic!("expected a failure, found {other:?}"),
    }
}

fn add(document: &mut Document, name: &str, datum: Datum) -> FeatureId {
    let mut transaction = document.transaction("Add");
    let feature = transaction.add_feature(name, FeatureKind::Datum(datum));
    document.apply(transaction.finish()).unwrap();
    feature
}

fn at(base: PointReference) -> Datum {
    Datum::Point(DatumPoint {
        base,
        offset: [millimetres(0.0), millimetres(0.0), millimetres(0.0)],
    })
}

struct Model {
    document: Document,
    height: ParameterId,
    base: FeatureId,
    sketch: FeatureId,
    points: [EntityId; 2],
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
        })),
    );
    let mut marks = Sketch::new(Plane::XZ);
    let first = marks.add_point(Point2::new(2.0, 3.0));
    let second = marks.add_point(Point2::new(7.0, 3.0));
    let sketch = transaction.add_feature("Marks", FeatureKind::from(marks));
    document.apply(transaction.finish()).unwrap();
    Model {
        document,
        height,
        base,
        sketch,
        points: [first, second],
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

fn corner_reference(model: &Model, point: Point3) -> PointReference {
    let evaluation = evaluate(&model.document);
    PointReference::Vertex {
        body: model.base,
        vertex: corner(evaluation.body(model.base).unwrap(), point),
    }
}

#[test]
fn a_datum_point_sits_at_a_corner_with_its_offsets_and_follows_the_body() {
    let mut model = model();
    let base = corner_reference(&model, Point3::new(10.0, 8.0, 4.0));
    let point = add(
        &mut model.document,
        "Point 1",
        Datum::Point(DatumPoint {
            base,
            offset: [millimetres(1.0), millimetres(0.0), millimetres(-2.0)],
        }),
    );

    let before = result(&evaluate(&model.document), point);
    let height = model.document.parse("9 mm").unwrap();
    model
        .document
        .apply(Transaction::single(
            "Taller",
            Edit::SetParameterExpression {
                id: model.height,
                expression: height,
            },
        ))
        .unwrap();
    let after = result(&evaluate(&model.document), point);

    assert_eq!(before, DatumResult::Point(Point3::new(11.0, 8.0, 2.0)));
    assert_eq!(after, DatumResult::Point(Point3::new(11.0, 8.0, 7.0)));
    assert!(
        model
            .document
            .feature(point)
            .unwrap()
            .kind
            .bodies_used()
            .contains(&model.base)
    );
}

#[test]
fn a_plane_passes_through_three_corners_and_refuses_three_in_a_line() {
    let mut model = model();
    let corners = [
        Point3::new(0.0, 0.0, 4.0),
        Point3::new(10.0, 0.0, 4.0),
        Point3::new(10.0, 8.0, 4.0),
    ]
    .map(|point| corner_reference(&model, point));
    let in_a_line = [
        PointReference::Origin,
        corner_reference(&model, Point3::new(10.0, 0.0, 0.0)),
        PointReference::Origin,
    ];
    let through = add(
        &mut model.document,
        "Plane 1",
        Datum::PlaneThrough(PlaneThrough::Points(corners)),
    );
    let refused = add(
        &mut model.document,
        "Plane 2",
        Datum::PlaneThrough(PlaneThrough::Points(in_a_line)),
    );

    let evaluation = evaluate(&model.document);
    let plane = result(&evaluation, through).plane().unwrap();
    let error = failure(&evaluation, refused);

    assert!((plane.normal().cross(Vector3::Z)).length() < CLOSE);
    assert!(plane.signed_distance(Point3::new(3.0, 3.0, 4.0)).abs() < CLOSE);
    assert!(error.reason.contains("lie on one line"), "{}", error.reason);
}

#[test]
fn a_midway_plane_halves_parallel_planes_and_bisects_crossing_ones() {
    let mut model = model();
    let evaluation = evaluate(&model.document);
    let solid = evaluation.body(model.base).unwrap();
    let (top, _) = solid
        .faces()
        .find(|(_, face)| match face.surface() {
            caditor_kernel::Surface::Plane(plane) => plane.frame().origin().z > 3.0,
            _ => false,
        })
        .unwrap();
    let top = FaceAttachment {
        body: model.base,
        face: caditor_kernel::FaceReference::capture(solid, top).unwrap(),
    };
    let halfway = add(
        &mut model.document,
        "Plane 1",
        Datum::PlaneThrough(PlaneThrough::Midway(
            PlaneReference::Principal(PrincipalPlane::Xy),
            PlaneReference::Face(top),
        )),
    );
    let bisector = add(
        &mut model.document,
        "Plane 2",
        Datum::PlaneThrough(PlaneThrough::Midway(
            PlaneReference::Principal(PrincipalPlane::Xz),
            PlaneReference::Principal(PrincipalPlane::Yz),
        )),
    );

    let evaluation = evaluate(&model.document);
    let middle = result(&evaluation, halfway).plane().unwrap();
    let between = result(&evaluation, bisector).plane().unwrap();

    assert!(middle.signed_distance(Point3::new(5.0, 5.0, 2.0)).abs() < CLOSE);
    assert!(middle.normal().cross(Vector3::Z).length() < CLOSE);
    assert!(between.normal().dot(Vector3::Z).abs() < CLOSE);
    assert!((between.normal().x.abs() - between.normal().y.abs()).abs() < CLOSE);
    assert!(between.signed_distance(Point3::new(0.0, 0.0, 9.0)).abs() < CLOSE);
}

#[test]
fn planes_and_axes_are_placed_from_axes_planes_and_sketch_points() {
    let mut model = model();
    let [first, second] = model.points.map(|entity| PointReference::Sketch {
        sketch: model.sketch,
        entity,
    });
    let along_marks = add(
        &mut model.document,
        "Axis 1",
        Datum::Axis(DatumAxis::Points(first.clone(), second.clone())),
    );
    let upright = add(
        &mut model.document,
        "Axis 2",
        Datum::Axis(DatumAxis::NormalTo(
            PlaneReference::Principal(PrincipalPlane::Xy),
            second.clone(),
        )),
    );
    let containing = add(
        &mut model.document,
        "Plane 1",
        Datum::PlaneThrough(PlaneThrough::AxisAndPoint(
            AxisReference::Principal(PrincipalAxis::Z),
            first.clone(),
        )),
    );
    let square = add(
        &mut model.document,
        "Plane 2",
        Datum::PlaneThrough(PlaneThrough::NormalTo(
            AxisReference::Principal(PrincipalAxis::X),
            second,
        )),
    );
    let on_axis = add(
        &mut model.document,
        "Plane 3",
        Datum::PlaneThrough(PlaneThrough::AxisAndPoint(
            AxisReference::Principal(PrincipalAxis::Z),
            PointReference::Origin,
        )),
    );

    let evaluation = evaluate(&model.document);
    let marks = Plane::XZ;
    let first_at = marks.to_world(Point2::new(2.0, 3.0));
    let second_at = marks.to_world(Point2::new(7.0, 3.0));
    let line = result(&evaluation, along_marks).axis().unwrap();
    let up = result(&evaluation, upright).axis().unwrap();
    let through = result(&evaluation, containing).plane().unwrap();
    let crossing = result(&evaluation, square).plane().unwrap();
    let error = failure(&evaluation, on_axis);

    assert!(line.origin().distance(first_at) < CLOSE);
    assert!(line.direction().cross(second_at - first_at).length() < CLOSE);
    assert!(up.origin().distance(second_at) < CLOSE);
    assert!(up.direction().cross(Vector3::Z).length() < CLOSE);
    assert!(through.signed_distance(first_at).abs() < CLOSE);
    assert!(through.signed_distance(Point3::new(0.0, 0.0, 50.0)).abs() < CLOSE);
    assert!(crossing.normal().cross(Vector3::X).length() < CLOSE);
    assert!(crossing.signed_distance(second_at).abs() < CLOSE);
    assert!(
        error.reason.contains("lies on the Z axis"),
        "{}",
        error.reason
    );
}

#[test]
fn a_datum_point_takes_the_centre_of_a_round_edge() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let mut disc = Sketch::new(Plane::XY);
    disc.add_circle(Point2::new(3.0, -2.0), 5.0);
    let disc = transaction.add_feature("Disc", FeatureKind::from(disc));
    let pin = transaction.add_feature(
        "Pin",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: disc,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(millimetres(6.0), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
        })),
    );
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document);
    let solid = evaluation.body(pin).unwrap();
    let (rim, _) = solid
        .edges()
        .find(|(_, edge)| matches!(edge.curve(), Curve::Circle(circle) if circle.center().z > 3.0))
        .unwrap();
    let centre = PointReference::Centre {
        body: pin,
        edge: Box::new(EdgeReference::capture(solid, rim).unwrap()),
    };
    let point = add(&mut document, "Point 1", at(centre));

    let found = result(&evaluate(&document), point);

    assert_eq!(found, DatumResult::Point(Point3::new(3.0, -2.0, 6.0)));
}

#[test]
fn a_point_reference_to_another_kind_of_datum_is_refused() {
    let mut model = model();
    let axis = add(
        &mut model.document,
        "Axis 1",
        Datum::Axis(DatumAxis::Along(AxisReference::Principal(PrincipalAxis::X))),
    );
    let mut transaction = model.document.transaction("Add");
    transaction.add_feature(
        "Point 1",
        FeatureKind::Datum(at(PointReference::Datum(axis))),
    );

    let refused = model.document.apply(transaction.finish());

    assert_eq!(refused, Err(EditError::NotAPoint("Axis 1".to_owned())));
}

#[test]
fn a_point_is_its_own_kind_of_datum() {
    let point = at(PointReference::Origin);
    let axis = Datum::Axis(DatumAxis::Points(
        PointReference::Origin,
        PointReference::Origin,
    ));
    let plane = Datum::PlaneThrough(PlaneThrough::Midway(
        PlaneReference::Principal(PrincipalPlane::Xy),
        PlaneReference::Principal(PrincipalPlane::Xz),
    ));

    assert_eq!(point.title(), "Point");
    assert!(point.is_point() && !point.is_plane() && !point.is_axis());
    assert!(axis.is_axis());
    assert!(plane.is_plane());
    assert!(plane.same_kind(&Datum::Plane(DatumPlane {
        base: PlaneReference::Principal(PrincipalPlane::Xy),
        rotation: None,
        offset: millimetres(1.0),
    })));
    assert!(!point.same_kind(&axis));
}

#[test]
fn an_axis_runs_along_a_line_of_a_sketch_and_follows_it() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let mut guide = Sketch::new(Plane::XZ);
    let line = guide.add_line(Point2::new(1.0, 2.0), Point2::new(4.0, 6.0));
    let guide = transaction.add_feature("Guide", FeatureKind::from(guide));
    document.apply(transaction.finish()).unwrap();
    let axis = add(
        &mut document,
        "Axis 1",
        Datum::Axis(DatumAxis::Along(AxisReference::Sketch {
            sketch: guide,
            entity: line,
        })),
    );
    let refused = add(
        &mut document,
        "Axis 2",
        Datum::Axis(DatumAxis::Along(AxisReference::Sketch {
            sketch: guide,
            entity: EntityId::from_raw(0),
        })),
    );

    let evaluation = evaluate(&document);
    let ray = result(&evaluation, axis).axis().unwrap();
    let error = failure(&evaluation, refused);

    let start = Plane::XZ.to_world(Point2::new(1.0, 2.0));
    let end = Plane::XZ.to_world(Point2::new(4.0, 6.0));
    assert!(ray.origin().distance(start) < CLOSE);
    assert!(ray.direction().cross(end - start).length() < CLOSE);
    assert!(
        error.reason.contains("is no longer a line"),
        "{}",
        error.reason
    );
    assert!(
        document
            .feature(axis)
            .unwrap()
            .kind
            .features()
            .contains(&guide)
    );
    assert_eq!(
        describe_axis(
            &document,
            &AxisReference::Sketch {
                sketch: guide,
                entity: line
            }
        ),
        "Line 2 of Guide"
    );
}
