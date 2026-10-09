use std::f64::consts::{FRAC_PI_2, PI, TAU};

use caditor_geometry::{Plane, Point2, Point3, RigidTransform, Vector3};

use super::*;
use crate::{curve2::Circle2, test_support::Random};

fn tilted_frame() -> Plane {
    Plane::with_x_axis(
        Point3::new(3.0, -2.0, 5.0),
        Vector3::new(1.0, 2.0, 2.0),
        Vector3::new(1.0, -1.0, 0.3),
    )
    .unwrap()
}

fn spline() -> BSplineCurve {
    BSpline::rational(
        3,
        vec![0.0, 0.0, 0.0, 0.0, 1.0, 2.5, 4.0, 4.0, 4.0, 4.0],
        vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(4.0, 6.0, 1.0),
            Point3::new(9.0, 7.0, -2.0),
            Point3::new(12.0, 1.0, 0.0),
            Point3::new(15.0, -3.0, 4.0),
            Point3::new(20.0, 2.0, 1.0),
        ],
        vec![1.0, 0.8, 1.6, 1.0, 0.7, 1.0],
    )
    .unwrap()
}

fn wavy_spline() -> Curve {
    let points = (0..80)
        .map(|index| {
            let along = f64::from(index);
            Point3::new(along * 0.5, 2.0 * (along * 0.7).sin(), (along * 0.3).cos())
        })
        .collect();
    BSpline::clamped_uniform(3, points).unwrap().into()
}

fn curves() -> Vec<(Curve, Interval)> {
    vec![
        (
            Line::new(Point3::new(1.0, 2.0, 3.0), Vector3::new(1.0, -2.0, 0.5))
                .unwrap()
                .into(),
            Interval::new(-4.0, 7.0).unwrap(),
        ),
        (
            Circle::new(tilted_frame(), 6.0).unwrap().into(),
            Interval::new(0.3, 4.5).unwrap(),
        ),
        (
            Ellipse::new(tilted_frame(), 8.0, 3.0).unwrap().into(),
            Interval::new(-1.0, 5.0).unwrap(),
        ),
        (spline().into(), Interval::new(0.0, 4.0).unwrap()),
        (wavy_spline(), Interval::new(0.1, 0.85).unwrap()),
    ]
}

fn dense(curve: &Curve, range: Interval, count: usize) -> Vec<(f64, Point3)> {
    range
        .split(count)
        .map(|parameter| (parameter, curve.point(parameter)))
        .collect()
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
            assert!(
                (first - exact.first).length() < 1e-6 * (1.0 + exact.first.length()),
                "{curve:?} at {parameter}"
            );
            assert!(
                (second - exact.second).length() < 1e-5 * (1.0 + exact.second.length()),
                "{curve:?} at {parameter}"
            );
        }
    }
}

#[test]
fn closest_parameter_recovers_points_on_the_curve() {
    let mut random = Random::new(7);
    for (curve, range) in curves() {
        for _ in 0..200 {
            let parameter = range.at(random.unit());
            let point = curve.point(parameter);
            let found = curve.closest_parameter(point, range);
            assert!(
                curve.point(found).distance(point) < 1e-9,
                "{curve:?} at {parameter} found {found}"
            );
        }
    }
}

#[test]
fn closest_parameter_beats_dense_sampling_for_points_off_the_curve() {
    let mut random = Random::new(11);
    for (curve, range) in curves() {
        let samples = dense(&curve, range, 4000);
        for _ in 0..100 {
            let point = random.point(25.0);
            let found = curve.closest_parameter(point, range);
            assert!(range.contains(found));
            let distance = curve.point(found).distance(point);
            let sampled = samples
                .iter()
                .map(|(_, sample)| sample.distance(point))
                .fold(f64::INFINITY, f64::min);
            assert!(distance <= sampled + 1e-9, "{curve:?} for {point}");
        }
    }
}

#[test]
fn closest_parameter_on_a_circle_follows_the_range_across_the_seam() {
    let circle = Curve::from(Circle::new(Plane::XY, 2.0).unwrap());
    let range = Interval::new(1.5 * PI, 2.5 * PI).unwrap();
    let found = circle.closest_parameter(Point3::new(5.0, 0.5, 1.0), range);
    assert!((found - (TAU + 0.5f64.atan2(5.0))).abs() < 1e-12);
    let outside = circle.closest_parameter(Point3::new(-5.0, 0.1, 0.0), range);
    assert!((outside - 2.5 * PI).abs() < 1e-12);
    let on_axis = circle.closest_parameter(Point3::new(0.0, 0.0, 3.0), range);
    assert_eq!(on_axis, range.start());
    assert_eq!(circle.period(), Some(TAU));
    assert_eq!(circle.domain().bounded(), Some(Interval::FULL_TURN));
}

