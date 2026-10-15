use std::{collections::BTreeSet, f64::consts::PI};

use caditor_geometry::{Plane, Point2, Vector2};

use super::*;
use crate::{
    build::{AngularExtent, Axis2, LinearExtent, extrude, extrude_tapered, revolve},
    naming::FaceName,
    test_support::{arc, assert_watertight, circle, line, rectangle, spline},
    tolerance::SamplingTolerance,
    topology::Solid,
};

const FEATURE: u64 = 9;

fn l_shape() -> Vec<ProfileCurve> {
    vec![
        line(2, (20.0, 0.0), (0.0, 0.0)),
        line(1, (0.0, 10.0), (0.0, 0.0)),
    ]
}

fn area(regions: &[Region]) -> f64 {
    regions.iter().map(Region::area).sum()
}

fn volume(solid: &Solid) -> f64 {
    assert_eq!(solid.validate(), Ok(()));
    let mesh = solid
        .tessellate(&SamplingTolerance::new(1e-3, 0.05).unwrap())
        .unwrap();
    assert_watertight("wall", &mesh);
    mesh.mass_properties().volume
}

fn assert_near(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 2e-3 * expected.abs(),
        "{actual} vs {expected}"
    );
}

#[test]
fn an_l_shaped_chain_takes_its_wall_inside_outside_or_centred() {
    let curves = l_shape();

    let centred = wall_regions(&curves, 2.0, WallSide::Centred).unwrap();
    let inside = wall_regions(&curves, 2.0, WallSide::Inside).unwrap();
    let outside = wall_regions(&curves, 2.0, WallSide::Outside).unwrap();

    assert_eq!(centred.len(), 1);
    assert!((area(&centred) - 60.0).abs() < 1e-9);
    assert!((area(&inside) - 56.0).abs() < 1e-9);
    assert!((area(&outside) - 64.0).abs() < 1e-9);
    assert!(inside[0].contains(Point2::new(1.0, 5.0)));
    assert!(outside[0].contains(Point2::new(-1.0, 5.0)));

    let solid = extrude(
        &Plane::XY,
        &centred,
        LinearExtent::one_side(5.0).unwrap(),
        FEATURE,
    )
    .unwrap();
    assert_near(volume(&solid), 300.0);
    assert_eq!(solid.faces().count(), 8);
    let names: BTreeSet<FaceName> = solid.faces().map(|(_, face)| face.name()).collect();
    assert_eq!(names.len(), 8);
}

#[test]
fn a_wall_keeps_its_names_when_the_chain_is_drawn_the_other_way() {
    let reversed = vec![
        line(2, (0.0, 0.0), (20.0, 0.0)),
        line(1, (0.0, 0.0), (0.0, 10.0)),
    ];
    let names = |curves: &[ProfileCurve]| -> BTreeSet<FaceName> {
        let regions = wall_regions(curves, 2.0, WallSide::Inside).unwrap();
        let solid = extrude(
            &Plane::XY,
            &regions,
            LinearExtent::one_side(1.0).unwrap(),
            FEATURE,
        )
        .unwrap();
        solid
            .faces()
            .map(|(_, face)| face.name())
            .filter(|name| {
                *name != FaceName::start_cap(FEATURE, regions[0].key())
                    && *name != FaceName::end_cap(FEATURE, regions[0].key())
            })
            .collect()
    };

    let flipped = names(&reversed);

    assert_eq!(flipped.len(), 6);
    assert_eq!(
        area(&wall_regions(&reversed, 2.0, WallSide::Inside).unwrap()),
        56.0
    );
    assert_ne!(flipped, names(&l_shape()));
}

#[test]
fn a_closed_outline_becomes_a_ring_and_a_circle_a_tube() {
    let square = rectangle(1, (0.0, 0.0), (10.0, 10.0));

    let inside = wall_regions(&square, 1.0, WallSide::Inside).unwrap();
    let outside = wall_regions(&square, 1.0, WallSide::Outside).unwrap();

    assert_eq!(inside.len(), 1);
    assert_eq!(inside[0].holes().len(), 1);
    assert!((area(&inside) - (100.0 - 64.0)).abs() < 1e-9);
    assert!((area(&outside) - (144.0 - 100.0)).abs() < 1e-9);

    let tube = wall_regions(&[circle(5, (1.0, 2.0), 3.0)], 0.5, WallSide::Centred).unwrap();
    assert!((area(&tube) - PI * (3.25_f64.powi(2) - 2.75_f64.powi(2))).abs() < 1e-9);
    let solid = extrude(
        &Plane::XY,
        &tube,
        LinearExtent::one_side(4.0).unwrap(),
        FEATURE,
    )
    .unwrap();
    assert_near(volume(&solid), 4.0 * area(&tube));
}

