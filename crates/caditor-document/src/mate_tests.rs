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
            taper: None,
            wall: None,
            direction: None,
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

fn peg_edge_on(model: &Model, x: f64, y: f64) -> AxisReference {
    let evaluation = evaluate(&model.document, &mut Recompute::default());
    let peg = evaluation.body(model.peg).unwrap();
    let (edge, _) = peg
        .edges()
        .find(|(id, _)| {
            crate::datum::edge_ray(peg, *id).is_some_and(|ray| {
                ray.direction().cross(Vector3::Z).length() < 1e-9
                    && (ray.origin().x - x).abs() < 1e-9
                    && (ray.origin().y - y).abs() < 1e-9
            })
        })
        .unwrap();
    AxisReference::capture_edge(model.peg, peg, edge).unwrap()
}

fn peg_face(model: &Model, normal: Vector3, at: Point3) -> FaceReference {
    let evaluation = evaluate(&model.document, &mut Recompute::default());
    face(evaluation.body(model.peg).unwrap(), normal, at)
}

fn flush_and_concentric(model: &Model, target: AxisReference) -> MatePair {
    MatePair::FaceAxis(Box::new(FaceAxisMate {
        faces: FaceMate {
            face: peg_face(model, -Vector3::Z, Point3::ZERO),
            target: plate_top(model),
            distance: Expression::parse_stored("0 mm").unwrap(),
        },
        axes: AxisMate {
            axis: peg_edge_on(model, 20.0, 0.0),
            target,
        },
    }))
}

fn first_failure(evaluation: &Evaluation) -> String {
    evaluation.failures().next().unwrap().1.reason.clone()
}

fn replace_pair(model: &mut Model, mate: FeatureId, pair: MatePair) {
    let mut changed = model
        .document
        .feature(mate)
        .unwrap()
        .kind
        .mate()
        .unwrap()
        .clone();
    changed.pair = pair;
    model
        .document
        .apply(Transaction::single(
            "Change",
            Edit::SetFeatureKind {
                id: mate,
                kind: FeatureKind::Mate(changed),
            },
        ))
        .unwrap();
}

#[test]
fn a_face_and_axis_mate_makes_the_body_flush_and_concentric_in_one_feature() {
    let mut model = model();
    let pair = flush_and_concentric(&model, AxisReference::Principal(PrincipalAxis::Z));
    add_mate(&mut model, pair, false);

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    assert_eq!(evaluation.failed_count(), 0);
    let (low, high) = bounds(&evaluation, model.peg);
    assert!(near(low, Point3::new(0.0, 0.0, 4.0)), "{low:?}");
    assert!(near(high, Point3::new(10.0, 5.0, 10.0)), "{high:?}");
}

#[test]
fn a_face_and_axis_mate_whose_axis_runs_along_the_plane_fails_in_words() {
    let mut model = model();
    let pair = flush_and_concentric(&model, AxisReference::Principal(PrincipalAxis::X));
    add_mate(&mut model, pair, false);

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    let reason = first_failure(&evaluation);
    assert!(
        reason.contains("runs along the plane it mates onto"),
        "{reason}"
    );
}

fn side_at_angle(model: &Model, angle: &str) -> MatePair {
    MatePair::Angle(Box::new(AngleMate {
        sides: AngleSides::Faces(FacePair {
            face: peg_face(model, Vector3::X, Point3::new(30.0, 0.0, 0.0)),
            target: plate_top(model),
        }),
        angle: Expression::parse_stored(angle).unwrap(),
    }))
}

#[test]
fn an_angle_mate_turns_the_body_about_the_line_where_the_planes_meet() {
    let mut model = model();
    let square = side_at_angle(&model, "90 deg");
    let mate = add_mate(&mut model, square, false);

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    let (low, high) = bounds(&evaluation, model.peg);
    assert!(near(low, Point3::new(20.0, 0.0, 0.0)), "{low:?}");
    assert!(near(high, Point3::new(30.0, 5.0, 6.0)), "{high:?}");

    let facing = side_at_angle(&model, "0 deg");
    replace_pair(&mut model, mate, facing);
    let evaluation = evaluate(&model.document, &mut Recompute::default());

    assert_eq!(evaluation.failed_count(), 0);
    let (low, high) = bounds(&evaluation, model.peg);
    assert!(near(low, Point3::new(26.0, 0.0, 4.0)), "{low:?}");
    assert!(near(high, Point3::new(32.0, 5.0, 14.0)), "{high:?}");
}