#[test]
fn sampling_holds_the_chord_and_angle_tolerances() {
    let tolerance = SamplingTolerance::new(0.01, 0.2).unwrap();
    for (curve, range) in curves() {
        let samples = curve.sample(range, &tolerance);
        assert_eq!(samples.first().unwrap().parameter, range.start());
        assert_eq!(samples.last().unwrap().parameter, range.end());
        assert!(samples.len() < 10_000);
        for pair in samples.windows(2) {
            let [a, b] = pair else {
                panic!("windows of two")
            };
            assert!(a.parameter < b.parameter);
            for fraction in [0.25, 0.5, 0.75] {
                let middle = curve.point(a.parameter + (b.parameter - a.parameter) * fraction);
                let deviation = crate::coordinates::distance_to_segment(middle, a.point, b.point);
                assert!(
                    deviation <= 0.01 * 1.05,
                    "{curve:?} deviates by {deviation}"
                );
            }
            let turning = crate::coordinates::angle_between(
                curve.evaluate(a.parameter).first,
                curve.evaluate(b.parameter).first,
            );
            assert!(turning <= 0.2 + 1e-9);
        }
    }
}

#[test]
fn sampling_stays_bounded_for_extreme_tolerances() {
    let tolerance = SamplingTolerance::new(1e-300, 1e-300).unwrap();
    for (curve, range) in curves() {
        let samples = curve.sample(range, &tolerance);
        assert!(samples.len() <= (1 << 17));
        assert!(samples.iter().all(|sample| sample.point.is_finite()));
    }
}

#[test]
fn lengths_match_closed_forms_and_dense_polylines() {
    let circle = Curve::from(Circle::new(tilted_frame(), 6.0).unwrap());
    assert!((circle.length(Interval::new(0.0, FRAC_PI_2).unwrap()) - 3.0 * PI).abs() < 1e-12);
    for (curve, range) in curves() {
        let polyline: f64 = dense(&curve, range, 200_000)
            .windows(2)
            .map(|pair| pair[0].1.distance(pair[1].1))
            .sum();
        let length = curve.length(range);
        assert!(
            (length - polyline).abs() < 1e-7 * length,
            "{curve:?}: {length} vs {polyline}"
        );
    }
}

#[test]
fn bounding_boxes_contain_the_curve_and_are_tight_for_conics() {
    for (curve, range) in curves() {
        let bounds = curve.bounding_box(range).expanded(1e-9);
        let samples = dense(&curve, range, 5000);
        assert!(samples.iter().all(|(_, point)| {
            point.cmpge(bounds.min()).all() && point.cmple(bounds.max()).all()
        }));
        if matches!(curve, Curve::Circle(_) | Curve::Ellipse(_) | Curve::Line(_)) {
            let tight =
                caditor_geometry::Aabb::from_points(samples.iter().map(|(_, point)| *point))
                    .unwrap();
            assert!((tight.min() - bounds.min()).length() < 1e-5);
            assert!((tight.max() - bounds.max()).length() < 1e-5);
        }
    }
}

#[test]
fn reversal_retraces_every_curve_backwards() {
    for (curve, range) in curves() {
        let reversed = curve.reversed();
        let mirrored = curve.reversed_range(range);
        assert!((mirrored.length() - range.length()).abs() < 1e-12);
        for index in 0..=20 {
            let parameter = range.at(index as f64 / 20.0);
            let back = curve.reversed_parameter(parameter);
            assert!(mirrored.contains(back) || (back - mirrored.end()).abs() < 1e-12);
            assert!(reversed.point(back).distance(curve.point(parameter)) < 1e-9);
            let tangent = reversed.evaluate(back).first + curve.evaluate(parameter).first;
            assert!(tangent.length() < 1e-9 * (1.0 + curve.evaluate(parameter).first.length()));
        }
    }
}

