use std::{collections::BTreeMap, f64::consts::PI};

use caditor_geometry::{Point3, Vector3};

use super::{
    tests::{check, face_facing, swept},
    *,
};
use crate::{
    fixtures::{cuboid, cylinder, hollow_cuboid},
    test_support::{arc, line},
};

fn walls(thickness: f64, side: ShellSide, faces: &[(FaceId, f64)]) -> Walls {
    Walls {
        thickness,
        side,
        faces: faces.iter().copied().collect::<BTreeMap<_, _>>(),
    }
}

fn run(solid: &Solid, open: &[FaceId], walls: &Walls) -> Solid {
    match shell_walls(solid, open, walls, 70) {
        Ok(result) => result,
        Err(error) => panic!("shell failed: {error}"),
    }
}

#[test]
fn a_box_shelled_outside_keeps_its_own_faces_as_the_cavity() {
    let solid = cuboid(Vector3::splat(10.0));
    let top = face_facing(&solid, Vector3::Z, Point3::new(5.0, 5.0, 10.0));

    let outside = run(&solid, &[top], &walls(1.0, ShellSide::Outside, &[]));
    let both = run(&solid, &[top], &walls(1.0, ShellSide::BothSides, &[]));
    let closed = run(&solid, &[], &walls(1.0, ShellSide::Outside, &[]));

    check("outside", &outside, 12.0 * 12.0 * 11.0 - 1000.0);
    check("both sides", &both, 11.0 * 11.0 * 10.5 - 9.0 * 9.0 * 9.5);
    check("closed outside", &closed, 12.0 * 12.0 * 12.0 - 1000.0);
    assert_eq!(outside.shells().count(), 1);
    assert_eq!(closed.shells().count(), 2);

    let bottom = face_facing(&solid, Vector3::NEG_Z, Point3::ZERO);
    let bottom_name = solid.face(bottom).unwrap().name();
    let names: Vec<FaceName> = outside.faces().map(|(_, face)| face.name()).collect();
    assert!(names.contains(&bottom_name));
    assert!(names.contains(&FaceName::shell(70, bottom_name)));
}

#[test]
fn a_cylinder_shelled_outside_or_both_ways_grows_its_radius() {
    let cup = cylinder(5.0, 10.0);
    let top = face_facing(&cup, Vector3::Z, Point3::new(0.0, 0.0, 10.0));

    let outside = run(&cup, &[top], &walls(1.0, ShellSide::Outside, &[]));
    let both = run(&cup, &[top], &walls(1.0, ShellSide::BothSides, &[]));

    check("outside", &outside, 36.0 * PI * 11.0 - 25.0 * PI * 10.0);
    check(
        "both sides",
        &both,
        5.5 * 5.5 * PI * 10.5 - 4.5 * 4.5 * PI * 9.5,
    );
}

#[test]
fn faces_given_their_own_thickness_keep_walls_of_it() {
    let solid = cuboid(Vector3::splat(10.0));
    let top = face_facing(&solid, Vector3::Z, Point3::new(5.0, 5.0, 10.0));
    let bottom = face_facing(&solid, Vector3::NEG_Z, Point3::ZERO);
    let right = face_facing(&solid, Vector3::X, Point3::new(10.0, 5.0, 5.0));

    let thick_floor = run(
        &solid,
        &[top],
        &walls(1.0, ShellSide::Inside, &[(bottom, 3.0)]),
    );
    let two_walls = run(
        &solid,
        &[top],
        &walls(1.0, ShellSide::Inside, &[(bottom, 3.0), (right, 2.0)]),
    );
    let thin_floor_outside = run(
        &solid,
        &[top],
        &walls(2.0, ShellSide::Outside, &[(bottom, 0.5)]),
    );

    check("thick floor", &thick_floor, 1000.0 - 8.0 * 8.0 * 7.0);
    check("two walls", &two_walls, 1000.0 - 7.0 * 8.0 * 7.0);
    let thick_floor_both = run(
        &solid,
        &[top],
        &walls(1.0, ShellSide::BothSides, &[(bottom, 3.0)]),
    );

    check(
        "thin floor outside",
        &thin_floor_outside,
        14.0 * 14.0 * 10.5 - 1000.0,
    );
    check(
        "thick floor both sides",
        &thick_floor_both,
        11.0 * 11.0 * 11.5 - 9.0 * 9.0 * 8.5,
    );
}

