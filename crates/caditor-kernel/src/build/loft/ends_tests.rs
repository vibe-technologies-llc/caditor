use std::{
    collections::BTreeSet,
    f64::consts::{PI, TAU},
};

use caditor_geometry::{Plane, Point3, Vector3};

use super::*;
use crate::{
    measure::mass_properties_tessellating,
    naming::FaceName,
    profile::{Profile, ProfileCurve, Selection},
    test_support::{assert_watertight, circle, rectangle},
    tolerance::SamplingTolerance,
    topology::Solid,
};

const FEATURE: u64 = 7;
const RING: f64 = 10.0;

fn region(curves: &[ProfileCurve]) -> Region {
    let mut regions = Profile::new(curves)
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap();
    regions.remove(0)
}

fn profile(plane: Plane, curves: &[ProfileCurve]) -> LoftProfile {
    LoftProfile {
        plane,
        region: region(curves),
    }
}

fn volume(solid: &Solid) -> f64 {
    mass_properties_tessellating(solid, &SamplingTolerance::new(1e-3, 0.05).unwrap())
        .unwrap()
        .properties
        .volume
}

fn assert_close(name: &str, found: f64, expected: f64, relative: f64) {
    assert!(
        (found - expected).abs() <= relative * expected.abs(),
        "{name}: {found} against {expected}"
    );
}

fn assert_sound(name: &str, solid: &Solid) {
    assert_eq!(solid.validate(), Ok(()), "{name}");
    assert_watertight(name, &solid.tessellate(&solid.default_tolerance()).unwrap());
    let names: BTreeSet<FaceName> = solid.faces().map(|(_, face)| face.name()).collect();
    assert_eq!(names.len(), solid.faces().count(), "{name}");
}

fn round_the_ring(count: usize, curves: impl Fn(u64) -> Vec<ProfileCurve>) -> Vec<LoftProfile> {
    (0..count)
        .map(|index| {
            let angle = TAU * index as f64 / count as f64;
            let radial = Vector3::new(angle.cos(), angle.sin(), 0.0);
            let plane = Plane::from_frame(
                Point3::ZERO + radial * RING,
                Vector3::Z.cross(radial),
                radial,
            )
            .unwrap();
            profile(plane, &curves(10 * index as u64 + 1))
        })
        .collect()
}

fn closed(skin: LoftSkin) -> (LoftSkin, LoftEnds) {
    (
        skin,
        LoftEnds {
            closed: true,
            ..LoftEnds::OPEN
        },
    )
}

fn squared(start: Option<f64>, end: Option<f64>) -> (LoftSkin, LoftEnds) {
    (
        LoftSkin::Smooth,
        LoftEnds {
            start,
            end,
            ..LoftEnds::OPEN
        },
    )
}

#[test]
fn a_loft_closing_back_to_its_first_profile_makes_a_ring_without_caps() {
    let squares = round_the_ring(12, |first| rectangle(first, (-1.0, -1.0), (1.0, 1.0)));
    let discs = round_the_ring(8, |first| vec![circle(first, (0.0, 0.0), 1.0)]);

    let smooth = loft_through(&squares, None, closed(LoftSkin::Smooth), FEATURE).unwrap();
    let ruled = loft_through(&squares, None, closed(LoftSkin::Ruled), FEATURE).unwrap();
    let round = loft_through(&discs, None, closed(LoftSkin::Smooth), FEATURE).unwrap();

    assert_sound("smooth ring", &smooth);
    assert_sound("ruled ring", &ruled);
    assert_sound("round ring", &round);
    assert_eq!(smooth.faces().count(), 4);
    assert_eq!(ruled.faces().count(), 4 * 12);
    assert_eq!(round.faces().count(), 1);
    assert_close("smooth ring", volume(&smooth), 4.0 * TAU * RING, 5e-3);
    assert_close("round ring", volume(&round), PI * TAU * RING, 5e-3);
    let ruled_volume = volume(&ruled);
    assert!(ruled_volume < 4.0 * TAU * RING && ruled_volume > 0.95 * 4.0 * TAU * RING);
}

