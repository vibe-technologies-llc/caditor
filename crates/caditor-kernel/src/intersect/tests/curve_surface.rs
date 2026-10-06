use std::f64::consts::{FRAC_PI_2, TAU};

use caditor_geometry::{Plane, Point2, Point3, Vector3};

use super::*;
use crate::{
    bspline::BSpline,
    curve::{Circle, Curve, Ellipse, Line},
    intersect::{CurveSurfaceIntersection, intersect_curve_surface, intersect_surfaces},
    surface::{Cone, Cylinder, Extrusion, Revolution, Sphere, Torus},
    test_support::Random,
};

const BRUTE_SAMPLES: usize = 1500;
const SWEPT_SAMPLES: usize = 500;
const RESYNC: usize = 25;

fn surfaces() -> Vec<(&'static str, Surface)> {
    let profile: Curve = BSpline::clamped_uniform(
        3,
        vec![
            Point3::new(1.0, 0.0, -4.0),
            Point3::new(3.0, 0.0, -2.0),
            Point3::new(2.0, 0.0, 0.5),
            Point3::new(3.5, 0.0, 2.0),
            Point3::new(2.5, 0.0, 4.0),
        ],
    )
    .unwrap()
    .into();
    let wave: Curve = BSpline::clamped_uniform(
        3,
        vec![
            Point3::new(-5.0, 0.0, 0.0),
            Point3::new(-2.0, 3.0, 0.0),
            Point3::new(1.0, -2.0, 0.0),
            Point3::new(4.0, 2.0, 0.0),
            Point3::new(6.0, -1.0, 0.0),
        ],
    )
    .unwrap()
    .into();
    let tilted = Plane::with_x_axis(
        Point3::new(0.3, -0.2, 0.1),
        Vector3::new(0.2, 0.3, 1.0),
        Vector3::X,
    )
    .unwrap();
    vec![
        (
            "plane",
            plane(Point3::new(0.2, 0.1, 0.3), Vector3::new(0.3, -0.4, 1.0)),
        ),
        ("cylinder", Cylinder::new(tilted, 2.5).unwrap().into()),
        ("cone", Cone::new(tilted, 1.5, 0.5).unwrap().into()),
        ("sphere", Sphere::new(tilted, 3.0).unwrap().into()),
        ("torus", Torus::new(tilted, 3.0, 1.2).unwrap().into()),
        (
            "extrusion",
            Extrusion::new(wave, Vector3::new(0.1, 0.2, 1.0))
                .unwrap()
                .into(),
        ),
        (
            "revolution",
            Revolution::new(profile, Point3::ZERO, Vector3::new(0.1, 0.0, 1.0))
                .unwrap()
                .into(),
        ),
    ]
}

fn random_curves(random: &mut Random) -> Vec<(Curve, Interval)> {
    let center = random.point(1.5);
    let frame = Plane::new(center, random.point(1.0) + Vector3::splat(0.01)).unwrap();
    let spline: Curve = BSpline::clamped_uniform(3, (0..6).map(|_| random.point(5.0)).collect())
        .unwrap()
        .into();
    vec![
        (
            Line::new(center, random.point(1.0) + Vector3::splat(0.01))
                .unwrap()
                .into(),
            Interval::new(-9.0, 9.0).unwrap(),
        ),
        (
            Circle::new(frame, random.between(1.0, 4.0)).unwrap().into(),
            Interval::FULL_TURN,
        ),
        (
            Ellipse::new(frame, random.between(3.0, 5.0), random.between(1.0, 3.0))
                .unwrap()
                .into(),
            Interval::new(0.5, 5.5).unwrap(),
        ),
        (spline, Interval::UNIT),
    ]
}

fn signed_near(surface: &Surface, point: Point3, hint: Option<Point2>) -> (f64, f64, Point2) {
    let uv = match hint {
        Some(hint) => crate::surface::refine_projection(surface, point, hint),
        None => surface.project(point, None),
    };
    let offset = point - surface.point_at(uv);
    let normal = surface.normal(uv.x, uv.y).unwrap_or(Vector3::Z);
    (offset.dot(normal), offset.length(), uv)
}

fn signed(surface: &Surface, point: Point3) -> (f64, f64) {
    let (value, distance, _) = signed_near(surface, point, None);
    (value, distance)
}