#[test]
fn a_chain_of_arcs_and_lines_walls_like_a_bent_strip() {
    let curves = vec![
        line(1, (0.0, 0.0), (10.0, 0.0)),
        arc(2, (10.0, 5.0), (10.0, 0.0), (15.0, 5.0)),
        line(3, (15.0, 5.0), (15.0, 12.0)),
    ];
    let length = 10.0 + 0.5 * PI * 5.0 + 7.0;

    let strip = wall_regions(&curves, 1.0, WallSide::Centred).unwrap();

    assert!((area(&strip) - length).abs() < 1e-9);
    let solid = extrude(
        &Plane::XY,
        &strip,
        LinearExtent::one_side(2.0).unwrap(),
        FEATURE,
    )
    .unwrap();
    assert_near(volume(&solid), 2.0 * length);

    let tapered = extrude_tapered(
        &Plane::XY,
        &strip,
        LinearExtent::one_side(1.0).unwrap(),
        0.1,
        FEATURE,
    )
    .unwrap();
    assert!(volume(&tapered) < length);
}

#[test]
fn a_wall_revolves_into_a_thin_cup() {
    let curves = vec![
        line(1, (0.0, 0.0), (4.0, 0.0)),
        line(2, (4.0, 0.0), (4.0, 6.0)),
    ];
    let wall = wall_regions(&curves, 0.5, WallSide::Inside).unwrap();
    let axis = Axis2::new(Point2::ZERO, Vector2::Y).unwrap();

    let cup = revolve(&Plane::XY, &wall, axis, AngularExtent::full(), FEATURE).unwrap();

    let base = PI * 3.5 * 3.5 * 0.5;
    let side = PI * (16.0 - 12.25) * 6.0;
    assert_near(volume(&cup), base + side);
}

#[test]
fn walls_that_cannot_be_built_say_why() {
    let branching = vec![
        line(1, (0.0, 0.0), (10.0, 0.0)),
        line(2, (0.0, 0.0), (0.0, 10.0)),
        line(3, (0.0, 0.0), (-5.0, -5.0)),
    ];
    assert_eq!(
        wall_regions(&branching, 1.0, WallSide::Centred),
        Err(WallError::Branches {
            entities: vec![1, 2, 3]
        })
    );
    assert_eq!(
        wall_regions(&[circle(5, (0.0, 0.0), 1.0)], 2.0, WallSide::Inside),
        Err(WallError::TooThick { entities: vec![5] })
    );
    assert_eq!(
        wall_regions(&l_shape(), 0.0, WallSide::Inside),
        Err(WallError::NotPositive)
    );
    let hairpin = vec![
        line(1, (0.0, 0.0), (10.0, 0.0)),
        line(2, (10.0, 0.0), (10.0, 1.0)),
        line(3, (10.0, 1.0), (0.0, 1.0)),
    ];
    assert!(matches!(
        wall_regions(&hairpin, 3.0, WallSide::Inside),
        Err(WallError::CrossesItself | WallError::TooThick { .. })
    ));
    let nearly_closed = vec![
        line(1, (0.0, 0.0), (10.0, 0.0)),
        line(2, (10.0, 0.0), (10.0, 10.0)),
        line(3, (10.0, 10.0), (0.0, 10.0)),
        line(4, (0.0, 10.0), (0.0, 1.0)),
    ];
    assert_eq!(
        wall_regions(&nearly_closed, 3.0, WallSide::Centred),
        Err(WallError::CrossesItself)
    );
}

fn drawn_length(curve: &ProfileCurve) -> f64 {
    let (path, range) = curve.curve().unwrap();
    path.length(range)
}

fn ellipse_arc(entity: u64) -> ProfileCurve {
    ProfileCurve::elliptical_arc(
        entity,
        Point2::new(1.0, 2.0),
        Vector2::new(8.0, 0.0),
        4.0,
        (Point2::new(9.0, 2.0), Point2::new(-7.0, 2.0)),
    )
}

#[test]
fn a_spline_walls_like_a_strip_as_wide_as_asked_along_its_whole_length() {
    let curve = spline(
        4,
        &[
            (0.0, 0.0),
            (6.0, 5.0),
            (12.0, -3.0),
            (20.0, 4.0),
            (26.0, 0.0),
        ],
    );
    let length = drawn_length(&curve);

    let sides = [WallSide::Inside, WallSide::Outside]
        .map(|side| wall_regions(std::slice::from_ref(&curve), 1.5, side).unwrap());
    assert!((area(&sides[0]) + area(&sides[1]) - 3.0 * length).abs() < 2e-3 * length);

    for side in [WallSide::Centred, WallSide::Inside, WallSide::Outside] {
        let strip = wall_regions(std::slice::from_ref(&curve), 1.5, side).unwrap();

        assert_eq!(strip.len(), 1);
        if side == WallSide::Centred {
            assert!((area(&strip) - 1.5 * length).abs() < 1e-3 * length);
        }
        let solid = extrude(
            &Plane::XY,
            &strip,
            LinearExtent::one_side(2.0).unwrap(),
            FEATURE,
        )
        .unwrap();
        assert_near(volume(&solid), 2.0 * area(&strip));
    }

    let centred = wall_regions(std::slice::from_ref(&curve), 1.5, WallSide::Centred).unwrap();
    let (path, range) = curve.curve().unwrap();
    let middle = path.point(range.middle());
    assert!(centred[0].contains(middle));
    assert!(!centred[0].contains(middle + Vector2::new(0.0, 4.0)));
}

