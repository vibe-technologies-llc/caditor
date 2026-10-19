use std::{collections::BTreeSet, f64::consts::PI};

use caditor_geometry::{Plane, Point3, Vector3};

use super::{
    MoveError, ReplaceError, edge_faces, replace_faces,
    tests::{check, face_facing, polygon, swept},
};
use crate::{
    boolean::{BooleanOperation, boolean},
    fixtures::{cuboid, cylinder},
    naming::FaceName,
    surface::Surface,
    test_support::assert_cancelled_anywhere,
    topology::{FaceId, Solid},
};

fn names(solid: &Solid) -> BTreeSet<FaceName> {
    solid.faces().map(|(_, face)| face.name()).collect()
}

fn plane(origin: (f64, f64, f64), normal: Vector3) -> Plane {
    Plane::new(Point3::new(origin.0, origin.1, origin.2), normal).unwrap()
}

fn replaced(solid: &Solid, faces: &[FaceId], target: &Plane) -> Solid {
    match replace_faces(solid, faces, target) {
        Ok(result) => result,
        Err(error) => panic!("replacing failed: {error}"),
    }
}

fn block_top(block: &Solid) -> FaceId {
    face_facing(block, Vector3::Z, Point3::new(5.0, 5.0, 10.0))
}

fn lies_on(solid: &Solid, target: &Plane) -> bool {
    solid.faces().any(|(_, face)| match face.surface() {
        Surface::Plane(surface) => {
            let frame = surface.frame();
            frame.normal().cross(target.normal()).length() < 1e-9
                && target.signed_distance(frame.origin()).abs() < 1e-9
        }
        _ => false,
    })
}

fn bevelled_block() -> Solid {
    swept(
        &polygon(&[
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 8.0),
            (8.0, 10.0),
            (0.0, 10.0),
        ]),
        10.0,
    )
}

#[test]
fn a_top_replaced_by_a_higher_plane_raises_the_sides_with_it() {
    let block = cuboid(Vector3::splat(10.0));
    let top = block_top(&block);
    let target = plane((0.0, 0.0, 15.0), Vector3::Z);

    let result = replaced(&block, &[top], &target);

    check("result", &result, 10.0 * 10.0 * 15.0);
    assert_eq!(names(&result), names(&block));
    assert!(lies_on(&result, &target));
}

#[test]
fn a_top_replaced_by_a_tilted_plane_slopes_and_trims_the_sides() {
    let block = cuboid(Vector3::splat(10.0));
    let top = block_top(&block);
    let slope = 20f64.to_radians();
    let target = plane(
        (0.0, 0.0, 8.0),
        Vector3::new(-slope.sin(), 0.0, slope.cos()),
    );

    let result = replaced(&block, &[top], &target);

    check("result", &result, 10.0 * 10.0 * (8.0 + 5.0 * slope.tan()));
    assert_eq!(result.faces().count(), 6);
    assert!(lies_on(&result, &target));
}

#[test]
fn the_target_s_own_facing_does_not_turn_the_face_over() {
    let block = cuboid(Vector3::splat(10.0));
    let top = block_top(&block);

    let up = replaced(&block, &[top], &plane((0.0, 0.0, 6.0), Vector3::Z));
    let down = replaced(&block, &[top], &plane((0.0, 0.0, 6.0), Vector3::NEG_Z));

    check("up", &up, 600.0);
    check("down", &down, 600.0);
}

#[test]
fn a_cylinder_s_top_replaced_by_a_slanted_plane_meets_its_side_in_an_ellipse() {
    let drum = cylinder(5.0, 10.0);
    let top = face_facing(&drum, Vector3::Z, Point3::new(0.0, 0.0, 10.0));
    let slope = 15f64.to_radians();
    let target = plane(
        (0.0, 0.0, 12.0),
        Vector3::new(slope.sin(), 0.0, slope.cos()),
    );

    let result = replaced(&drum, &[top], &target);

    check("result", &result, PI * 25.0 * 12.0);
}

