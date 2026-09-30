use std::{collections::BTreeSet, f64::consts::FRAC_PI_2};

use caditor_geometry::{Plane, Point3, RigidTransform, Vector3};

use super::*;
use crate::{
    build::{LinearExtent, extrude},
    naming::{FaceOrigin, FaceReference},
    profile::{Profile, Selection},
    test_support::{assert_cancelled_anywhere, assert_watertight, circle, rectangle},
    tolerance::SamplingTolerance,
};

const FEATURE: u64 = 9;

fn block(min: (f64, f64), max: (f64, f64), height: f64) -> Solid {
    let regions = Profile::new(&rectangle(1, min, max))
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

fn along_x(spacing: f64, count: u32) -> Vec<PatternCopy> {
    (1..count)
        .map(|step| PatternCopy {
            index: [step, 0],
            placement: RigidTransform::translation(Vector3::X * spacing * f64::from(step)).unwrap(),
        })
        .collect()
}

fn names(solid: &Solid) -> BTreeSet<FaceName> {
    solid.faces().map(|(_, face)| face.name()).collect()
}

#[test]
fn copies_standing_apart_become_lumps_named_after_the_copy() {
    let original = block((0.0, 0.0), (10.0, 10.0), 5.0);

    let patterned = pattern(&original, &along_x(20.0, 3), FEATURE).unwrap();

    check("apart", &patterned, 3.0 * 500.0);
    assert_eq!(patterned.faces().count(), 18);
    assert_eq!(patterned.shells().count(), 3);
    let expected: BTreeSet<FaceName> = names(&original)
        .into_iter()
        .flat_map(|name| {
            [
                name,
                FaceName::pattern(FEATURE, [1, 0], name),
                FaceName::pattern(FEATURE, [2, 0], name),
            ]
        })
        .collect();
    assert_eq!(names(&patterned), expected);
    assert!(patterned.faces().all(|(_, face)| {
        original
            .faces()
            .any(|(_, source)| source.origin() == face.origin())
    }));
}

#[test]
fn overlapping_copies_join_into_one_lump() {
    let original = block((0.0, 0.0), (10.0, 10.0), 5.0);

    let patterned = pattern(&original, &along_x(6.0, 4), FEATURE).unwrap();

    check("overlapping", &patterned, 28.0 * 10.0 * 5.0);
    assert_eq!(patterned.shells().count(), 1);
}

#[test]
fn copies_turned_about_an_axis_keep_their_names_when_the_count_changes() {
    let regions = Profile::new(&[circle(1, (20.0, 0.0), 4.0)])
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap();
    let original = extrude(
        &Plane::XY,
        &regions,
        LinearExtent::one_side(3.0).unwrap(),
        1,
    )
    .unwrap();
    let turned = |count: u32| -> Vec<PatternCopy> {
        (1..count)
            .map(|step| PatternCopy {
                index: [step, 0],
                placement: RigidTransform::rotation_about(
                    Point3::ZERO,
                    Vector3::Z,
                    FRAC_PI_2 * f64::from(step),
                )
                .unwrap(),
            })
            .collect()
    };

    let three = pattern(&original, &turned(3), FEATURE).unwrap();
    let four = pattern(&original, &turned(4), FEATURE).unwrap();

    let disc = std::f64::consts::PI * 16.0 * 3.0;
    check("three", &three, 3.0 * disc);
    check("four", &four, 4.0 * disc);
    let top = original
        .faces()
        .find(|(_, face)| face.origin() == Some(FaceOrigin::EndCap { feature: 1 }))
        .map(|(_, face)| face.name())
        .unwrap();
    let copied_top = FaceName::pattern(FEATURE, [2, 0], top);
    let in_three = three
        .faces()
        .find(|(_, face)| face.name() == copied_top)
        .map(|(id, _)| id)
        .unwrap();
    let reference = FaceReference::capture(&three, in_three).unwrap();
    let found = reference.resolve(&four).unwrap();
    assert_eq!(four.face(found).unwrap().name(), copied_top);
}

#[test]
fn a_pattern_can_be_cancelled_anywhere() {
    let original = block((0.0, 0.0), (10.0, 10.0), 5.0);
    let copies = along_x(6.0, 3);

    assert_cancelled_anywhere(
        "pattern",
        || pattern(&original, &copies, FEATURE),
        |error| matches!(error, PatternError::Cancelled(_)),
    );
}
