use std::collections::BTreeSet;

use caditor_expression::{Expression, ParameterId};
use caditor_geometry::{Point3, Vector3};
use caditor_kernel::{FaceName, FaceReference, Solid, Surface};

use crate::{
    combine_tests::{block, evaluate, rectangle},
    *,
};

struct Model {
    document: Document,
    height: ParameterId,
    plate: FeatureId,
    peg: FeatureId,
}

fn model() -> Model {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let height = transaction.add_parameter("height", transaction.parse("4 mm").unwrap());
    let outline = transaction.add_feature(
        "Plate outline",
        FeatureKind::from(rectangle((-10.0, -5.0), (10.0, 5.0))),
    );
    let plate = transaction.add_feature(
        "Plate",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::Parameter(height), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
        })),
    );
    let peg = block(&mut transaction, "Peg", (20.0, 0.0), (30.0, 5.0), "6 mm");
    document.apply(transaction.finish()).unwrap();
    Model {
        document,
        height,
        plate,
        peg,
    }
}

fn face(solid: &Solid, normal: Vector3, at: Point3) -> FaceReference {
    let (id, _) = solid
        .faces()
        .find(|(_, face)| match face.surface() {
            Surface::Plane(plane) => {
                let outward = plane.frame().normal() * face.sense().sign();
                (outward - normal).length() < 1e-9
                    && (plane.frame().origin() - at).dot(normal).abs() < 1e-9
            }
            _ => false,
        })
        .unwrap();
    FaceReference::capture(solid, id).unwrap()
}

fn plane_of(document: &Document, body: FeatureId, normal: Vector3, at: Point3) -> PlaneReference {
    let evaluation = evaluate(document, &mut Recompute::default());
    PlaneReference::Face(FaceAttachment {
        body,
        face: face(evaluation.body(body).unwrap(), normal, at),
    })
}

fn mate_faces(
    model: &mut Model,
    moving: (Vector3, Point3),
    distance: &str,
    flipped: bool,
) -> FeatureId {
    let evaluation = evaluate(&model.document, &mut Recompute::default());
    let peg = evaluation.body(model.peg).unwrap();
    let pair = MatePair::Faces(Box::new(FaceMate {
        face: face(peg, moving.0, moving.1),
        target: plate_top(model),
        distance: Expression::parse_stored(distance).unwrap(),
    }));
    add_mate(model, pair, flipped)
}

fn add_mate(model: &mut Model, pair: MatePair, flipped: bool) -> FeatureId {
    let mut transaction = model.document.transaction("Mate");
    let mate = transaction.add_feature(
        "Mate 1",
        FeatureKind::Mate(Mate {
            body: model.peg,
            pair,
            flipped,
        }),
    );
    model.document.apply(transaction.finish()).unwrap();
    mate
}

fn plate_top(model: &Model) -> PlaneReference {
    let evaluation = evaluate(&model.document, &mut Recompute::default());
    let top = evaluation
        .body(model.plate)
        .unwrap()
        .bounding_box()
        .unwrap()
        .max()
        .z;
    plane_of(
        &model.document,
        model.plate,
        Vector3::Z,
        Point3::new(0.0, 0.0, top),
    )
}

fn bounds(evaluation: &Evaluation, body: FeatureId) -> (Point3, Point3) {
    let bounds = evaluation.body(body).unwrap().bounding_box().unwrap();
    (bounds.min(), bounds.max())
}

fn near(found: Point3, expected: Point3) -> bool {
    (found - expected).length() < 1e-6
}

fn names(solid: &Solid) -> BTreeSet<FaceName> {
    solid.faces().map(|(_, face)| face.name()).collect()
}

#[test]
fn a_face_mated_flush_onto_another_sits_on_it_and_follows_it_when_it_moves() {
    let mut model = model();
    let before = names(
        evaluate(&model.document, &mut Recompute::default())
            .body(model.peg)
            .unwrap(),
    );
    let mut transaction = model.document.transaction("Mate");
    let evaluation = evaluate(&model.document, &mut Recompute::default());
    let bottom = face(
        evaluation.body(model.peg).unwrap(),
        -Vector3::Z,
        Point3::ZERO,
    );
    let mate = transaction.add_feature(
        "Mate 1",
        FeatureKind::Mate(Mate {
            body: model.peg,
            pair: MatePair::Faces(Box::new(FaceMate {
                face: bottom,
                target: plate_top(&model),
                distance: Expression::parse_stored("0 mm").unwrap(),
            })),
            flipped: false,
        }),
    );
    model.document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();

    let evaluation = evaluate(&model.document, &mut engine);

    assert_eq!(evaluation.failed_count(), 0);
    let (low, high) = bounds(&evaluation, model.peg);
    assert!(near(low, Point3::new(20.0, 0.0, 4.0)), "{low:?}");
    assert!(near(high, Point3::new(30.0, 5.0, 10.0)), "{high:?}");
    assert_eq!(names(evaluation.body(model.peg).unwrap()), before);
    assert_eq!(
        model.document.feature(mate).unwrap().bodies(),
        vec![model.peg]
    );

    let expression = model.document.parse("9 mm").unwrap();
    model
        .document
        .apply(Transaction::single(
            "Thicker",
            Edit::SetParameterExpression {
                id: model.height,
                expression,
            },
        ))
        .unwrap();
    let evaluation = evaluate(&model.document, &mut engine);

    assert_eq!(evaluation.failed_count(), 0);
    let (low, _) = bounds(&evaluation, model.peg);
    assert!(near(low, Point3::new(20.0, 0.0, 9.0)), "{low:?}");
}

