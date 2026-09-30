use std::f64::consts::{PI, TAU};

use caditor_geometry::{Point3, RigidTransform, Vector3};

use super::*;
use crate::{
    fixtures::{cuboid, cylinder, holed_block, sphere},
    topology::{EdgeId, FaceId, Solid},
};

const CLOSE: f64 = 1e-9;
const NUMERICAL: f64 = 1e-6;

fn moved(solid: &Solid, offset: Vector3) -> Solid {
    solid
        .transformed(&RigidTransform::translation(offset).unwrap())
        .unwrap()
}

fn edge_between(solid: &Solid, from: Point3, to: Point3) -> EdgeId {
    solid
        .edges()
        .find(|(_, edge)| {
            let interval = edge.interval();
            let ends = [
                edge.curve().point(interval.start()),
                edge.curve().point(interval.end()),
            ];
            (ends[0].distance(from) < CLOSE && ends[1].distance(to) < CLOSE)
                || (ends[0].distance(to) < CLOSE && ends[1].distance(from) < CLOSE)
        })
        .map(|(id, _)| id)
        .expect("the solid has that edge")
}

fn face_where(solid: &Solid, test: impl Fn(&Surface) -> bool) -> FaceId {
    solid
        .faces()
        .find(|(_, face)| test(face.surface()))
        .map(|(id, _)| id)
        .expect("the solid has that face")
}

fn flat_face_at(solid: &Solid, point: Point3, normal: Vector3) -> FaceId {
    face_where(solid, |surface| match surface {
        Surface::Plane(plane) => {
            let frame = plane.frame();
            frame.normal().cross(normal).length() < CLOSE
                && (point - frame.origin()).dot(frame.normal()).abs() < CLOSE
        }
        _ => false,
    })
}

fn edge(solid: &Solid, edge: EdgeId) -> Element<'_> {
    Element::Edge { solid, edge }
}

fn face(solid: &Solid, face: FaceId) -> Element<'_> {
    Element::Face { solid, face }
}

fn assert_separation(found: Separation, distance: f64, accuracy: Accuracy, tolerance: f64) {
    assert!(
        (found.distance - distance).abs() < tolerance,
        "{} instead of {distance}",
        found.distance
    );
    assert!((found.from.distance(found.to) - found.distance).abs() < CLOSE);
    assert_eq!(found.accuracy, accuracy);
}

#[test]
fn points_are_apart_by_the_length_between_them() {
    let found = distance(
        Element::Point(Point3::ZERO),
        Element::Point(Point3::new(3.0, 4.0, 12.0)),
    )
    .unwrap();

    assert_separation(found, 13.0, Accuracy::Exact, CLOSE);
    assert_eq!(found.offset(), Vector3::new(3.0, 4.0, 12.0));
}

#[test]
fn a_point_is_measured_square_to_a_straight_edge_or_to_its_nearer_end() {
    let block = cuboid(Vector3::new(10.0, 20.0, 30.0));
    let along_x = edge_between(&block, Point3::ZERO, Point3::new(10.0, 0.0, 0.0));

    let beside = distance(
        Element::Point(Point3::new(5.0, -3.0, 4.0)),
        edge(&block, along_x),
    )
    .unwrap();
    let beyond = distance(
        edge(&block, along_x),
        Element::Point(Point3::new(13.0, -4.0, 0.0)),
    )
    .unwrap();

    assert_separation(beside, 5.0, Accuracy::Exact, CLOSE);
    assert!(beside.to.distance(Point3::new(5.0, 0.0, 0.0)) < CLOSE);
    assert_separation(beyond, 5.0, Accuracy::Exact, CLOSE);
    assert!(beyond.from.distance(Point3::new(10.0, 0.0, 0.0)) < CLOSE);
}

#[test]
fn a_point_is_measured_square_to_a_flat_face_or_to_its_nearest_edge() {
    let block = cuboid(Vector3::new(10.0, 20.0, 30.0));
    let top = flat_face_at(&block, Point3::new(0.0, 0.0, 30.0), Vector3::Z);

    let above = distance(
        Element::Point(Point3::new(5.0, 5.0, 40.0)),
        face(&block, top),
    )
    .unwrap();
    let outside = distance(
        Element::Point(Point3::new(15.0, 5.0, 40.0)),
        face(&block, top),
    )
    .unwrap();

    assert_separation(above, 10.0, Accuracy::Exact, CLOSE);
    assert!(above.to.distance(Point3::new(5.0, 5.0, 30.0)) < CLOSE);
    assert_separation(outside, 125f64.sqrt(), Accuracy::Exact, CLOSE);
}

