use std::sync::Arc;

use caditor_geometry::{Point2, Point3};

use super::*;
use crate::interrupt::interruptible;

const TOLERANCE: f64 = 1e-8;

fn knots(count: usize, degree: usize) -> Vec<f64> {
    let spans = count - degree;
    std::iter::repeat_n(0.0, degree + 1)
        .chain((1..spans).map(|index| index as f64 / spans as f64))
        .chain(std::iter::repeat_n(1.0, degree + 1))
        .collect()
}

fn rolling(weights: bool) -> BSplineSurface {
    let (columns, rows) = (9, 7);
    let mut points = Vec::new();
    for row in 0..rows {
        for column in 0..columns {
            let (x, y) = (column as f64 * 4.0, row as f64 * 3.0);
            points.push(Point3::new(x, y, 2.0 * (x * 0.15).sin() + (y * 0.2).cos()));
        }
    }
    let weights = weights.then(|| {
        (0..columns * rows)
            .map(|index| 1.0 + 0.25 * (index % 4) as f64)
            .collect()
    });
    BSplineSurface::new(
        5,
        3,
        knots(columns, 5),
        knots(rows, 3),
        columns,
        points,
        weights,
    )
    .unwrap()
}

fn on_side(surface: &BSplineSurface, side: SurfaceSide, along: f64) -> Point3 {
    let (u, v) = match side {
        SurfaceSide::VStart => (along, surface.v_domain().start()),
        SurfaceSide::VEnd => (along, surface.v_domain().end()),
        SurfaceSide::UStart => (surface.u_domain().start(), along),
        SurfaceSide::UEnd => (surface.u_domain().end(), along),
    };
    surface.point(u, v)
}

fn lifted(surface: &BSplineSurface, side: SurfaceSide) -> impl Fn(f64) -> Option<Point3> + '_ {
    move |along: f64| {
        let base = on_side(surface, side, along);
        let lift = Vector3::new(0.3, -0.2, 1.0) * 2e-4 * (7.0 * along).sin()
            + Vector3::new(1.0, 0.5, 0.0) * 1e-4 * (3.0 * along).cos();
        Some(base + lift)
    }
}

fn worst_miss(
    surface: &BSplineSurface,
    side: SurfaceSide,
    target: &dyn Fn(f64) -> Option<Point3>,
) -> f64 {
    (0..=2000)
        .map(|index| {
            let along = index as f64 / 2000.0;
            on_side(surface, side, along).distance(target(along).unwrap())
        })
        .fold(0.0, f64::max)
}

fn bent(
    surface: &BSplineSurface,
    side: SurfaceSide,
    target: &dyn Fn(f64) -> Option<Point3>,
    pins: Vec<(f64, Point3)>,
) -> BSplineSurface {
    let bend = SideBend {
        side,
        targets: vec![BendTarget {
            range: Interval::UNIT,
            point: target,
            strict: true,
        }],
        pins,
        knots: Vec::new(),
        held: [false; 2],
    };
    surface.bent_side(&bend, TOLERANCE).unwrap().surface
}

#[test]
fn every_side_follows_a_smooth_target_through_its_pins() {
    for weighted in [false, true] {
        let surface = rolling(weighted);
        for side in [
            SurfaceSide::VStart,
            SurfaceSide::VEnd,
            SurfaceSide::UStart,
            SurfaceSide::UEnd,
        ] {
            let target = lifted(&surface, side);
            let pins = [0.0, 0.37, 1.0]
                .map(|along| (along, target(along).unwrap()))
                .to_vec();
            let result = bent(&surface, side, &target, pins.clone());

            assert!(
                worst_miss(&result, side, &target) <= 2.0 * TOLERANCE,
                "{side:?} weighted {weighted}"
            );
            for (along, point) in pins {
                assert!(on_side(&result, side, along).distance(point) <= 1e-12);
            }
        }
    }
}

