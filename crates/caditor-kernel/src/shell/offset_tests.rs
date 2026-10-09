use std::collections::BTreeSet;

use caditor_geometry::{Point3, Vector3};

use super::{
    OffsetError, offset_faces,
    tests::{check, face_facing, polygon, swept, volume},
};
use crate::{
    blend::{BlendShape, blend, tangent_faces},
    boolean::{BooleanOperation, boolean},
    fixtures::{cuboid, cylinder, frustum, holed_block, spline_topped_block},
    naming::FaceName,
    surface::Surface,
    topology::{EdgeId, FaceId, Solid},
};

fn names(solid: &Solid) -> BTreeSet<FaceName> {
    solid.faces().map(|(_, face)| face.name()).collect()
}

fn moved(solid: &Solid, faces: &[FaceId], distance: f64) -> Solid {
    match offset_faces(solid, faces, distance) {
        Ok(result) => result,
        Err(error) => panic!("offset failed: {error}"),
    }
}

#[test]
fn a_face_moves_along_its_normal_and_the_neighbours_follow() {
    let block = cuboid(Vector3::splat(10.0));
    let top = face_facing(&block, Vector3::Z, Point3::new(5.0, 5.0, 10.0));

    let taller = moved(&block, &[top], 3.0);
    check("taller", &taller, 1300.0);
    assert_eq!(taller.faces().count(), 6);
    assert_eq!(names(&taller), names(&block));

    let shorter = moved(&block, &[top], -4.0);
    check("shorter", &shorter, 600.0);
    assert_eq!(names(&shorter), names(&block));
}

#[test]
fn several_faces_move_together_by_the_same_distance() {
    let block = cuboid(Vector3::splat(10.0));
    let top = face_facing(&block, Vector3::Z, Point3::new(5.0, 5.0, 10.0));
    let right = face_facing(&block, Vector3::X, Point3::new(10.0, 5.0, 5.0));

    check(
        "both",
        &moved(&block, &[top, right], 2.0),
        12.0 * 10.0 * 12.0,
    );
}

#[test]
fn a_round_wall_changes_radius_about_its_axis() {
    let post = cylinder(5.0, 10.0);
    let wall = post
        .faces()
        .find(|(_, face)| matches!(face.surface(), Surface::Cylinder(_)))
        .map(|(id, _)| id)
        .unwrap();

    let wider = moved(&post, &[wall], 1.0);
    check("wider", &wider, 36.0 * std::f64::consts::PI * 10.0);
    assert_eq!(names(&wider), names(&post));

    let thinner = moved(&post, &[wall], -2.0);
    check("thinner", &thinner, 9.0 * std::f64::consts::PI * 10.0);
}

#[test]
fn a_bore_widens_when_its_wall_moves_away_from_the_material() {
    let plate = holed_block(20.0, 6.0, 3.0);
    let bore = plate
        .faces()
        .find(|(_, face)| matches!(face.surface(), Surface::Cylinder(_)))
        .map(|(id, _)| id)
        .unwrap();

    let narrower = moved(&plate, &[bore], 1.0);
    let expected = 20.0 * 20.0 * 6.0 - 4.0 * std::f64::consts::PI * 6.0;
    check("narrower", &narrower, expected);

    let wider = moved(&plate, &[bore], -1.0);
    check(
        "wider",
        &wider,
        20.0 * 20.0 * 6.0 - 16.0 * std::f64::consts::PI * 6.0,
    );
}

#[test]
fn a_boss_grows_taller_and_a_step_moves_its_neighbours() {
    let base = swept(
        &polygon(&[(0.0, 0.0), (20.0, 0.0), (20.0, 20.0), (0.0, 20.0)]),
        5.0,
    );
    let boss = swept(
        &polygon(&[(5.0, 5.0), (10.0, 5.0), (10.0, 10.0), (5.0, 10.0)]),
        8.0,
    );
    let body = boolean(&base, &boss, BooleanOperation::Union).unwrap();
    let top = face_facing(&body, Vector3::Z, Point3::new(7.0, 7.0, 8.0));

    check(
        "taller boss",
        &moved(&body, &[top], 2.0),
        2000.0 + 25.0 * 5.0,
    );
    check(
        "shorter boss",
        &moved(&body, &[top], -2.0),
        2000.0 + 25.0 * 1.0,
    );
}

