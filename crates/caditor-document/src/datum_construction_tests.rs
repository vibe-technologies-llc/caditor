use caditor_expression::{Expression, ParameterId, Unit};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::{Curve, EdgeReference, FaceReference, Solid, Surface};
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

fn extrude(sketch: FeatureId, height: Expression) -> FeatureKind {
    FeatureKind::Solid(SolidFeature::Extrude(Extrude {
        sketch,
        regions: RegionChoice::All,
        extent: ExtrudeExtent::one_side(height, false),
        operation: BodyOperation::NewBody,
        start: None,
        other_bodies: Vec::new(),
    }))
}

fn revolve(sketch: FeatureId) -> FeatureKind {
    FeatureKind::Solid(SolidFeature::Revolve(Revolve {
        sketch,
        regions: RegionChoice::All,
        axis: RevolveAxis::Sketch(EntityId::VERTICAL_AXIS),
        extent: RevolveExtent::Full,
        operation: BodyOperation::NewBody,
        start: None,
        other_bodies: Vec::new(),
        side: None,
    }))
}

struct Parts {
    document: Document,
    height: ParameterId,
    block: FeatureId,
    pin: FeatureId,
    cone: FeatureId,
    ring: FeatureId,
}

fn parts() -> Parts {
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
    let block = transaction.add_feature("Block", extrude(outline, Expression::Parameter(height)));

    let mut disc = Sketch::new(Plane::XY);
    disc.add_circle(Point2::new(30.0, -2.0), 5.0);
    let disc = transaction.add_feature("Disc", FeatureKind::from(disc));
    let pin = transaction.add_feature("Pin", extrude(disc, millimetres(6.0)));

    let mut profile = Sketch::new(Plane::XZ);
    let section = [
        Point2::new(0.0, 0.0),
        Point2::new(4.0, 0.0),
        Point2::new(2.0, 6.0),
        Point2::new(0.0, 6.0),
    ];
    for index in 0..4 {
        profile.add_line(section[index], section[(index + 1) % 4]);
    }
    let profile = transaction.add_feature("Profile", FeatureKind::from(profile));
    let cone = transaction.add_feature("Cone", revolve(profile));

    let mut tube = Sketch::new(Plane::XZ);
    tube.add_circle(Point2::new(20.0, 5.0), 1.0);
    let tube = transaction.add_feature("Tube", FeatureKind::from(tube));
    let ring = transaction.add_feature("Ring", revolve(tube));

    document.apply(transaction.finish()).unwrap();
    Parts {
        document,
        height,
        block,
        pin,
        cone,
        ring,
    }
}

fn face_where(solid: &Solid, wanted: impl Fn(&Surface) -> bool) -> FaceReference {
    let (face, _) = solid
        .faces()
        .find(|(_, face)| wanted(face.surface()))
        .unwrap();
    FaceReference::capture(solid, face).unwrap()
}

fn edge_between(solid: &Solid, from: Point3, to: Point3) -> EdgeReference {
    let (edge, _) = solid
        .edges()
        .find(|(_, edge)| {
            let ends =
                [edge.start(), edge.end()].map(|vertex| solid.vertex(vertex).unwrap().point());
            let matches = |point: Point3, other: Point3| point.distance(other) < CLOSE;
            (matches(ends[0], from) && matches(ends[1], to))
                || (matches(ends[0], to) && matches(ends[1], from))
        })
        .unwrap();
    EdgeReference::capture(solid, edge).unwrap()
}

fn block_edge(parts: &Parts, from: Point3, to: Point3) -> AxisReference {
    let evaluation = evaluate(&parts.document);
    let solid = evaluation.body(parts.block).unwrap();
    AxisReference::Edge {
        body: parts.block,
        edge: Box::new(edge_between(solid, from, to)),
    }
}

fn along(parts: &Parts, from: Point3, to: Point3, distance: Expression) -> CurveStation {
    let evaluation = evaluate(&parts.document);
    let solid = evaluation.body(parts.block).unwrap();
    CurveStation {
        body: parts.block,
        edge: Box::new(edge_between(solid, from, to)),
        distance,
    }
}

fn offset_point(x: f64, y: f64, z: f64) -> Datum {
    Datum::Point(DatumPoint {
        base: PointReference::Origin,
        offset: [x, y, z].map(millimetres),
    })
}

