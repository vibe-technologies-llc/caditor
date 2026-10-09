use caditor_expression::{Expression, ParameterId, Unit};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::{VertexName, vertex_names};
use caditor_sketch::Sketch;

use crate::*;

const CLOSE: f64 = 1e-9;

struct Model {
    document: Document,
    height: ParameterId,
    base: FeatureId,
}

fn evaluate(document: &Document) -> Evaluation {
    Recompute::default().run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
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
    document.apply(transaction.finish()).unwrap();
    Model {
        document,
        height,
        base,
    }
}

fn corner(model: &Model, point: Point3) -> PointReference {
    let evaluation = evaluate(&model.document);
    let solid = evaluation.body(model.base).unwrap();
    let names = vertex_names(solid);
    let (id, _) = solid
        .vertices()
        .find(|(_, vertex)| vertex.point().distance(point) < CLOSE)
        .unwrap();
    let vertex: VertexName = names
        .iter()
        .find(|(vertex, _)| **vertex == id)
        .map(|(_, name)| *name)
        .unwrap();
    PointReference::Vertex {
        body: model.base,
        vertex,
    }
}

fn add(document: &mut Document, name: &str, kind: FeatureKind) -> FeatureId {
    let mut transaction = document.transaction("Add");
    let feature = transaction.add_feature(name, kind);
    document.apply(transaction.finish()).unwrap();
    feature
}

fn coordinate_system(origin: PointReference, x_axis: PrincipalAxis) -> Datum {
    Datum::Frame(Box::new(DatumFrame {
        origin,
        x_axis: AxisReference::Principal(x_axis),
        ..DatumFrame::world()
    }))
}

fn datum(evaluation: &Evaluation, feature: FeatureId) -> DatumResult {
    match evaluation.feature(feature).unwrap() {
        FeatureStatus {
            state: FeatureState::UpToDate,
            result: Some(result),
            ..
        } => *result.datum().unwrap(),
        other => panic!("the datum failed: {other:?}"),
    }
}

fn frame(evaluation: &Evaluation, feature: FeatureId) -> Plane {
    datum(evaluation, feature).frame().unwrap()
}

fn near(found: Vector3, expected: [f64; 3]) -> bool {
    (found - Vector3::from_array(expected)).length() < CLOSE
}

#[test]
fn a_coordinate_system_stands_at_its_point_with_x_along_its_axis_and_follows_the_body() {
    let mut model = model();
    let origin = corner(&model, Point3::new(10.0, 8.0, 4.0));
    let system = add(
        &mut model.document,
        "Coordinate system 1",
        FeatureKind::Datum(coordinate_system(origin, PrincipalAxis::Y)),
    );

    let placed = frame(&evaluate(&model.document), system);
    let taller = model.document.parse("9 mm").unwrap();
    model
        .document
        .apply(Transaction::single(
            "Taller",
            Edit::SetParameterExpression {
                id: model.height,
                expression: taller,
            },
        ))
        .unwrap();
    let followed = frame(&evaluate(&model.document), system);

    assert!(near(placed.origin(), [10.0, 8.0, 4.0]));
    assert!(near(placed.x_axis(), [0.0, 1.0, 0.0]));
    assert!(near(placed.y_axis(), [-1.0, 0.0, 0.0]));
    assert!(near(placed.normal(), [0.0, 0.0, 1.0]));
    assert!(near(followed.origin(), [10.0, 8.0, 9.0]));
    assert_eq!(
        model
            .document
            .feature(system)
            .unwrap()
            .kind
            .datum()
            .map(Datum::title),
        Some("Coordinate system")
    );
}

#[test]
fn reversing_its_axes_keeps_the_system_right_handed() {
    let mut model = model();
    let system = add(
        &mut model.document,
        "Coordinate system 1",
        FeatureKind::Datum(Datum::Frame(Box::new(DatumFrame {
            reverse_x: true,
            reverse_z: true,
            ..DatumFrame::world()
        }))),
    );

    let placed = frame(&evaluate(&model.document), system);

    assert!(near(placed.x_axis(), [-1.0, 0.0, 0.0]));
    assert!(near(placed.normal(), [0.0, 0.0, -1.0]));
    assert!(near(placed.y_axis(), [0.0, 1.0, 0.0]));
}

#[test]
fn an_x_axis_square_to_the_plane_is_refused_in_words() {
    let mut model = model();
    let system = add(
        &mut model.document,
        "Coordinate system 1",
        FeatureKind::Datum(coordinate_system(PointReference::Origin, PrincipalAxis::Z)),
    );

    let evaluation = evaluate(&model.document);

    let FeatureState::Failed(error) = &evaluation.feature(system).unwrap().state else {
        panic!("a degenerate coordinate system must fail");
    };
    assert_eq!(
        error.reason,
        "The Z axis stands square to the XY plane, so it gives no direction in that plane for \
         the X axis."
    );
    assert!(error.remedy.contains("runs along the plane"));
}

