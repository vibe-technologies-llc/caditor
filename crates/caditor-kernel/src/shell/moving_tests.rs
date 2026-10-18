use std::{
    collections::BTreeSet,
    f64::consts::{FRAC_PI_2, PI},
};

use caditor_geometry::{Point3, RigidTransform, Vector3};

use super::{
    MoveError, move_faces,
    tests::{check, face_facing, polygon, swept},
};
use crate::{
    blend::{BlendShape, blend},
    boolean::{BooleanOperation, boolean},
    fixtures::{cuboid, cylinder, holed_block},
    naming::FaceName,
    surface::Surface,
    test_support::assert_cancelled_anywhere,
    topology::{EdgeId, FaceId, Solid},
};

fn names(solid: &Solid) -> BTreeSet<FaceName> {
    solid.faces().map(|(_, face)| face.name()).collect()
}

fn carried(solid: &Solid, faces: &[FaceId], motion: RigidTransform) -> Solid {
    match move_faces(solid, faces, &motion) {
        Ok(result) => result,
        Err(error) => panic!("move failed: {error}"),
    }
}

fn shifted(offset: Vector3) -> RigidTransform {
    RigidTransform::translation(offset).unwrap()
}

fn turned(origin: Point3, axis: Vector3, degrees: f64) -> RigidTransform {
    RigidTransform::rotation_about(origin, axis, degrees.to_radians()).unwrap()
}

fn edge_through(solid: &Solid, middle: Point3) -> EdgeId {
    solid
        .edges()
        .find(|(_, edge)| (edge.curve().point(edge.interval().middle()) - middle).length() < 1e-9)
        .map(|(id, _)| id)
        .unwrap()
}

fn round_faces(solid: &Solid) -> Vec<FaceId> {
    solid
        .faces()
        .filter(|(_, face)| !matches!(face.surface(), Surface::Plane(_)))
        .map(|(id, _)| id)
        .collect()
}

#[test]
fn a_face_moved_along_its_normal_takes_its_neighbours_with_it() {
    let block = cuboid(Vector3::splat(10.0));
    let top = face_facing(&block, Vector3::Z, Point3::new(5.0, 5.0, 10.0));

    let taller = carried(&block, &[top], shifted(Vector3::new(0.0, 0.0, 3.0)));
    let lower = carried(&block, &[top], shifted(Vector3::new(0.0, 0.0, -4.0)));

    check("taller", &taller, 1300.0);
    check("lower", &lower, 600.0);
    assert_eq!(names(&taller), names(&block));
    assert_eq!(taller.faces().count(), 6);
}

#[test]
fn a_face_moved_aslant_moves_only_by_its_share_along_the_normal() {
    let block = cuboid(Vector3::splat(10.0));
    let top = face_facing(&block, Vector3::Z, Point3::new(5.0, 5.0, 10.0));

    let raised = carried(&block, &[top], shifted(Vector3::new(4.0, -3.0, 2.0)));

    check("raised", &raised, 1200.0);
}

#[test]
fn a_wall_tilted_about_its_bottom_edge_leans_out_and_the_top_follows() {
    let block = cuboid(Vector3::splat(10.0));
    let side = face_facing(&block, Vector3::X, Point3::new(10.0, 5.0, 5.0));
    let hinge = Point3::new(10.0, 0.0, 0.0);

    let outward = carried(&block, &[side], turned(hinge, Vector3::Y, 10.0));
    let inward = carried(&block, &[side], turned(hinge, Vector3::Y, -10.0));

    let wedge = 500.0 * 10f64.to_radians().tan();
    check("outward", &outward, 1000.0 + wedge);
    check("inward", &inward, 1000.0 - wedge);
    assert_eq!(names(&outward), names(&block));
}

#[test]
fn a_top_tilted_about_its_middle_cuts_an_ellipse_from_a_round_post() {
    let post = cylinder(5.0, 10.0);
    let top = face_facing(&post, Vector3::Z, Point3::new(0.0, 0.0, 10.0));

    let slanted = carried(
        &post,
        &[top],
        turned(Point3::new(0.0, 0.0, 10.0), Vector3::X, 20.0),
    );

    check("slanted", &slanted, 250.0 * PI);
    assert_eq!(names(&slanted), names(&post));
}

#[test]
fn a_bore_moves_sideways_through_the_plate() {
    let plate = holed_block(20.0, 6.0, 3.0);
    let bore = round_faces(&plate);

    let moved = carried(&plate, &bore, shifted(Vector3::new(4.0, -2.5, 0.0)));

    check("moved", &moved, 20.0 * 20.0 * 6.0 - 9.0 * PI * 6.0);
    assert_eq!(names(&moved), names(&plate));
}

