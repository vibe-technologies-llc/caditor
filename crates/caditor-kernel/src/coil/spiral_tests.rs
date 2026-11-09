use std::f64::consts::{FRAC_PI_2, PI, TAU};

use caditor_geometry::{Maths, Plane, Point3, Vector3};

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
        final_pitch: None,
        table: Vec::new(),
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
                let turned = (point.y.atan2_libm(point.x) - previous + PI).rem_euclid(TAU) - PI;
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
            spec.radius_at(angle).hypot_libm(spec.pitch / TAU) * spec.sweep() / 100_000.0
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
        ..right.clone()
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
            ..spec.clone()
        }),
        Err(SpiralError::NoRadius)
    );
    assert_eq!(
        spiral(&SpiralSpec {
            pitch: 0.0,
            ..spec.clone()
        }),
        Err(SpiralError::NoPitch)
    );
    assert_eq!(
        spiral(&SpiralSpec {
            turns: 0.01,
            ..spec.clone()
        }),
        Err(SpiralError::TooFewTurns { turns: 0.01 })
    );
    assert_eq!(
        spiral(&SpiralSpec {
            turns: 600.0,
            ..spec.clone()
        }),
        Err(SpiralError::TooManyTurns { turns: 600.0 })
    );
    assert_eq!(
        spiral(&SpiralSpec {
            pitch: 1e6,
            ..spec.clone()
        }),
        Err(SpiralError::TooLarge)
    );
    assert_eq!(
        spiral(&SpiralSpec {
            start_angle: f64::NAN,
            ..spec.clone()
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
    let reach = spec.radius.hypot_libm(growth);
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

fn progressive_spiral(radius: f64, first: f64, last: f64, turns: f64) -> SpiralSpec {
    SpiralSpec {
        final_pitch: Some(last),
        ..spiral_spec(radius, first, turns)
    }
}

fn tabled_spiral(radius: f64, first: f64, turns: f64, rows: &[(f64, f64)]) -> SpiralSpec {
    SpiralSpec {
        table: rows
            .iter()
            .map(|&(turns, pitch)| PitchStop { turns, pitch })
            .collect(),
        ..spiral_spec(radius, first, turns)
    }
}

fn assert_lies_on_its_law(spec: &SpiralSpec) {
    let pieces = spiral(spec).unwrap();
    let mut previous = 0.0;
    let mut joined: Option<Point3> = None;
    for turn in &pieces {
        let range = turn.range();
        if let Some(end) = joined {
            let start = turn.curve().point(range.start());
            assert!(start.distance(end) < 1e-9, "{start} then {end}");
        }
        for step in 0..=200 {
            let point = turn.curve().point(range.at(f64::from(step) / 200.0));
            let turned = (point.y.atan2_libm(point.x) - previous + PI).rem_euclid(TAU) - PI;
            let angle = previous + turned;
            previous = angle;

            assert!(point.z.abs() < 1e-12, "{point}");
            assert!(
                (point.truncate().length() - spec.radius_at(angle)).abs() <= 2.0 * SPIRAL_TOLERANCE,
                "{point} at {angle} against {}",
                spec.radius_at(angle)
            );
        }
        joined = Some(turn.curve().point(range.end()));
    }
    assert!((previous - spec.sweep()).abs() < 1e-9, "{previous}");
}

#[test]
fn a_progressive_flat_spiral_grows_by_a_pitch_changing_evenly() {
    let spec = progressive_spiral(5.0, 2.0, 6.0, 4.0);

    assert_lies_on_its_law(&spec);

    assert!((spec.outer_radius() - (5.0 + 0.5 * (2.0 + 6.0) * 4.0)).abs() < 1e-12);
    assert!((spec.radius_at(TAU * 2.0) - (5.0 + 2.0 * 2.0 + 0.5 * 1.0 * 2.0 * 2.0)).abs() < 1e-12);
    assert_eq!(spec.narrowest_pitch(), 2.0);
    assert!(spec.has_changing_pitch());
}

#[test]
fn a_flat_spiral_follows_a_table_of_pitches_and_keeps_the_last_one() {
    let spec = tabled_spiral(4.0, 2.0, 7.0, &[(2.0, 2.0), (4.0, 6.0)]);

    assert_lies_on_its_law(&spec);

    assert!((spec.radius_at(TAU * 2.0) - 8.0).abs() < 1e-12);
    assert!((spec.radius_at(TAU * 4.0) - 16.0).abs() < 1e-12);
    assert!((spec.outer_radius() - (16.0 + 3.0 * 6.0)).abs() < 1e-12);
    assert!(
        spec.point_at(TAU * 4.0)
            .is_some_and(|point| point.distance(Point3::new(16.0, 0.0, 0.0)) < 1e-9)
    );
}

#[test]
fn a_table_of_one_row_at_the_end_is_the_progressive_flat_spiral() {
    let tabled = tabled_spiral(5.0, 2.0, 4.0, &[(4.0, 6.0)]);
    let progressive = progressive_spiral(5.0, 2.0, 6.0, 4.0);

    assert_eq!(spiral(&tabled), spiral(&progressive));
}

#[test]
fn the_length_of_a_flat_spiral_of_changing_pitch_is_its_integral() {
    for spec in [
        progressive_spiral(3.0, 1.5, 5.0, 4.25),
        tabled_spiral(3.0, 2.0, 5.0, &[(1.5, 2.0), (3.0, 4.0)]),
    ] {
        let rough: f64 = (0..400_000)
            .map(|step| {
                let angle = spec.sweep() * (f64::from(step) + 0.5) / 400_000.0;
                let growth = spec.radius_at(angle + 1e-6) - spec.radius_at(angle - 1e-6);
                spec.radius_at(angle).hypot_libm(growth / 2e-6) * spec.sweep() / 400_000.0
            })
            .sum();
        let curve: f64 = spiral(&spec)
            .unwrap()
            .iter()
            .map(|turn| turn.curve().length(turn.range()))
            .sum();

        assert!(
            (spec.length() - rough).abs() < 1e-5 * rough,
            "{} against {rough}",
            spec.length()
        );
        assert!(
            (curve - spec.length()).abs() < 1e-5 * spec.length(),
            "{curve} against {}",
            spec.length()
        );
    }
}

#[test]
fn a_flat_spiral_of_changing_pitch_is_judged_by_its_narrowest_pitch() {
    let across = 2.5;
    let narrow = progressive_spiral(6.0, 3.0, 6.0, 4.0);
    let narrow_end = progressive_spiral(6.0, 6.0, 2.0, 4.0);
    let least = narrow.least_pitch_for(across);

    assert_eq!(narrow_end.narrowest_pitch(), 2.0);
    assert!(narrow.turns_clear_of(across));
    assert!(!narrow_end.turns_clear_of(across));
    assert!(least > across && least < 1.2 * across, "{least}");
    assert!(
        SpiralSpec {
            pitch: least * 1.001,
            final_pitch: Some(least * 1.001),
            ..narrow.clone()
        }
        .turns_clear_of(across)
    );
    assert!(
        !SpiralSpec {
            pitch: least * 0.95,
            final_pitch: Some(least * 1.5),
            ..narrow.clone()
        }
        .turns_clear_of(across)
    );
    assert!(!tabled_spiral(6.0, 6.0, 4.0, &[(2.0, 2.0), (3.0, 6.0)]).turns_clear_of(across));
}

#[test]
fn a_pipe_follows_a_flat_spiral_of_changing_pitch_into_a_valid_watertight_solid() {
    let spec = progressive_spiral(6.0, 3.0, 5.0, 1.5);
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
    let expected = PI * 0.25 * size * size * spec.length();
    let measured = solid
        .tessellate(&SamplingTolerance::new(1e-3, 0.02).unwrap())
        .unwrap()
        .mass_properties()
        .volume;

    assert_eq!(solid.validate(), Ok(()));
    assert_watertight(
        "pipe along a changing flat spiral",
        &solid.tessellate(&solid.default_tolerance()).unwrap(),
    );
    assert!(
        (measured - expected).abs() < 2e-2 * expected,
        "{measured} against {expected}"
    );
}

#[test]
fn a_flat_spiral_pitch_law_out_of_range_is_refused() {
    let spec = spiral_spec(5.0, 2.0, 4.0);

    assert_eq!(
        spiral(&SpiralSpec {
            final_pitch: Some(0.0),
            ..spec.clone()
        }),
        Err(SpiralError::NoPitch)
    );
    assert_eq!(
        spiral(&SpiralSpec {
            final_pitch: Some(f64::NAN),
            ..spec.clone()
        }),
        Err(SpiralError::NonFinite)
    );
    assert_eq!(
        spiral(&tabled_spiral(5.0, 2.0, 4.0, &[(2.0, 3.0), (2.0, 4.0)])),
        Err(SpiralError::TableOrder { turns: 2.0 })
    );
    assert_eq!(
        spiral(&tabled_spiral(5.0, 2.0, 4.0, &[(5.0, 3.0)])),
        Err(SpiralError::TableBeyond {
            turns: 5.0,
            spiral: 4.0
        })
    );
    assert_eq!(
        spiral(&tabled_spiral(5.0, 2.0, 4.0, &[(2.0, 0.0)])),
        Err(SpiralError::NoPitch)
    );
    assert_eq!(
        spiral(&SpiralSpec {
            final_pitch: Some(3.0),
            ..tabled_spiral(5.0, 2.0, 4.0, &[(2.0, 3.0)])
        }),
        Err(SpiralError::TableWithFinalPitch)
    );
}
