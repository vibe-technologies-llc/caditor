use std::{collections::BTreeSet, f64::consts::PI};

use caditor_geometry::{Plane, Point3, RigidTransform, Vector3};

use super::{
    MoveError, ReplaceError, replace_faces, replace_faces_onto_round,
    tests::{check, face_facing, polygon, swept},
};
use crate::{
    blend::{BlendShape, blend},
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

    let merged = replaced(&block, &[bevel], &plane((0.0, 10.0, 0.0), Vector3::Y));
    let widened = replaced(&block, &[side], &plane((12.0, 0.0, 0.0), Vector3::X));

    check("merged", &merged, 100.0 * 10.0);
    assert_eq!(merged.faces().count(), 6);
    assert_eq!(merged.edges().count(), 12);
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

fn stepped() -> Solid {
    swept(
        &polygon(&[
            (0.0, 0.0),
            (20.0, 0.0),
            (20.0, 5.0),
            (10.0, 5.0),
            (10.0, 10.0),
            (0.0, 10.0),
        ]),
        10.0,
    )
}

#[test]
fn a_step_levelled_up_merges_the_tops_and_drops_the_riser() {
    let step = stepped();
    let lower = face_facing(&step, Vector3::Y, Point3::new(15.0, 5.0, 5.0));
    let lower_name = step.face(lower).unwrap().name();

    let result = replaced(&step, &[lower], &plane((0.0, 10.0, 0.0), Vector3::Y));

    check("result", &result, 20.0 * 10.0 * 10.0);
    assert_eq!(result.faces().count(), 6);
    assert_eq!(result.edges().count(), 12);
    assert!(result.faces().any(|(_, face)| face.name() == lower_name));
}

#[test]
fn a_step_levelled_down_merges_the_tops_and_drops_the_riser() {
    let step = stepped();
    let upper = face_facing(&step, Vector3::Y, Point3::new(5.0, 10.0, 5.0));

    let result = replaced(&step, &[upper], &plane((0.0, 5.0, 0.0), Vector3::Y));

    check("result", &result, 20.0 * 5.0 * 10.0);
    assert_eq!(result.faces().count(), 6);
    assert_eq!(result.edges().count(), 12);
}

#[test]
fn two_slopes_of_a_roof_land_as_one_flat_top() {
    let roof = swept(
        &polygon(&[
            (0.0, 0.0),
            (20.0, 0.0),
            (20.0, 10.0),
            (10.0, 14.0),
            (0.0, 10.0),
        ]),
        8.0,
    );
    let slopes = [
        face_facing(
            &roof,
            Vector3::new(4.0, 10.0, 0.0).normalize(),
            Point3::new(15.0, 12.0, 4.0),
        ),
        face_facing(
            &roof,
            Vector3::new(-4.0, 10.0, 0.0).normalize(),
            Point3::new(5.0, 12.0, 4.0),
        ),
    ];

    let result = replaced(&roof, &slopes, &plane((0.0, 12.0, 0.0), Vector3::Y));

    check("result", &result, 20.0 * 12.0 * 8.0);
    assert_eq!(result.faces().count(), 6);
    assert_eq!(result.edges().count(), 12);
}

#[test]
fn a_pocket_s_floor_levelled_with_the_top_fills_it() {
    let base = cuboid(Vector3::new(20.0, 20.0, 10.0));
    let tool = swept(
        &polygon(&[(5.0, 5.0), (15.0, 5.0), (15.0, 15.0), (5.0, 15.0)]),
        30.0,
    );
    let tool_mapped = tool
        .transformed(&RigidTransform::translation(Vector3::new(0.0, 0.0, 5.0)).unwrap())
        .unwrap();
    let pocketed = boolean(&base, &tool_mapped, BooleanOperation::Difference).unwrap();
    let floor = face_facing(&pocketed, Vector3::Z, Point3::new(10.0, 10.0, 5.0));

    let result = replaced(&pocketed, &[floor], &plane((0.0, 0.0, 10.0), Vector3::Z));

    check("result", &result, 4000.0);
    assert_eq!(result.faces().count(), 6);
    assert!(result.faces().all(|(_, face)| face.loops().len() == 1));
}

#[test]
fn a_blind_hole_s_floor_levelled_with_the_top_fills_it() {
    let base = cuboid(Vector3::new(20.0, 20.0, 10.0));
    let bore = cylinder(3.0, 30.0)
        .transformed(&RigidTransform::translation(Vector3::new(10.0, 10.0, 4.0)).unwrap())
        .unwrap();
    let holed = boolean(&base, &bore, BooleanOperation::Difference).unwrap();
    let floor = face_facing(&holed, Vector3::Z, Point3::new(10.0, 10.0, 4.0));

    let result = replaced(&holed, &[floor], &plane((0.0, 0.0, 10.0), Vector3::Z));

    check("result", &result, 4000.0);
    assert_eq!(result.faces().count(), 6);
}

#[test]
fn levelling_a_tab_in_a_notch_drops_three_risers() {
    let body = swept(
        &polygon(&[
            (0.0, 0.0),
            (30.0, 0.0),
            (30.0, 10.0),
            (20.0, 10.0),
            (20.0, 5.0),
            (10.0, 5.0),
            (10.0, 10.0),
            (0.0, 10.0),
        ]),
        10.0,
    );
    let notch = face_facing(&body, Vector3::Y, Point3::new(15.0, 5.0, 5.0));

    let result = replaced(&body, &[notch], &plane((0.0, 10.0, 0.0), Vector3::Y));

    check("result", &result, 30.0 * 10.0 * 10.0);
    assert_eq!(result.faces().count(), 6);
    assert_eq!(result.edges().count(), 12);
}

#[test]
fn a_target_beyond_the_step_above_still_leaves_the_riser_folded_and_refused() {
    let step = stepped();
    let lower = face_facing(&step, Vector3::Y, Point3::new(15.0, 5.0, 5.0));

    let past = replace_faces(&step, &[lower], &plane((0.0, 12.0, 0.0), Vector3::Y));

    assert!(past.is_err(), "{past:?}");
}

fn wall_of(solid: &Solid) -> FaceId {
    solid
        .faces()
        .find(|(_, face)| matches!(face.surface(), Surface::Cylinder(_)))
        .map(|(id, _)| id)
        .unwrap()
}

fn round_surface(radius: f64, x: f64) -> Surface {
    let frame = Plane::from_frame(Point3::new(x, 0.0, 0.0), Vector3::Z, Vector3::X).unwrap();
    crate::surface::Cylinder::new(frame, radius).unwrap().into()
}

#[test]
fn a_drum_s_wall_onto_a_coaxial_cylinder_takes_its_radius() {
    let drum = cylinder(5.0, 10.0);
    let wall = wall_of(&drum);

    let wider = replace_faces_onto_round(&drum, &[wall], &round_surface(7.0, 0.0)).unwrap();
    let narrower = replace_faces_onto_round(&drum, &[wall], &round_surface(3.0, 0.0)).unwrap();

    check("wider", &wider, PI * 49.0 * 10.0);
    check("narrower", &narrower, PI * 9.0 * 10.0);
    assert_eq!(names(&wider), names(&drum));
}

#[test]
fn a_bore_onto_a_coaxial_cylinder_takes_its_radius() {
    let base = cuboid(Vector3::new(20.0, 20.0, 10.0));
    let bore = cylinder(3.0, 30.0)
        .transformed(&RigidTransform::translation(Vector3::new(10.0, 10.0, -5.0)).unwrap())
        .unwrap();
    let plate = boolean(&base, &bore, BooleanOperation::Difference).unwrap();
    let wall = wall_of(&plate);
    let frame = Plane::from_frame(Point3::new(10.0, 10.0, 0.0), Vector3::Z, Vector3::X).unwrap();
    let target: Surface = crate::surface::Cylinder::new(frame, 5.0).unwrap().into();

    let result = replace_faces_onto_round(&plate, &[wall], &target).unwrap();

    check("result", &result, 4000.0 - PI * 25.0 * 10.0);
}

#[test]
fn a_drum_s_wall_onto_a_parallel_cylinder_of_its_radius_shifts_it() {
    let base = cuboid(Vector3::new(20.0, 20.0, 5.0));
    let boss = cylinder(3.0, 9.0)
        .transformed(&RigidTransform::translation(Vector3::new(10.0, 10.0, 0.0)).unwrap())
        .unwrap();
    let body = boolean(&base, &boss, BooleanOperation::Union).unwrap();
    let wall = wall_of(&body);
    let frame =
        Plane::from_frame(Point3::new(13.0, 10.0, 0.0), Vector3::NEG_Z, Vector3::X).unwrap();
    let target: Surface = crate::surface::Cylinder::new(frame, 3.0).unwrap().into();

    let result = replace_faces_onto_round(&body, &[wall], &target).unwrap();

    check("result", &result, 2000.0 + PI * 9.0 * 4.0);
}

#[test]
fn round_targets_refuse_what_no_motion_can_hold() {
    let drum = cylinder(5.0, 10.0);
    let wall = wall_of(&drum);
    let top = face_facing(&drum, Vector3::Z, Point3::new(0.0, 0.0, 10.0));

    assert_eq!(
        replace_faces_onto_round(&drum, &[wall], &round_surface(7.0, 3.0)),
        Err(ReplaceError::OffAxis(wall))
    );
    assert_eq!(
        replace_faces_onto_round(&drum, &[top], &round_surface(7.0, 0.0)),
        Err(ReplaceError::FlatOntoRound(top))
    );
    assert_eq!(
        replace_faces_onto_round(&drum, &[wall], &round_surface(5.0, 0.0)),
        Err(ReplaceError::AlreadyOnTarget)
    );
}

#[test]
fn the_body_of_a_levelled_step_can_be_blended_along_its_joined_edges() {
    let step = stepped();
    let lower = face_facing(&step, Vector3::Y, Point3::new(15.0, 5.0, 5.0));

    let level = replaced(&step, &[lower], &plane((0.0, 10.0, 0.0), Vector3::Y));
    let joined_edge = level
        .edges()
        .find(|(_, edge)| {
            let middle = edge.curve().point(edge.interval().middle());
            (middle.y - 10.0).abs() < 1e-9
                && (middle.z - 10.0).abs() < 1e-9
                && (middle.x - 10.0).abs() < 1e-9
        })
        .map(|(id, _)| id)
        .unwrap();

    let rounded = blend(
        &level,
        &[joined_edge],
        BlendShape::Fillet { radius: 1.0 },
        9,
    )
    .unwrap();

    assert_eq!(rounded.faces().count(), 7);
}

#[test]
fn replacing_can_be_cancelled() {
    let block = cuboid(Vector3::splat(10.0));
    let top = block_top(&block);
    let target = plane((0.0, 0.0, 14.0), Vector3::new(0.1, 0.0, 1.0));

    let step = stepped();
    let lower = face_facing(&step, Vector3::Y, Point3::new(15.0, 5.0, 5.0));
    let level = plane((0.0, 10.0, 0.0), Vector3::Y);
    let drum = cylinder(5.0, 10.0);
    let wall = wall_of(&drum);
    let wider = round_surface(7.0, 0.0);

    assert_cancelled_anywhere(
        "replace",
        || replace_faces(&block, &[top], &target),
        |error| matches!(error, ReplaceError::Move(MoveError::Cancelled(_))),
    );
    assert_cancelled_anywhere(
        "level",
        || replace_faces(&step, &[lower], &level),
        |error| matches!(error, ReplaceError::Move(MoveError::Cancelled(_))),
    );
    assert_cancelled_anywhere(
        "round",
        || replace_faces_onto_round(&drum, &[wall], &wider),
        |error| matches!(error, ReplaceError::Move(MoveError::Cancelled(_))),
    );
}
