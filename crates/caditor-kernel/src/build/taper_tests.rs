use std::{collections::BTreeSet, f64::consts::PI};

use caditor_geometry::{Plane, Point3, Vector3};

use super::*;
use crate::{
    naming::FaceName,
    profile::{Profile, ProfileCurve, Region, Selection},
    surface::Surface,
    test_support::{arc, assert_watertight, circle, line, rectangle, spline},
    tolerance::SamplingTolerance,
    topology::Solid,
};

const FEATURE: u64 = 7;

fn regions(curves: &[ProfileCurve]) -> Vec<Region> {
    Profile::new(curves)
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap()
}

fn volume(solid: &Solid) -> f64 {
    assert_eq!(solid.validate(), Ok(()));
    let mesh = solid
        .tessellate(&SamplingTolerance::new(1e-3, 0.05).unwrap())
        .unwrap();
    assert_watertight("tapered", &mesh);
    mesh.mass_properties().volume
}

fn frustum(height: f64, bottom: f64, top: f64) -> f64 {
    height / 3.0 * (bottom + top + (bottom * top).sqrt())
}

fn assert_near(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 2e-3 * expected,
        "{actual} vs {expected}"
    );
}

#[test]
fn a_tapered_square_is_a_frustum_of_a_pyramid() {
    let square = regions(&rectangle(1, (-5.0, -5.0), (5.0, 5.0)));
    let angle = 10.0_f64.to_radians();

    let solid = extrude_tapered(
        &Plane::XY,
        &square,
        LinearExtent::one_side(4.0).unwrap(),
        angle,
        FEATURE,
    )
    .unwrap();

    let top = 10.0 - 2.0 * 4.0 * angle.tan();
    assert_near(volume(&solid), frustum(4.0, 100.0, top * top));
    assert_eq!(solid.faces().count(), 6);
    let names: BTreeSet<FaceName> = solid.faces().map(|(_, face)| face.name()).collect();
    assert_eq!(names.len(), 6);
    assert!(names.contains(&FaceName::end_cap(FEATURE, square[0].key())));
}

#[test]
fn a_negative_taper_draws_out_and_a_reversed_extrusion_tapers_downwards() {
    let square = regions(&rectangle(1, (-5.0, -5.0), (5.0, 5.0)));
    let angle = 15.0_f64.to_radians();

    let solid = extrude_tapered(
        &Plane::XY,
        &square,
        LinearExtent::one_side(-3.0).unwrap(),
        -angle,
        FEATURE,
    )
    .unwrap();

    let bottom = 10.0 + 2.0 * 3.0 * angle.tan();
    assert_near(volume(&solid), frustum(3.0, 100.0, bottom * bottom));
    let lowest = solid
        .vertices()
        .map(|(_, vertex)| vertex.point())
        .fold(f64::INFINITY, |low, point| low.min(point.z));
    assert!((lowest + 3.0).abs() < 1e-9);
}

#[test]
fn a_tapered_circle_is_a_frustum_of_a_cone() {
    let disc = regions(&[circle(1, (2.0, 1.0), 6.0)]);
    let angle = 20.0_f64.to_radians();

    let solid = extrude_tapered(
        &Plane::XY,
        &disc,
        LinearExtent::one_side(5.0).unwrap(),
        angle,
        FEATURE,
    )
    .unwrap();

    let top = 6.0 - 5.0 * angle.tan();
    assert_near(
        volume(&solid),
        PI * 5.0 / 3.0 * (36.0 + 6.0 * top + top * top),
    );
    assert!(
        solid
            .faces()
            .any(|(_, face)| matches!(face.surface(), Surface::Cone(_)))
    );
}

#[test]
fn a_symmetric_taper_narrows_both_ways_from_the_sketch() {
    let disc = regions(&[circle(1, (0.0, 0.0), 4.0)]);
    let angle = 10.0_f64.to_radians();

    let solid = extrude_tapered(
        &Plane::XY,
        &disc,
        LinearExtent::symmetric(6.0).unwrap(),
        angle,
        FEATURE,
    )
    .unwrap();

    let end = 4.0 - 3.0 * angle.tan();
    let half = PI * 3.0 / 3.0 * (16.0 + 4.0 * end + end * end);
    assert_near(volume(&solid), 2.0 * half);
    let side = FaceName::side(FEATURE, disc[0].outer().pieces()[0].id());
    let behind = FaceName::side_behind(FEATURE, disc[0].outer().pieces()[0].id());
    let names: BTreeSet<FaceName> = solid.faces().map(|(_, face)| face.name()).collect();
    assert!(names.contains(&side) && names.contains(&behind));
}

#[test]
fn a_taper_from_a_start_offset_starts_from_the_profile_as_drawn() {
    let square = regions(&rectangle(1, (0.0, 0.0), (8.0, 8.0)));
    let angle = 5.0_f64.to_radians();

    let solid = extrude_tapered(
        &Plane::XY,
        &square,
        LinearExtent::new(2.0, 6.0).unwrap(),
        angle,
        FEATURE,
    )
    .unwrap();

    let top = 8.0 - 2.0 * 4.0 * angle.tan();
    assert_near(volume(&solid), frustum(4.0, 64.0, top * top));
}