#[test]
fn bending_a_side_leaves_the_opposite_side_and_the_far_rows_alone() {
    let surface = rolling(false);
    let target = lifted(&surface, SurfaceSide::VStart);
    let result = bent(&surface, SurfaceSide::VStart, &target, Vec::new());
    let first_span_end = surface.v_knots().get(4).copied().unwrap();

    for index in 0..=200 {
        let u = index as f64 / 200.0;
        for v in [first_span_end, 0.5, 0.9, 1.0] {
            assert!(result.point(u, v).distance(surface.point(u, v)) <= 1e-12);
        }
    }
}

#[test]
fn a_held_corner_stays_while_the_rest_of_the_side_moves() {
    let surface = rolling(false);
    let target = lifted(&surface, SurfaceSide::VEnd);
    let corner = on_side(&surface, SurfaceSide::VEnd, 0.0);
    let bend = SideBend {
        side: SurfaceSide::VEnd,
        targets: vec![BendTarget {
            range: Interval::new(0.2, 1.0).unwrap(),
            point: &target,
            strict: true,
        }],
        pins: Vec::new(),
        knots: Vec::new(),
        held: [true, false],
    };
    let result = surface.bent_side(&bend, TOLERANCE).unwrap().surface;

    assert_eq!(on_side(&result, SurfaceSide::VEnd, 0.0), corner);
    for index in 0..=400 {
        let along = 0.2 + 0.8 * index as f64 / 400.0;
        let miss = on_side(&result, SurfaceSide::VEnd, along).distance(target(along).unwrap());
        assert!(miss <= 2.0 * TOLERANCE, "{miss} at {along}");
    }
}

#[test]
fn a_straight_side_of_a_ruled_surface_is_raised_to_cubic_and_bent() {
    let (columns, rows) = (6, 2);
    let points: Vec<Point3> = (0..rows)
        .flat_map(|row| {
            (0..columns).map(move |column| {
                let x = column as f64 * 5.0;
                Point3::new(x, (x * 0.1).sin(), row as f64 * 10.0)
            })
        })
        .collect();
    let surface = BSplineSurface::new(
        5,
        1,
        knots(columns, 5),
        knots(rows, 1),
        columns,
        points,
        None,
    )
    .unwrap();
    let side = SurfaceSide::UStart;
    let target = lifted(&surface, side);
    let result = bent(&surface, side, &target, Vec::new());

    assert_eq!(result.v_degree(), 3);
    assert!(worst_miss(&result, side, &target) <= 2.0 * TOLERANCE);
}

#[test]
fn requested_knots_join_the_side_and_land_on_existing_ones_when_close() {
    let surface = rolling(false);
    let target = lifted(&surface, SurfaceSide::VStart);
    let existing = surface.u_knots().get(6).copied().unwrap();
    let bend = SideBend {
        side: SurfaceSide::VStart,
        targets: vec![BendTarget {
            range: Interval::UNIT,
            point: &target,
            strict: true,
        }],
        pins: Vec::new(),
        knots: vec![0.3, 0.3, existing + 1e-7],
        held: [false; 2],
    };
    let result = surface.bent_side(&bend, TOLERANCE).unwrap().surface;
    let count = |value: f64| {
        result
            .u_knots()
            .iter()
            .filter(|knot| **knot == value)
            .count()
    };

    assert!(count(0.3) >= 2);
    assert_eq!(count(existing + 1e-7), 0);
    assert!(count(existing) >= 1);
}

#[test]
fn a_target_with_a_step_is_out_of_reach() {
    let surface = rolling(false);
    let target = |along: f64| {
        let step = if along < 0.5 { 0.0 } else { 1e-3 };
        Some(on_side(&surface, SurfaceSide::VStart, along) + Vector3::Z * step)
    };
    let bend = SideBend {
        side: SurfaceSide::VStart,
        targets: vec![BendTarget {
            range: Interval::UNIT,
            point: &target,
            strict: true,
        }],
        pins: Vec::new(),
        knots: Vec::new(),
        held: [false; 2],
    };

    assert!(matches!(
        surface.bent_side(&bend, TOLERANCE),
        Err(BendError::FallsShort(_) | BendError::TooDetailed(_))
    ));
}

