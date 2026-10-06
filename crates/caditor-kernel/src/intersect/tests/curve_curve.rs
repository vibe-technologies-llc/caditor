use std::f64::consts::TAU;

use caditor_geometry::{Plane, Point2, Point3, Vector3};

use crate::{
    bspline::BSpline,
    curve::{Circle, Curve, Line},
    curve2::{Circle2, Curve2, Line2},
    intersect::{CurveCurveIntersection, intersect_curves, intersect_curves2},
    interval::Interval,
    test_support::Random,
    tolerance::LINEAR_RESOLUTION,
};

fn line2(from: (f64, f64), to: (f64, f64)) -> (Curve2, Interval) {
    let (a, b) = (Point2::new(from.0, from.1), Point2::new(to.0, to.1));
    (
        Line2::through(a, b).unwrap().into(),
        Interval::new(0.0, a.distance(b)).unwrap(),
    )
}

fn circle2(center: (f64, f64), radius: f64) -> Curve2 {
    Circle2::new(Point2::new(center.0, center.1), radius)
        .unwrap()
        .into()
}

fn spline2(points: &[(f64, f64)]) -> Curve2 {
    BSpline::clamped_uniform(3, points.iter().map(|(x, y)| Point2::new(*x, *y)).collect())
        .unwrap()
        .into()
}

fn check2(
    first: &Curve2,
    first_range: Interval,
    second: &Curve2,
    second_range: Interval,
    found: &CurveCurveIntersection<Point2>,
) {
    for point in &found.points {
        assert!(first_range.contains(point.first));
        assert!(second_range.contains(point.second));
        assert!(first.point(point.first).distance(point.point) < 1e-12);
        assert!(
            second.point(point.second).distance(point.point) <= LINEAR_RESOLUTION,
            "{point:?}"
        );
    }
    for pair in found.points.windows(2) {
        assert!(pair[1].first > pair[0].first);
    }
    for overlap in &found.overlaps {
        for parameter in overlap.first.split(8) {
            let point = first.point(parameter);
            let other = second.closest_parameter(point, second_range);
            assert!(second.point(other).distance(point) <= LINEAR_RESOLUTION);
        }
    }
}

fn brute2(
    first: &Curve2,
    first_range: Interval,
    second: &Curve2,
    second_range: Interval,
) -> Vec<Point2> {
    let a: Vec<Point2> = first_range.split(800).map(|t| first.point(t)).collect();
    let b: Vec<Point2> = second_range.split(800).map(|t| second.point(t)).collect();
    let mut crossings = Vec::new();
    for pa in a.windows(2) {
        for pb in b.windows(2) {
            let (p, r) = (pa[0], pa[1] - pa[0]);
            let (q, s) = (pb[0], pb[1] - pb[0]);
            let denominator = r.perp_dot(s);
            if denominator.abs() < 1e-14 {
                continue;
            }
            let t = (q - p).perp_dot(s) / denominator;
            let u = (q - p).perp_dot(r) / denominator;
            if (0.0..1.0).contains(&t) && (0.0..1.0).contains(&u) {
                crossings.push(p + r * t);
            }
        }
    }
    crossings
}

