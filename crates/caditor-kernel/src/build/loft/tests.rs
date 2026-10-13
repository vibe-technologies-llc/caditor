use std::{collections::BTreeSet, f64::consts::PI};

use caditor_geometry::{Plane, Point3, Vector3};

use super::*;
use crate::{
    measure::mass_properties_tessellating,
    naming::FaceName,
    profile::{Profile, ProfileCurve, Selection},
    test_support::{assert_cancelled_anywhere, assert_watertight, circle, line, rectangle},
    tolerance::SamplingTolerance,
    topology::Solid,
};

const FEATURE: u64 = 7;

fn region(curves: &[ProfileCurve]) -> Region {
    let mut regions = Profile::new(curves)
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap();
    assert_eq!(regions.len(), 1);
    regions.remove(0)
}

fn level(height: f64) -> Plane {
    Plane::new(Point3::new(0.0, 0.0, height), Vector3::Z).unwrap()
}

fn profile(plane: Plane, curves: &[ProfileCurve]) -> LoftProfile {
    LoftProfile {
        plane,
        region: region(curves),
    }
}

fn square(first: u64, half: f64) -> Vec<ProfileCurve> {
    rectangle(first, (-half, -half), (half, half))
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
    assert_eq!(
        names.len(),
        solid.faces().count(),
        "{name}: repeated face names"
    );
}

fn frustum(bottom: f64, top: f64, height: f64) -> f64 {
    height / 3.0 * (bottom + top + (bottom * top).sqrt())
}

#[test]
fn a_square_lofted_to_a_smaller_square_is_the_frustum() {
    let profiles = [
        profile(level(0.0), &square(1, 10.0)),
        profile(level(10.0), &square(11, 5.0)),
    ];

    let ruled = loft(&profiles, None, LoftSkin::Ruled, FEATURE).unwrap();
    let smooth = loft(&profiles, None, LoftSkin::Smooth, FEATURE).unwrap();

    assert_sound("ruled", &ruled);
    assert_close("ruled", volume(&ruled), frustum(400.0, 100.0, 10.0), 1e-6);
    assert_close("smooth", volume(&smooth), frustum(400.0, 100.0, 10.0), 1e-6);
    assert_eq!(ruled.faces().count(), 6);
}

#[test]
fn a_circle_lofted_to_a_square_is_valid_and_watertight() {
    let profiles = [
        profile(level(0.0), &[circle(1, (0.0, 0.0), 6.0)]),
        profile(level(8.0), &square(11, 4.0)),
    ];

    let solid = loft(&profiles, None, LoftSkin::Ruled, FEATURE).unwrap();
    let found = volume(&solid);

    assert_sound("circle to square", &solid);
    assert_eq!(solid.faces().count(), 6);
    assert!(found > 8.0 * 64.0 && found < 8.0 * PI * 36.0, "{found}");
}

#[test]
fn a_square_lofted_to_a_circle_and_back_runs_smoothly_through_all_three() {
    let profiles = [
        profile(level(0.0), &square(1, 5.0)),
        profile(level(6.0), &[circle(11, (0.0, 0.0), 3.0)]),
        profile(level(12.0), &square(21, 5.0)),
    ];

    let smooth = loft(&profiles, None, LoftSkin::Smooth, FEATURE).unwrap();
    let ruled = loft(&profiles, None, LoftSkin::Ruled, FEATURE).unwrap();

    assert_sound("smooth", &smooth);
    assert_sound("ruled", &ruled);
    assert_eq!(smooth.faces().count(), 6);
    assert_eq!(ruled.faces().count(), 10);
    assert!(volume(&smooth) < volume(&ruled));
}

#[test]
fn circles_loft_into_a_smooth_vase() {
    let profiles = [
        profile(level(0.0), &[circle(1, (0.0, 0.0), 4.0)]),
        profile(level(5.0), &[circle(2, (0.0, 0.0), 6.0)]),
        profile(level(10.0), &[circle(3, (0.0, 0.0), 3.0)]),
    ];

    let solid = loft(&profiles, None, LoftSkin::Smooth, FEATURE).unwrap();

    assert_sound("vase", &solid);
    assert_eq!(solid.faces().count(), 3);
    let found = volume(&solid);
    assert!(
        found > PI * 9.0 * 10.0 && found < PI * 36.0 * 10.0,
        "{found}"
    );
}

#[test]
fn two_circles_loft_into_the_cone_between_them() {
    let profiles = [
        profile(level(0.0), &[circle(1, (0.0, 0.0), 4.0)]),
        profile(level(9.0), &[circle(2, (0.0, 0.0), 2.0)]),
    ];

    let solid = loft(&profiles, None, LoftSkin::Ruled, FEATURE).unwrap();

    assert_sound("cone", &solid);
    assert_close(
        "cone",
        volume(&solid),
        frustum(PI * 16.0, PI * 4.0, 9.0),
        1e-4,
    );
}