fn principal(plane: PrincipalPlane) -> PlaneReference {
    PlaneReference::Principal(plane)
}

#[test]
fn a_plane_lies_tangent_to_a_cylinder_on_the_side_of_a_point() {
    let mut parts = parts();
    let evaluation = evaluate(&parts.document);
    let solid = evaluation.body(parts.pin).unwrap();
    let wall = face_where(solid, |surface| matches!(surface, Surface::Cylinder(_)));
    let toward = add(&mut parts.document, "Side", offset_point(30.0, 20.0, 1.0));
    let tangent = add(
        &mut parts.document,
        "Tangent",
        Datum::PlaneThrough(PlaneThrough::Tangent(Box::new(FaceTangent {
            body: parts.pin,
            face: wall,
            toward: PointReference::Datum(toward),
        }))),
    );

    let evaluation = evaluate(&parts.document);
    let plane = result(&evaluation, tangent).plane().unwrap();

    assert!((plane.normal() - Vector3::Y).length() < CLOSE);
    assert!(plane.signed_distance(Point3::new(30.0, 3.0, 2.0)).abs() < CLOSE);
    assert!(plane.signed_distance(Point3::new(30.0, -2.0, 2.0)) < 0.0);
}

#[test]
fn a_tangent_plane_to_a_cone_touches_it_along_a_generator() {
    let mut parts = parts();
    let evaluation = evaluate(&parts.document);
    let solid = evaluation.body(parts.cone).unwrap();
    let slope = face_where(solid, |surface| matches!(surface, Surface::Cone(_)));
    let toward = add(&mut parts.document, "Side", offset_point(10.0, 0.0, 0.0));
    let tangent = add(
        &mut parts.document,
        "Tangent",
        Datum::PlaneThrough(PlaneThrough::Tangent(Box::new(FaceTangent {
            body: parts.cone,
            face: slope,
            toward: PointReference::Datum(toward),
        }))),
    );

    let evaluation = evaluate(&parts.document);
    let plane = result(&evaluation, tangent).plane().unwrap();
    let outward = Vector3::new(6.0, 0.0, 2.0).normalize();

    assert!((plane.normal() - outward).length() < CLOSE);
    assert!(plane.signed_distance(Point3::new(4.0, 0.0, 0.0)).abs() < CLOSE);
    assert!(plane.signed_distance(Point3::new(2.0, 0.0, 6.0)).abs() < CLOSE);
}

#[test]
fn a_tangent_plane_fails_alone_when_the_side_is_on_the_axis_or_the_face_is_flat() {
    let mut parts = parts();
    let evaluation = evaluate(&parts.document);
    let solid = evaluation.body(parts.pin).unwrap();
    let wall = face_where(solid, |surface| matches!(surface, Surface::Cylinder(_)));
    let cap = face_where(solid, |surface| matches!(surface, Surface::Plane(_)));
    let on_axis = add(
        &mut parts.document,
        "On axis",
        offset_point(30.0, -2.0, 1.0),
    );
    let off_axis = add(
        &mut parts.document,
        "Off axis",
        offset_point(30.0, 20.0, 1.0),
    );
    let centred = add(
        &mut parts.document,
        "Centred",
        Datum::PlaneThrough(PlaneThrough::Tangent(Box::new(FaceTangent {
            body: parts.pin,
            face: wall,
            toward: PointReference::Datum(on_axis),
        }))),
    );
    let flat = add(
        &mut parts.document,
        "Flat",
        Datum::PlaneThrough(PlaneThrough::Tangent(Box::new(FaceTangent {
            body: parts.pin,
            face: cap,
            toward: PointReference::Datum(off_axis),
        }))),
    );

    let evaluation = evaluate(&parts.document);

    assert!(
        failure(&evaluation, centred)
            .reason
            .contains("lies on its axis"),
        "{}",
        failure(&evaluation, centred).reason
    );
    assert!(
        failure(&evaluation, flat)
            .reason
            .contains("no longer a cylindrical or conical face"),
        "{}",
        failure(&evaluation, flat).reason
    );
}