#[test]
fn lines_and_circles_meet_analytically() {
    let (a, ra) = line2((0.0, 0.0), (4.0, 4.0));
    let (b, rb) = line2((0.0, 4.0), (4.0, 0.0));
    let found = intersect_curves2(&a, ra, &b, rb).unwrap();
    check2(&a, ra, &b, rb, &found);
    assert_eq!(found.points.len(), 1);
    assert!(found.points[0].point.distance(Point2::new(2.0, 2.0)) < 1e-12);
    let (c, rc) = line2((1.0, 1.0), (6.0, 6.0));
    let found = intersect_curves2(&a, ra, &c, rc).unwrap();
    assert_eq!(found.overlaps.len(), 1);
    let overlap = found.overlaps[0];
    assert!(
        (overlap.first.start() - 2f64.sqrt()).abs() < 1e-9,
        "{found:?}"
    );
    assert!((overlap.first.end() - ra.end()).abs() < 1e-9);
    assert!(found.points.is_empty());
    let (d, rd) = line2((4.0, 4.0), (8.0, 3.0));
    let found = intersect_curves2(&a, ra, &d, rd).unwrap();
    assert_eq!(found.points.len(), 1);
    assert_eq!(found.points[0].first, ra.end());
    assert_eq!(found.points[0].second, 0.0);
    assert!(!found.points[0].tangent);
    let wheel = circle2((0.0, 0.0), 2.0);
    let (touching, rt) = line2((-3.0, 2.0), (3.0, 2.0));
    let found = intersect_curves2(&touching, rt, &wheel, Interval::FULL_TURN).unwrap();
    assert_eq!(found.points.len(), 1);
    assert!(found.points[0].tangent);
    let other = circle2((3.0, 0.0), 2.0);
    let found =
        intersect_curves2(&wheel, Interval::FULL_TURN, &other, Interval::FULL_TURN).unwrap();
    check2(
        &wheel,
        Interval::FULL_TURN,
        &other,
        Interval::FULL_TURN,
        &found,
    );
    assert_eq!(found.points.len(), 2);
    let kissing = circle2((4.0, 0.0), 2.0);
    let found =
        intersect_curves2(&wheel, Interval::FULL_TURN, &kissing, Interval::FULL_TURN).unwrap();
    assert_eq!(found.points.len(), 1);
    assert!(found.points[0].tangent);
    let same = circle2((0.0, 0.0), 2.0);
    let found = intersect_curves2(
        &wheel,
        Interval::new(0.0, 3.0).unwrap(),
        &same,
        Interval::new(2.0, 5.0).unwrap(),
    )
    .unwrap();
    assert_eq!(found.overlaps.len(), 1);
    assert!((found.overlaps[0].first.start() - 2.0).abs() < 1e-9);
    assert!((found.overlaps[0].first.end() - 3.0).abs() < 1e-9);
}

#[test]
fn splines_meet_lines_circles_and_each_other_like_brute_force() {
    let mut random = Random::new(3);
    for _ in 0..6 {
        let points: Vec<(f64, f64)> = (0..6)
            .map(|_| {
                let point = random.point2(5.0);
                (point.x, point.y)
            })
            .collect();
        let first = spline2(&points);
        let others: Vec<(Curve2, Interval)> = vec![
            line2(
                (-6.0, random.between(-3.0, 3.0)),
                (6.0, random.between(-3.0, 3.0)),
            ),
            (
                circle2((random.between(-1.0, 1.0), 0.5), 2.5),
                Interval::FULL_TURN,
            ),
            (
                spline2(
                    &(0..5)
                        .map(|_| {
                            let point = random.point2(5.0);
                            (point.x, point.y)
                        })
                        .collect::<Vec<_>>(),
                ),
                Interval::UNIT,
            ),
        ];
        for (second, second_range) in &others {
            let found = intersect_curves2(&first, Interval::UNIT, second, *second_range).unwrap();
            check2(&first, Interval::UNIT, second, *second_range, &found);
            for crossing in brute2(&first, Interval::UNIT, second, *second_range) {
                assert!(
                    found
                        .points
                        .iter()
                        .any(|point| point.point.distance(crossing) < 1e-3),
                    "missed {crossing:?} in {found:?}"
                );
            }
        }
    }
    let wave = spline2(&[(0.0, 0.0), (1.0, 1.0), (2.0, -1.0), (3.0, 1.0), (4.0, 0.0)]);
    let found = intersect_curves2(&wave, Interval::UNIT, &wave, Interval::UNIT).unwrap();
    assert_eq!(found.overlaps.len(), 1);
    assert!(found.points.is_empty());
}

