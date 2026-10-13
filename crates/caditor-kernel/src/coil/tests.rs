use std::f64::consts::PI;

use caditor_geometry::{Plane, Point3, Vector3};

use super::*;
use crate::{
    boolean::{BooleanOperation, boolean},
    fixtures,
    measure::extent,
    naming::{FaceName, FaceOrigin},
    tolerance::SamplingTolerance,
};

const FEATURE: u64 = 11;

fn upright() -> Plane {
    Plane::from_frame(Point3::ZERO, Vector3::Z, Vector3::X).unwrap()
}

fn spring(shape: CoilShape) -> CoilSpec {
    CoilSpec {
        frame: upright(),
        radius: 10.0,
        shape,
        size: 2.0,
        placement: CoilPlacement::Centred,
        pitch: 4.0,
        turns: 3.0,
        taper: 0.0,
        hand: Handedness::Right,
    }
}

fn volume(solid: &Solid) -> f64 {
    let tolerance = SamplingTolerance::new(1e-3, 0.02).unwrap();
    solid
        .tessellate(&tolerance)
        .unwrap()
        .mass_properties()
        .volume
}

fn assert_volume(name: &str, solid: &Solid, expected: f64) {
    let measured = volume(solid);
    assert!(
        (measured - expected).abs() < 1e-3 * expected,
        "{name}: {measured} mm³, expected {expected} mm³"
    );
}

#[test]
fn a_round_spring_has_the_volume_of_its_wire() {
    let spec = spring(CoilShape::Round);
    let made = coil(&spec, FEATURE).unwrap();

    let expected = PI * 1.0 * TAU * 10.0 * 3.0;

    assert!((spec.volume() - expected).abs() < 1e-9);
    assert_volume("round", &made, expected);
    assert_eq!(made.faces().count(), 3 * 4 + 2);
}

#[test]
fn a_square_coil_sits_on_its_start_plane_inside_its_radius() {
    let spec = CoilSpec {
        placement: CoilPlacement::Inside,
        turns: 2.5,
        ..spring(CoilShape::Square)
    };
    let made = coil(&spec, FEATURE).unwrap();
    let bounds = extent(&made).unwrap().unwrap();

    assert_volume("square", &made, 4.0 * TAU * 9.0 * 2.5);
    assert!(bounds.min().z.abs() < 1e-6, "{bounds:?}");
    assert!((bounds.max().z - (10.0 + 2.0)).abs() < 1e-3, "{bounds:?}");
    assert!(bounds.max().x <= 10.0 + 1e-6, "{bounds:?}");
}

#[test]
fn a_tapered_coil_widens_along_its_axis() {
    let spec = CoilSpec {
        taper: 10.0_f64.to_radians(),
        placement: CoilPlacement::Outside,
        ..spring(CoilShape::Round)
    };
    let made = coil(&spec, FEATURE).unwrap();
    let rise = 12.0 * 10.0_f64.to_radians().tan();

    let expected = PI * TAU * 3.0 * (11.0 + 0.5 * rise);

    assert!((spec.volume() - expected).abs() < 1e-9);
    assert_volume("tapered", &made, expected);
    let square = coil(
        &CoilSpec {
            shape: CoilShape::Square,
            ..spec
        },
        FEATURE,
    )
    .unwrap();
    assert_volume(
        "tapered square",
        &square,
        4.0 * TAU * 3.0 * (11.0 + 0.5 * rise),
    );
}

#[test]
fn hands_turn_opposite_ways() {
    let right = coil(&spring(CoilShape::Square), FEATURE).unwrap();
    let left = coil(
        &CoilSpec {
            hand: Handedness::Left,
            ..spring(CoilShape::Square)
        },
        FEATURE,
    )
    .unwrap();
    let quarter = |solid: &Solid| {
        let bounds = solid.bounding_box().unwrap();
        bounds.min().y + bounds.max().y
    };

    assert!((volume(&right) - volume(&left)).abs() < 1e-3 * volume(&right));
    assert!(quarter(&right) * quarter(&left) < 0.0 || quarter(&right).abs() < 1e-6);
}

#[test]
fn faces_are_named_by_turn_and_side() {
    let made = coil(&spring(CoilShape::Square), FEATURE).unwrap();
    let names: Vec<FaceName> = made.faces().map(|(_, face)| face.name()).collect();

    for turn in 0..3 {
        for side in 0..4 {
            let part = COIL_PARTS.side(turn, side);
            assert!(names.contains(&FaceName::coil(FEATURE, part)));
            assert_eq!(
                CoilFace::of_entity(part),
                CoilFace::SIDES.get(side).copied()
            );
        }
    }
    assert!(names.contains(&FaceName::coil(FEATURE, COIL_PARTS.start)));
    assert!(names.contains(&FaceName::coil(FEATURE, COIL_PARTS.end)));
    assert_eq!(CoilFace::of_entity(COIL_PARTS.end), Some(CoilFace::End));
    assert!(made.faces().all(|(_, face)| matches!(
        face.origin(),
        Some(FaceOrigin::Side {
            feature: FEATURE,
            ..
        })
    )));
}

#[test]
fn a_coil_cuts_a_helical_groove_into_a_shaft() {
    let shaft = fixtures::cylinder(10.0, 20.0);
    let spec = CoilSpec {
        frame: Plane::from_frame(Point3::new(0.0, 0.0, 2.0), Vector3::Z, Vector3::X).unwrap(),
        turns: 3.0,
        ..spring(CoilShape::Round)
    };
    let tool = coil(&spec, FEATURE).unwrap();
    let cut = boolean(&shaft, &tool, BooleanOperation::Difference).unwrap();

    let removed = 0.5 * PI * TAU * 3.0 * (10.0 - 4.0 / (3.0 * PI));

    assert_volume("grooved", &cut, PI * 100.0 * 20.0 - removed);
}

#[test]
fn refusals_name_what_is_wrong() {
    let close = CoilSpec {
        pitch: 2.0,
        ..spring(CoilShape::Round)
    };
    let to_axis = CoilSpec {
        radius: 1.0,
        placement: CoilPlacement::Inside,
        ..spring(CoilShape::Round)
    };
    let shrinking = CoilSpec {
        taper: -40.0_f64.to_radians(),
        ..spring(CoilShape::Square)
    };

    assert!(matches!(
        coil(&close, FEATURE),
        Err(CoilError::TurnsOverlap { least, .. }) if (least - 2.02).abs() < 1e-9
    ));
    assert_eq!(coil(&to_axis, FEATURE), Err(CoilError::ReachesAxis));
    assert_eq!(coil(&shrinking, FEATURE), Err(CoilError::ReachesAxis));
    assert_eq!(
        coil(
            &CoilSpec {
                taper: 85.0_f64.to_radians(),
                ..spring(CoilShape::Round)
            },
            FEATURE
        ),
        Err(CoilError::TaperTooSteep)
    );
    assert_eq!(
        coil(
            &CoilSpec {
                turns: 0.01,
                ..spring(CoilShape::Round)
            },
            FEATURE
        ),
        Err(CoilError::TooFewTurns { turns: 0.01 })
    );
}