#[test]
fn a_void_opened_outside_loses_the_floor_of_its_lining() {
    let hollow = hollow_cuboid(10.0, 4.0);
    let floor = face_facing(&hollow, Vector3::Z, Point3::new(5.0, 5.0, 3.0));

    let opened = run(&hollow, &[floor], &walls(1.0, ShellSide::Outside, &[]));

    let skin = 12.0 * 12.0 * 12.0 - 1000.0;
    let lining = 4.0 * 4.0 * 4.0 - 2.0 * 2.0 * 2.0 - 4.0 * 4.0;
    check("void opened outside", &opened, skin + lining);
    assert_eq!(opened.shells().count(), 3);
}

#[test]
fn a_cylinder_s_own_thickness_sets_its_wall_alone() {
    let cup = cylinder(5.0, 10.0);
    let top = face_facing(&cup, Vector3::Z, Point3::new(0.0, 0.0, 10.0));
    let side = cup
        .faces()
        .find(|(_, face)| matches!(face.surface(), Surface::Cylinder(_)))
        .map(|(id, _)| id)
        .unwrap();

    let thick_side = run(&cup, &[top], &walls(1.0, ShellSide::Inside, &[(side, 2.0)]));

    check("thick side", &thick_side, 25.0 * PI * 10.0 - 9.0 * PI * 9.0);
}

fn arch() -> Solid {
    swept(
        &[
            line(1, (0.0, 0.0), (10.0, 0.0)),
            line(2, (10.0, 0.0), (10.0, 8.0)),
            arc(3, (5.0, 3.0), (10.0, 8.0), (0.0, 8.0)),
            line(4, (0.0, 8.0), (0.0, 0.0)),
        ],
        6.0,
    )
}

fn under_circle(radius: f64, from: f64, to: f64) -> f64 {
    let primitive = |u: f64| {
        let root = (radius * radius - u * u).sqrt();
        0.5 * (u * root + radius * radius * (u / radius).asin())
    };
    primitive(to) - primitive(from)
}

#[test]
fn a_curved_face_parted_from_its_walls_by_sharp_edges_opens() {
    let solid = arch();
    let roof = solid
        .faces()
        .find(|(_, face)| matches!(face.surface(), Surface::Cylinder(_)))
        .map(|(id, _)| id)
        .unwrap();
    let radius = 50f64.sqrt();

    let opened = run(&solid, &[roof], &walls(1.0, ShellSide::Inside, &[]));

    let section = 30.0 + under_circle(radius, -5.0, 5.0);
    let cavity_section = 16.0 + under_circle(radius, -4.0, 4.0);
    check("open roof", &opened, section * 6.0 - cavity_section * 4.0);

    let drum = cylinder(5.0, 10.0);
    let side = drum
        .faces()
        .find(|(_, face)| matches!(face.surface(), Surface::Cylinder(_)))
        .map(|(id, _)| id)
        .unwrap();
    let ends = run(&drum, &[side], &walls(1.0, ShellSide::Inside, &[]));
    check("drum ends", &ends, 2.0 * 25.0 * PI);
}

#[test]
fn refusals_name_the_face_at_fault() {
    let solid = cuboid(Vector3::splat(10.0));
    let top = face_facing(&solid, Vector3::Z, Point3::new(5.0, 5.0, 10.0));
    let bottom = face_facing(&solid, Vector3::NEG_Z, Point3::ZERO);
    let slot = swept(
        &[
            line(1, (0.0, 0.0), (10.0, 0.0)),
            arc(2, (10.0, 2.0), (10.0, 0.0), (10.0, 4.0)),
            line(3, (10.0, 4.0), (0.0, 4.0)),
            arc(4, (0.0, 2.0), (0.0, 4.0), (0.0, 0.0)),
        ],
        5.0,
    );
    let round_end = slot
        .faces()
        .find(|(_, face)| matches!(face.surface(), Surface::Cylinder(_)))
        .map(|(id, _)| id)
        .unwrap();

    assert_eq!(
        shell_walls(
            &solid,
            &[top],
            &walls(1.0, ShellSide::Inside, &[(top, 2.0)]),
            1
        ),
        Err(ShellError::OpenedFaceThickness(top))
    );
    assert_eq!(
        shell_walls(
            &solid,
            &[top],
            &walls(1.0, ShellSide::Inside, &[(bottom, 0.0)]),
            1
        ),
        Err(ShellError::InvalidFaceThickness(bottom))
    );
    assert_eq!(
        shell_walls(&slot, &[round_end], &walls(1.0, ShellSide::Inside, &[]), 1),
        Err(ShellError::CurvedOpening(round_end))
    );
}