#[test]
fn curves_meet_in_space() {
    let a: Curve = Line::new(Point3::ZERO, Vector3::X).unwrap().into();
    let b: Curve = Line::new(Point3::new(1.0, -1.0, 0.0), Vector3::Y)
        .unwrap()
        .into();
    let span = Interval::new(-5.0, 5.0).unwrap();
    let found = intersect_curves(&a, span, &b, span).unwrap();
    assert_eq!(found.points.len(), 1);
    assert!((found.points[0].first - 1.0).abs() < 1e-12);
    assert!((found.points[0].second - 1.0).abs() < 1e-12);
    let skew: Curve = Line::new(Point3::new(1.0, -1.0, 0.1), Vector3::Y)
        .unwrap()
        .into();
    assert!(
        intersect_curves(&a, span, &skew, span)
            .unwrap()
            .points
            .is_empty()
    );
    let ring: Curve = Circle::new(Plane::XY, 2.0).unwrap().into();
    let hoop: Curve = Circle::new(
        Plane::new(Point3::new(2.0, 0.0, 1.5), Vector3::Y).unwrap(),
        1.5,
    )
    .unwrap()
    .into();
    let found = intersect_curves(&ring, Interval::FULL_TURN, &hoop, Interval::FULL_TURN).unwrap();
    assert_eq!(found.points.len(), 1, "{found:?}");
    assert!(!found.points[0].tangent);
    assert!(found.points[0].point.distance(Point3::new(2.0, 0.0, 0.0)) < 1e-9);
    let spline: Curve = BSpline::clamped_uniform(
        3,
        vec![
            Point3::new(-3.0, 0.0, -1.0),
            Point3::new(-1.0, 0.0, 3.0),
            Point3::new(1.0, 0.0, -3.0),
            Point3::new(3.0, 0.0, 1.0),
        ],
    )
    .unwrap()
    .into();
    let found = intersect_curves(&spline, Interval::UNIT, &a, span).unwrap();
    assert_eq!(found.points.len(), 3);
    for point in &found.points {
        assert!(point.point.z.abs() < LINEAR_RESOLUTION);
        assert!(point.point.y.abs() < LINEAR_RESOLUTION);
    }
    let partial: Curve = Line::new(Point3::new(3.0, 0.0, 0.0), Vector3::NEG_X)
        .unwrap()
        .into();
    let found = intersect_curves(
        &a,
        Interval::new(0.0, 4.0).unwrap(),
        &partial,
        Interval::new(0.0, 2.0).unwrap(),
    )
    .unwrap();
    assert_eq!(found.overlaps.len(), 1);
    let overlap = found.overlaps[0];
    assert!((overlap.first.start() - 1.0).abs() < 1e-9);
    assert!((overlap.first.end() - 3.0).abs() < 1e-9);
    assert!((overlap.second_start - 2.0).abs() < 1e-9);
    assert!(overlap.second_end.abs() < 1e-9);
}

#[test]
fn a_circle_overlapping_an_arc_leaves_out_a_gap_far_shorter_than_the_samples() {
    let wheel: Curve = Circle::new(Plane::XY, 2.0).unwrap().into();
    let gap = (3.0, 3.004);
    let arc = Interval::new(gap.1, gap.0 + TAU).unwrap();

    let found = intersect_curves(&wheel, Interval::FULL_TURN, &wheel, arc).unwrap();

    assert_eq!(found.overlaps.len(), 2, "{found:?}");
    assert!((found.overlaps[0].first.end() - gap.0).abs() < 1e-9);
    assert!((found.overlaps[1].first.start() - gap.1).abs() < 1e-9);
}

#[test]
fn circles_crossing_at_a_grazing_angle_meet_where_they_cross() {
    let offset = 1e-3;
    let bore: Curve = Circle::new(Plane::XY, 2.5).unwrap().into();
    let plug: Curve = Circle::new(
        Plane::new(Point3::new(offset, 0.0, 0.0), Vector3::Z).unwrap(),
        2.5,
    )
    .unwrap()
    .into();
    let found = intersect_curves(&plug, Interval::FULL_TURN, &bore, Interval::FULL_TURN).unwrap();

    assert_eq!(found.points.len(), 2, "{found:?}");
    for point in &found.points {
        assert!((point.point.x - 0.5 * offset).abs() < 1e-9, "{point:?}");
    }
}
