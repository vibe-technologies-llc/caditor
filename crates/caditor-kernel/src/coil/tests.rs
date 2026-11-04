use std::f64::consts::{PI, TAU};

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

fn helix_spec(turns: f64, taper: f64) -> HelixSpec {
    HelixSpec {
        frame: upright(),
        radius: 10.0,
        pitch: 4.0,
        turns,
        taper,
        hand: Handedness::Right,
        start_angle: 0.0,
        ends: None,
        final_pitch: None,
        table: Vec::new(),
    }
}

#[test]
fn a_helix_is_one_curve_a_turn_on_its_cylinder_rising_a_pitch_each_turn() {
    let turns = helix(&helix_spec(2.5, 0.0)).unwrap();

    assert_eq!(turns.len(), 3);
    for (index, turn) in turns.iter().enumerate() {
        let range = turn.range();
        for step in 0..=40 {
            let point = turn.curve().point(range.at(f64::from(step) / 40.0));
            let along = (index as f64 + f64::from(step) / 40.0) * 2.5 / 3.0;
            let angle = point.y.atan2(point.x).rem_euclid(2.0 * PI);
            let expected = 4.0 * (along.floor() + angle / (2.0 * PI));

            assert!((point.truncate().length() - 10.0).abs() < 1e-9, "{point}");
            assert!(
                (point.z - expected).abs() < 1e-3 || (point.z - expected - 4.0).abs() < 1e-3,
                "{point} against {expected}"
            );
        }
    }
    let end = turns
        .last()
        .map(|turn| turn.curve().point(turn.range().end()));
    assert!(end.is_some_and(|end| (end.z - 10.0).abs() < 1e-9 && (end.x + 10.0).abs() < 1e-9));
}

#[test]
fn a_helix_drawn_in_to_its_axis_or_out_of_range_is_refused() {
    let steep = helix_spec(2.0, -60.0_f64.to_radians());
    let flat = HelixSpec {
        pitch: 0.0,
        ..helix_spec(2.0, 0.0)
    };

    assert_eq!(helix(&steep), Err(HelixError::ReachesAxis));
    assert_eq!(helix(&flat), Err(HelixError::NoPitch));
    assert_eq!(
        helix(&helix_spec(0.01, 0.0)),
        Err(HelixError::TooFewTurns { turns: 0.01 })
    );
    assert_eq!(
        helix(&helix_spec(2.0, 85.0_f64.to_radians())),
        Err(HelixError::TaperTooSteep)
    );
}

#[test]
fn a_start_angle_turns_where_the_helix_starts_about_its_axis_whichever_way_it_turns() {
    let quarter = PI / 2.0;
    let right = HelixSpec {
        start_angle: quarter,
        ..helix_spec(1.25, 0.0)
    };
    let left = HelixSpec {
        hand: Handedness::Left,
        ..right.clone()
    };

    let first = |spec: &HelixSpec| {
        let turns = helix(spec).unwrap();
        let start = turns
            .first()
            .map(|turn| turn.curve().point(turn.range().start()))
            .unwrap();
        let end = turns
            .last()
            .map(|turn| turn.curve().point(turn.range().end()))
            .unwrap();
        (start, end)
    };
    let (right_start, right_end) = first(&right);
    let (left_start, left_end) = first(&left);

    assert!(
        right_start.distance(Point3::new(0.0, 10.0, 0.0)) < 1e-9,
        "{right_start}"
    );
    assert!(
        left_start.distance(Point3::new(0.0, 10.0, 0.0)) < 1e-9,
        "{left_start}"
    );
    assert!(
        right_end.distance(Point3::new(-10.0, 0.0, 5.0)) < 1e-9,
        "{right_end}"
    );
    assert!(
        left_end.distance(Point3::new(10.0, 0.0, 5.0)) < 1e-9,
        "{left_end}"
    );
    assert_eq!(right.start(), Some(right_start));
    assert!(
        right
            .point_at(2.5 * PI)
            .is_some_and(|point| point.distance(right_end) < 1e-9)
    );
    assert_eq!(
        helix(&HelixSpec {
            start_angle: f64::NAN,
            ..right
        }),
        Err(HelixError::NonFinite)
    );
}

