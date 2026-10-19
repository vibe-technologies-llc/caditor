use caditor_geometry::{Plane, Point2};

use super::GearPair;
use crate::{
    EntityId, Sketch,
    curve::Faceting,
    gear::{GearCentre, GearError, GearOutline, SpurGear},
};

const MODULE: f64 = 2.0;
const DEPTH: f64 = 5e-3;

fn driver(teeth: u32) -> SpurGear {
    SpurGear {
        module: MODULE,
        teeth,
        pressure_angle: 20.0,
        profile_shift: 0.0,
        root_fillet: 0.5,
        bore: 0.0,
    }
}

fn pair(teeth: u32, partner_teeth: u32) -> GearPair {
    GearPair {
        driver: driver(teeth),
        partner_teeth,
        partner_bore: 0.0,
        direction: 0.0,
    }
}

fn polygon(outline: &GearOutline) -> Vec<Point2> {
    outline
        .pieces
        .iter()
        .flat_map(|piece| {
            let mut points = piece.faceted(Faceting::within(2e-3));
            points.pop();
            points
        })
        .collect()
}

fn inside(polygon: &[Point2], point: Point2) -> bool {
    let mut inside = false;
    for (a, b) in polygon.iter().zip(polygon.iter().cycle().skip(1)) {
        if (a.y > point.y) != (b.y > point.y)
            && point.x < a.x + (point.y - a.y) / (b.y - a.y) * (b.x - a.x)
        {
            inside = !inside;
        }
    }
    inside
}

fn distance_to(polygon: &[Point2], point: Point2) -> f64 {
    polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .map(|(a, b)| {
            let along = *b - *a;
            let share = ((point - *a).dot(along) / along.length_squared()).clamp(0.0, 1.0);
            point.distance(*a + along * share)
        })
        .fold(f64::INFINITY, f64::min)
}

fn deepest_overlap(first: &[Point2], second: &[Point2]) -> f64 {
    first
        .iter()
        .filter(|point| inside(second, **point))
        .map(|point| distance_to(second, *point))
        .fold(0.0, f64::max)
}

fn closest(first: &[Point2], second: &[Point2]) -> f64 {
    first
        .iter()
        .map(|point| distance_to(second, *point))
        .fold(f64::INFINITY, f64::min)
}

fn meshed(pair: &GearPair, turn: f64) -> (Vec<Point2>, Vec<Point2>) {
    let centre = Point2::new(3.0, -4.0);
    let driver = pair.driver.outline_turned(centre, turn).unwrap();
    let partner = pair
        .partner()
        .outline_turned(pair.partner_centre(centre), pair.partner_turn(turn))
        .unwrap();
    (polygon(&driver), polygon(&partner))
}

#[test]
fn the_centres_are_module_times_half_the_teeth_apart_along_x() {
    for (teeth, partner) in [(20, 30), (17, 17), (24, 60)] {
        let pair = pair(teeth, partner);
        let outlines = pair.outlines(Point2::new(5.0, 7.0)).unwrap();

        let apart = outlines.partner.centre - outlines.driver.centre;

        assert!((apart.x - MODULE * f64::from(teeth + partner) / 2.0).abs() < 1e-12);
        assert_eq!(apart.y, 0.0);
        assert!(
            (outlines.driver.circles.pitch + outlines.partner.circles.pitch - apart.x).abs()
                < 1e-12
        );
    }
}

#[test]
fn the_teeth_mesh_touching_without_overlapping_in_every_turn_of_the_pair() {
    for (teeth, partner) in [(20, 30), (24, 24), (18, 45), (40, 19)] {
        let pair = pair(teeth, partner);
        let step = std::f64::consts::TAU / f64::from(teeth);
        for share in [0.0, 0.13, 0.5, 0.77] {
            let (first, second) = meshed(&pair, step * share);

            assert!(
                deepest_overlap(&first, &second) < DEPTH,
                "{teeth}:{partner} at {share}: {}",
                deepest_overlap(&first, &second)
            );
            assert!(
                deepest_overlap(&second, &first) < DEPTH,
                "{teeth}:{partner} at {share}"
            );
            assert!(
                closest(&first, &second) < DEPTH,
                "{teeth}:{partner} at {share}: {}",
                closest(&first, &second)
            );
        }
    }
}

#[test]
fn turned_a_tooth_off_the_mesh_the_gears_would_collide() {
    let pair = pair(20, 30);
    let centre = Point2::ZERO;
    let driver = pair.driver.outline_turned(centre, 0.0).unwrap();
    let wrong = pair
        .partner()
        .outline_turned(pair.partner_centre(centre), pair.partner_turn(0.0) + 0.1)
        .unwrap();

    assert!(deepest_overlap(&polygon(&driver), &polygon(&wrong)) > DEPTH);
}

#[test]
fn the_partner_takes_the_opposite_shift_so_the_standard_distance_still_meshes() {
    let pair = GearPair {
        driver: SpurGear {
            profile_shift: 0.3,
            ..driver(14)
        },
        partner_teeth: 40,
        partner_bore: 6.0,
        direction: 0.0,
    };
    let partner = pair.partner();
    let (first, second) = meshed(&pair, 0.0);

    assert_eq!(partner.profile_shift, -0.3);
    assert_eq!(partner.module, MODULE);
    assert_eq!(partner.bore, 6.0);
    assert!(deepest_overlap(&first, &second) < DEPTH);
    assert!(deepest_overlap(&second, &first) < DEPTH);
}

#[test]
fn a_partner_that_cannot_be_cut_is_refused_naming_it() {
    let error = pair(40, 10).outlines(Point2::ZERO).unwrap_err();

    let GearError::Partner(inner) = &error else {
        panic!("{error:?}");
    };
    assert!(matches!(**inner, GearError::Undercut { teeth: 10, .. }));
    assert!(
        error
            .to_string()
            .starts_with("the meshing gear: 10 teeth are undercut")
    );
}

#[test]
fn a_drawn_pair_is_two_closed_profiles_with_their_own_circles_and_a_centre_each() {
    let pair = GearPair {
        partner_bore: 8.0,
        ..pair(18, 27)
    };
    let mut sketch = Sketch::new(Plane::XY);

    let drawn = sketch
        .add_gear_pair(&pair, GearCentre::Point(EntityId::ORIGIN))
        .unwrap();

    assert!(sketch.open_ends().is_empty());
    assert_eq!(drawn.driver.circles.len(), 4);
    assert_eq!(drawn.partner.circles.len(), 4);
    assert!(drawn.driver.bore.is_none());
    assert!(drawn.partner.bore.is_some());
    assert_eq!(sketch.point(drawn.driver.centre), Some(Point2::ZERO));
    assert_eq!(
        sketch.point(drawn.partner.centre),
        Some(Point2::new(MODULE * 45.0 / 2.0, 0.0))
    );
    assert_eq!(sketch.constraints().len(), 1);
}