#[test]
fn a_loft_ending_at_a_point_is_a_pyramid() {
    let profiles = [profile(level(0.0), &square(1, 3.0))];

    let solid = loft(
        &profiles,
        Some(Point3::new(0.0, 0.0, 9.0)),
        LoftSkin::Ruled,
        FEATURE,
    )
    .unwrap();

    assert_sound("pyramid", &solid);
    assert_eq!(solid.faces().count(), 5);
    assert_close("pyramid", volume(&solid), 36.0 * 9.0 / 3.0, 1e-6);
}

#[test]
fn a_circle_ending_at_a_point_is_a_cone() {
    let profiles = [profile(level(0.0), &[circle(1, (0.0, 0.0), 3.0)])];

    let solid = loft(
        &profiles,
        Some(Point3::new(0.0, 0.0, 6.0)),
        LoftSkin::Ruled,
        FEATURE,
    )
    .unwrap();

    assert_sound("cone", &solid);
    assert_close("cone", volume(&solid), PI * 9.0 * 6.0 / 3.0, 1e-4);
}

#[test]
fn a_profile_drawn_facing_back_lofts_the_same() {
    let profiles = [
        profile(level(0.0), &square(1, 10.0)),
        profile(
            Plane::with_x_axis(Point3::new(0.0, 0.0, 10.0), -Vector3::Z, Vector3::X).unwrap(),
            &square(11, 5.0),
        ),
    ];

    let solid = loft(&profiles, None, LoftSkin::Ruled, FEATURE).unwrap();

    assert_sound("facing back", &solid);
    assert_close(
        "facing back",
        volume(&solid),
        frustum(400.0, 100.0, 10.0),
        1e-6,
    );
}

#[test]
fn profiles_of_different_counts_are_matched_by_splitting() {
    let triangle = [
        line(1, (-6.0, -4.0), (6.0, -4.0)),
        line(2, (6.0, -4.0), (0.0, 6.0)),
        line(3, (0.0, 6.0), (-6.0, -4.0)),
    ];
    let profiles = [
        profile(level(0.0), &triangle),
        profile(level(7.0), &square(11, 3.0)),
    ];

    let solid = loft(&profiles, None, LoftSkin::Ruled, FEATURE).unwrap();

    assert_sound("triangle to square", &solid);
    assert_eq!(solid.faces().count(), 6);
}

#[test]
fn a_slanted_top_lofts_from_a_level_bottom() {
    let slanted = Plane::new(
        Point3::new(1.0, 0.0, 10.0),
        Vector3::new(0.3, 0.0, 1.0).normalize(),
    )
    .unwrap();
    let profiles = [
        profile(level(0.0), &square(1, 5.0)),
        profile(slanted, &[circle(11, (0.0, 0.0), 3.0)]),
    ];

    let solid = loft(&profiles, None, LoftSkin::Smooth, FEATURE).unwrap();

    assert_sound("slanted", &solid);
}

#[test]
fn faces_keep_their_names_when_a_later_profile_changes() {
    let names = |top: f64| -> BTreeSet<FaceName> {
        let profiles = [
            profile(level(0.0), &square(1, 10.0)),
            profile(level(10.0), &square(11, top)),
        ];
        loft(&profiles, None, LoftSkin::Ruled, FEATURE)
            .unwrap()
            .faces()
            .map(|(_, face)| face.name())
            .collect()
    };

    assert_eq!(names(5.0), names(7.0));
}

#[test]
fn unloftable_profiles_are_refused_in_words() {
    let ring = [circle(1, (0.0, 0.0), 5.0), circle(2, (0.0, 0.0), 2.0)];
    let holed = [
        profile(level(0.0), &ring),
        profile(level(5.0), &square(11, 3.0)),
    ];
    let flat = [
        profile(level(0.0), &square(1, 3.0)),
        profile(level(0.0), &[circle(11, (0.0, 0.0), 1.0)]),
    ];
    let along = [
        profile(level(0.0), &square(1, 3.0)),
        profile(level(0.0), &[circle(11, (20.0, 0.0), 1.0)]),
    ];

    assert_eq!(
        loft(&holed, None, LoftSkin::Ruled, FEATURE),
        Err(LoftError::Holes { profile: 0 })
    );
    assert_eq!(
        loft(&flat, None, LoftSkin::Ruled, FEATURE),
        Err(LoftError::Coincident { profiles: [0, 1] })
    );
    assert_eq!(
        loft(&along, None, LoftSkin::Ruled, FEATURE),
        Err(LoftError::AlongProfile { profile: 0 })
    );
    assert_eq!(
        loft(&along[..1], None, LoftSkin::Ruled, FEATURE),
        Err(LoftError::TooFewProfiles)
    );
}

#[test]
fn a_loft_can_be_cancelled_anywhere() {
    let profiles = [
        profile(level(0.0), &square(1, 4.0)),
        profile(level(5.0), &[circle(11, (0.0, 0.0), 2.0)]),
        profile(level(9.0), &square(21, 3.0)),
    ];

    assert_cancelled_anywhere(
        "loft",
        || loft(&profiles, None, LoftSkin::Smooth, FEATURE),
        |error| matches!(error, LoftError::Cancelled(_)),
    );
}