#[test]
fn a_missing_target_and_a_seam_are_refused() {
    let surface = rolling(false);
    let nowhere = |_: f64| None;
    let bend = SideBend {
        side: SurfaceSide::VStart,
        targets: vec![BendTarget {
            range: Interval::UNIT,
            point: &nowhere,
            strict: true,
        }],
        pins: Vec::new(),
        knots: Vec::new(),
        held: [false; 2],
    };
    assert!(matches!(
        surface.bent_side(&bend, TOLERANCE),
        Err(BendError::NoTarget(_))
    ));

    let (columns, rows) = (7, 4);
    let points: Vec<Point3> = (0..rows)
        .flat_map(|row| {
            (0..columns).map(move |column| {
                let angle = std::f64::consts::TAU * column as f64 / (columns - 1) as f64;
                Point3::new(5.0 * angle.cos(), 5.0 * angle.sin(), row as f64)
            })
        })
        .collect();
    let tube = BSplineSurface::new(
        3,
        3,
        knots(columns, 3),
        knots(rows, 3),
        columns,
        points,
        None,
    )
    .unwrap();
    let target = |along: f64| Some(tube.point(along, 0.0));
    let bend = SideBend {
        side: SurfaceSide::VStart,
        targets: vec![BendTarget {
            range: Interval::UNIT,
            point: &target,
            strict: true,
        }],
        pins: Vec::new(),
        knots: Vec::new(),
        held: [false; 2],
    };
    assert_eq!(tube.bent_side(&bend, TOLERANCE), Err(BendError::Unbendable));
}

#[test]
fn a_cancelled_bend_says_so() {
    let surface = rolling(false);
    let target = lifted(&surface, SurfaceSide::VStart);
    let bend = SideBend {
        side: SurfaceSide::VStart,
        targets: vec![BendTarget {
            range: Interval::UNIT,
            point: &target,
            strict: true,
        }],
        pins: Vec::new(),
        knots: Vec::new(),
        held: [false; 2],
    };

    let result = interruptible(Arc::new(|| true), || surface.bent_side(&bend, TOLERANCE));
    assert_eq!(result, Err(BendError::Cancelled));
}

#[test]
fn sides_restrictions_and_extensions_agree_with_the_surface() {
    for weighted in [false, true] {
        let surface = rolling(weighted);
        for side in [
            SurfaceSide::VStart,
            SurfaceSide::VEnd,
            SurfaceSide::UStart,
            SurfaceSide::UEnd,
        ] {
            let curve = surface.side(side).unwrap();
            for index in 0..=50 {
                let along = index as f64 / 50.0;
                assert!(curve.point(along).distance(on_side(&surface, side, along)) <= 1e-12);
            }
        }

        let (u, v) = (
            Interval::new(0.13, 0.71).unwrap(),
            Interval::new(0.2, 1.0).unwrap(),
        );
        let piece = surface.restricted(u, v).unwrap();
        assert_eq!(piece.u_domain(), u);
        assert_eq!(piece.v_domain(), v);
        for index in 0..=20 {
            let uv = Point2::new(u.at(index as f64 / 20.0), v.at((20 - index) as f64 / 20.0));
            assert!(piece.point(uv.x, uv.y).distance(surface.point(uv.x, uv.y)) <= 1e-12);
        }

        for inside in [0.3, 0.999_999] {
            let at = surface.extended_evaluate(inside, 0.4);
            assert!(at.point.distance(surface.point(inside, 0.4)) <= 1e-12);
        }
        let edge = surface.extended_evaluate(1.0, 0.4);
        let beyond = surface.extended_evaluate(1.0 + 1e-4, 0.4);
        let taylor = edge.point + edge.du * 1e-4 + edge.duu * 0.5e-8;
        assert!(beyond.point.distance(taylor) <= 1e-8);
    }
}