#[test]
fn an_elliptical_arc_walls_to_either_side_and_a_whole_ellipse_becomes_a_ring() {
    let arc = ellipse_arc(7);
    let length = drawn_length(&arc);

    let sides = [WallSide::Inside, WallSide::Outside]
        .map(|side| wall_regions(std::slice::from_ref(&arc), 1.0, side).unwrap());
    assert!((area(&sides[0]) + area(&sides[1]) - 2.0 * length).abs() < 2e-3 * length);

    for side in [WallSide::Centred, WallSide::Inside, WallSide::Outside] {
        let strip = wall_regions(std::slice::from_ref(&arc), 1.0, side).unwrap();

        if side == WallSide::Centred {
            assert!((area(&strip) - length).abs() < 1e-3 * length);
        }
        let solid = extrude(
            &Plane::XY,
            &strip,
            LinearExtent::one_side(3.0).unwrap(),
            FEATURE,
        )
        .unwrap();
        assert_near(volume(&solid), 3.0 * area(&strip));
    }

    let whole = ProfileCurve::ellipse(8, Point2::ZERO, Vector2::new(6.0, 0.0), 3.0);
    let perimeter = drawn_length(&whole);
    let rings = [WallSide::Inside, WallSide::Outside]
        .map(|side| wall_regions(std::slice::from_ref(&whole), 0.5, side).unwrap());
    assert!((area(&rings[0]) + area(&rings[1]) - perimeter).abs() < 2e-3 * perimeter);

    let ring = wall_regions(std::slice::from_ref(&whole), 0.5, WallSide::Centred).unwrap();
    assert_eq!(ring.len(), 1);
    assert_eq!(ring[0].holes().len(), 1);
    assert!((area(&ring) - 0.5 * perimeter).abs() < 2e-3 * perimeter);
    let pipe = extrude(
        &Plane::XY,
        &ring,
        LinearExtent::one_side(2.0).unwrap(),
        FEATURE,
    )
    .unwrap();
    assert_near(volume(&pipe), 2.0 * area(&ring));
}

#[test]
fn a_spline_meeting_a_line_or_an_arc_smoothly_or_at_a_corner_keeps_its_wall_whole() {
    let curve = spline(2, &[(10.0, 0.0), (14.0, 6.0), (20.0, 8.0), (26.0, 4.0)]);
    let length = drawn_length(&curve);

    let corner = wall_regions(
        &[line(1, (0.0, 0.0), (10.0, 0.0)), curve.clone()],
        1.0,
        WallSide::Centred,
    )
    .unwrap();
    assert_eq!(corner.len(), 1);
    assert!((area(&corner) - (10.0 + length)).abs() < 1.0);
    assert!(corner[0].contains(Point2::new(5.0, 0.2)));
    assert!(corner[0].contains(Point2::new(10.1, 0.5)));

    let (path, range) = curve.curve().unwrap();
    let ahead = path.evaluate(range.end()).first.normalize();
    let end = path.point(range.end());
    let carried = vec![
        curve,
        line(
            3,
            (end.x, end.y),
            (end.x + 9.0 * ahead.x, end.y + 9.0 * ahead.y),
        ),
    ];
    let smooth = wall_regions(&carried, 1.0, WallSide::Centred).unwrap();
    assert_eq!(smooth.len(), 1);
    assert!((area(&smooth) - (length + 9.0)).abs() < 1e-3 * (length + 9.0));
    let solid = extrude(
        &Plane::XY,
        &smooth,
        LinearExtent::one_side(2.0).unwrap(),
        FEATURE,
    )
    .unwrap();
    assert_near(volume(&solid), 2.0 * area(&smooth));
}

#[test]
fn a_curve_wall_too_thick_for_its_bend_or_meeting_another_curve_sharply_says_so() {
    let whole = ProfileCurve::ellipse(6, Point2::ZERO, Vector2::new(4.0, 0.0), 2.0);
    assert!(matches!(
        wall_regions(std::slice::from_ref(&whole), 3.0, WallSide::Inside),
        Err(WallError::TooThick { entities }) if entities == vec![6]
    ));

    let first = spline(1, &[(0.0, 0.0), (4.0, 3.0), (8.0, 0.0)]);
    let second = spline(2, &[(8.0, 0.0), (9.0, 4.0), (8.0, 8.0)]);
    assert_eq!(
        wall_regions(&[first, second], 1.0, WallSide::Centred),
        Err(WallError::Apart {
            entities: vec![1, 2]
        })
    );
}