#[test]
fn a_plane_stands_square_to_a_round_edge_at_a_distance_along_it() {
    let mut parts = parts();
    let evaluation = evaluate(&parts.document);
    let solid = evaluation.body(parts.pin).unwrap();
    let (rim, _) = solid
        .edges()
        .find(|(_, edge)| matches!(edge.curve(), Curve::Circle(circle) if circle.center().z > 3.0))
        .unwrap();
    let rim = EdgeReference::capture(solid, rim).unwrap();
    let centre = Point3::new(30.0, -2.0, 6.0);
    let quarter = std::f64::consts::PI * 5.0 / 2.0;
    let station = |distance: f64| {
        Datum::PlaneThrough(PlaneThrough::SquareToCurve(Box::new(CurveStation {
            body: parts.pin,
            edge: Box::new(rim),
            distance: millimetres(distance),
        })))
    };
    let start = add(&mut parts.document, "Start", station(0.0));
    let turned = add(&mut parts.document, "Quarter", station(quarter));
    let backwards = add(&mut parts.document, "Back", station(-quarter));

    let evaluation = evaluate(&parts.document);
    let first = result(&evaluation, start).plane().unwrap();
    let second = result(&evaluation, turned).plane().unwrap();
    let third = result(&evaluation, backwards).plane().unwrap();

    for plane in [first, second, third] {
        let radial = plane.origin() - centre;
        assert!((radial.length() - 5.0).abs() < CLOSE);
        assert!(plane.normal().dot(radial).abs() < 1e-7);
        assert!(plane.normal().z.abs() < CLOSE);
    }
    assert!(
        (second.origin() - centre)
            .dot(first.origin() - centre)
            .abs()
            < 1e-7
    );
    assert!((third.origin() - centre).dot(first.origin() - centre).abs() < 1e-7);
    assert!(second.origin().distance(third.origin()) > 9.0);
}

#[test]
fn a_point_lies_a_distance_along_a_straight_edge_and_follows_a_parameter() {
    let mut parts = parts();
    let station = along(
        &parts,
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(10.0, 0.0, 0.0),
        Expression::Parameter(parts.height),
    );
    let point = add(
        &mut parts.document,
        "Point 1",
        Datum::PointBy(PointBy::Along(Box::new(station))),
    );
    let before = result(&evaluate(&parts.document), point).point().unwrap();
    let longer = parts.document.parse("7 mm").unwrap();
    parts
        .document
        .apply(Transaction::single(
            "Longer",
            Edit::SetParameterExpression {
                id: parts.height,
                expression: longer,
            },
        ))
        .unwrap();
    let after = result(&evaluate(&parts.document), point).point().unwrap();

    let reached = |point: Point3, distance: f64| {
        point.y.abs() < CLOSE
            && point.z.abs() < CLOSE
            && ((point.x - distance).abs() < CLOSE || (10.0 - point.x - distance).abs() < CLOSE)
    };

    assert!(reached(before, 4.0));
    assert!(reached(after, 7.0));
    assert!(
        parts
            .document
            .feature(point)
            .unwrap()
            .kind
            .parameters()
            .contains(&parts.height)
    );
}

#[test]
fn a_point_along_an_edge_counts_back_from_the_end_and_refuses_a_distance_past_it() {
    let mut parts = parts();
    let edge = |distance: f64| {
        Datum::PointBy(PointBy::Along(Box::new(along(
            &parts,
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(10.0, 0.0, 0.0),
            millimetres(distance),
        ))))
    };
    let forward = edge(3.0);
    let backward = edge(-3.0);
    let beyond = edge(12.0);
    let forward = add(&mut parts.document, "Forward", forward);
    let backward = add(&mut parts.document, "Backward", backward);
    let beyond = add(&mut parts.document, "Beyond", beyond);

    let evaluation = evaluate(&parts.document);
    let first = result(&evaluation, forward).point().unwrap();
    let second = result(&evaluation, backward).point().unwrap();
    let error = failure(&evaluation, beyond);

    assert!((first.x + second.x - 10.0).abs() < CLOSE);
    assert!((first.x - second.x).abs() - 4.0 < CLOSE);
    assert!(
        error.reason.contains("only 10.000 mm long"),
        "{}",
        error.reason
    );
}

