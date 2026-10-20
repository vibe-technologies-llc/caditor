use caditor_geometry::{Point3, Vector3};

use super::{
    tests::{edge_through, fillet, run, spandrel},
    *,
};
use crate::{
    boolean::{BooleanOperation, boolean},
    fixtures::cuboid,
    naming::FaceOrigin,
    test_support::assert_watertight,
    tolerance::SamplingTolerance,
};

const EARLIER: f64 = 2.0;
const HEIGHT: f64 = 20.0;
const RISE: f64 = HEIGHT - EARLIER;
const QUADRATURE_CELLS: usize = 600;
const VOLUME_TOLERANCE: f64 = 0.02;

fn rim_edges(solid: &Solid, (x, y): (f64, f64), z: f64) -> Vec<EdgeId> {
    vec![
        edge_through(solid, (x + 20.0, y, z)),
        edge_through(solid, (x + 40.0, y + 15.0, z)),
        edge_through(solid, (x + 20.0, y + 30.0, z)),
        edge_through(solid, (x, y + 15.0, z)),
    ]
}

fn uprights(solid: &Solid, (x, y): (f64, f64), z: f64) -> Vec<EdgeId> {
    [(0.0, 0.0), (40.0, 0.0), (40.0, 30.0), (0.0, 30.0)]
        .map(|(dx, dy)| edge_through(solid, (x + dx, y + dy, z)))
        .to_vec()
}

fn top_rounded_box() -> Solid {
    let solid = cuboid(Vector3::new(40.0, 30.0, HEIGHT));
    let top = rim_edges(&solid, (0.0, 0.0), HEIGHT);
    run(&solid, &top, fillet(EARLIER))
}

fn under_the_rounds(x: f64) -> f64 {
    if x < EARLIER {
        (EARLIER * EARLIER - (x - EARLIER).powi(2)).max(0.0).sqrt()
    } else {
        EARLIER
    }
}

fn corner_removed(size: f64, removed: impl Fn(f64, f64) -> bool) -> f64 {
    let cell = size / QUADRATURE_CELLS as f64;
    let middles = (0..QUADRATURE_CELLS).map(|index| (index as f64 + 0.5) * cell);
    let half: f64 = middles
        .clone()
        .flat_map(|x| middles.clone().map(move |y| (x, y)))
        .filter(|(x, y)| x <= y && removed(*x, *y))
        .map(|(x, _)| under_the_rounds(x) * cell * cell)
        .sum();
    2.0 * half
}

fn rounded_off(size: f64) -> f64 {
    let corner = corner_removed(size, |x, y| {
        (x - size).powi(2) + (y - size).powi(2) > size * size
    });
    4.0 * (spandrel(size) * RISE + corner)
}

fn bevelled_off(size: f64) -> f64 {
    let corner = corner_removed(size, |x, y| x + y < size);
    4.0 * (size * size / 2.0 * RISE + corner)
}

fn ball_corner_off() -> f64 {
    let under_the_mitre = 16.0 / 3.0;
    let ball_octant = std::f64::consts::PI * EARLIER.powi(3) / 6.0;
    4.0 * (spandrel(EARLIER) * RISE + under_the_mitre - ball_octant)
}

fn volume(solid: &Solid) -> f64 {
    solid
        .tessellate(&SamplingTolerance::new(1e-5, 0.005).unwrap())
        .unwrap()
        .mass_properties()
        .volume
}

fn assert_valid(name: &str, solid: &Solid) {
    assert_eq!(solid.validate(), Ok(()), "{name}");
    assert_watertight(name, &solid.tessellate(&solid.default_tolerance()).unwrap());
}

fn assert_change(name: &str, before: &Solid, after: &Solid, expected: f64) {
    let found = (volume(before) - volume(after)).abs();
    assert!(
        (found - expected).abs() <= VOLUME_TOLERANCE,
        "{name}: changed by {found} instead of {expected}"
    );
}

fn sphere_corners(solid: &Solid) -> usize {
    solid
        .faces()
        .filter(|(_, face)| {
            matches!(face.surface(), Surface::Sphere(_))
                && face.origin() == Some(FaceOrigin::Fillet { feature: 60 })
        })
        .count()
}

#[test]
fn the_uprights_of_a_box_rounded_on_top_are_filleted_and_chamfered() {
    let topped = top_rounded_box();

    let verticals = uprights(&topped, (0.0, 0.0), RISE / 2.0);

    for size in [1.0, 2.0, 3.0] {
        let filleted = blend(&topped, &verticals, fillet(size), 60).unwrap();
        let chamfered = blend(
            &topped,
            &verticals,
            BlendShape::Chamfer { distance: size },
            60,
        )
        .unwrap();

        let rounded_name = format!("{size} mm fillet");
        let bevelled_name = format!("{size} mm chamfer");
        let expected = if size == EARLIER {
            ball_corner_off()
        } else {
            rounded_off(size)
        };

        assert_valid(&rounded_name, &filleted);
        assert_change(&rounded_name, &topped, &filleted, expected);
        assert_valid(&bevelled_name, &chamfered);
        assert_change(&bevelled_name, &topped, &chamfered, bevelled_off(size));
    }
}