#[test]
fn a_point_is_measured_to_round_faces_along_their_normal() {
    let tube = cylinder(5.0, 10.0);
    let side = face_where(&tube, |surface| matches!(surface, Surface::Cylinder(_)));
    let ball = sphere(5.0);
    let skin = face_where(&ball, |surface| matches!(surface, Surface::Sphere(_)));

    let to_side = distance(
        Element::Point(Point3::new(10.0, 0.0, 5.0)),
        face(&tube, side),
    )
    .unwrap();
    let to_skin = distance(
        Element::Point(Point3::new(0.0, 6.0, 8.0)),
        face(&ball, skin),
    )
    .unwrap();

    assert_separation(to_side, 5.0, Accuracy::Exact, CLOSE);
    assert!(to_side.to.distance(Point3::new(5.0, 0.0, 5.0)) < CLOSE);
    assert_separation(to_skin, 5.0, Accuracy::Exact, CLOSE);
}

#[test]
fn skew_and_parallel_straight_edges_are_measured_exactly() {
    let block = cuboid(Vector3::new(10.0, 20.0, 30.0));
    let along_x = edge_between(&block, Point3::ZERO, Point3::new(10.0, 0.0, 0.0));
    let upright = edge_between(
        &block,
        Point3::new(0.0, 20.0, 0.0),
        Point3::new(0.0, 20.0, 30.0),
    );
    let other = moved(&block, Vector3::new(0.0, 0.0, 50.0));
    let above = edge_between(
        &other,
        Point3::new(0.0, 0.0, 50.0),
        Point3::new(10.0, 0.0, 50.0),
    );

    let skew = distance(edge(&block, along_x), edge(&block, upright)).unwrap();
    let parallel = distance(edge(&block, along_x), edge(&other, above)).unwrap();

    assert_separation(skew, 20.0, Accuracy::Exact, CLOSE);
    assert_separation(parallel, 50.0, Accuracy::Exact, CLOSE);
}

#[test]
fn parallel_flat_faces_are_apart_by_the_gap_between_their_planes() {
    let block = cuboid(Vector3::new(10.0, 20.0, 30.0));
    let bottom = flat_face_at(&block, Point3::ZERO, Vector3::Z);
    let top = flat_face_at(&block, Point3::new(0.0, 0.0, 30.0), Vector3::Z);
    let side = flat_face_at(&block, Point3::ZERO, Vector3::X);
    let beside = moved(&block, Vector3::new(25.0, 5.0, 12.0));
    let facing = flat_face_at(&beside, Point3::new(25.0, 0.0, 0.0), Vector3::X);

    let across = distance(face(&block, bottom), face(&block, top)).unwrap();
    let touching = distance(face(&block, top), face(&block, side)).unwrap();
    let apart = distance(face(&block, side), face(&beside, facing)).unwrap();

    assert_separation(across, 30.0, Accuracy::Exact, CLOSE);
    assert_separation(touching, 0.0, Accuracy::Exact, CLOSE);
    assert_separation(apart, 25.0, Accuracy::Exact, CLOSE);
}

#[test]
fn a_straight_edge_crossing_a_flat_face_touches_it() {
    let block = cuboid(Vector3::new(10.0, 10.0, 10.0));
    let bottom = flat_face_at(&block, Point3::ZERO, Vector3::Z);
    let pierce = moved(&block, Vector3::new(5.0, 5.0, -5.0));
    let upright = edge_between(
        &pierce,
        Point3::new(5.0, 5.0, -5.0),
        Point3::new(5.0, 5.0, 5.0),
    );

    let found = distance(edge(&pierce, upright), face(&block, bottom)).unwrap();

    assert_separation(found, 0.0, Accuracy::Exact, CLOSE);
    assert!(found.to.distance(Point3::new(5.0, 5.0, 0.0)) < CLOSE);
}

#[test]
fn round_faces_are_measured_numerically_and_said_to_be_approximate() {
    let tube = cylinder(5.0, 10.0);
    let other = moved(&tube, Vector3::new(20.0, 0.0, 3.0));
    let side = face_where(&tube, |surface| matches!(surface, Surface::Cylinder(_)));
    let other_side = face_where(&other, |surface| matches!(surface, Surface::Cylinder(_)));

    let found = distance(face(&tube, side), face(&other, other_side)).unwrap();

    assert_separation(found, 10.0, Accuracy::Approximate, NUMERICAL);
}