#[test]
fn tops_of_separate_bosses_land_on_one_plane() {
    let plate = swept(
        &polygon(&[(0.0, 0.0), (30.0, 0.0), (30.0, 10.0), (0.0, 10.0)]),
        5.0,
    );
    let low = swept(
        &polygon(&[(2.0, 2.0), (8.0, 2.0), (8.0, 8.0), (2.0, 8.0)]),
        9.0,
    );
    let high = swept(
        &polygon(&[(20.0, 2.0), (26.0, 2.0), (26.0, 8.0), (20.0, 8.0)]),
        12.0,
    );
    let body = boolean(&plate, &low, BooleanOperation::Union).unwrap();
    let body = boolean(&body, &high, BooleanOperation::Union).unwrap();
    let tops = [
        face_facing(&body, Vector3::Z, Point3::new(5.0, 5.0, 9.0)),
        face_facing(&body, Vector3::Z, Point3::new(23.0, 5.0, 12.0)),
    ];

    let result = replaced(&body, &tops, &plane((0.0, 0.0, 15.0), Vector3::Z));

    check("result", &result, 30.0 * 10.0 * 5.0 + 2.0 * 36.0 * 10.0);
}

#[test]
fn a_bevel_replaced_by_a_plane_of_its_own_body_takes_the_square_corner() {
    let block = bevelled_block();
    let side = face_facing(&block, Vector3::X, Point3::new(10.0, 4.0, 5.0));
    let bevel = face_facing(
        &block,
        Vector3::new(1.0, 1.0, 0.0).normalize(),
        Point3::new(9.0, 9.0, 5.0),
    );

    let merged = replace_faces(&block, &[bevel], &plane((0.0, 10.0, 0.0), Vector3::Y));
    let widened = replaced(&block, &[side], &plane((12.0, 0.0, 0.0), Vector3::X));

    assert!(matches!(merged, Err(ReplaceError::Merging { .. })));
    check("widened", &widened, (120.0 - 8.0) * 10.0);
}

#[test]
fn refusals_name_the_face_that_cannot_be_replaced() {
    let block = cuboid(Vector3::splat(10.0));
    let top = block_top(&block);
    let drum = cylinder(5.0, 10.0);
    let round = drum
        .faces()
        .find(|(_, face)| matches!(face.surface(), Surface::Cylinder(_)))
        .map(|(id, _)| id)
        .unwrap();

    let nothing = replace_faces(&block, &[], &plane((0.0, 0.0, 5.0), Vector3::Z));
    let square = replace_faces(&block, &[top], &plane((5.0, 0.0, 0.0), Vector3::X));
    let beyond = replace_faces(&block, &[top], &plane((0.0, 0.0, -2.0), Vector3::Z));
    let flat = replace_faces(&block, &[top], &plane((0.0, 0.0, 0.0), Vector3::Z));
    let still = replace_faces(&block, &[top], &plane((0.0, 0.0, 10.0), Vector3::NEG_Z));
    let curved = replace_faces(&drum, &[round], &plane((0.0, 0.0, 5.0), Vector3::Z));
    let gone = replace_faces(
        &block,
        &[FaceId::from_index(999).unwrap()],
        &plane((0.0, 0.0, 5.0), Vector3::Z),
    );

    assert_eq!(nothing, Err(ReplaceError::NothingToReplace));
    assert_eq!(square, Err(ReplaceError::SquareToTarget(top)));
    assert_eq!(beyond, Err(ReplaceError::BeyondBody(top)));
    assert_eq!(flat, Err(ReplaceError::BeyondBody(top)));
    assert_eq!(still, Err(ReplaceError::AlreadyOnTarget));
    assert_eq!(curved, Err(ReplaceError::NotFlat(round)));
    assert!(matches!(gone, Err(ReplaceError::MissingFace(_))));
}

#[test]
fn a_step_levelled_with_its_riser_still_standing_names_an_edge_of_the_riser() {
    let step = swept(
        &polygon(&[
            (0.0, 0.0),
            (20.0, 0.0),
            (20.0, 5.0),
            (10.0, 5.0),
            (10.0, 10.0),
            (0.0, 10.0),
        ]),
        10.0,
    );
    let lower = face_facing(&step, Vector3::Y, Point3::new(15.0, 5.0, 5.0));
    let riser = face_facing(&step, Vector3::X, Point3::new(10.0, 7.0, 5.0));

    let error = replace_faces(&step, &[lower], &plane((0.0, 10.0, 0.0), Vector3::Y));

    assert!(matches!(
        error,
        Err(ReplaceError::Move(MoveError::EdgeCollapses(edge))) if edge_faces(&step, edge).contains(&riser)
    ));
}

#[test]
fn replacing_can_be_cancelled() {
    let block = cuboid(Vector3::splat(10.0));
    let top = block_top(&block);
    let target = plane((0.0, 0.0, 14.0), Vector3::new(0.1, 0.0, 1.0));

    assert_cancelled_anywhere(
        "replace",
        || replace_faces(&block, &[top], &target),
        |error| matches!(error, ReplaceError::Move(MoveError::Cancelled(_))),
    );
}
