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
        wall_regions(
            &[spline(4, &[(0.0, 0.0), (3.0, 2.0), (6.0, 0.0)])],
            1.0,
            WallSide::Centred
        ),
        Err(WallError::UnsupportedCurve { entities: vec![4] })
    );
    assert_eq!(
        wall_regions(
            &[ProfileCurve::ellipse(
                6,
                Point2::ZERO,
                Vector2::new(4.0, 0.0),
                2.0
            )],
            0.5,
            WallSide::Inside
        ),
        Err(WallError::UnsupportedCurve { entities: vec![6] })
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