#[test]
fn uprights_filleted_as_the_top_was_end_in_the_ball_corners_of_a_box_filleted_at_once() {
    let solid = cuboid(Vector3::new(40.0, 30.0, HEIGHT));
    let topped = top_rounded_box();

    let mut every = rim_edges(&solid, (0.0, 0.0), HEIGHT);
    every.extend(uprights(&solid, (0.0, 0.0), HEIGHT / 2.0));
    let at_once = run(&solid, &every, fillet(EARLIER));
    let verticals = uprights(&topped, (0.0, 0.0), RISE / 2.0);
    let after = blend(&topped, &verticals, fillet(EARLIER), 60).unwrap();

    assert_valid("after", &after);
    assert_eq!(sphere_corners(&after), 4);
    assert_eq!(after.faces().count(), at_once.faces().count());
    assert!((volume(&after) - volume(&at_once)).abs() <= VOLUME_TOLERANCE);
}

#[test]
fn the_corners_of_a_pocket_rounded_at_the_floor_are_filled() {
    let block = cuboid(Vector3::new(60.0, 50.0, 30.0));
    let cavity = cuboid(Vector3::new(40.0, 30.0, 25.0))
        .transformed(&RigidTransform::translation(Vector3::new(10.0, 10.0, 10.0)).unwrap())
        .unwrap();
    let pocketed = boolean(&block, &cavity, BooleanOperation::Difference).unwrap();
    let floor = rim_edges(&pocketed, (10.0, 10.0), 10.0);
    let rounded = run(&pocketed, &floor, fillet(EARLIER));

    let corners = uprights(&rounded, (10.0, 10.0), 20.0);

    for size in [1.0, 3.0] {
        let filleted = blend(&rounded, &corners, fillet(size), 60).unwrap();
        let chamfered = blend(
            &rounded,
            &corners,
            BlendShape::Chamfer { distance: size },
            60,
        )
        .unwrap();

        let rounded_name = format!("{size} mm pocket fillet");
        let bevelled_name = format!("{size} mm pocket chamfer");

        assert_valid(&rounded_name, &filleted);
        assert_change(&rounded_name, &rounded, &filleted, rounded_off(size));
        assert_valid(&bevelled_name, &chamfered);
        assert_change(&bevelled_name, &rounded, &chamfered, bevelled_off(size));
    }
}

#[test]
fn an_upright_under_one_rounded_edge_rounds_over_its_end_unless_larger() {
    let solid = cuboid(Vector3::new(40.0, 30.0, HEIGHT));
    let rounded = run(
        &solid,
        &[edge_through(&solid, (20.0, 0.0, HEIGHT))],
        fillet(EARLIER),
    );
    let upright = edge_through(&rounded, (0.0, 0.0, RISE / 2.0));
    let end = rounded
        .edges()
        .find(|(_, edge)| {
            matches!(edge.curve(), Curve::Circle(_))
                && edge.curve().point(edge.interval().start()).x.abs() < 1e-9
        })
        .map(|(id, _)| id)
        .unwrap();

    let smaller = blend(&rounded, &[upright], fillet(1.0), 60).unwrap();
    let larger = blend(&rounded, &[upright], fillet(3.0), 60);

    assert_valid("smaller", &smaller);
    assert!(
        smaller
            .vertices()
            .all(|(_, vertex)| vertex.point().distance(Point3::new(0.0, 0.0, RISE)) > 0.5)
    );
    assert_eq!(larger, Err(BlendError::TooLarge(end)));
}

#[test]
fn an_upright_of_a_boss_whose_foot_was_rounded_is_refused_at_its_foot() {
    let plate = cuboid(Vector3::new(60.0, 50.0, 10.0));
    let boss = cuboid(Vector3::new(40.0, 30.0, HEIGHT))
        .transformed(&RigidTransform::translation(Vector3::new(10.0, 10.0, 10.0)).unwrap())
        .unwrap();
    let bossed = boolean(&plate, &boss, BooleanOperation::Union).unwrap();
    let foot = rim_edges(&bossed, (10.0, 10.0), 10.0);
    let rounded = run(&bossed, &foot, fillet(EARLIER));

    let verticals = uprights(&rounded, (10.0, 10.0), 20.0);
    let refused = blend(&rounded, &verticals, fillet(1.0), 60);

    assert!(
        matches!(
            refused,
            Err(BlendError::UnsupportedEnd {
                vertex: Some(_),
                ..
            })
        ),
        "{refused:?}"
    );
}