#[test]
fn an_angle_past_a_half_turn_fails_the_mate_in_words() {
    let mut model = model();
    let pair = side_at_angle(&model, "200 deg");
    add_mate(&mut model, pair, false);

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    let reason = first_failure(&evaluation);
    assert!(reason.contains("outside 0 to 180 deg"), "{reason}");
}

fn round_body(model: &mut Model, shape: PrimitiveShape, x: f64) -> (FeatureId, FaceReference) {
    let mut transaction = model.document.transaction("Round");
    let body = transaction.add_feature(
        "Round",
        FeatureKind::Primitive(Primitive {
            shape,
            plane: PlaneReference::Principal(PrincipalPlane::Xy),
            at: [
                Expression::parse_stored(&format!("{x} mm")).unwrap(),
                Expression::parse_stored("0 mm").unwrap(),
            ],
            anchor: PrimitiveAnchor::Centre,
            reversed: false,
            operation: BodyOperation::NewBody,
        }),
    );
    model.document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&model.document, &mut Recompute::default());
    let solid = evaluation.body(body).unwrap();
    let (id, _) = solid
        .faces()
        .find(|(_, face)| matches!(face.surface(), Surface::Sphere(_) | Surface::Cylinder(_)))
        .unwrap();
    (body, FaceReference::capture(solid, id).unwrap())
}

fn rest_on_plate(model: &mut Model, body: FeatureId, face: FaceReference, flipped: bool) {
    let target = plate_top(model);
    let mut transaction = model.document.transaction("Mate");
    transaction.add_feature(
        "Mate 1",
        FeatureKind::Mate(Mate {
            body,
            pair: MatePair::Tangent(Box::new(FacePair { face, target })),
            flipped,
        }),
    );
    model.document.apply(transaction.finish()).unwrap();
}

fn ball() -> PrimitiveShape {
    PrimitiveShape::Sphere {
        diameter: Expression::parse_stored("10 mm").unwrap(),
    }
}

#[test]
fn a_tangent_mate_rests_a_sphere_on_a_plane_on_either_side() {
    let mut model = model();
    let (body, face) = round_body(&mut model, ball(), 50.0);
    rest_on_plate(&mut model, body, face, false);

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    assert_eq!(evaluation.failed_count(), 0);
    let (low, high) = bounds(&evaluation, body);
    assert!((low.z - 4.0).abs() < 1e-6, "{low:?}");
    assert!((high.z - 14.0).abs() < 1e-6, "{high:?}");
    assert!((low.x - 45.0).abs() < 1e-6, "{low:?}");

    let mut below = self::model();
    let (body, face) = round_body(&mut below, ball(), 50.0);
    rest_on_plate(&mut below, body, face, true);

    let evaluation = evaluate(&below.document, &mut Recompute::default());

    let (_, high) = bounds(&evaluation, body);
    assert!((high.z - 4.0).abs() < 1e-6, "{high:?}");
}

#[test]
fn a_tangent_mate_lays_a_cylinder_down_along_the_plane() {
    let mut model = model();
    let rod = PrimitiveShape::Cylinder {
        diameter: Expression::parse_stored("4 mm").unwrap(),
        height: Expression::parse_stored("10 mm").unwrap(),
    };
    let (body, face) = round_body(&mut model, rod, 50.0);
    rest_on_plate(&mut model, body, face, false);

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    assert_eq!(evaluation.failed_count(), 0);
    let (low, high) = bounds(&evaluation, body);
    assert!((low.z - 4.0).abs() < 1e-6, "{low:?}");
    assert!((high.z - 8.0).abs() < 1e-6, "{high:?}");
}

#[test]
fn a_tangent_mate_on_a_flat_face_fails_naming_it() {
    let mut model = model();
    let face = peg_face(&model, -Vector3::Z, Point3::ZERO);
    let peg = model.peg;
    rest_on_plate(&mut model, peg, face, false);

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    let reason = first_failure(&evaluation);
    assert!(
        reason.contains("no longer a whole cylinder or sphere"),
        "{reason}"
    );
}

fn peg_corner(model: &Model, point: Point3) -> PointReference {
    let evaluation = evaluate(&model.document, &mut Recompute::default());
    let peg = evaluation.body(model.peg).unwrap();
    let names = caditor_kernel::vertex_names(peg);
    let (id, _) = peg
        .vertices()
        .find(|(_, vertex)| vertex.point().distance(point) < 1e-9)
        .unwrap();
    let vertex = names
        .iter()
        .find(|(vertex, _)| **vertex == id)
        .map(|(_, name)| *name)
        .unwrap();
    PointReference::Vertex {
        body: model.peg,
        vertex,
    }
}