#[test]
fn a_rounded_slot_with_a_hole_tapers_with_curved_corners() {
    let curves = vec![
        line(1, (0.0, 0.0), (10.0, 0.0)),
        arc(2, (10.0, 3.0), (10.0, 0.0), (10.0, 6.0)),
        line(3, (10.0, 6.0), (0.0, 6.0)),
        arc(4, (0.0, 3.0), (0.0, 6.0), (0.0, 0.0)),
        circle(5, (5.0, 3.0), 1.0),
    ];
    let slot = regions(&curves);
    let angle = 8.0_f64.to_radians();

    let solid = extrude_tapered(
        &Plane::XY,
        &slot,
        LinearExtent::one_side(2.0).unwrap(),
        angle,
        FEATURE,
    )
    .unwrap();

    let inset = 2.0 * angle.tan();
    let area = |inset: f64| {
        let radius = 3.0 - inset;
        10.0 * 2.0 * radius + PI * radius * radius - PI * (1.0 + inset).powi(2)
    };
    let expected = 2.0 / 6.0 * (area(0.0) + 4.0 * area(0.5 * inset) + area(inset));
    assert_near(volume(&solid), expected);
}

#[test]
fn a_tapered_corner_between_a_line_and_an_arc_follows_both_faces() {
    let curves = vec![
        line(1, (0.0, 0.0), (6.0, 0.0)),
        arc(2, (0.0, 0.0), (6.0, 0.0), (0.0, 6.0)),
        line(3, (0.0, 6.0), (0.0, 0.0)),
    ];
    let quarter = regions(&curves);
    let angle = 6.0_f64.to_radians();

    let solid = extrude_tapered(
        &Plane::XY,
        &quarter,
        LinearExtent::one_side(3.0).unwrap(),
        angle,
        FEATURE,
    )
    .unwrap();

    let measured = volume(&solid);
    let full = 0.25 * PI * 36.0 * 3.0;
    assert!(measured < full && measured > 0.7 * full, "{measured}");
    assert!(
        solid
            .edges()
            .any(|(_, edge)| matches!(edge.curve(), crate::curve::Curve::Intersection(_)))
    );
}

#[test]
fn a_taper_closing_the_profile_before_the_end_is_refused() {
    let square = regions(&rectangle(1, (0.0, 0.0), (4.0, 4.0)));

    let error = extrude_tapered(
        &Plane::XY,
        &square,
        LinearExtent::one_side(10.0).unwrap(),
        30.0_f64.to_radians(),
        FEATURE,
    )
    .unwrap_err();

    assert!(matches!(error, SweepError::TaperCloses { .. }), "{error:?}");

    let disc = regions(&[circle(1, (0.0, 0.0), 2.0)]);
    let error = extrude_tapered(
        &Plane::XY,
        &disc,
        LinearExtent::one_side(10.0).unwrap(),
        30.0_f64.to_radians(),
        FEATURE,
    )
    .unwrap_err();
    assert_eq!(error, SweepError::TaperCloses { entities: vec![1] });
}

#[test]
fn a_taper_refuses_splines_tilted_ends_and_steep_angles() {
    let blob = regions(&[spline(
        3,
        &[(0.0, 0.0), (6.0, -1.0), (7.0, 5.0), (1.0, 6.0), (0.0, 0.0)],
    )]);
    let extent = LinearExtent::one_side(2.0).unwrap();
    assert_eq!(
        extrude_tapered(&Plane::XY, &blob, extent, 0.1, FEATURE),
        Err(SweepError::TaperedSpline { entities: vec![3] })
    );

    let square = regions(&rectangle(1, (0.0, 0.0), (4.0, 4.0)));
    let tilted = Plane::new(Point3::new(0.0, 0.0, 3.0), Vector3::new(0.3, 0.0, 1.0)).unwrap();
    let up_to =
        LinearExtent::between(LinearBound::Offset(0.0), LinearBound::Plane(tilted)).unwrap();
    assert_eq!(
        extrude_tapered(&Plane::XY, &square, up_to, 0.1, FEATURE),
        Err(SweepError::TaperedTiltedEnd)
    );
    assert_eq!(
        extrude_tapered(&Plane::XY, &square, extent, 89.5_f64.to_radians(), FEATURE),
        Err(SweepError::TaperTooSteep)
    );
}

#[test]
fn no_taper_is_the_plain_extrusion() {
    let square = regions(&rectangle(1, (0.0, 0.0), (4.0, 4.0)));
    let extent = LinearExtent::one_side(2.0).unwrap();

    assert_eq!(
        extrude_tapered(&Plane::XY, &square, extent, 0.0, FEATURE),
        extrude(&Plane::XY, &square, extent, FEATURE)
    );
}