#[test]
fn rigid_transforms_move_every_point() {
    let transform = RigidTransform::rotation_about(
        Point3::new(1.0, 0.0, 2.0),
        Vector3::new(0.3, 1.0, -0.2),
        1.1,
    )
    .unwrap()
    .then(&RigidTransform::translation(Vector3::new(5.0, -3.0, 8.0)).unwrap());
    for (curve, range) in curves() {
        let moved = curve.transformed(&transform).unwrap();
        for index in 0..=10 {
            let parameter = range.at(index as f64 / 10.0);
            let expected = transform.apply_point(curve.point(parameter));
            assert!(moved.point(parameter).distance(expected) < 1e-9);
        }
    }
}

#[test]
fn constructors_reject_degenerate_and_non_finite_input() {
    assert_eq!(
        Line::new(Point3::ZERO, Vector3::ZERO),
        Err(GeometryError::ZeroDirection)
    );
    assert_eq!(
        Line::new(Point3::new(f64::NAN, 0.0, 0.0), Vector3::X),
        Err(GeometryError::NonFinite)
    );
    assert_eq!(
        Circle::new(Plane::XY, 0.0),
        Err(GeometryError::NonPositive(0.0))
    );
    assert_eq!(
        Circle::new(Plane::XY, f64::INFINITY),
        Err(GeometryError::NonFinite)
    );
    assert_eq!(
        Ellipse::new(Plane::XY, 1.0, -1.0),
        Err(GeometryError::NonPositive(-1.0))
    );
    assert_eq!(
        Circle::new(Plane::XY, 1e-9),
        Err(GeometryError::BelowResolution(1e-9))
    );
    assert_eq!(
        Circle::new(Plane::XY, 1e7),
        Err(GeometryError::BeyondMaximum(1e7))
    );
    assert_eq!(
        Circle2::new(Point2::ZERO, 1e7),
        Err(GeometryError::BeyondMaximum(1e7))
    );
    let broken = Plane::new(Point3::new(f64::INFINITY, 0.0, 0.0), Vector3::Z).unwrap();
    assert_eq!(Circle::new(broken, 1.0), Err(GeometryError::NonFinite));
}

#[test]
fn an_intersection_curve_rebuilt_through_rough_points_lies_on_both_surfaces() {
    use crate::{
        IntersectionCurve,
        surface::{Cylinder, Surface},
    };

    let upright: Surface = Cylinder::new(Plane::XY, 8.0).unwrap().into();
    let across: Surface = Cylinder::new(
        Plane::from_frame(Point3::new(-20.0, 0.0, 0.0), Vector3::X, Vector3::Y).unwrap(),
        5.0,
    )
    .unwrap()
    .into();
    let rough: Vec<Point3> = (0..=24)
        .map(|index| {
            let angle = std::f64::consts::TAU * index as f64 / 24.0;
            let (y, z) = (5.0 * angle.cos(), 5.0 * angle.sin());
            let x = (64.0 - y * y).sqrt();
            Point3::new(x + 1e-4 * angle.sin(), y, z + 1e-4)
        })
        .collect();
    let curve =
        IntersectionCurve::through([upright.clone(), across.clone()], &rough, true).unwrap();
    assert!(curve.is_closed());
    let domain = curve.domain();
    for step in 0..=200 {
        let point = curve.point(domain.at(step as f64 / 200.0));
        assert!(
            upright.distance(point) < 1e-6,
            "{}",
            upright.distance(point)
        );
        assert!(across.distance(point) < 1e-6, "{}", across.distance(point));
    }
    assert!(
        IntersectionCurve::through([upright.clone(), across.clone()], &[Point3::ZERO], false)
            .is_none()
    );

    let stopped = crate::interruptible(std::sync::Arc::new(|| true), || {
        IntersectionCurve::through([upright, across], &rough, true)
    });
    assert!(stopped.is_none());
}

#[test]
fn lengths_up_to_a_cap_and_comparisons_agree_with_the_length() {
    for (curve, range) in curves() {
        let length = curve.length(range);

        assert!((curve.length_up_to(range, f64::INFINITY) - length).abs() <= 1e-12 * length);
        assert_eq!(curve.length_up_to(range, 0.5 * length), 0.5 * length);
        assert!(curve.is_longer_than(range, 0.999 * length), "{curve:?}");
        assert!(!curve.is_longer_than(range, 1.001 * length), "{curve:?}");
    }
}

