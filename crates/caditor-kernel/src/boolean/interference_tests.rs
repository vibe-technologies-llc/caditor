use caditor_geometry::{RigidTransform, Vector3};

use super::{BooleanError, Interference, interference};
use crate::{
    fixtures::{cuboid, cylinder, sphere, torus},
    test_support::assert_cancelled_anywhere,
    tolerance::SamplingTolerance,
    topology::Solid,
};

fn moved(solid: Solid, offset: (f64, f64, f64)) -> Solid {
    let transform =
        RigidTransform::translation(Vector3::new(offset.0, offset.1, offset.2)).unwrap();
    solid.transformed(&transform).unwrap()
}

fn block(min: (f64, f64, f64), max: (f64, f64, f64)) -> Solid {
    moved(
        cuboid(Vector3::new(max.0 - min.0, max.1 - min.1, max.2 - min.2)),
        min,
    )
}

fn volume(solid: &Solid) -> f64 {
    solid
        .tessellate(&SamplingTolerance::new(1e-3, 0.1).unwrap())
        .unwrap()
        .mass_properties()
        .volume
}

fn found(first: &Solid, second: &Solid) -> Interference {
    match interference(first, second) {
        Ok(found) => found,
        Err(error) => panic!("checking interference failed: {error}"),
    }
}

fn touch_point(found: &Interference) -> caditor_geometry::Point3 {
    match found {
        Interference::Touching(point) => *point,
        other => panic!("expected the solids to touch, found {other:?}"),
    }
}

#[test]
fn overlapping_blocks_give_the_solid_they_share() {
    let first = block((0.0, 0.0, 0.0), (2.0, 2.0, 2.0));
    let second = block((1.0, 1.0, 1.0), (3.0, 3.0, 3.0));

    let Interference::Overlapping(overlap) = found(&first, &second) else {
        panic!("the blocks overlap");
    };

    assert_eq!(overlap.validate(), Ok(()));
    assert!((volume(&overlap) - 1.0).abs() < 1e-6);
}

#[test]
fn a_solid_inside_another_overlaps_it_whole() {
    let outer = block((0.0, 0.0, 0.0), (10.0, 10.0, 10.0));
    let inner = block((4.0, 4.0, 4.0), (5.0, 6.0, 7.0));

    let Interference::Overlapping(overlap) = found(&outer, &inner) else {
        panic!("the inner block lies inside the outer one");
    };

    assert!((volume(&overlap) - 6.0).abs() < 1e-6);
}

#[test]
fn solids_apart_are_apart_whether_or_not_their_boxes_meet() {
    let first = block((0.0, 0.0, 0.0), (2.0, 2.0, 2.0));
    let far = block((5.0, 0.0, 0.0), (6.0, 1.0, 1.0));
    let ball = moved(sphere(1.0), (3.2, 3.2, 3.2));
    let near = block((2.001, 0.0, 0.0), (3.0, 2.0, 2.0));

    assert_eq!(found(&first, &far), Interference::Apart);
    assert_eq!(found(&first, &ball), Interference::Apart);
    assert_eq!(found(&first, &near), Interference::Apart);
}

#[test]
fn blocks_touching_on_a_face_an_edge_or_a_corner_touch() {
    let first = block((0.0, 0.0, 0.0), (2.0, 2.0, 2.0));
    let beside = block((2.0, 0.5, 0.5), (3.0, 1.5, 1.5));
    let along_an_edge = block((2.0, 2.0, 0.0), (3.0, 3.0, 2.0));
    let at_a_corner = block((2.0, 2.0, 2.0), (3.0, 3.0, 3.0));

    let face = touch_point(&found(&first, &beside));
    let edge = touch_point(&found(&first, &along_an_edge));
    let corner = touch_point(&found(&first, &at_a_corner));

    assert!(face.distance(caditor_geometry::Point3::new(2.0, 1.0, 1.0)) < 1e-6);
    assert!((edge.x - 2.0).abs() < 1e-6 && (edge.y - 2.0).abs() < 1e-6);
    assert!(corner.distance(caditor_geometry::Point3::new(2.0, 2.0, 2.0)) < 1e-6);
}

#[test]
fn curved_solids_resting_on_a_block_touch_where_they_rest() {
    let base = block((0.0, 0.0, 0.0), (10.0, 10.0, 2.0));
    let ball = moved(sphere(1.5), (5.0, 5.0, 3.5));
    let post = moved(cylinder(1.0, 3.0), (4.0, 4.0, 2.0));

    let rest = touch_point(&found(&base, &ball));
    let foot = touch_point(&found(&base, &post));

    assert!(rest.distance(caditor_geometry::Point3::new(5.0, 5.0, 2.0)) < 1e-5);
    assert!(foot.distance(caditor_geometry::Point3::new(4.0, 4.0, 2.0)) < 1e-6);
}

#[test]
fn a_cylinder_through_a_block_overlaps_it_by_the_part_inside() {
    let plate = block((0.0, 0.0, 0.0), (10.0, 10.0, 4.0));
    let pin = moved(cylinder(2.0, 6.0), (5.0, 5.0, -1.0));

    let Interference::Overlapping(overlap) = found(&plate, &pin) else {
        panic!("the pin passes through the plate");
    };

    let expected = 16.0 * std::f64::consts::PI;
    assert!((volume(&overlap) - expected).abs() < 1e-2 * expected);
}

#[test]
fn checking_interference_cancelled_at_any_poll_stops_with_cancelled() {
    let first = block((0.0, 0.0, 0.0), (2.0, 2.0, 2.0));
    let overlapping = moved(cylinder(0.5, 3.0), (1.0, 1.0, -0.5));
    let touching = moved(sphere(1.0), (1.0, 1.0, 3.0));

    for (name, second) in [("overlapping", overlapping), ("touching", touching)] {
        assert_cancelled_anywhere(
            name,
            || interference(&first, &second),
            |error| matches!(error, BooleanError::Cancelled(_)),
        );
    }
}

#[test]
fn a_block_pressed_into_a_tilted_torus_between_bound_samples_overlaps_it() {
    let turn = RigidTransform::rotation_about(
        caditor_geometry::Point3::ZERO,
        Vector3::X,
        std::f64::consts::FRAC_PI_4,
    )
    .unwrap();
    let ring = torus(20.0, 5.0).transformed(&turn).unwrap();
    let top = 20.0 * std::f64::consts::FRAC_1_SQRT_2 + 5.0;
    let pressed = block((-1.0, -25.0, top - 0.1), (1.0, 25.0, top + 5.0));

    assert!(matches!(
        found(&ring, &pressed),
        Interference::Overlapping(_)
    ));
}