#[test]
fn nothing_to_move_or_no_distance_is_refused() {
    let block = cuboid(Vector3::splat(10.0));
    let top = face_facing(&block, Vector3::Z, Point3::new(5.0, 5.0, 10.0));

    assert_eq!(
        offset_faces(&block, &[top], 0.0),
        Err(OffsetError::InvalidDistance)
    );
    assert_eq!(
        offset_faces(&block, &[top], f64::NAN),
        Err(OffsetError::InvalidDistance)
    );
    assert_eq!(
        offset_faces(&block, &[], 1.0),
        Err(OffsetError::NothingToMove)
    );
}

#[test]
fn pushing_a_face_through_the_opposite_one_is_refused() {
    let block = cuboid(Vector3::splat(10.0));
    let top = face_facing(&block, Vector3::Z, Point3::new(5.0, 5.0, 10.0));

    assert!(offset_faces(&block, &[top], -12.0).is_err());
    assert!(offset_faces(&block, &[top], -10.0).is_err());
}

#[test]
fn a_tooth_pushed_into_another_part_of_the_body_is_refused() {
    let hook = swept(
        &polygon(&[
            (0.0, 0.0),
            (35.0, 0.0),
            (35.0, 5.0),
            (5.0, 5.0),
            (5.0, 25.0),
            (25.0, 25.0),
            (25.0, 20.0),
            (30.0, 20.0),
            (30.0, 25.0),
            (35.0, 25.0),
            (35.0, 30.0),
            (0.0, 30.0),
        ]),
        6.0,
    );
    let tip = face_facing(&hook, Vector3::NEG_Y, Point3::new(27.0, 20.0, 3.0));
    let area = 35.0 * 5.0 + 5.0 * 20.0 + 35.0 * 5.0 + 5.0 * 5.0;

    check(
        "longer tooth",
        &moved(&hook, &[tip], 10.0),
        (area + 50.0) * 6.0,
    );
    assert!(matches!(
        offset_faces(&hook, &[tip], 18.0),
        Err(OffsetError::Crosses { .. } | OffsetError::Invalid { .. })
    ));
}

#[test]
fn a_wall_pushed_across_the_gap_it_borders_is_refused() {
    let arms = swept(
        &polygon(&[
            (0.0, 0.0),
            (30.0, 0.0),
            (30.0, 20.0),
            (20.0, 20.0),
            (20.0, 5.0),
            (10.0, 5.0),
            (10.0, 20.0),
            (0.0, 20.0),
        ]),
        6.0,
    );
    let inner = face_facing(&arms, Vector3::NEG_X, Point3::new(20.0, 10.0, 3.0));

    check("narrowed gap", &moved(&arms, &[inner], 4.0), 3060.0);
    assert!(matches!(
        offset_faces(&arms, &[inner], 12.0),
        Err(OffsetError::EdgeCollapses(_))
    ));
}

#[test]
fn a_missing_face_is_named() {
    let block = cuboid(Vector3::splat(10.0));
    let other = holed_block(20.0, 6.0, 3.0);
    let stray = other.faces().map(|(id, _)| id).max().unwrap();
    assert_eq!(
        offset_faces(&block, &[stray], 1.0),
        Err(OffsetError::MissingFace(stray))
    );
}

#[test]
fn a_slanted_neighbour_extends_along_its_own_slope() {
    let cone = frustum(5.0, 3.0, 10.0);
    let top = face_facing(&cone, Vector3::Z, Point3::new(0.0, 0.0, 10.0));
    let slope = -0.2;
    let volume = |height: f64| {
        let top_radius = 3.0 + slope * (height - 10.0);
        std::f64::consts::PI / 3.0 * height * (25.0 + 5.0 * top_radius + top_radius * top_radius)
    };

    check("taller", &moved(&cone, &[top], 4.0), volume(14.0));
    check("shorter", &moved(&cone, &[top], -4.0), volume(6.0));
}