#[test]
fn the_polyline_through_the_seeds_bounds_the_length_closely_from_below() {
    for (curve, range) in curves() {
        let length = curve.length(range);
        let polyline = curve.polyline_length_up_to(range, f64::INFINITY).unwrap();
        let capped = curve.polyline_length_up_to(range, 0.5 * polyline).unwrap();

        assert!(polyline <= length * (1.0 + 1e-12), "{curve:?}");
        assert!(
            polyline >= 0.97 * length,
            "{polyline} of {length}: {curve:?}"
        );
        assert_eq!(capped, 0.5 * polyline);
    }

    let stopped = crate::interruptible(std::sync::Arc::new(|| true), || {
        wavy_spline().polyline_length_up_to(Interval::UNIT, f64::INFINITY)
    });

    assert_eq!(stopped, Err(crate::interrupt::Interrupted));
}

#[test]
fn a_closed_curve_is_longer_than_the_chord_between_its_ends() {
    let circle = Curve::from(Circle::new(Plane::XY, 1e-3).unwrap());
    let loop_spline = Curve::from(
        BSpline::clamped_uniform(
            2,
            vec![
                Point3::ZERO,
                Point3::new(1e-3, 0.0, 0.0),
                Point3::new(1e-3, 1e-3, 0.0),
                Point3::ZERO,
            ],
        )
        .unwrap(),
    );

    assert!(circle.is_longer_than(Interval::FULL_TURN, 6e-3));
    assert!(loop_spline.is_longer_than(Interval::UNIT, 1e-3));
    assert!(!loop_spline.is_longer_than(Interval::UNIT, 1e-2));
}

#[test]
fn closest_parameter_on_a_spline_of_many_spans_finds_the_nearest_point() {
    let mut random = Random::new(29);
    let curve = wavy_spline();
    let range = Interval::UNIT;
    let samples = dense(&curve, range, 40_000);
    for _ in 0..200 {
        let point = curve.point(random.unit()) + random.point(1.5);

        let found = curve.closest_parameter(point, range);
        let sampled = samples
            .iter()
            .map(|(_, sample)| sample.distance(point))
            .fold(f64::INFINITY, f64::min);

        assert!(
            curve.point(found).distance(point) <= sampled + 1e-9,
            "{point}"
        );
    }
}

#[test]
fn a_circle_curves_towards_its_centre_by_the_inverse_of_its_radius() {
    let frame = tilted_frame();
    let circle: Curve = Circle::new(frame, 6.0).unwrap().into();
    for parameter in [0.0, 1.0, 2.5, 5.0] {
        let curvature = circle.curvature(parameter).unwrap();
        let towards_centre = (frame.origin() - circle.point(parameter)).normalize();

        assert!((curvature.length() - 1.0 / 6.0).abs() < 1e-12);
        assert!(curvature.normalize().dot(towards_centre) > 1.0 - 1e-12);
    }
}

#[test]
fn a_line_has_no_curvature() {
    let line: Curve = Line::new(Point3::new(1.0, 2.0, 3.0), Vector3::new(1.0, -2.0, 0.5))
        .unwrap()
        .into();

    assert_eq!(line.curvature(2.0), Some(Vector3::ZERO));
}

#[test]
fn an_ellipse_is_tightest_at_the_ends_of_its_major_axis() {
    let ellipse: Curve = Ellipse::new(Plane::XY, 8.0, 3.0).unwrap().into();

    let at_major = ellipse.curvature(0.0).unwrap();
    let at_minor = ellipse.curvature(FRAC_PI_2).unwrap();

    assert!((at_major - Vector3::new(-8.0 / 9.0, 0.0, 0.0)).length() < 1e-12);
    assert!((at_minor - Vector3::new(0.0, -3.0 / 64.0, 0.0)).length() < 1e-12);
}

#[test]
fn a_parabola_spline_curves_by_its_closed_form_whatever_its_speed() {
    let parabola: Curve = BSpline::new(
        2,
        vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        vec![
            Point3::new(-1.0, 1.0, 0.0),
            Point3::new(0.0, -1.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
        ],
    )
    .unwrap()
    .into();

    let vertex = parabola.curvature(0.5).unwrap();
    let end = parabola.curvature(1.0).unwrap();

    assert!((vertex - Vector3::new(0.0, 2.0, 0.0)).length() < 1e-12);
    assert!((end.length() - 2.0 / 5.0_f64.powf(1.5)).abs() < 1e-12);
    assert!(end.dot(Vector3::new(-2.0, 1.0, 0.0)) > 0.0);
}