fn closed_spring_helix(turns: f64, end_turns: f64) -> HelixSpec {
    HelixSpec {
        ends: Some(HelixEnds::with_default_change(end_turns, 2.2)),
        ..helix_spec(turns, 0.0)
    }
}

#[test]
fn closed_ends_wind_at_their_own_pitch_and_change_to_the_body_pitch_smoothly() {
    let spec = closed_spring_helix(5.0, 1.0);
    let turns = helix(&spec).unwrap();

    assert_eq!(turns.len(), 6);
    assert!((spec.height() - 15.5).abs() < 1e-12, "{}", spec.height());

    let mut unwrapped = 0.0_f64;
    let mut previous: Option<Point3> = None;
    let mut last_tangent: Option<Vector3> = None;
    for turn in &turns {
        let range = turn.range();
        let first = turn.curve().evaluate(range.start()).first.normalize();
        if let Some(tangent) = last_tangent {
            assert!(
                tangent.angle_between(first) < 1e-9,
                "{tangent} then {first}"
            );
        }
        for step in 0..=64 {
            let point = turn.curve().point(range.at(f64::from(step) / 64.0));
            if let Some(before) = previous {
                let turned = point.y.atan2(point.x) - before.y.atan2(before.x);
                unwrapped += (turned + PI).rem_euclid(2.0 * PI) - PI;
            }
            previous = Some(point);
            let expected = spec.point_at(unwrapped).unwrap();

            assert!((point.truncate().length() - 10.0).abs() < 1e-9, "{point}");
            assert!(
                (point.z - expected.z).abs() < 1e-3,
                "{point} against {expected}"
            );
        }
        last_tangent = Some(turn.curve().evaluate(range.end()).first.normalize());
    }
    let end = previous.unwrap();

    assert!(end.distance(Point3::new(10.0, 0.0, 15.5)) < 1e-9, "{end}");
    assert!(
        spec.point_at(PI)
            .is_some_and(|point| (point.z - 1.1).abs() < 1e-12)
    );
}

#[test]
fn closed_ends_find_the_body_pitch_or_turns_from_a_height() {
    let ends = HelixEnds::with_default_change(1.0, 2.2);
    let plan = HelixPitch {
        ends: Some(ends),
        final_pitch: None,
        table: Vec::new(),
    };

    assert!((plan.height(4.0, 5.0) - 15.5).abs() < 1e-12);
    assert!((plan.pitch_for(15.5, 5.0) - 4.0).abs() < 1e-12);
    assert!((plan.turns_for(15.5, 4.0) - 5.0).abs() < 1e-12);
    assert!((ends.turns_taken() - 3.0).abs() < 1e-12);
}

#[test]
fn closed_ends_longer_than_the_helix_or_of_a_sliver_of_a_turn_are_refused() {
    assert_eq!(
        helix(&closed_spring_helix(2.0, 1.0)),
        Err(HelixError::EndsTooLong {
            taken: 3.0,
            turns: 2.0
        })
    );
    assert_eq!(
        helix(&closed_spring_helix(3.01, 1.0)),
        Err(HelixError::EndsTooLong {
            taken: 3.0,
            turns: 3.01
        })
    );
    assert_eq!(
        helix(&closed_spring_helix(5.0, 0.01)),
        Err(HelixError::EndTurns { turns: 0.01 })
    );
    assert_eq!(
        helix(&closed_spring_helix(3.0, 1.0)).map(|turns| turns.len()),
        Ok(4)
    );
    assert_eq!(
        helix(&closed_spring_helix(4.0, 0.0)).map(|turns| turns.len()),
        Ok(5)
    );
}