#[test]
fn a_closed_loft_needs_three_profiles_and_no_end_point_or_square_ends() {
    let squares = round_the_ring(4, |first| rectangle(first, (-1.0, -1.0), (1.0, 1.0)));
    let two = squares.get(..2).unwrap();
    let with_squares = (
        LoftSkin::Smooth,
        LoftEnds {
            closed: true,
            start: Some(1.0),
            end: None,
        },
    );

    let too_few = loft_through(two, None, closed(LoftSkin::Smooth), FEATURE);
    let pointed = loft_through(
        &squares,
        Some(Point3::new(0.0, 0.0, 30.0)),
        closed(LoftSkin::Smooth),
        FEATURE,
    );
    let squared_ring = loft_through(&squares, None, with_squares, FEATURE);

    assert_eq!(too_few.err(), Some(LoftError::TooFewToClose));
    assert_eq!(pointed.err(), Some(LoftError::ClosedWithApex));
    assert_eq!(squared_ring.err(), Some(LoftError::SquareOnClosed));
}

fn level(height: f64, across: f64) -> Plane {
    Plane::new(Point3::new(across, 0.0, height), Vector3::Z).unwrap()
}

#[test]
fn square_ends_leave_and_reach_the_profiles_along_their_normals() {
    let profiles = [
        profile(level(0.0, 0.0), &rectangle(1, (-1.0, -1.0), (1.0, 1.0))),
        profile(level(10.0, 5.0), &rectangle(11, (-1.0, -1.0), (1.0, 1.0))),
    ];

    let solid = loft_through(&profiles, None, squared(Some(1.0), Some(1.0)), FEATURE).unwrap();
    let starting = loft_through(&profiles, None, squared(Some(2.0), None), FEATURE).unwrap();

    assert_sound("square ends", &solid);
    assert_sound("square start", &starting);
    assert_close("square ends", volume(&solid), 40.0, 1e-4);
    assert_close("square start", volume(&starting), 40.0, 1e-4);
    let rising: Vec<Vector3> = solid
        .edges()
        .filter_map(|(_, edge)| {
            let interval = edge.interval();
            let [start, end] = [interval.start(), interval.end()].map(|at| edge.curve().point(at));
            (start.z.abs() < 1e-9 && end.z > 1.0 || end.z.abs() < 1e-9 && start.z > 1.0).then(
                || {
                    let at = if start.z.abs() < 1e-9 {
                        interval.start()
                    } else {
                        interval.end()
                    };
                    edge.curve().evaluate(at).first.normalize()
                },
            )
        })
        .collect();
    assert_eq!(rising.len(), 4);
    assert!(
        rising
            .iter()
            .all(|tangent| tangent.cross(Vector3::Z).length() < 1e-9),
        "{rising:?}"
    );
}

#[test]
fn square_ends_need_a_smooth_loft_a_last_profile_and_a_weight_above_zero() {
    let profiles = [
        profile(level(0.0, 0.0), &rectangle(1, (-1.0, -1.0), (1.0, 1.0))),
        profile(level(10.0, 0.0), &rectangle(11, (-1.0, -1.0), (1.0, 1.0))),
    ];
    let ruled = (
        LoftSkin::Ruled,
        LoftEnds {
            start: Some(1.0),
            ..LoftEnds::OPEN
        },
    );

    let refused_ruled = loft_through(&profiles, None, ruled, FEATURE);
    let at_apex = loft_through(
        profiles.get(..1).unwrap(),
        Some(Point3::new(0.0, 0.0, 10.0)),
        squared(Some(1.0), Some(1.0)),
        FEATURE,
    );
    let weightless = loft_through(&profiles, None, squared(Some(0.0), None), FEATURE);
    let endless = loft_through(&profiles, None, squared(None, Some(f64::NAN)), FEATURE);

    assert_eq!(refused_ruled.err(), Some(LoftError::SquareOnRuled));
    assert_eq!(at_apex.err(), Some(LoftError::SquareAtApex));
    assert_eq!(weightless.err(), Some(LoftError::UnusableWeight));
    assert_eq!(endless.err(), Some(LoftError::UnusableWeight));
}
