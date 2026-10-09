use caditor_expression::{Expression, ParameterId, Unit};
use caditor_geometry::{Aabb, Plane, Point2, Vector3};
use caditor_kernel::{FaceId, Solid, Surface};
use caditor_sketch::{EntityId, Sketch};

use crate::*;

fn rectangle(plane: Plane, min: (f64, f64), max: (f64, f64)) -> Sketch {
    let mut sketch = Sketch::new(plane);
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

fn millimetres(value: f64) -> Expression {
    Expression::Measure(value, Unit::Millimetre)
}

fn evaluate(document: &Document) -> Evaluation {
    Recompute::default().run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
}

fn add(document: &mut Document, name: &str, kind: FeatureKind) -> FeatureId {
    let mut transaction = document.transaction(format!("Add {name}"));
    let feature = transaction.add_feature(name, kind);
    document.apply(transaction.finish()).unwrap();
    feature
}

fn level(base: PrincipalPlane, offset: f64) -> FeatureKind {
    FeatureKind::Datum(Datum::Plane(DatumPlane {
        base: PlaneReference::Principal(base),
        rotation: None,
        offset: millimetres(offset),
    }))
}

fn extrusion(sketch: FeatureId, start: Option<SolidStart>) -> FeatureKind {
    FeatureKind::Solid(SolidFeature::Extrude(Extrude {
        sketch,
        regions: RegionChoice::All,
        extent: ExtrudeExtent::one_side(millimetres(4.0), false),
        operation: BodyOperation::NewBody,
        start,
        other_bodies: Vec::new(),
        taper: None,
        wall: None,
        direction: None,
    }))
}

fn revolution(sketch: FeatureId, start: Option<SolidStart>) -> FeatureKind {
    FeatureKind::Solid(SolidFeature::Revolve(Revolve {
        sketch,
        regions: RegionChoice::All,
        axis: RevolveAxis::Sketch(EntityId::VERTICAL_AXIS),
        extent: RevolveExtent::Full,
        operation: BodyOperation::NewBody,
        start,
        other_bodies: Vec::new(),
        side: None,
        wall: None,
    }))
}

fn bounds(evaluation: &Evaluation, body: FeatureId) -> Aabb {
    evaluation.body(body).unwrap().bounding_box().unwrap()
}

fn failure(evaluation: &Evaluation, feature: FeatureId) -> FeatureError {
    match &evaluation.feature(feature).unwrap().state {
        FeatureState::Failed(error) => error.clone(),
        other => panic!("expected a failure, found {other:?}"),
    }
}

fn top_face(solid: &Solid, height: f64) -> FaceId {
    solid
        .faces()
        .find(|(_, face)| match face.surface() {
            Surface::Plane(plane) => {
                plane.frame().normal() * face.sense().sign() == Vector3::Z
                    && (plane.frame().origin().z - height).abs() < 1e-9
            }
            _ => false,
        })
        .map(|(id, _)| id)
        .unwrap()
}

#[test]
fn an_extrusion_starts_from_a_datum_plane_wherever_the_plane_lies() {
    let mut document = Document::default();
    let shelf = add(&mut document, "Shelf", level(PrincipalPlane::Xy, 5.0));
    let outline = add(
        &mut document,
        "Outline",
        FeatureKind::from(rectangle(Plane::XY, (0.0, 0.0), (10.0, 8.0))),
    );
    let block = add(
        &mut document,
        "Block",
        extrusion(
            outline,
            Some(SolidStart::Plane(PlaneReference::Datum(shelf))),
        ),
    );

    let evaluation = evaluate(&document);

    assert_eq!(evaluation.failed_count(), 0);
    let found = bounds(&evaluation, block);
    assert!((found.min().z - 5.0).abs() < 1e-9);
    assert!((found.max().z - 9.0).abs() < 1e-9);
    assert!(
        document
            .feature(block)
            .unwrap()
            .kind
            .features()
            .contains(&shelf)
    );
}

#[test]
fn an_extrusion_starting_from_a_face_follows_the_face_when_its_body_changes() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Plate");
    let thickness = transaction.add_parameter("thickness", transaction.parse("10 mm").unwrap());
    let plate_outline = transaction.add_feature(
        "Plate outline",
        FeatureKind::from(rectangle(Plane::XY, (0.0, 0.0), (40.0, 40.0))),
    );
    let plate = transaction.add_feature(
        "Plate",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: plate_outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::Parameter(thickness), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    );
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document);
    let solid = evaluation.body(plate).unwrap();
    let (attachment, _) = FaceAttachment::capture(plate, solid, top_face(solid, 10.0)).unwrap();
    let outline = add(
        &mut document,
        "Boss outline",
        FeatureKind::from(rectangle(Plane::XY, (5.0, 5.0), (15.0, 15.0))),
    );
    let boss = add(
        &mut document,
        "Boss",
        extrusion(
            outline,
            Some(SolidStart::Plane(PlaneReference::Face(attachment))),
        ),
    );

    let first = evaluate(&document);
    let expression = document.parse("15 mm").unwrap();
    document
        .apply(Transaction::single(
            "Edit",
            Edit::SetParameterExpression {
                id: thickness,
                expression,
            },
        ))
        .unwrap();
    let thicker = evaluate(&document);

    assert_eq!(first.failed_count(), 0);
    assert!((bounds(&first, boss).min().z - 10.0).abs() < 1e-9);
    assert_eq!(thicker.failed_count(), 0);
    assert!((bounds(&thicker, boss).min().z - 15.0).abs() < 1e-9);
    assert!((bounds(&thicker, boss).max().z - 19.0).abs() < 1e-9);
}