#[test]
fn its_axes_and_planes_place_datums_sketches_and_mirrors() {
    let mut model = model();
    let origin = corner(&model, Point3::new(10.0, 8.0, 4.0));
    let system = add(
        &mut model.document,
        "Coordinate system 1",
        FeatureKind::Datum(coordinate_system(origin, PrincipalAxis::Y)),
    );
    let axis = add(
        &mut model.document,
        "Axis 1",
        FeatureKind::Datum(Datum::Axis(DatumAxis::Along(AxisReference::Frame {
            frame: system,
            axis: PrincipalAxis::X,
        }))),
    );
    let plane = PlaneReference::Frame {
        frame: system,
        plane: PrincipalPlane::Yz,
    };
    let sketch = add(
        &mut model.document,
        "Sketch on it",
        FeatureKind::Sketch(SketchFeature {
            sketch: Sketch::new(Plane::XY),
            attachment: Some(SketchAttachment::Frame {
                frame: system,
                plane: PrincipalPlane::Xz,
            }),
            projections: Default::default(),
        }),
    );
    let mirror = add(
        &mut model.document,
        "Mirror 1",
        FeatureKind::Mirror(Mirror {
            body: model.base,
            plane,
            keep_original: false,
            mirrored: Vec::new(),
        }),
    );

    let evaluation = evaluate(&model.document);

    let ray = datum(&evaluation, axis).axis().unwrap();
    assert!(near(ray.origin(), [10.0, 8.0, 4.0]));
    assert!(near(ray.direction(), [0.0, 1.0, 0.0]));
    let lies_on = evaluation
        .feature(sketch)
        .unwrap()
        .result
        .as_deref()
        .and_then(FeatureResult::sketch)
        .unwrap()
        .geometry
        .plane();
    assert!(near(lies_on.origin(), [10.0, 8.0, 4.0]));
    assert!(near(lies_on.normal(), [1.0, 0.0, 0.0]));
    let mirrored = evaluation.body(model.base).unwrap().bounding_box().unwrap();
    assert!(near(mirrored.min(), [0.0, 8.0, 0.0]));
    assert!(near(mirrored.max(), [10.0, 16.0, 4.0]));
    assert_eq!(evaluation.failed_count(), 0);
    let users = model.document.dependents_of(&[system]);
    assert!(
        [axis, sketch, mirror]
            .iter()
            .all(|user| users.contains(user))
    );
}

#[test]
fn a_move_in_a_coordinate_system_shifts_and_turns_along_its_axes() {
    let mut model = model();
    let origin = corner(&model, Point3::new(10.0, 0.0, 0.0));
    let system = add(
        &mut model.document,
        "Coordinate system 1",
        FeatureKind::Datum(coordinate_system(origin, PrincipalAxis::Y)),
    );
    let mut transaction = model.document.transaction("Move");
    let values = |texts: [&str; 3]| texts.map(|text| transaction.parse(text).unwrap());
    let movement = Move {
        body: model.base,
        offset: values(["5 mm", "0 mm", "0 mm"]),
        turn: values(["0 deg", "0 deg", "90 deg"]),
        copy: false,
        about: TurnCentre::Origin,
        frame: Some(system),
    };
    transaction.add_feature("Move 1", FeatureKind::Move(movement));
    model.document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&model.document);

    let moved = evaluation.body(model.base).unwrap().bounding_box().unwrap();
    assert_eq!(evaluation.failed_count(), 0);
    assert!(near(moved.min(), [2.0, -5.0, 0.0]));
    assert!(near(moved.max(), [10.0, 5.0, 4.0]));
}

#[test]
fn a_reference_to_a_coordinate_system_must_name_one() {
    let mut model = model();
    let point = add(
        &mut model.document,
        "Point 1",
        FeatureKind::Datum(Datum::Point(DatumPoint {
            base: PointReference::Origin,
            offset: std::array::from_fn(|_| Expression::Measure(0.0, Unit::Millimetre)),
        })),
    );
    let mut transaction = model.document.transaction("Add");
    transaction.add_feature(
        "Axis 1",
        FeatureKind::Datum(Datum::Axis(DatumAxis::Along(AxisReference::Frame {
            frame: point,
            axis: PrincipalAxis::X,
        }))),
    );

    let refused = model.document.apply(transaction.finish());

    assert_eq!(
        refused.err(),
        Some(EditError::NotACoordinateSystem("Point 1".to_owned()))
    );
}
