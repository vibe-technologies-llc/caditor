use std::collections::BTreeSet;

use caditor_geometry::{Plane, Point3, Vector3};

use super::{
    DraftError, DraftedFace, draft_faces,
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

fn walls(solid: &Solid, middle: Point3, half: f64, inward: bool) -> Vec<FaceId> {
    let sign = if inward { -1.0 } else { 1.0 };
    [Vector3::X, Vector3::NEG_X, Vector3::Y, Vector3::NEG_Y]
        .into_iter()
        .map(|normal| face_facing(solid, normal * sign, middle + normal * half))
        .collect()
}

fn at(faces: &[FaceId], degrees: f64) -> Vec<DraftedFace> {
    faces
        .iter()
        .map(|face| DraftedFace {
            face: *face,
            angle: degrees.to_radians(),
        })
        .collect()
}

fn floor_at(height: f64) -> Plane {
    Plane::new(Point3::new(0.0, 0.0, height), Vector3::Z).unwrap()
}

fn drafted(solid: &Solid, faces: &[DraftedFace], neutral: &Plane) -> Solid {
    match draft_faces(solid, faces, neutral) {
        Ok(result) => result,
        Err(error) => panic!("draft failed: {error}"),
    }
}

fn frustum(bottom: f64, top: f64, height: f64) -> f64 {
    let (low, high) = (bottom * bottom, top * top);
    height / 3.0 * (low + high + (low * high).sqrt())
}

fn block_walls(block: &Solid) -> Vec<FaceId> {
    walls(block, Point3::new(5.0, 5.0, 5.0), 5.0, false)
}

#[test]
fn a_block_drafted_from_its_floor_narrows_toward_the_pull() {
    let block = cuboid(Vector3::splat(10.0));
    let sides = block_walls(&block);
    let shrink = 2.0 * 10.0 * 5f64.to_radians().tan();

    let narrowed = drafted(&block, &at(&sides, 5.0), &floor_at(0.0));
    let widened = drafted(&block, &at(&sides, -5.0), &floor_at(0.0));

    check("narrowed", &narrowed, frustum(10.0, 10.0 - shrink, 10.0));
    check("widened", &widened, frustum(10.0, 10.0 + shrink, 10.0));
    assert_eq!(names(&narrowed), names(&block));
    assert_eq!(narrowed.faces().count(), 6);
}

#[test]
fn a_neutral_plane_across_the_middle_keeps_the_middle_where_it_is() {
    let block = cuboid(Vector3::splat(10.0));
    let sides = block_walls(&block);
    let step = 2.0 * 5.0 * 3f64.to_radians().tan();

    let result = drafted(&block, &at(&sides, 3.0), &floor_at(5.0));

    check("result", &result, frustum(10.0 + step, 10.0 - step, 10.0));
}

#[test]
fn a_reversed_pull_narrows_the_other_way() {
    let block = cuboid(Vector3::splat(10.0));
    let sides = block_walls(&block);
    let shrink = 2.0 * 10.0 * 4f64.to_radians().tan();
    let ceiling = Plane::new(Point3::new(0.0, 0.0, 10.0), Vector3::NEG_Z).unwrap();

    let result = drafted(&block, &at(&sides, 4.0), &ceiling);

    check("result", &result, frustum(10.0 - shrink, 10.0, 10.0));
}

#[test]
fn a_pocket_drafted_from_its_floor_opens_toward_the_pull() {
    let plate = swept(
        &polygon(&[(0.0, 0.0), (30.0, 0.0), (30.0, 30.0), (0.0, 30.0)]),
        8.0,
    );
    let hole = swept(
        &polygon(&[(10.0, 10.0), (20.0, 10.0), (20.0, 20.0), (10.0, 20.0)]),
        8.0,
    );
    let holed = boolean(&plate, &hole, BooleanOperation::Difference).unwrap();
    let sides = walls(&holed, Point3::new(15.0, 15.0, 4.0), 5.0, true);
    let opening = 2.0 * 8.0 * 6f64.to_radians().tan();

    let result = drafted(&holed, &at(&sides, 6.0), &floor_at(0.0));

    check(
        "result",
        &result,
        30.0 * 30.0 * 8.0 - frustum(10.0, 10.0 + opening, 8.0),
    );
}

#[test]
fn faces_take_angles_of_their_own() {
    let block = cuboid(Vector3::splat(10.0));
    let east = face_facing(&block, Vector3::X, Point3::new(10.0, 5.0, 5.0));
    let west = face_facing(&block, Vector3::NEG_X, Point3::new(0.0, 5.0, 5.0));
    let faces = [
        DraftedFace {
            face: east,
            angle: 10f64.to_radians(),
        },
        DraftedFace {
            face: west,
            angle: -5f64.to_radians(),
        },
    ];
    let wedge = |degrees: f64| 500.0 * degrees.to_radians().tan();

    let result = drafted(&block, &faces, &floor_at(0.0));

    check("result", &result, 1000.0 - wedge(10.0) + wedge(5.0));
}

#[test]
fn a_drafted_wall_drafted_again_at_nought_stands_square() {
    let block = cuboid(Vector3::splat(10.0));
    let sides = block_walls(&block);
    let leaning = drafted(&block, &at(&sides, 7.0), &floor_at(0.0));
    let leaning_sides: Vec<FaceId> = leaning
        .faces()
        .filter(|(_, face)| match face.surface() {
            Surface::Plane(plane) => plane.frame().normal().z.abs() < 0.5,
            _ => false,
        })
        .map(|(id, _)| id)
        .collect();

    let square = drafted(&leaning, &at(&leaning_sides, 0.0), &floor_at(0.0));

    assert_eq!(leaning_sides.len(), 4);
    check("square", &square, 1000.0);
}

#[test]
fn faces_that_cannot_be_drafted_are_refused_by_name() {
    let block = cuboid(Vector3::splat(10.0));
    let top = face_facing(&block, Vector3::Z, Point3::new(5.0, 5.0, 10.0));
    let sides = block_walls(&block);
    let post = cylinder(5.0, 10.0);
    let round = post
        .faces()
        .find(|(_, face)| matches!(face.surface(), Surface::Cylinder(_)))
        .map(|(id, _)| id)
        .unwrap();

    let parallel = draft_faces(&block, &at(&[top], 5.0), &floor_at(0.0));
    let curved = draft_faces(&post, &at(&[round], 5.0), &floor_at(0.0));
    let steep = draft_faces(&block, &at(&sides, 89.5), &floor_at(0.0));
    let none = draft_faces(&block, &[], &floor_at(0.0));
    let square = draft_faces(&block, &at(&sides, 0.0), &floor_at(0.0));

    assert_eq!(parallel, Err(DraftError::AlongNeutral(top)));
    assert_eq!(curved, Err(DraftError::NotFlat(round)));
    assert_eq!(steep, Err(DraftError::TooSteep(sides[0])));
    assert_eq!(none, Err(DraftError::NothingToDraft));
    assert_eq!(square, Err(DraftError::AlreadyDrafted));
}

#[test]
fn a_draft_too_steep_for_the_block_fails_rather_than_folding() {
    let block = cuboid(Vector3::new(10.0, 10.0, 40.0));
    let sides = walls(&block, Point3::new(5.0, 5.0, 20.0), 5.0, false);

    let refused = draft_faces(&block, &at(&sides, 20.0), &floor_at(0.0));

    assert!(matches!(refused, Err(DraftError::Move(_))), "{refused:?}");
}

#[test]
fn drafting_can_be_cancelled() {
    let block = cuboid(Vector3::splat(10.0));
    let sides = block_walls(&block);
    let faces = at(&sides, 5.0);

    assert_cancelled_anywhere(
        "draft",
        || draft_faces(&block, &faces, &floor_at(0.0)),
        |error| matches!(error, DraftError::Move(super::MoveError::Cancelled(_))),
    );
}