#[test]
fn a_plane_passes_through_two_lines_that_cross_or_run_parallel() {
    let mut parts = parts();
    let along_x = block_edge(
        &parts,
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(10.0, 0.0, 0.0),
    );
    let along_y = block_edge(
        &parts,
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(0.0, 8.0, 0.0),
    );
    let far_x = block_edge(
        &parts,
        Point3::new(0.0, 8.0, 0.0),
        Point3::new(10.0, 8.0, 0.0),
    );
    let up = block_edge(
        &parts,
        Point3::new(10.0, 8.0, 0.0),
        Point3::new(10.0, 8.0, 4.0),
    );
    let crossing = add(
        &mut parts.document,
        "Crossing",
        Datum::PlaneThrough(PlaneThrough::Lines(along_x.clone(), along_y)),
    );
    let parallel = add(
        &mut parts.document,
        "Parallel",
        Datum::PlaneThrough(PlaneThrough::Lines(along_x.clone(), far_x)),
    );
    let skew = add(
        &mut parts.document,
        "Skew",
        Datum::PlaneThrough(PlaneThrough::Lines(along_x.clone(), up)),
    );
    let same = add(
        &mut parts.document,
        "Same",
        Datum::PlaneThrough(PlaneThrough::Lines(along_x.clone(), along_x)),
    );

    let evaluation = evaluate(&parts.document);
    let flat = result(&evaluation, crossing).plane().unwrap();
    let level = result(&evaluation, parallel).plane().unwrap();

    assert!(flat.normal().cross(Vector3::Z).length() < CLOSE);
    assert!(level.normal().cross(Vector3::Z).length() < CLOSE);
    assert!(level.signed_distance(Point3::new(5.0, 4.0, 0.0)).abs() < CLOSE);
    assert!(
        failure(&evaluation, skew)
            .reason
            .contains("do not lie in one plane"),
        "{}",
        failure(&evaluation, skew).reason
    );
    assert!(
        failure(&evaluation, same)
            .reason
            .contains("are the same line"),
        "{}",
        failure(&evaluation, same).reason
    );
}

#[test]
fn a_point_sits_where_two_edges_cross_and_a_miss_is_explained() {
    let mut parts = parts();
    let along_x = block_edge(
        &parts,
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(10.0, 0.0, 0.0),
    );
    let up_left = block_edge(
        &parts,
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(0.0, 0.0, 4.0),
    );
    let far_x = block_edge(
        &parts,
        Point3::new(0.0, 8.0, 0.0),
        Point3::new(10.0, 8.0, 0.0),
    );
    let up_right = block_edge(
        &parts,
        Point3::new(10.0, 8.0, 0.0),
        Point3::new(10.0, 8.0, 4.0),
    );
    let along_y = block_edge(
        &parts,
        Point3::new(10.0, 0.0, 0.0),
        Point3::new(10.0, 8.0, 0.0),
    );
    let sideways = add(
        &mut parts.document,
        "Sideways",
        Datum::PointBy(PointBy::LinesCross(along_x.clone(), along_y)),
    );
    let meeting = add(
        &mut parts.document,
        "Meeting",
        Datum::PointBy(PointBy::LinesCross(along_x.clone(), up_left)),
    );
    let apart = add(
        &mut parts.document,
        "Apart",
        Datum::PointBy(PointBy::LinesCross(along_x.clone(), far_x)),
    );
    let passing = add(
        &mut parts.document,
        "Passing",
        Datum::PointBy(PointBy::LinesCross(along_x, up_right)),
    );

    let evaluation = evaluate(&parts.document);

    assert_eq!(
        result(&evaluation, meeting),
        DatumResult::Point(Point3::ZERO)
    );
    assert!(
        result(&evaluation, sideways)
            .point()
            .unwrap()
            .distance(Point3::new(10.0, 0.0, 0.0))
            < CLOSE
    );
    assert!(failure(&evaluation, apart).reason.contains("run parallel"));
    assert!(
        failure(&evaluation, passing)
            .reason
            .contains("pass each other without meeting")
    );
}

