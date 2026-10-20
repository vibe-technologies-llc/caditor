use std::f64::consts::{FRAC_PI_2, PI, TAU};

use caditor_geometry::{Plane, Point3, Vector3};

use super::*;
use crate::{
    build::{PipeSection, PipeShape, pipe},
    test_support::assert_watertight,
    tolerance::SamplingTolerance,
};

const FEATURE: u64 = 17;

fn flat() -> Plane {
    Plane::from_frame(Point3::ZERO, Vector3::Z, Vector3::X).unwrap()
}

fn spiral_spec(radius: f64, pitch: f64, turns: f64) -> SpiralSpec {
    SpiralSpec {
        frame: flat(),
        radius,
        pitch,
        turns,
        hand: Handedness::Right,
        start_angle: 0.0,
    }
}

fn start_and_end(turns: &[HelixTurn]) -> (Point3, Point3) {
    let first = turns.first().unwrap();
    let last = turns.last().unwrap();
    (
        first.curve().point(first.range().start()),
        last.curve().point(last.range().end()),
    )
}

#[test]
fn a_flat_spiral_lies_on_its_archimedean_spiral_in_the_frame_s_plane() {
    for (radius, pitch, turns) in [(5.0, 2.0, 3.5), (0.5, 4.0, 2.0), (40.0, 0.3, 6.0)] {
        let spec = spiral_spec(radius, pitch, turns);

        let pieces = spiral(&spec).unwrap();

        assert_eq!(pieces.len(), turns.ceil() as usize);
        let mut previous = 0.0;
        for turn in &pieces {
            let range = turn.range();
            for step in 0..=400 {
                let point = turn.curve().point(range.at(f64::from(step) / 400.0));
                let turned = (point.y.atan2(point.x) - previous + PI).rem_euclid(TAU) - PI;
                let angle = previous + turned;
                previous = angle;

                assert!(point.z.abs() < 1e-12, "{point}");
                assert!(
                    (point.truncate().length() - spec.radius_at(angle)).abs()
                        <= 2.0 * SPIRAL_TOLERANCE,
                    "{point} at {angle} against {}",
                    spec.radius_at(angle)
                );
            }
        }
        let (start, end) = start_and_end(&pieces);

        assert!((previous - spec.sweep()).abs() < 1e-9, "{previous}");
        assert!(start.distance(Point3::new(radius, 0.0, 0.0)) < 1e-12);
        assert!(end.distance(spec.point_at(spec.sweep()).unwrap()) < 1e-9);
        assert!((end.truncate().length() - spec.outer_radius()).abs() < 1e-9);
    }
}

#[test]
fn a_flat_spiral_is_as_long_as_its_closed_form() {
    let spec = spiral_spec(3.0, 2.5, 4.25);

    let pieces = spiral(&spec).unwrap();
    let length: f64 = pieces
        .iter()
        .map(|turn| turn.curve().length(turn.range()))
        .sum();
    let rough: f64 = (0..100_000)
        .map(|step| {
            let angle = spec.sweep() * (f64::from(step) + 0.5) / 100_000.0;
            spec.radius_at(angle).hypot(spec.pitch / TAU) * spec.sweep() / 100_000.0
        })
        .sum();

    assert!((spec.length() - rough).abs() < 1e-6 * rough, "{rough}");
    assert!(
        (length - spec.length()).abs() < 1e-6 * spec.length(),
        "{length} against {}",
        spec.length()
    );
}

#[test]
fn a_start_angle_and_a_hand_turn_the_flat_spiral_without_mirroring_its_start() {
    let right = SpiralSpec {
        start_angle: FRAC_PI_2,
        ..spiral_spec(10.0, 2.0, 1.25)
    };
    let left = SpiralSpec {
        hand: Handedness::Left,
        ..right
    };

    let (right_start, right_end) = start_and_end(&spiral(&right).unwrap());
    let (left_start, left_end) = start_and_end(&spiral(&left).unwrap());

    assert!(right_start.distance(Point3::new(0.0, 10.0, 0.0)) < 1e-9);
    assert!(left_start.distance(Point3::new(0.0, 10.0, 0.0)) < 1e-9);
    assert!(
        right_end.distance(Point3::new(-12.5, 0.0, 0.0)) < 1e-9,
        "{right_end}"
    );
    assert!(
        left_end.distance(Point3::new(12.5, 0.0, 0.0)) < 1e-9,
        "{left_end}"
    );
    assert_eq!(right.start(), Some(right_start));
}

#[test]
fn a_flat_spiral_out_of_range_is_refused() {
    let spec = spiral_spec(5.0, 2.0, 3.0);

    assert_eq!(
        spiral(&SpiralSpec {
            radius: 0.0,
            ..spec
        }),
        Err(SpiralError::NoRadius)
    );
    assert_eq!(
        spiral(&SpiralSpec { pitch: 0.0, ..spec }),
        Err(SpiralError::NoPitch)
    );
    assert_eq!(
        spiral(&SpiralSpec {
            turns: 0.01,
            ..spec
        }),
        Err(SpiralError::TooFewTurns { turns: 0.01 })
    );
    assert_eq!(
        spiral(&SpiralSpec {
            turns: 600.0,
            ..spec
        }),
        Err(SpiralError::TooManyTurns { turns: 600.0 })
    );
    assert_eq!(
        spiral(&SpiralSpec { pitch: 1e6, ..spec }),
        Err(SpiralError::TooLarge)
    );
    assert_eq!(
        spiral(&SpiralSpec {
            start_angle: f64::NAN,
            ..spec
        }),
        Err(SpiralError::NonFinite)
    );
}

#[test]
fn a_pipe_follows_a_flat_spiral_into_a_valid_watertight_clock_spring() {
    let spec = spiral_spec(6.0, 3.0, 1.5);
    let size = 1.5;
    let path: Vec<_> = spiral(&spec)
        .unwrap()
        .iter()
        .zip(1_u128..)
        .map(|(turn, key)| turn.path_segment(key))
        .collect();
    let section = PipeSection {
        shape: PipeShape::Round,
        size,
        wall: None,
    };

    let solid = pipe(&path, &section, 0.0, Vector3::Z, FEATURE).unwrap();
    let growth = spec.pitch / TAU;
    let reach = spec.radius.hypot(growth);
    let sweep = spec.sweep();
    let area = PI * 0.25 * size * size;
    let expected = area / reach
        * ((spec.radius.powi(2) + growth.powi(2)) * sweep
            + growth * spec.radius * sweep * sweep / 2.0);
    let measured = solid
        .tessellate(&SamplingTolerance::new(1e-3, 0.02).unwrap())
        .unwrap()
        .mass_properties()
        .volume;

    assert_eq!(solid.validate(), Ok(()));
    assert_watertight(
        "pipe along a flat spiral",
        &solid.tessellate(&solid.default_tolerance()).unwrap(),
    );
    assert!(
        (measured - expected).abs() < 1e-3 * expected,
        "{measured} against {expected}"
    );
}