#[test]
fn round_edges_are_measured_numerically_and_said_to_be_approximate() {
    let tube = cylinder(5.0, 10.0);
    let other = moved(&tube, Vector3::new(20.0, 0.0, 0.0));
    let rim = |solid: &Solid, height: f64| {
        solid
            .edges()
            .find(|(_, edge)| {
                matches!(edge.curve(), Curve::Circle(circle) if (circle.center().z - height).abs() < CLOSE)
            })
            .map(|(id, _)| id)
            .unwrap()
    };

    let found = distance(
        edge(&tube, rim(&tube, 0.0)),
        edge(&other, rim(&other, 10.0)),
    )
    .unwrap();

    assert_separation(
        found,
        (100.0f64 + 100.0).sqrt(),
        Accuracy::Approximate,
        NUMERICAL,
    );
}

#[test]
fn cylinder_axes_are_apart_by_the_distance_between_their_lines() {
    let tube = cylinder(5.0, 10.0);
    let parallel = moved(&tube, Vector3::new(12.0, 5.0, 40.0));
    let side = face_where(&tube, |surface| matches!(surface, Surface::Cylinder(_)));
    let other_side = face_where(&parallel, |surface| matches!(surface, Surface::Cylinder(_)));
    let first = axis_of(face(&tube, side)).unwrap().unwrap();
    let second = axis_of(face(&parallel, other_side)).unwrap().unwrap();
    let crossing = Axis {
        origin: Point3::new(0.0, 7.0, 3.0),
        direction: Vector3::X,
    };

    let apart = axis_separation(first, second);
    let skew = axis_separation(first, crossing);

    assert_separation(apart, 13.0, Accuracy::Exact, CLOSE);
    assert_separation(skew, 7.0, Accuracy::Exact, CLOSE);
    assert!(skew.from.distance(Point3::new(0.0, 0.0, 3.0)) < CLOSE);
}

#[test]
fn angles_between_edges_and_faces_are_measured_at_corners_and_between_directions() {
    let block = cuboid(Vector3::new(10.0, 20.0, 30.0));
    let along_x = edge_between(&block, Point3::ZERO, Point3::new(10.0, 0.0, 0.0));
    let along_y = edge_between(&block, Point3::ZERO, Point3::new(0.0, 20.0, 0.0));
    let far_upright = edge_between(
        &block,
        Point3::new(10.0, 20.0, 0.0),
        Point3::new(10.0, 20.0, 30.0),
    );
    let top = flat_face_at(&block, Point3::new(0.0, 0.0, 30.0), Vector3::Z);
    let bottom = flat_face_at(&block, Point3::ZERO, Vector3::Z);
    let side = flat_face_at(&block, Point3::ZERO, Vector3::X);
    let degrees = |found: Option<Angle>| found.unwrap().radians.to_degrees();

    let corner = angle(edge(&block, along_x), edge(&block, along_y)).unwrap();
    let skew = angle(edge(&block, along_x), edge(&block, far_upright)).unwrap();
    let planes = angle(face(&block, top), face(&block, side)).unwrap();
    let opposite = angle(face(&block, top), face(&block, bottom)).unwrap();
    let upright_to_top = angle(edge(&block, far_upright), face(&block, top)).unwrap();
    let flat_to_top = angle(face(&block, top), edge(&block, along_x)).unwrap();

    assert_eq!(corner.unwrap().kind, AngleKind::Corner);
    assert!((degrees(corner) - 90.0).abs() < CLOSE);
    assert_eq!(skew.unwrap().kind, AngleKind::Lines);
    assert!((degrees(skew) - 90.0).abs() < CLOSE);
    assert_eq!(planes.unwrap().kind, AngleKind::Planes);
    assert!((degrees(planes) - 90.0).abs() < CLOSE);
    assert!(degrees(opposite).abs() < CLOSE);
    assert_eq!(upright_to_top.unwrap().kind, AngleKind::LineAndPlane);
    assert!((degrees(upright_to_top) - 90.0).abs() < CLOSE);
    assert!(degrees(flat_to_top).abs() < CLOSE);
    assert_eq!(
        angle(Element::Point(Point3::ZERO), face(&block, top)).unwrap(),
        None
    );
}

#[test]
fn a_slanted_corner_is_measured_inside_it() {
    let block = cuboid(Vector3::new(10.0, 10.0, 10.0));
    let along_x = edge_between(&block, Point3::ZERO, Point3::new(10.0, 0.0, 0.0));
    let turned = block
        .transformed(&RigidTransform::rotation_about(Point3::ZERO, Vector3::Z, PI / 6.0).unwrap())
        .unwrap();
    let turned_y = {
        let end = RigidTransform::rotation_about(Point3::ZERO, Vector3::Z, PI / 6.0)
            .unwrap()
            .apply_point(Point3::new(0.0, 10.0, 0.0));
        edge_between(&turned, Point3::ZERO, end)
    };

    let found = angle(edge(&block, along_x), edge(&turned, turned_y))
        .unwrap()
        .unwrap();

    assert_eq!(found.kind, AngleKind::Corner);
    assert!((found.radians.to_degrees() - 120.0).abs() < CLOSE);
}