#[test]
fn a_point_sits_where_an_edge_meets_a_plane() {
    let mut parts = parts();
    let upright = block_edge(
        &parts,
        Point3::new(10.0, 8.0, 0.0),
        Point3::new(10.0, 8.0, 4.0),
    );
    let along_x = block_edge(
        &parts,
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(10.0, 0.0, 0.0),
    );
    let raised = add(
        &mut parts.document,
        "Raised",
        Datum::Plane(DatumPlane {
            base: principal(PrincipalPlane::Xy),
            rotation: None,
            offset: millimetres(9.0),
        }),
    );
    let high = add(
        &mut parts.document,
        "High",
        Datum::PointBy(PointBy::AxisAndPlane(
            upright.clone(),
            PlaneReference::Datum(raised),
        )),
    );
    let parallel = add(
        &mut parts.document,
        "Parallel",
        Datum::PointBy(PointBy::AxisAndPlane(
            along_x.clone(),
            PlaneReference::Datum(raised),
        )),
    );
    let lying = add(
        &mut parts.document,
        "Lying",
        Datum::PointBy(PointBy::AxisAndPlane(
            along_x,
            principal(PrincipalPlane::Xy),
        )),
    );

    let evaluation = evaluate(&parts.document);

    assert!(
        result(&evaluation, high)
            .point()
            .unwrap()
            .distance(Point3::new(10.0, 8.0, 9.0))
            < CLOSE
    );
    assert!(
        failure(&evaluation, parallel)
            .reason
            .contains("runs parallel")
    );
    assert!(
        failure(&evaluation, lying)
            .reason
            .contains("crosses it everywhere")
    );
}

#[test]
fn a_point_sits_where_three_planes_meet_and_refuses_planes_sharing_a_line() {
    let mut parts = parts();
    let raised = add(
        &mut parts.document,
        "Raised",
        Datum::Plane(DatumPlane {
            base: principal(PrincipalPlane::Xy),
            rotation: None,
            offset: millimetres(5.0),
        }),
    );
    let corner = add(
        &mut parts.document,
        "Corner",
        Datum::PointBy(PointBy::ThreePlanes([
            PlaneReference::Datum(raised),
            principal(PrincipalPlane::Xz),
            principal(PrincipalPlane::Yz),
        ])),
    );
    let sharing = add(
        &mut parts.document,
        "Sharing",
        Datum::PointBy(PointBy::ThreePlanes([
            principal(PrincipalPlane::Xy),
            principal(PrincipalPlane::Xz),
            PlaneReference::Datum(raised),
        ])),
    );

    let evaluation = evaluate(&parts.document);

    assert!(
        result(&evaluation, corner)
            .point()
            .unwrap()
            .distance(Point3::new(0.0, 0.0, 5.0))
            < CLOSE
    );
    assert!(
        failure(&evaluation, sharing)
            .reason
            .contains("do not meet in a single point")
    );
}

#[test]
fn a_point_sits_at_the_centre_of_a_torus_and_a_flat_face_gives_none() {
    let mut parts = parts();
    let evaluation = evaluate(&parts.document);
    let torus = evaluation.body(parts.ring).unwrap();
    let skin = face_where(torus, |surface| matches!(surface, Surface::Torus(_)));
    let block = evaluation.body(parts.block).unwrap();
    let side = face_where(block, |surface| matches!(surface, Surface::Plane(_)));
    let centre = add(
        &mut parts.document,
        "Centre",
        Datum::Point(DatumPoint {
            base: PointReference::SurfaceCentre {
                body: parts.ring,
                face: skin,
            },
            offset: [0.0; 3].map(millimetres),
        }),
    );
    let flat = add(
        &mut parts.document,
        "Flat",
        Datum::Point(DatumPoint {
            base: PointReference::SurfaceCentre {
                body: parts.block,
                face: side,
            },
            offset: [0.0; 3].map(millimetres),
        }),
    );

    let evaluation = evaluate(&parts.document);

    assert_eq!(
        result(&evaluation, centre),
        DatumResult::Point(Point3::new(0.0, 0.0, 5.0))
    );
    assert!(
        failure(&evaluation, flat)
            .reason
            .contains("no longer a sphere or a torus")
    );
}

#[test]
fn constructed_points_and_planes_report_what_they_use() {
    let mut parts = parts();
    let upright = block_edge(
        &parts,
        Point3::new(10.0, 8.0, 0.0),
        Point3::new(10.0, 8.0, 4.0),
    );
    let plane = add(
        &mut parts.document,
        "Raised",
        Datum::Plane(DatumPlane {
            base: principal(PrincipalPlane::Xy),
            rotation: None,
            offset: millimetres(9.0),
        }),
    );
    let point = add(
        &mut parts.document,
        "Meeting",
        Datum::PointBy(PointBy::AxisAndPlane(upright, PlaneReference::Datum(plane))),
    );
    let kind = &parts.document.feature(point).unwrap().kind;

    assert!(kind.features().contains(&plane));
    assert!(kind.bodies_used().contains(&parts.block));
    assert!(kind.datum().unwrap().is_point());
}