#[test]
fn a_face_mated_at_a_distance_stands_off_along_the_target_normal() {
    let mut model = model();
    let target = plate_top(&model);
    let evaluation = evaluate(&model.document, &mut Recompute::default());
    let pair = MatePair::Faces(Box::new(FaceMate {
        face: face(
            evaluation.body(model.peg).unwrap(),
            -Vector3::Z,
            Point3::ZERO,
        ),
        target,
        distance: Expression::parse_stored("2 mm").unwrap(),
    }));
    add_mate(&mut model, pair, false);

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    let (low, high) = bounds(&evaluation, model.peg);
    assert!(near(low, Point3::new(20.0, 0.0, 6.0)), "{low:?}");
    assert!(near(high, Point3::new(30.0, 5.0, 12.0)), "{high:?}");
}

#[test]
fn a_flipped_face_mate_lines_the_faces_up_facing_the_same_way() {
    let mut model = model();
    let target = plate_top(&model);
    let evaluation = evaluate(&model.document, &mut Recompute::default());
    let pair = MatePair::Faces(Box::new(FaceMate {
        face: face(
            evaluation.body(model.peg).unwrap(),
            Vector3::Z,
            Point3::new(0.0, 0.0, 6.0),
        ),
        target,
        distance: Expression::parse_stored("0 mm").unwrap(),
    }));
    add_mate(&mut model, pair, true);

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    let (low, high) = bounds(&evaluation, model.peg);
    assert!(near(low, Point3::new(20.0, 0.0, -2.0)), "{low:?}");
    assert!(near(high, Point3::new(30.0, 5.0, 4.0)), "{high:?}");
}

#[test]
fn a_side_face_mated_onto_a_top_face_turns_the_body_to_lie_on_it() {
    let mut model = model();
    let mate = mate_faces(
        &mut model,
        (Vector3::X, Point3::new(30.0, 0.0, 0.0)),
        "0 mm",
        false,
    );

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    assert_eq!(evaluation.failed_count(), 0, "{mate:?}");
    let (low, high) = bounds(&evaluation, model.peg);
    assert!((low.z - 4.0).abs() < 1e-6, "{low:?}");
    assert!((high.z - 14.0).abs() < 1e-6, "{high:?}");
    assert!((high.x - low.x - 6.0).abs() < 1e-6);
}

#[test]
fn an_axis_mated_onto_another_puts_the_body_on_that_line() {
    let mut model = model();
    let evaluation = evaluate(&model.document, &mut Recompute::default());
    let peg = evaluation.body(model.peg).unwrap();
    let (edge, _) = peg
        .edges()
        .find(|(id, _)| {
            crate::datum::edge_ray(peg, *id).is_some_and(|ray| {
                ray.direction().cross(Vector3::Z).length() < 1e-9
                    && (ray.origin().x - 20.0).abs() < 1e-9
                    && ray.origin().y.abs() < 1e-9
            })
        })
        .unwrap();
    let axis = AxisReference::capture_edge(model.peg, peg, edge).unwrap();
    let pair = MatePair::Axes(Box::new(AxisMate {
        axis,
        target: AxisReference::Principal(PrincipalAxis::Z),
    }));
    add_mate(&mut model, pair, false);

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    assert_eq!(evaluation.failed_count(), 0);
    let (low, high) = bounds(&evaluation, model.peg);
    assert!(near(low, Point3::new(0.0, 0.0, 0.0)), "{low:?}");
    assert!(near(high, Point3::new(10.0, 5.0, 6.0)), "{high:?}");
}

#[test]
fn a_face_no_longer_on_the_moving_body_fails_the_mate_alone_in_words() {
    let mut model = model();
    let evaluation = evaluate(&model.document, &mut Recompute::default());
    let pair = MatePair::Faces(Box::new(FaceMate {
        face: face(
            evaluation.body(model.plate).unwrap(),
            -Vector3::Z,
            Point3::ZERO,
        ),
        target: plate_top(&model),
        distance: Expression::parse_stored("0 mm").unwrap(),
    }));
    let mate = add_mate(&mut model, pair, false);

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    let (failed, error) = evaluation.failures().next().unwrap();
    assert_eq!(failed, mate);
    assert!(
        error
            .reason
            .contains("which it mates, is no longer part of Peg"),
        "{}",
        error.reason
    );
    let (low, _) = bounds(&evaluation, model.peg);
    assert!(near(low, Point3::new(20.0, 0.0, 0.0)));
}