#[test]
fn faces_beside_a_curved_one_are_left_as_they_were() {
    let block = spline_topped_block(10.0, 10.0, 2.0);
    let bottom = face_facing(&block, Vector3::NEG_Z, Point3::ZERO);
    let before = volume(&block);

    let taller = moved(&block, &[bottom], 3.0);
    check("taller", &taller, before + 300.0);
    assert_eq!(names(&taller), names(&block));
}

#[test]
fn a_chamfer_beside_the_moved_face_extends_with_it() {
    let block = cuboid(Vector3::splat(10.0));
    let rim = block
        .edges()
        .find(|(_, edge)| {
            let middle = edge.curve().point(edge.interval().middle());
            (middle - Point3::new(5.0, 0.0, 10.0)).length() < 1e-9
        })
        .map(|(id, _)| id)
        .unwrap();
    let chamfered = blend(&block, &[rim], BlendShape::Chamfer { distance: 2.0 }, 5).unwrap();
    let top = face_facing(&chamfered, Vector3::Z, Point3::new(5.0, 5.0, 10.0));

    let result = moved(&chamfered, &[top], -1.0);
    check("lowered", &result, 895.0);
    assert_eq!(names(&result), names(&chamfered));
}

#[test]
fn a_rounded_body_offsets_as_one_chain_of_tangent_faces() {
    let block = cuboid(Vector3::splat(10.0));
    let every: Vec<EdgeId> = block.edges().map(|(id, _)| id).collect();
    let rounded = blend(&block, &every, BlendShape::Fillet { radius: 2.0 }, 5).unwrap();
    let top = face_facing(&rounded, Vector3::Z, Point3::new(5.0, 5.0, 10.0));
    let chain = tangent_faces(&rounded, &[top]);
    let steiner = |side: f64, radius: f64| {
        let flat = side - 2.0 * radius;
        flat.powi(3)
            + 6.0 * radius * flat * flat
            + 3.0 * std::f64::consts::PI * radius * radius * flat
            + 4.0 / 3.0 * std::f64::consts::PI * radius.powi(3)
    };

    let grown = moved(&rounded, &chain, 1.0);
    check("grown", &grown, steiner(12.0, 3.0));
    assert_eq!(names(&grown), names(&rounded));
    check("shrunk", &moved(&rounded, &chain, -1.0), steiner(8.0, 1.0));
}

#[test]
fn a_fillet_beside_a_moved_face_that_stays_is_refused_in_words() {
    let block = cuboid(Vector3::splat(10.0));
    let every: Vec<EdgeId> = block.edges().map(|(id, _)| id).collect();
    let rounded = blend(&block, &every, BlendShape::Fillet { radius: 2.0 }, 5).unwrap();
    let top = face_facing(&rounded, Vector3::Z, Point3::new(5.0, 5.0, 10.0));

    assert!(offset_faces(&rounded, &[top], 1.0).is_err());
}

#[test]
fn a_rim_rounded_on_top_grows_with_its_fillets_and_walls() {
    let block = cuboid(Vector3::splat(10.0));
    let rim: Vec<EdgeId> = block
        .edges()
        .filter(|(_, edge)| {
            let start = edge.curve().point(edge.interval().start());
            let end = edge.curve().point(edge.interval().end());
            (start.z - 10.0).abs() < 1e-9 && (end.z - 10.0).abs() < 1e-9
        })
        .map(|(id, _)| id)
        .collect();
    let rounded = blend(&block, &rim, BlendShape::Fillet { radius: 2.0 }, 5).unwrap();
    let top = face_facing(&rounded, Vector3::Z, Point3::new(5.0, 5.0, 10.0));
    let chain = tangent_faces(&rounded, &[top]);
    let rounded_box = |side: f64, height: f64, radius: f64| {
        let corner = 0.09375 * radius.powi(3);
        side * side * height - (1.0 - std::f64::consts::FRAC_PI_4) * radius * radius * 4.0 * side
            + 4.0 * corner
    };

    check(
        "grown",
        &moved(&rounded, &chain, 1.0),
        rounded_box(12.0, 11.0, 3.0),
    );
    check(
        "shrunk",
        &moved(&rounded, &chain, -1.0),
        rounded_box(8.0, 9.0, 1.0),
    );
}