fn progressive_helix(turns: f64, first: f64, last: f64) -> HelixSpec {
    HelixSpec {
        pitch: first,
        final_pitch: Some(last),
        table: Vec::new(),
        ..helix_spec(turns, 0.0)
    }
}

fn assert_follows_the_law(spec: &HelixSpec) {
    let turns = helix(spec).unwrap();
    let mut unwrapped = 0.0_f64;
    let mut previous: Option<Point3> = None;
    let mut last_tangent: Option<Vector3> = None;
    for turn in &turns {
        let range = turn.range();
        let first = turn.curve().evaluate(range.start()).first.normalize();
        if let Some(tangent) = last_tangent {
            assert!(
                tangent.angle_between(first) < 1e-9,
                "{tangent} then {first}"
            );
        }
        for step in 0..=64 {
            let point = turn.curve().point(range.at(f64::from(step) / 64.0));
            if let Some(before) = previous {
                let turned = point.y.atan2(point.x) - before.y.atan2(before.x);
                unwrapped += (turned + PI).rem_euclid(2.0 * PI) - PI;
            }
            previous = Some(point);
            let expected = spec.point_at(unwrapped).unwrap();

            assert!((point.truncate().length() - 10.0).abs() < 1e-9, "{point}");
            assert!(
                (point.z - expected.z).abs() < 1e-3,
                "{point} against {expected}"
            );
        }
        last_tangent = Some(turn.curve().evaluate(range.end()).first.normalize());
    }
    let end = previous.unwrap();

    assert!(
        (end.z - spec.height()).abs() < 1e-9,
        "{end} against {}",
        spec.height()
    );
}

#[test]
fn a_progressive_helix_changes_its_pitch_evenly_from_one_end_to_the_other() {
    let spec = progressive_helix(6.0, 2.0, 6.0);

    assert_follows_the_law(&spec);
    assert!((spec.height() - 24.0).abs() < 1e-12, "{}", spec.height());
    assert!(
        spec.point_at(TAU * 6.0)
            .is_some_and(|point| (point.z - 24.0).abs() < 1e-12)
    );
    assert!(
        spec.point_at(TAU * 3.0)
            .is_some_and(|point| (point.z - 9.0).abs() < 1e-12)
    );
}

#[test]
fn closed_ends_change_over_the_turns_asked_and_a_progressive_body_follows() {
    let ends = HelixEnds {
        turns: 1.0,
        pitch: 2.2,
        change: 1.5,
    };
    let spec = HelixSpec {
        ends: Some(ends),
        ..progressive_helix(10.0, 4.0, 6.0)
    };

    assert_follows_the_law(&spec);
    assert!((spec.height() - 40.2).abs() < 1e-12, "{}", spec.height());
    assert!((spec.body_start() - TAU * 2.5).abs() < 1e-12);
}

#[test]
fn a_progressive_helix_is_sized_from_a_height_by_its_mean_pitch() {
    let plan = HelixPitch {
        ends: Some(HelixEnds::with_default_change(1.0, 2.2)),
        final_pitch: Some(6.0),
        table: Vec::new(),
    };
    let height = plan.height(4.0, 8.0);

    assert!((plan.pitch_for(height, 8.0) - 4.0).abs() < 1e-12);
    assert!((plan.turns_for(height, 4.0) - 8.0).abs() < 1e-12);
}

#[test]
fn a_progression_with_no_turns_to_change_over_or_a_change_of_a_sliver_is_refused() {
    let all_ends = HelixSpec {
        ends: Some(HelixEnds::with_default_change(1.0, 2.2)),
        ..progressive_helix(3.0, 3.0, 5.0)
    };

    assert_eq!(
        helix(&all_ends),
        Err(HelixError::ProgressionWithoutBody {
            from: 3.0,
            to: 5.0,
            turns: 3.0
        })
    );
    assert_eq!(
        helix(&HelixSpec {
            ends: Some(HelixEnds {
                turns: 1.0,
                pitch: 2.2,
                change: 0.01
            }),
            ..progressive_helix(8.0, 3.0, 5.0)
        }),
        Err(HelixError::EndChange { turns: 0.01 })
    );
    assert_eq!(
        helix(&progressive_helix(4.0, 3.0, 0.0)),
        Err(HelixError::NoPitch)
    );
}

