use std::f64::consts::{PI, TAU};

use caditor_geometry::{Plane, Point2, Point3, RigidTransform2, Vector2, Vector3};

use super::*;
use crate::test_support::Random;

fn curves() -> Vec<(Curve2, Interval)> {
    vec![
        (
            Line2::new(Point2::new(1.0, 2.0), Vector2::new(3.0, -1.0))
                .unwrap()
                .into(),
            Interval::new(-2.0, 5.0).unwrap(),
        ),
        (
            Circle2::with_axes(Point2::new(-1.0, 4.0), 3.0, Vector2::new(1.0, 1.0), false)
                .unwrap()
                .into(),
            Interval::new(0.5, 5.5).unwrap(),
        ),
        (
            BSplineCurve2::clamped_uniform(
                3,
                vec![
                    Point2::ZERO,
                    Point2::new(2.0, 5.0),
                    Point2::new(6.0, 5.0),
                    Point2::new(7.0, -1.0),
                    Point2::new(11.0, 2.0),
                ],
            )
            .unwrap()
            .into(),
            Interval::UNIT,
        ),
    ]
}

#[test]
fn derivatives_match_finite_differences() {
    let step = 1e-6;
    for (curve, range) in curves() {
        for index in 1..10 {
            let parameter = range.at(index as f64 / 10.0);
            let exact = curve.evaluate(parameter);
            let ahead = curve.evaluate(parameter + step);
            let behind = curve.evaluate(parameter - step);
            let first = (ahead.point - behind.point) / (2.0 * step);
            let second = (ahead.first - behind.first) / (2.0 * step);
            assert!((first - exact.first).length() < 1e-6 * (1.0 + exact.first.length()));
            assert!((second - exact.second).length() < 1e-5 * (1.0 + exact.second.length()));
        }
    }
}

#[test]
fn closest_parameter_round_trips_and_beats_dense_sampling() {
    let mut random = Random::new(3);
    for (curve, range) in curves() {
        let samples: Vec<Point2> = range
            .split(4000)
            .map(|parameter| curve.point(parameter))
            .collect();
        for _ in 0..100 {
            let parameter = range.at(random.unit());
            let on_curve = curve.point(parameter);
            let found = curve.closest_parameter(on_curve, range);
            assert!(curve.point(found).distance(on_curve) < 1e-9);

            let point = random.point2(12.0);
            let found = curve.closest_parameter(point, range);
            let sampled = samples
                .iter()
                .map(|sample| sample.distance(point))
                .fold(f64::INFINITY, f64::min);
            assert!(curve.point(found).distance(point) <= sampled + 1e-9);
        }
    }
}

#[test]
fn a_clockwise_circle_reverses_to_counter_clockwise() {
    let clockwise = Circle2::with_axes(Point2::ZERO, 1.0, Vector2::X, false).unwrap();
    assert!(!clockwise.is_counter_clockwise());
    assert!(clockwise.reversed().is_counter_clockwise());
    let curve = Curve2::from(clockwise);
    assert!(curve.point(PI / 2.0).distance(Point2::new(0.0, -1.0)) < 1e-15);
    assert_eq!(curve.period(), Some(TAU));
}

#[test]
fn sampling_lengths_and_bounds_agree_with_dense_polylines() {
    let tolerance = SamplingTolerance::new(0.005, 0.1).unwrap();
    for (curve, range) in curves() {
        let samples = curve.sample(range, &tolerance);
        for pair in samples.windows(2) {
            let middle = curve.point(0.5 * (pair[0].parameter + pair[1].parameter));
            let deviation =
                crate::coordinates::distance_to_segment(middle, pair[0].point, pair[1].point);
            assert!(deviation <= 0.005 * 1.05);
        }
        let dense: Vec<Point2> = range
            .split(100_000)
            .map(|parameter| curve.point(parameter))
            .collect();
        let polyline: f64 = dense.windows(2).map(|pair| pair[0].distance(pair[1])).sum();
        assert!((curve.length(range) - polyline).abs() < 1e-7 * polyline);
        let bounds = curve.bounding_box(range).expanded(1e-9);
        assert!(dense.iter().all(|point| bounds.contains(*point)));
    }
}

#[test]
fn reversal_and_planar_transforms_preserve_the_trace() {
    let transform = RigidTransform2::new(0.8, Vector2::new(-3.0, 7.0)).unwrap();
    for (curve, range) in curves() {
        let reversed = curve.reversed();
        let moved = curve.transformed(&transform).unwrap();
        for index in 0..=10 {
            let parameter = range.at(index as f64 / 10.0);
            let point = curve.point(parameter);
            assert!(
                reversed
                    .point(curve.reversed_parameter(parameter))
                    .distance(point)
                    < 1e-9
            );
            assert!(
                moved
                    .point(parameter)
                    .distance(transform.apply_point(point))
                    < 1e-9
            );
        }
    }
}

#[test]
fn lifting_onto_a_plane_keeps_the_parametrization() {
    let plane = Plane::with_x_axis(
        Point3::new(1.0, 2.0, 3.0),
        Vector3::new(0.0, 1.0, 1.0),
        Vector3::X,
    )
    .unwrap();
    for (curve, range) in curves() {
        let lifted = curve.on_plane(&plane).unwrap();
        for index in 0..=10 {
            let parameter = range.at(index as f64 / 10.0);
            let expected = plane.to_world(curve.point(parameter));
            assert!(lifted.point(parameter).distance(expected) < 1e-12);
        }
    }
}