#[test]
fn a_boss_moves_across_its_base_with_its_walls() {
    let base = swept(
        &polygon(&[(0.0, 0.0), (30.0, 0.0), (30.0, 20.0), (0.0, 20.0)]),
        5.0,
    );
    let boss = swept(
        &polygon(&[(5.0, 5.0), (10.0, 5.0), (10.0, 10.0), (5.0, 10.0)]),
        8.0,
    );
    let body = boolean(&base, &boss, BooleanOperation::Union).unwrap();
    let raised: Vec<FaceId> = [
        (Vector3::Z, Point3::new(7.0, 7.0, 8.0)),
        (Vector3::X, Point3::new(10.0, 7.0, 7.0)),
        (Vector3::NEG_X, Point3::new(5.0, 7.0, 7.0)),
        (Vector3::Y, Point3::new(7.0, 10.0, 7.0)),
        (Vector3::NEG_Y, Point3::new(7.0, 5.0, 7.0)),
    ]
    .into_iter()
    .map(|(normal, through)| face_facing(&body, normal, through))
    .collect();

    let moved = carried(&body, &raised, shifted(Vector3::new(12.0, 3.0, 0.0)));

    check("moved", &moved, 3000.0 + 75.0);
}

#[test]
fn a_face_that_would_pass_its_opposite_is_refused() {
    let block = cuboid(Vector3::splat(10.0));
    let top = face_facing(&block, Vector3::Z, Point3::new(5.0, 5.0, 10.0));

    let refused = move_faces(&block, &[top], &shifted(Vector3::new(0.0, 0.0, -12.0)));

    assert!(refused.is_err(), "{refused:?}");
}

#[test]
fn a_wall_tilted_flat_is_refused() {
    let block = cuboid(Vector3::splat(10.0));
    let side = face_facing(&block, Vector3::X, Point3::new(10.0, 5.0, 5.0));

    let refused = move_faces(
        &block,
        &[side],
        &turned(
            Point3::new(10.0, 0.0, 0.0),
            Vector3::Y,
            FRAC_PI_2.to_degrees(),
        ),
    );

    assert!(refused.is_err(), "{refused:?}");
}

#[test]
fn moving_nothing_or_by_nothing_is_refused() {
    let block = cuboid(Vector3::splat(10.0));
    let top = face_facing(&block, Vector3::Z, Point3::new(5.0, 5.0, 10.0));

    let still = move_faces(&block, &[top], &shifted(Vector3::ZERO));
    let none = move_faces(&block, &[], &shifted(Vector3::Z));

    assert_eq!(still, Err(MoveError::Unmoved));
    assert_eq!(none, Err(MoveError::NothingToMove));
}

#[test]
fn moving_a_face_can_be_cancelled() {
    let block = cuboid(Vector3::splat(10.0));
    let side = face_facing(&block, Vector3::X, Point3::new(10.0, 5.0, 5.0));
    let motion = turned(Point3::new(10.0, 0.0, 0.0), Vector3::Y, 10.0);

    assert_cancelled_anywhere(
        "tilt",
        || move_faces(&block, &[side], &motion),
        |error| matches!(error, MoveError::Cancelled(_)),
    );
}

#[test]
fn a_bore_tilted_about_a_line_across_its_middle_slants_through_the_plate() {
    let plate = holed_block(20.0, 6.0, 3.0);
    let bore = round_faces(&plate);
    let tilt = 15f64.to_radians();

    let slanted = carried(
        &plate,
        &bore,
        turned(Point3::new(10.0, 10.0, 3.0), Vector3::Y, tilt.to_degrees()),
    );

    check(
        "slanted",
        &slanted,
        20.0 * 20.0 * 6.0 - 9.0 * PI * 6.0 / tilt.cos(),
    );
}

#[test]
fn a_top_moves_up_with_the_fillet_along_its_rim() {
    let block = cuboid(Vector3::splat(10.0));
    let rim = edge_through(&block, Point3::new(5.0, 0.0, 10.0));
    let rounded = blend(&block, &[rim], BlendShape::Fillet { radius: 2.0 }, 5).unwrap();
    let top = face_facing(&rounded, Vector3::Z, Point3::new(5.0, 5.0, 10.0));
    let fillet = round_faces(&rounded);
    let lift = shifted(Vector3::new(0.0, 0.0, 3.0));
    let rounded_volume = 1000.0 - 10.0 * (4.0 - PI);

    let alone = move_faces(&rounded, &[top], &lift);
    let together = carried(&rounded, &[&[top], &fillet[..]].concat(), lift);

    assert!(alone.is_err(), "{alone:?}");
    check("together", &together, rounded_volume + 300.0);
    assert_eq!(names(&together), names(&rounded));
}