#[test]
fn edges_report_their_length_and_circles_their_centre_and_radius() {
    let tube = cylinder(5.0, 10.0);
    let (rim, _) = tube
        .edges()
        .find(|(_, edge)| matches!(edge.curve(), Curve::Circle(_)))
        .unwrap();
    let block = cuboid(Vector3::new(10.0, 20.0, 30.0));
    let along_y = edge_between(&block, Point3::ZERO, Point3::new(0.0, 20.0, 0.0));

    let circle = edge_measure(&tube, rim).unwrap();
    let line = edge_measure(&block, along_y).unwrap();

    assert!((circle.length - TAU * 5.0).abs() < CLOSE);
    assert_eq!(circle.length_accuracy, Accuracy::Exact);
    let EdgeForm::Circle {
        center,
        radius,
        sweep,
        ..
    } = circle.form
    else {
        panic!("a round edge is a circle");
    };
    assert!(center.x.abs() < CLOSE && center.y.abs() < CLOSE);
    assert!((radius - 5.0).abs() < CLOSE);
    assert!((sweep - TAU).abs() < CLOSE);
    assert!((line.length - 20.0).abs() < CLOSE);
    assert!(matches!(line.form, EdgeForm::Line { .. }));
}

#[test]
fn flat_faces_have_exact_areas_holes_and_round_edges_included() {
    let block = cuboid(Vector3::new(10.0, 20.0, 30.0));
    let top = flat_face_at(&block, Point3::new(0.0, 0.0, 30.0), Vector3::Z);
    let tube = cylinder(5.0, 10.0);
    let cap = flat_face_at(&tube, Point3::new(0.0, 0.0, 10.0), Vector3::Z);
    let side = face_where(&tube, |surface| matches!(surface, Surface::Cylinder(_)));
    let holed = holed_block(20.0, 5.0, 4.0);
    let holed_top = flat_face_at(&holed, Point3::new(0.0, 0.0, 5.0), Vector3::Z);

    let (rectangle, rectangle_accuracy) = planar_area(&block, top).unwrap().unwrap();
    let (disc, disc_accuracy) = planar_area(&tube, cap).unwrap().unwrap();
    let (pierced, _) = planar_area(&holed, holed_top).unwrap().unwrap();

    assert!((rectangle - 200.0).abs() < CLOSE);
    assert_eq!(rectangle_accuracy, Accuracy::Exact);
    assert!((disc - PI * 25.0).abs() < CLOSE);
    assert_eq!(disc_accuracy, Accuracy::Exact);
    assert!((pierced - (400.0 - PI * 16.0)).abs() < CLOSE);
    assert_eq!(planar_area(&tube, side).unwrap(), None);
}

#[test]
fn faces_report_their_shape() {
    let tube = cylinder(5.0, 10.0);
    let side = face_where(&tube, |surface| matches!(surface, Surface::Cylinder(_)));
    let block = cuboid(Vector3::new(10.0, 20.0, 30.0));
    let bottom = flat_face_at(&block, Point3::ZERO, Vector3::Z);

    let FaceForm::Cylinder { axis, radius } = face_form(&tube, side).unwrap() else {
        panic!("the side of a cylinder is a cylinder");
    };
    let FaceForm::Plane { normal, .. } = face_form(&block, bottom).unwrap() else {
        panic!("the bottom of a block is flat");
    };

    assert!((radius - 5.0).abs() < CLOSE);
    assert!(axis.direction.cross(Vector3::Z).length() < CLOSE);
    assert!((normal - Vector3::NEG_Z).length() < CLOSE);
}

#[test]
fn a_missing_edge_or_face_is_an_error() {
    let block = cuboid(Vector3::new(1.0, 1.0, 1.0));
    let tube = cylinder(5.0, 10.0);
    let (edge, _) = block.edges().last().unwrap();
    let (face, _) = block.faces().last().unwrap();

    assert_eq!(
        distance(
            Element::Point(Point3::ZERO),
            Element::Edge { solid: &tube, edge }
        ),
        Err(MeasureError::MissingEdge(edge))
    );
    assert_eq!(face_form(&tube, face), Err(MeasureError::MissingFace(face)));
}