#[test]
fn a_start_plane_that_is_not_parallel_to_the_sketch_fails_the_feature_alone() {
    let mut document = Document::default();
    let outline = add(
        &mut document,
        "Outline",
        FeatureKind::from(rectangle(Plane::XY, (0.0, 0.0), (10.0, 8.0))),
    );
    let block = add(
        &mut document,
        "Block",
        extrusion(
            outline,
            Some(SolidStart::Plane(PlaneReference::Principal(
                PrincipalPlane::Xz,
            ))),
        ),
    );

    let evaluation = evaluate(&document);

    let error = failure(&evaluation, block);
    assert_eq!(
        error.reason,
        "The XZ plane is not parallel to the plane of Outline, so the extrusion cannot start from \
         it."
    );
    assert_eq!(
        error.remedy,
        "Choose a face or plane parallel to the sketch, or enter a distance."
    );
    assert_eq!(error.fix, Some(FixTarget::Feature(block)));
}

#[test]
fn a_revolution_starts_off_its_sketch_plane_by_a_distance_or_at_a_plane() {
    let mut document = Document::default();
    let section = add(
        &mut document,
        "Section",
        FeatureKind::from(rectangle(Plane::XZ, (2.0, 0.0), (4.0, 3.0))),
    );
    let shelf = add(&mut document, "Shelf", level(PrincipalPlane::Xz, 6.0));
    let plain = add(&mut document, "Plain", revolution(section, None));
    let lifted = add(
        &mut document,
        "Lifted",
        revolution(section, Some(SolidStart::Distance(millimetres(5.0)))),
    );
    let placed = add(
        &mut document,
        "Placed",
        revolution(
            section,
            Some(SolidStart::Plane(PlaneReference::Datum(shelf))),
        ),
    );

    let evaluation = evaluate(&document);

    assert_eq!(evaluation.failed_count(), 0);
    let middle = |body| {
        let found = bounds(&evaluation, body);
        (found.min().y + found.max().y) / 2.0
    };
    let normal_y = Plane::XZ.normal().y;
    assert!(middle(plain).abs() < 1e-9);
    assert!((middle(lifted) - 5.0 * normal_y).abs() < 1e-9);
    assert!((middle(placed).abs() - 6.0).abs() < 1e-9);
    let size = |body| bounds(&evaluation, body).max().x - bounds(&evaluation, body).min().x;
    assert!((size(lifted) - size(plain)).abs() < 1e-9);
    assert!(
        document
            .feature(placed)
            .unwrap()
            .kind
            .features()
            .contains(&shelf)
    );
}

#[test]
fn a_start_plane_is_reported_by_the_parameters_and_references_it_uses() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Lift");
    let lift: ParameterId = transaction.add_parameter("lift", transaction.parse("2 mm").unwrap());
    document.apply(transaction.finish()).unwrap();
    let solid = SolidFeature::Revolve(Revolve {
        sketch: FeatureId::from_raw(1),
        regions: RegionChoice::All,
        axis: RevolveAxis::Sketch(EntityId::VERTICAL_AXIS),
        extent: RevolveExtent::Full,
        operation: BodyOperation::NewBody,
        start: Some(SolidStart::Distance(Expression::Parameter(lift))),
        other_bodies: Vec::new(),
        side: None,
        wall: None,
    });

    assert!(solid.uses_parameter(lift));
    assert!(solid.parameters().contains(&lift));
}