fn brute_crossings(curve: &Curve, range: Interval, surface: &Surface) -> Vec<f64> {
    let swept = matches!(surface, Surface::Extrusion(_) | Surface::Revolution(_));
    let count = if swept { SWEPT_SAMPLES } else { BRUTE_SAMPLES };
    let parameters: Vec<f64> = range.split(count).collect();
    let mut hint = None;
    let samples: Vec<(f64, Point2)> = parameters
        .iter()
        .enumerate()
        .map(|(index, parameter)| {
            let resync = !swept || index % RESYNC == 0;
            let (value, _, uv) = signed_near(
                surface,
                curve.point(*parameter),
                if resync { None } else { hint },
            );
            hint = Some(uv);
            (value, uv)
        })
        .collect();
    let mut crossings = Vec::new();
    for (index, pair) in parameters.windows(2).enumerate() {
        let (mut low, mut high) = (pair[0], pair[1]);
        let ((a, uv), (b, _)) = (samples[index], samples[index + 1]);
        if (a < 0.0) == (b < 0.0) {
            continue;
        }
        let near = swept.then_some(uv);
        for _ in 0..60 {
            let middle = 0.5 * (low + high);
            let (value, _, _) = signed_near(surface, curve.point(middle), near);
            if (value < 0.0) == (a < 0.0) {
                low = middle;
            } else {
                high = middle;
            }
        }
        let root = 0.5 * (low + high);
        if signed(surface, curve.point(root)).1 <= 1e-7 {
            crossings.push(root);
        }
    }
    crossings
}

fn check_result(
    name: &str,
    curve: &Curve,
    range: Interval,
    surface: &Surface,
    found: &CurveSurfaceIntersection,
) {
    for point in &found.points {
        assert!(range.contains(point.parameter), "{name}");
        assert!(
            curve.point(point.parameter).distance(point.point) < 1e-12,
            "{name}"
        );
        assert!(
            distance(surface, point.point) <= LINEAR_RESOLUTION,
            "{name}: {point:?}"
        );
        assert!(
            surface.point_at(point.uv).distance(point.point) <= LINEAR_RESOLUTION,
            "{name}: uv {:?}",
            point.uv
        );
    }
    if let (Some(first), Some(last)) = (found.points.first(), found.points.last())
        && found.points.len() > 1
        && curve.period().is_some()
    {
        assert!(
            first.point.distance(last.point) > LINEAR_RESOLUTION,
            "{name}: wrapped twice"
        );
    }
    for pair in found.points.windows(2) {
        assert!(pair[1].parameter > pair[0].parameter, "{name}: unsorted");
        assert!(
            pair[1].point.distance(pair[0].point) > LINEAR_RESOLUTION,
            "{name}: duplicated point"
        );
    }
    for overlap in &found.overlaps {
        for parameter in overlap.range.split(16) {
            assert!(
                distance(surface, curve.point(parameter)) <= LINEAR_RESOLUTION,
                "{name}: overlap leaves the surface"
            );
        }
    }
}

