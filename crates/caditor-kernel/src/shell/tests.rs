use std::f64::consts::PI;

use caditor_geometry::{Plane, Point2, Point3, RigidTransform, Vector2, Vector3};

use super::*;
use crate::{
    blend::{BlendShape, blend},
    build::{AngularExtent, Axis2, LinearExtent, extrude, revolve},
    fixtures::{cuboid, cylinder},
    profile::{Profile, ProfileCurve, Selection},
    test_support::{arc, assert_watertight, line},
    tolerance::SamplingTolerance,
};

fn polygon(points: &[(f64, f64)]) -> Vec<ProfileCurve> {
    (0..points.len())
        .map(|index| {
            line(
                index as u64 + 1,
                points[index],
                points[(index + 1) % points.len()],
            )
        })
        .collect()
}

fn swept(curves: &[ProfileCurve], height: f64) -> Solid {
    let regions = Profile::new(curves)
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap();
    extrude(
        &Plane::XY,
        &regions,
        LinearExtent::one_side(height).unwrap(),
        1,
    )
    .unwrap()
}

fn volume(solid: &Solid) -> f64 {
    solid
        .tessellate(&SamplingTolerance::new(1e-3, 0.05).unwrap())
        .unwrap()
        .mass_properties()
        .volume
}

fn check(name: &str, solid: &Solid, expected: f64) {
    assert_eq!(solid.validate(), Ok(()), "{name}");
    assert_watertight(name, &solid.tessellate(&solid.default_tolerance()).unwrap());
    let found = volume(solid);
    assert!(
        (found - expected).abs() <= 2e-3 * expected.abs().max(1.0),
        "{name}: volume {found} instead of {expected}"
    );
}

fn face_facing(solid: &Solid, normal: Vector3, through: Point3) -> FaceId {
    solid
        .faces()
        .find(|(_, face)| match face.surface() {
            Surface::Plane(plane) => {
                let outward = plane.frame().normal() * face.sense().sign();
                outward.dot(normal) > 0.999 && plane.frame().signed_distance(through).abs() < 1e-9
            }
            _ => false,
        })
        .map(|(id, _)| id)
        .expect("the face exists")
}

fn run(solid: &Solid, open: &[FaceId], thickness: f64) -> Solid {
    match shell(solid, open, thickness, 70) {
        Ok(result) => result,
        Err(error) => panic!("shell failed: {error}"),
    }
}

#[test]
fn an_open_box_keeps_walls_of_the_thickness_and_names_its_inner_faces() {
    let solid = cuboid(Vector3::splat(10.0));
    let top = face_facing(&solid, Vector3::Z, Point3::new(5.0, 5.0, 10.0));
    let result = run(&solid, &[top], 1.0);
    check("open box", &result, 1000.0 - 8.0 * 8.0 * 9.0);
    assert_eq!(result.faces().count(), 11);
    assert_eq!(result.shells().count(), 1);
    let bottom = face_facing(&solid, Vector3::NEG_Z, Point3::ZERO);
    let bottom_name = solid.face(bottom).unwrap().name();
    let inner = result
        .faces()
        .find(|(_, face)| face.name() == FaceName::shell(70, bottom_name))
        .expect("the floor of the cavity is named after the bottom");
    assert_eq!(inner.1.origin(), Some(FaceOrigin::Shell { feature: 70 }));

    let closed = run(&solid, &[], 1.0);
    check("hollow box", &closed, 1000.0 - 512.0);
    assert_eq!(closed.shells().count(), 2);
}

#[test]
fn curved_walls_are_offset_around_their_axis() {
    let cup = cylinder(5.0, 10.0);
    let top = face_facing(&cup, Vector3::Z, Point3::new(0.0, 0.0, 10.0));
    check("cup", &run(&cup, &[top], 1.0), 250.0 * PI - 16.0 * PI * 9.0);

    let plate = cuboid(Vector3::new(10.0, 10.0, 4.0));
    let transform = RigidTransform::translation(Vector3::new(5.0, 5.0, -1.0)).unwrap();
    let drill = cylinder(2.0, 6.0).transformed(&transform).unwrap();
    let holed = boolean(&plate, &drill, BooleanOperation::Difference).unwrap();
    let top = face_facing(&holed, Vector3::Z, Point3::new(1.0, 1.0, 4.0));
    let outer = (100.0 - 4.0 * PI) * 4.0;
    let cavity = (81.0 - 6.25 * PI) * 3.5;
    check("holed plate", &run(&holed, &[top], 0.5), outer - cavity);
}