fn tabled_helix(turns: f64, first: f64, rows: &[(f64, f64)]) -> HelixSpec {
    HelixSpec {
        pitch: first,
        table: rows
            .iter()
            .map(|&(turns, pitch)| PitchStop { turns, pitch })
            .collect(),
        ..helix_spec(turns, 0.0)
    }
}

#[test]
fn a_helix_follows_a_table_of_pitches_by_turn_and_keeps_the_last_one() {
    let spec = tabled_helix(6.0, 2.0, &[(2.0, 2.0), (4.0, 6.0)]);

    assert_follows_the_law(&spec);
    assert!((spec.height() - 24.0).abs() < 1e-12, "{}", spec.height());
    assert!(
        spec.point_at(TAU * 2.0)
            .is_some_and(|point| (point.z - 4.0).abs() < 1e-12)
    );
    assert!(
        spec.point_at(TAU * 4.0)
            .is_some_and(|point| (point.z - 12.0).abs() < 1e-12)
    );
}

#[test]
fn a_table_of_one_row_at_the_end_is_the_progressive_helix() {
    let tabled = tabled_helix(6.0, 2.0, &[(6.0, 6.0)]);
    let progressive = progressive_helix(6.0, 2.0, 6.0);

    assert_eq!(helix(&tabled), helix(&progressive));
}

#[test]
fn a_table_sizes_the_helix_from_a_height_by_its_rows() {
    let rows = [
        PitchStop {
            turns: 2.0,
            pitch: 2.0,
        },
        PitchStop {
            turns: 4.0,
            pitch: 6.0,
        },
    ];
    let plan = HelixPitch {
        table: rows.to_vec(),
        ..HelixPitch::default()
    };
    let height = plan.height(2.0, 6.0);

    assert!((height - 24.0).abs() < 1e-12, "{height}");
    assert!((plan.pitch_for(height, 6.0) - 2.0).abs() < 1e-12);
    assert!((plan.turns_for(height, 2.0) - 6.0).abs() < 1e-9);
    assert!((plan.turns_for(10.0, 2.0) - (1.0 + 7.0_f64.sqrt())).abs() < 1e-9);
    assert!((plan.turns_for(4.0, 2.0) - 2.0).abs() < 1e-9);
}

#[test]
fn a_table_out_of_order_past_the_helix_or_with_other_laws_is_refused() {
    assert_eq!(
        helix(&tabled_helix(6.0, 2.0, &[(3.0, 2.0), (3.0, 4.0)])),
        Err(HelixError::TableOrder { turns: 3.0 })
    );
    assert_eq!(
        helix(&tabled_helix(6.0, 2.0, &[(0.0, 2.0)])),
        Err(HelixError::TableOrder { turns: 0.0 })
    );
    assert_eq!(
        helix(&tabled_helix(6.0, 2.0, &[(7.0, 2.0)])),
        Err(HelixError::TableBeyond {
            turns: 7.0,
            helix: 6.0
        })
    );
    assert_eq!(
        helix(&tabled_helix(6.0, 2.0, &[(3.0, 0.0)])),
        Err(HelixError::NoPitch)
    );
    assert_eq!(
        helix(&HelixSpec {
            final_pitch: Some(4.0),
            ..tabled_helix(6.0, 2.0, &[(3.0, 3.0)])
        }),
        Err(HelixError::TableWithOther {
            other: "a final pitch"
        })
    );
    assert_eq!(
        helix(&HelixSpec {
            ends: Some(HelixEnds::with_default_change(1.0, 2.2)),
            ..tabled_helix(6.0, 2.0, &[(3.0, 3.0)])
        }),
        Err(HelixError::TableWithOther {
            other: "closed ends"
        })
    );
}