#[test]
fn every_curve_against_every_surface_matches_brute_force() {
    let mut random = Random::new(21);
    for round in 0..3 {
        let curves = random_curves(&mut random);
        for (surface_name, surface) in surfaces() {
            for (index, (curve, range)) in curves.iter().enumerate() {
                let name = format!("round {round} curve {index} on {surface_name}");
                let found = intersect_curve_surface(curve, *range, &surface, None)
                    .unwrap_or_else(|error| panic!("{name}: {error}"));
                let brute = brute_crossings(curve, *range, &surface);
                check_result(&name, curve, *range, &surface, &found);
                for crossing in brute {
                    let point = curve.point(crossing);
                    let matched = found
                        .points
                        .iter()
                        .any(|hit| hit.point.distance(point) <= 1e-5)
                        || found
                            .overlaps
                            .iter()
                            .any(|overlap| overlap.range.contains(crossing));
                    assert!(
                        matched,
                        "{name}: missed the crossing at {crossing} ({point:?}); found {found:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn intersection_curves_meet_other_surfaces_and_lie_on_their_own() {
    let tube: Surface = Cylinder::new(frame(Point3::new(1.0, 0.0, 0.0), Vector3::Z), 1.0)
        .unwrap()
        .into();
    let ball: Surface = Sphere::new(Plane::XY, 2.0).unwrap().into();
    let result = intersect_surfaces(
        &around(&tube, (-5.0, 5.0)),
        &patch(&ball, (0.0, TAU), (-FRAC_PI_2, FRAC_PI_2)),
    )
    .unwrap();
    let branch = &result.branches()[0];
    let found = intersect_curve_surface(&branch.curve, branch.range, &ball, None).unwrap();
    assert!(found.points.is_empty());
    assert_eq!(found.overlaps.len(), 1);
    assert_eq!(found.overlaps[0].range, branch.range);
    let floor = plane(Point3::new(0.0, 0.0, 0.5), Vector3::Z);
    let found = intersect_curve_surface(&branch.curve, branch.range, &floor, None).unwrap();
    check_result("viviani", &branch.curve, branch.range, &floor, &found);
    assert_eq!(found.points.len(), 2);
    assert!(found.points.iter().all(|point| !point.tangent));
    for crossing in brute_crossings(&branch.curve, branch.range, &floor) {
        assert!(
            found
                .points
                .iter()
                .any(|point| (point.parameter - crossing).abs() < 1e-6)
        );
    }
}

fn single(found: &CurveSurfaceIntersection) -> (f64, bool) {
    assert_eq!(found.points.len(), 1, "{found:?}");
    assert!(found.overlaps.is_empty());
    (found.points[0].parameter, found.points[0].tangent)
}

#[test]
fn lines_cross_touch_and_lie_on_elementary_surfaces() {
    let span = Interval::new(-10.0, 10.0).unwrap();
    let floor = plane(Point3::ZERO, Vector3::Z);
    let flat: Curve = Line::new(Point3::new(0.0, 0.0, 0.0), Vector3::X)
        .unwrap()
        .into();
    let found = intersect_curve_surface(&flat, span, &floor, None).unwrap();
    assert_eq!(found.overlaps.len(), 1);
    assert_eq!(found.overlaps[0].range, span);
    assert!(found.points.is_empty());
    let steep: Curve = Line::new(Point3::new(1.0, 2.0, 3.0), Vector3::new(0.1, 0.2, -1.0))
        .unwrap()
        .into();
    assert!(!single(&intersect_curve_surface(&steep, span, &floor, None).unwrap()).1);
    let tube: Surface = Cylinder::new(Plane::XY, 2.0).unwrap().into();
    let ruling: Curve = Line::new(Point3::new(0.0, 2.0, 0.0), Vector3::Z)
        .unwrap()
        .into();
    let found = intersect_curve_surface(&ruling, span, &tube, None).unwrap();
    assert_eq!(found.overlaps.len(), 1);
    let grazing: Curve = Line::new(Point3::new(0.0, 2.0, 1.0), Vector3::new(1.0, 0.0, 0.3))
        .unwrap()
        .into();
    let (at, tangent) = single(&intersect_curve_surface(&grazing, span, &tube, None).unwrap());
    assert!(tangent);
    assert!(at.abs() < 1e-6);
    let through: Curve = Line::new(Point3::new(0.0, 1.0, 0.0), Vector3::new(1.0, 0.0, 0.2))
        .unwrap()
        .into();
    let found = intersect_curve_surface(&through, span, &tube, None).unwrap();
    assert_eq!(found.points.len(), 2);
    let ball: Surface = Sphere::new(Plane::XY, 3.0).unwrap().into();
    let skimming: Curve = Line::new(Point3::new(0.0, 0.0, 3.0), Vector3::X)
        .unwrap()
        .into();
    assert!(single(&intersect_curve_surface(&skimming, span, &ball, None).unwrap()).1);
    let ring: Surface = Torus::new(Plane::XY, 6.0, 2.0).unwrap().into();
    let axis: Curve = Line::new(Point3::new(-10.0, 0.0, 0.5), Vector3::X)
        .unwrap()
        .into();
    let long = Interval::new(0.0, 20.0).unwrap();
    let found = intersect_curve_surface(&axis, long, &ring, None).unwrap();
    assert_eq!(found.points.len(), 4);
    assert!(found.points.iter().all(|point| !point.tangent));
    let over_top: Curve = Line::new(Point3::new(-10.0, 6.0, 2.0), Vector3::X)
        .unwrap()
        .into();
    let (at, tangent) = single(&intersect_curve_surface(&over_top, long, &ring, None).unwrap());
    assert!(tangent);
    assert!((at - 10.0).abs() < 1e-5);
    let cone: Surface = Cone::new(Plane::XY, 1.0, 0.5).unwrap().into();
    let apex = match &cone {
        Surface::Cone(cone) => cone.apex(),
        _ => panic!("expected a cone"),
    };
    let along: Curve = Line::through(apex, Point3::new(1.0, 0.0, 0.0))
        .unwrap()
        .into();
    let found =
        intersect_curve_surface(&along, Interval::new(0.0, 5.0).unwrap(), &cone, None).unwrap();
    assert_eq!(found.overlaps.len(), 1);
    let across: Curve = Line::new(Point3::new(-5.0, 0.0, 2.0), Vector3::X)
        .unwrap()
        .into();
    let found =
        intersect_curve_surface(&across, Interval::new(0.0, 10.0).unwrap(), &cone, None).unwrap();
    assert_eq!(found.points.len(), 2);
    let below: Curve = Line::new(Point3::new(-5.0, 0.0, -5.0), Vector3::X)
        .unwrap()
        .into();
    assert!(
        intersect_curve_surface(&below, Interval::new(0.0, 10.0).unwrap(), &cone, None)
            .unwrap()
            .points
            .is_empty()
    );
}

#[test]
fn circles_cross_touch_and_lie_on_surfaces() {
    let floor = plane(Point3::ZERO, Vector3::Z);
    let lying: Curve = Circle::new(Plane::XY, 2.0).unwrap().into();
    let found = intersect_curve_surface(&lying, Interval::FULL_TURN, &floor, None).unwrap();
    assert_eq!(found.overlaps.len(), 1);
    let standing_frame = Plane::new(Point3::new(0.0, 0.0, 2.0), Vector3::Y).unwrap();
    let standing: Curve = Circle::new(standing_frame, 2.0).unwrap().into();
    let (_, tangent) =
        single(&intersect_curve_surface(&standing, Interval::FULL_TURN, &floor, None).unwrap());
    assert!(tangent);
    let crossing_frame = Plane::new(Point3::new(0.0, 0.0, 1.0), Vector3::Y).unwrap();
    let crossing: Curve = Circle::new(crossing_frame, 2.0).unwrap().into();
    let found = intersect_curve_surface(&crossing, Interval::FULL_TURN, &floor, None).unwrap();
    assert_eq!(found.points.len(), 2);
    let ball: Surface = Sphere::new(Plane::XY, 5.0).unwrap().into();
    let latitude = Plane::new(Point3::new(0.0, 0.0, 3.0), Vector3::Z).unwrap();
    let parallel: Curve = Circle::new(latitude, 4.0).unwrap().into();
    let found = intersect_curve_surface(&parallel, Interval::FULL_TURN, &ball, None).unwrap();
    assert_eq!(found.overlaps.len(), 1);
    let tube: Surface = Cylinder::new(Plane::XY, 4.0).unwrap().into();
    let found = intersect_curve_surface(&parallel, Interval::FULL_TURN, &tube, None).unwrap();
    assert_eq!(found.overlaps.len(), 1);
    let ring: Surface = Torus::new(Plane::XY, 6.0, 2.0).unwrap().into();
    let crest = Plane::new(Point3::new(0.0, 0.0, 2.0), Vector3::Z).unwrap();
    let crest: Curve = Circle::new(crest, 6.0).unwrap().into();
    let found = intersect_curve_surface(&crest, Interval::FULL_TURN, &ring, None).unwrap();
    assert_eq!(found.overlaps.len(), 1);
    let kissing_frame = Plane::new(Point3::new(8.0, 0.0, 0.0), Vector3::Y).unwrap();
    let kissing: Curve = Circle::new(kissing_frame, 3.0).unwrap().into();
    let found = intersect_curve_surface(&kissing, Interval::FULL_TURN, &ball, None).unwrap();
    assert_eq!(found.points.len(), 1, "{found:?}");
    assert!(found.points[0].tangent);
}

#[test]
fn points_at_range_ends_are_reported_once_and_patches_filter() {
    let floor = plane(Point3::ZERO, Vector3::Z);
    let drop: Curve = Line::new(Point3::new(0.0, 0.0, 5.0), Vector3::NEG_Z)
        .unwrap()
        .into();
    for range in [
        Interval::new(0.0, 5.0).unwrap(),
        Interval::new(5.0, 9.0).unwrap(),
        Interval::new(0.0, 5.0 - 1e-8).unwrap(),
    ] {
        let found = intersect_curve_surface(&drop, range, &floor, None).unwrap();
        let (at, tangent) = single(&found);
        assert!(!tangent);
        assert!(at == 5.0 || at == range.end(), "{at} in {range:?}");
    }
    let spline: Curve = BSpline::clamped_uniform(
        3,
        vec![
            Point3::new(0.0, 0.0, 3.0),
            Point3::new(1.0, 0.0, 1.0),
            Point3::new(2.0, 0.0, -1.0),
            Point3::new(3.0, 0.0, 0.0),
        ],
    )
    .unwrap()
    .into();
    let found = intersect_curve_surface(&spline, Interval::UNIT, &floor, None).unwrap();
    assert_eq!(found.points.len(), 2);
    assert_eq!(found.points[1].parameter, 1.0);
    let tube: Surface = Cylinder::new(Plane::XY, 2.0).unwrap().into();
    let through: Curve = Line::new(Point3::new(-5.0, 0.0, 0.0), Vector3::X)
        .unwrap()
        .into();
    let range = Interval::new(0.0, 10.0).unwrap();
    let found = intersect_curve_surface(
        &through,
        range,
        &tube,
        Some(uv_box((-1.0, 1.0), (-1.0, 1.0))),
    )
    .unwrap();
    let (at, _) = single(&found);
    assert!((at - 7.0).abs() < 1e-9);
    assert!(found.points[0].uv.x.abs() < 1e-9);
    let found = intersect_curve_surface(
        &through,
        range,
        &tube,
        Some(uv_box((2.0, 4.0), (-1.0, 1.0))),
    )
    .unwrap();
    let (at, _) = single(&found);
    assert!((at - 3.0).abs() < 1e-9);
    let lying: Curve = Line::new(Point3::new(0.0, 2.0, -4.0), Vector3::Z)
        .unwrap()
        .into();
    let found = intersect_curve_surface(
        &lying,
        Interval::new(0.0, 8.0).unwrap(),
        &tube,
        Some(uv_box((0.0, TAU), (-1.0, 3.0))),
    )
    .unwrap();
    assert_eq!(found.overlaps.len(), 1);
    assert!((found.overlaps[0].range.start() - 3.0).abs() < 1e-9);
    assert!((found.overlaps[0].range.end() - 7.0).abs() < 1e-9);
}

#[test]
fn a_long_edge_overlaps_a_small_face_between_its_samples() {
    let floor = plane(Point3::ZERO, Vector3::Z);
    let edge: Curve = Line::new(Point3::ZERO, Vector3::X).unwrap().into();
    let span = Interval::new(0.0, 200.0).unwrap();
    let face = uv_box((33.0, 34.0), (-1.0, 1.0));

    let found = intersect_curve_surface(&edge, span, &floor, Some(face)).unwrap();

    assert!(found.points.is_empty(), "{found:?}");
    assert_eq!(found.overlaps.len(), 1, "{found:?}");
    assert!((found.overlaps[0].range.start() - 33.0).abs() < 1e-7);
    assert!((found.overlaps[0].range.end() - 34.0).abs() < 1e-7);
}

#[test]
fn a_rim_overlaps_a_narrow_strip_of_its_cylinder() {
    let tube: Surface = Cylinder::new(Plane::XY, 2.0).unwrap().into();
    let rim: Curve = Circle::new(Plane::XY, 2.0).unwrap().into();
    let strip = uv_box((1.0, 1.02), (-1.0, 1.0));

    let found = intersect_curve_surface(&rim, Interval::FULL_TURN, &tube, Some(strip)).unwrap();

    assert_eq!(found.overlaps.len(), 1, "{found:?}");
    assert!((found.overlaps[0].range.start() - 1.0).abs() < 1e-7);
    assert!((found.overlaps[0].range.end() - 1.02).abs() < 1e-7);
}

#[test]
fn a_circle_crossing_a_cylinder_at_a_grazing_angle_is_met_at_its_roots() {
    let offset = 1e-3;
    let bore: Curve = Circle::new(Plane::XY, 2.5).unwrap().into();
    let plug: Surface = Cylinder::new(frame(Point3::new(offset, 0.0, 0.0), Vector3::Z), 2.5)
        .unwrap()
        .into();
    let found =
        intersect_curve_surface(&bore, crate::interval::Interval::FULL_TURN, &plug, None).unwrap();

    assert_eq!(found.points.len(), 2, "{found:?}");
    for point in &found.points {
        assert!(!point.tangent, "{point:?}");
        assert!((point.point.x - 0.5 * offset).abs() < 1e-9, "{point:?}");
    }
}
