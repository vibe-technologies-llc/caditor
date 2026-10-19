use std::{collections::BTreeSet, f64::consts::PI};

use caditor_geometry::{Plane, Point3, Vector3};

use super::{
    DraftError, DraftedFace, draft_faces,
    tests::{check, face_facing, polygon, swept},
};
use crate::{
    blend::tangent_faces,
    boolean::{BooleanOperation, boolean},
    build::{LinearExtent, extrude},
    fixtures::{cuboid, cylinder, frustum as frustum_solid, sphere},
    naming::FaceName,
    profile::{Profile, ProfileCurve, Selection},
    surface::Surface,
    test_support::{arc, assert_cancelled_anywhere, circle, line},
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
    let round = round_faces(&post)[0];
    let ball = sphere(5.0);
    let globe = ball.faces().next().map(|(id, _)| id).unwrap();
    let sideways = Plane::new(Point3::ZERO, Vector3::Y).unwrap();

    let parallel = draft_faces(&block, &at(&[top], 5.0), &floor_at(0.0));
    let curved = draft_faces(&ball, &at(&[globe], 5.0), &floor_at(0.0));
    let across = draft_faces(&post, &at(&[round], 5.0), &sideways);
    let pointed = draft_faces(&post, &at(&[round], 30.0), &floor_at(0.0));
    let steep = draft_faces(&block, &at(&sides, 89.5), &floor_at(0.0));
    let none = draft_faces(&block, &[], &floor_at(0.0));
    let square = draft_faces(&block, &at(&sides, 0.0), &floor_at(0.0));

    assert_eq!(parallel, Err(DraftError::AlongNeutral(top)));
    assert_eq!(curved, Err(DraftError::NotDraftable(globe)));
    assert_eq!(across, Err(DraftError::OffPull(round)));
    assert_eq!(pointed, Err(DraftError::NarrowsToPoint(round)));
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

fn round_faces(solid: &Solid) -> Vec<FaceId> {
    solid
        .faces()
        .filter(|(_, face)| matches!(face.surface(), Surface::Cylinder(_) | Surface::Cone(_)))
        .map(|(id, _)| id)
        .collect()
}

fn prism(curves: &[ProfileCurve], base: f64, height: f64) -> Solid {
    let regions = Profile::new(curves)
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap();
    let start = Plane::new(Point3::new(0.0, 0.0, base), Vector3::Z).unwrap();
    extrude(&start, &regions, LinearExtent::one_side(height).unwrap(), 2).unwrap()
}

fn plate(thickness: f64) -> Solid {
    swept(
        &polygon(&[(0.0, 0.0), (30.0, 0.0), (30.0, 30.0), (0.0, 30.0)]),
        thickness,
    )
}

fn round_frustum(bottom: f64, top: f64, height: f64) -> f64 {
    PI * height * (bottom * bottom + bottom * top + top * top) / 3.0
}

fn integrated(area: impl Fn(f64) -> f64, height: f64) -> f64 {
    let steps = 64;
    let step = height / steps as f64;
    (0..steps)
        .map(|index| {
            let low = index as f64 * step;
            step / 6.0 * (area(low) + 4.0 * area(low + 0.5 * step) + area(low + step))
        })
        .sum()
}

fn surfaces_of(solid: &Solid, faces: &[FaceId]) -> Vec<Surface> {
    faces
        .iter()
        .filter_map(|face| Some(solid.face(*face)?.surface().clone()))
        .collect()
}

#[test]
fn a_post_drafted_from_its_floor_becomes_a_cone_narrowing_toward_the_pull() {
    let post = cylinder(5.0, 10.0);
    let round = round_faces(&post);
    let top = 5.0 - 10.0 * 5f64.to_radians().tan();

    let narrowed = drafted(&post, &at(&round, 5.0), &floor_at(0.0));
    let cones = round_faces(&narrowed);

    check("narrowed", &narrowed, round_frustum(5.0, top, 10.0));
    assert_eq!(names(&narrowed), names(&post));
    assert_eq!(cones.len(), 1);
    assert!(
        matches!(&surfaces_of(&narrowed, &cones)[..], [Surface::Cone(_)]),
        "the round side is a cone"
    );
}

#[test]
fn a_neutral_plane_across_a_post_keeps_its_hinge_circle() {
    let post = cylinder(5.0, 10.0);
    let round = round_faces(&post);
    let step = 5.0 * 4f64.to_radians().tan();

    let result = drafted(&post, &at(&round, 4.0), &floor_at(5.0));

    check(
        "result",
        &result,
        round_frustum(5.0 + step, 5.0 - step, 10.0),
    );
}

#[test]
fn a_hole_drafted_from_its_floor_widens_toward_the_pull() {
    let drill = swept(&[circle(9, (15.0, 15.0), 4.0)], 8.0);
    let holed = boolean(&plate(8.0), &drill, BooleanOperation::Difference).unwrap();
    let wall = round_faces(&holed);
    let opening = 4.0 + 8.0 * 6f64.to_radians().tan();

    let result = drafted(&holed, &at(&wall, 6.0), &floor_at(0.0));

    assert_eq!(wall.len(), 1);
    check(
        "result",
        &result,
        900.0 * 8.0 - round_frustum(4.0, opening, 8.0),
    );
}

#[test]
fn a_boss_drafted_from_the_plate_it_stands_on_narrows_upward() {
    let boss = prism(&[circle(9, (15.0, 15.0), 4.0)], 5.0, 6.0);
    let joined = boolean(&plate(5.0), &boss, BooleanOperation::Union).unwrap();
    let side = round_faces(&joined);
    let top = 4.0 - 6.0 * 4f64.to_radians().tan();

    let result = drafted(&joined, &at(&side, 4.0), &floor_at(5.0));

    check(
        "result",
        &result,
        900.0 * 5.0 + round_frustum(4.0, top, 6.0),
    );
}

fn slot(center: (f64, f64), length: f64, half_width: f64) -> Vec<ProfileCurve> {
    let (x, y) = center;
    let (left, right) = (x - 0.5 * length, x + 0.5 * length);
    let (low, high) = (y - half_width, y + half_width);
    vec![
        line(1, (left, low), (right, low)),
        arc(2, (right, y), (right, low), (right, high)),
        line(3, (right, high), (left, high)),
        arc(4, (left, y), (left, high), (left, low)),
    ]
}

#[test]
fn a_slot_wall_is_drafted_as_one_smooth_chain() {
    let cutter = swept(&slot((15.0, 15.0), 10.0, 3.0), 8.0);
    let slotted = boolean(&plate(8.0), &cutter, BooleanOperation::Difference).unwrap();
    let wall = tangent_faces(&slotted, &round_faces(&slotted)[..1]);
    let slope = 5f64.to_radians().tan();
    let area = |height: f64| {
        let half = 3.0 + height * slope;
        20.0 * half + PI * half * half
    };

    let result = drafted(&slotted, &at(&wall, 5.0), &floor_at(0.0));

    assert_eq!(wall.len(), 4);
    check("result", &result, 900.0 * 8.0 - integrated(area, 8.0));
    assert_eq!(names(&result), names(&slotted));
}

fn rounded_rectangle(width: f64, depth: f64, radius: f64) -> Vec<ProfileCurve> {
    let (right, top) = (width, depth);
    vec![
        line(1, (radius, 0.0), (right - radius, 0.0)),
        arc(
            2,
            (right - radius, radius),
            (right - radius, 0.0),
            (right, radius),
        ),
        line(3, (right, radius), (right, top - radius)),
        arc(
            4,
            (right - radius, top - radius),
            (right, top - radius),
            (right - radius, top),
        ),
        line(5, (right - radius, top), (radius, top)),
        arc(
            6,
            (radius, top - radius),
            (radius, top),
            (0.0, top - radius),
        ),
        line(7, (0.0, top - radius), (0.0, radius)),
        arc(8, (radius, radius), (0.0, radius), (radius, 0.0)),
    ]
}

#[test]
fn a_rounded_block_is_drafted_round_its_whole_side() {
    let block = swept(&rounded_rectangle(20.0, 12.0, 3.0), 10.0);
    let side = tangent_faces(&block, &round_faces(&block)[..1]);
    let slope = 5f64.to_radians().tan();
    let area = |height: f64| {
        let inset = height * slope;
        let corner = 3.0 - inset;
        (20.0 - 2.0 * inset) * (12.0 - 2.0 * inset) - (4.0 - PI) * corner * corner
    };

    let result = drafted(&block, &at(&side, 5.0), &floor_at(0.0));

    assert_eq!(side.len(), 8);
    check("result", &result, integrated(area, 10.0));
}

#[test]
fn a_cone_is_redrafted_to_the_new_angle_and_back_to_a_cylinder() {
    let cone = frustum_solid(6.0, 4.0, 10.0);
    let side = round_faces(&cone);
    let top = 6.0 - 10.0 * 3f64.to_radians().tan();

    let gentler = drafted(&cone, &at(&side, 3.0), &floor_at(0.0));
    let square = drafted(&cone, &at(&side, 0.0), &floor_at(0.0));
    let already = draft_faces(
        &cone,
        &at(&side, 0.2f64.atan().to_degrees()),
        &floor_at(0.0),
    );

    check("gentler", &gentler, round_frustum(6.0, top, 10.0));
    check("square", &square, PI * 36.0 * 10.0);
    assert!(
        matches!(
            &surfaces_of(&square, &round_faces(&square))[..],
            [Surface::Cylinder(_)]
        ),
        "drafted at nought the cone stands as a cylinder"
    );
    assert_eq!(already, Err(DraftError::AlreadyDrafted));
}