#[test]
fn a_point_mate_puts_a_corner_on_a_point_or_onto_a_plane() {
    let mut model = model();
    let corner = peg_corner(&model, Point3::new(20.0, 0.0, 0.0));
    let pair = MatePair::Point(Box::new(PointMate {
        point: corner.clone(),
        target: PointTarget::Point(PointReference::Origin),
    }));
    let mate = add_mate(&mut model, pair, false);

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    assert_eq!(evaluation.failed_count(), 0);
    let (low, high) = bounds(&evaluation, model.peg);
    assert!(near(low, Point3::ZERO), "{low:?}");
    assert!(near(high, Point3::new(10.0, 5.0, 6.0)), "{high:?}");

    let onto_plane = MatePair::Point(Box::new(PointMate {
        point: corner,
        target: PointTarget::Plane(plate_top(&model)),
    }));
    replace_pair(&mut model, mate, onto_plane);
    let evaluation = evaluate(&model.document, &mut Recompute::default());

    let (low, _) = bounds(&evaluation, model.peg);
    assert!(near(low, Point3::new(20.0, 0.0, 4.0)), "{low:?}");
    assert_eq!(
        model.document.feature(mate).unwrap().kind.features(),
        BTreeSet::from([model.peg, model.plate])
    );
}

fn rest_peg_on(model: &mut Model, face: FaceReference, round: FaceAttachment, flipped: bool) {
    let pair = MatePair::FaceOnRound(Box::new(FaceOnRound { face, round }));
    add_mate(model, pair, flipped);
}

#[test]
fn a_flat_face_rests_on_a_sphere_of_another_body_on_either_side() {
    let mut model = model();
    let (ball_body, ball_face) = round_body(&mut model, ball(), 50.0);
    let bottom = peg_face(&model, -Vector3::Z, Point3::ZERO);
    let round = FaceAttachment {
        body: ball_body,
        face: ball_face,
    };
    rest_peg_on(&mut model, bottom, round, false);

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    assert_eq!(
        evaluation.failed_count(),
        0,
        "{:?}",
        evaluation.failures().next()
    );
    let (low, high) = bounds(&evaluation, model.peg);
    assert!((low.z - 5.0).abs() < 1e-6, "{low:?}");
    assert!((high.z - 11.0).abs() < 1e-6, "{high:?}");
    assert!((low.x - 20.0).abs() < 1e-6, "{low:?}");
    let mate = model.document.features().last().unwrap();
    assert!(mate.kind.bodies_used().contains(&ball_body));

    let mut below = self::model();
    let (ball_body, ball_face) = round_body(&mut below, ball(), 50.0);
    let bottom = peg_face(&below, -Vector3::Z, Point3::ZERO);
    let round = FaceAttachment {
        body: ball_body,
        face: ball_face,
    };
    rest_peg_on(&mut below, bottom, round, true);

    let evaluation = evaluate(&below.document, &mut Recompute::default());

    let (low, _) = bounds(&evaluation, below.peg);
    assert!((low.z + 5.0).abs() < 1e-6, "{low:?}");
}

#[test]
fn a_flat_face_rests_along_a_cylinder_of_another_body() {
    let mut model = model();
    let rod = PrimitiveShape::Cylinder {
        diameter: Expression::parse_stored("4 mm").unwrap(),
        height: Expression::parse_stored("10 mm").unwrap(),
    };
    let (rod_body, rod_face) = round_body(&mut model, rod, 50.0);
    let side = peg_face(&model, -Vector3::Y, Point3::ZERO);
    let round = FaceAttachment {
        body: rod_body,
        face: rod_face,
    };
    rest_peg_on(&mut model, side, round, false);

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    assert_eq!(
        evaluation.failed_count(),
        0,
        "{:?}",
        evaluation.failures().next()
    );
    let (low, high) = bounds(&evaluation, model.peg);
    assert!((low.y - 2.0).abs() < 1e-6, "{low:?}");
    assert!((high.y - 7.0).abs() < 1e-6, "{high:?}");
    assert!((low.z).abs() < 1e-6, "{low:?}");
}

#[test]
fn a_flat_face_resting_on_a_face_that_is_not_round_fails_naming_it() {
    let mut model = model();
    let bottom = peg_face(&model, -Vector3::Z, Point3::ZERO);
    let round = match plate_top(&model) {
        PlaneReference::Face(attachment) => attachment,
        other => panic!("expected a face, found {other:?}"),
    };
    rest_peg_on(&mut model, bottom, round, false);

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    let reason = first_failure(&evaluation);
    assert!(
        reason.contains("which it rests on, is no longer a whole cylinder or sphere"),
        "{reason}"
    );
}