#[test]
fn prisms_with_inside_corners_and_smooth_sides_are_shelled() {
    let l_shape = swept(
        &polygon(&[
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 4.0),
            (4.0, 4.0),
            (4.0, 10.0),
            (0.0, 10.0),
        ]),
        5.0,
    );
    let top = face_facing(&l_shape, Vector3::Z, Point3::new(1.0, 1.0, 5.0));
    let cavity_area = 8.0 * 2.0 + 2.0 * 6.0;
    check(
        "l shape",
        &run(&l_shape, &[top], 1.0),
        64.0 * 5.0 - cavity_area * 4.0,
    );

    let slot = swept(
        &[
            line(1, (0.0, 0.0), (10.0, 0.0)),
            arc(2, (10.0, 2.0), (10.0, 0.0), (10.0, 4.0)),
            line(3, (10.0, 4.0), (0.0, 4.0)),
            arc(4, (0.0, 2.0), (0.0, 4.0), (0.0, 0.0)),
        ],
        3.0,
    );
    let top = face_facing(&slot, Vector3::Z, Point3::new(5.0, 2.0, 3.0));
    let outer = 40.0 + 4.0 * PI;
    let cavity = 10.0 * 2.0 + PI;
    check("slot", &run(&slot, &[top], 1.0), 3.0 * outer - 2.0 * cavity);
}

#[test]
fn rounded_and_turned_bodies_are_shelled() {
    let solid = cuboid(Vector3::splat(10.0));
    let every: Vec<EdgeId> = solid.edges().map(|(id, _)| id).collect();
    let rounded = blend(&solid, &every, BlendShape::Fillet { radius: 2.0 }, 5).unwrap();
    let top = face_facing(&rounded, Vector3::Z, Point3::new(5.0, 5.0, 10.0));
    let result = run(&rounded, &[top], 0.5);
    let steiner = |side: f64, radius: f64| {
        let flat = side - 2.0 * radius;
        flat.powi(3)
            + 6.0 * radius * flat * flat
            + 3.0 * PI * radius * radius * flat
            + 4.0 / 3.0 * PI * radius.powi(3)
    };
    let lip = 6.0 * 6.0 * 0.5;
    check(
        "rounded box",
        &result,
        steiner(10.0, 2.0) - steiner(9.0, 1.5) - lip,
    );

    let regions = Profile::new(&polygon(&[
        (0.0, 0.0),
        (5.0, 0.0),
        (5.0, 4.0),
        (3.0, 4.0),
        (3.0, 10.0),
        (0.0, 10.0),
    ]))
    .unwrap()
    .select(&Selection::EvenDepth)
    .unwrap();
    let shaft = revolve(
        &Plane::XZ,
        &regions,
        Axis2::new(Point2::ZERO, Vector2::Y).unwrap(),
        AngularExtent::full(),
        1,
    )
    .unwrap();
    let end = face_facing(&shaft, Vector3::Z, Point3::new(0.0, 0.0, 10.0));
    let outer = PI * (25.0 * 4.0 + 9.0 * 6.0);
    let cavity = PI * (16.0 * 2.0 + 4.0 * 7.0);
    check("shaft", &run(&shaft, &[end], 1.0), outer - cavity);
}

#[test]
fn opened_faces_move_outward_so_the_body_only_needs_room_across_its_walls() {
    let plate = cuboid(Vector3::new(10.0, 10.0, 2.0));
    let top = face_facing(&plate, Vector3::Z, Point3::new(5.0, 5.0, 2.0));
    let tray = run(&plate, &[top], 1.5);
    check("thin tray", &tray, 200.0 - 7.0 * 7.0 * 0.5);
    assert_eq!(tray.faces().count(), 11);
    assert_eq!(shell(&plate, &[top], 2.5, 1), Err(ShellError::TooThick));

    let solid = cuboid(Vector3::splat(10.0));
    let top = face_facing(&solid, Vector3::Z, Point3::new(5.0, 5.0, 10.0));
    let bottom = face_facing(&solid, Vector3::NEG_Z, Point3::ZERO);
    let front = face_facing(&solid, Vector3::NEG_Y, Point3::ZERO);
    check(
        "tube",
        &run(&solid, &[top, bottom], 1.0),
        1000.0 - 8.0 * 8.0 * 10.0,
    );
    check(
        "open corner",
        &run(&solid, &[top, front], 1.0),
        1000.0 - 8.0 * 9.0 * 9.0,
    );
}

#[test]
fn walls_thicker_than_the_body_are_refused() {
    let solid = cuboid(Vector3::splat(10.0));
    let top = face_facing(&solid, Vector3::Z, Point3::new(5.0, 5.0, 10.0));
    assert_eq!(shell(&solid, &[top], 6.0, 1), Err(ShellError::TooThick));
    assert_eq!(
        shell(&solid, &[top], 0.0, 1),
        Err(ShellError::InvalidThickness)
    );
}
