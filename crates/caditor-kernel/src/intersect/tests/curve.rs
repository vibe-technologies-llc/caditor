use caditor_geometry::{Point3, RigidTransform, Vector3};

use super::*;
use crate::{
    curve::{Curve, IntersectionCurve, IntersectionNode},
    error::GeometryError,
    intersect::intersect_surfaces,
    surface::{Cylinder, Torus},
    tolerance::{INTERSECTION_TOLERANCE, SamplingTolerance},
};

fn marched() -> (Curve, Interval, [Surface; 2]) {
    let big: Surface = Cylinder::new(frame(Point3::ZERO, Vector3::Z), 2.0)
        .unwrap()
        .into();
    let thin: Surface = Cylinder::new(frame(Point3::new(0.0, 0.3, 0.0), Vector3::X), 1.0)
        .unwrap()
        .into();
    let result =
        intersect_surfaces(&around(&big, (-5.0, 5.0)), &around(&thin, (-5.0, 5.0))).unwrap();
    let branch = result.branches()[0].clone();
    assert!(matches!(branch.curve, Curve::Intersection(_)));
    assert!(branch.closed);
    (branch.curve, branch.range, [big, thin])
}

#[test]
fn the_interpolant_stays_on_both_surfaces() {
    let (curve, range, [big, thin]) = marched();
    let worst = range
        .split(3000)
        .map(|parameter| {
            let point = curve.point(parameter);
            big.distance(point).max(thin.distance(point))
        })
        .fold(0.0, f64::max);
    assert!(worst <= 2.0 * INTERSECTION_TOLERANCE, "{worst}");
    let Curve::Intersection(intersection) = &curve else {
        panic!("expected an intersection curve");
    };
    for parameter in range.split(50) {
        let refined = intersection.refined_point(parameter);
        assert!(big.distance(refined) < 1e-9 && thin.distance(refined) < 1e-9);
        let [first, second] = intersection.uv_at(parameter);
        let point = curve.point(parameter);
        assert!(big.point_at(first).distance(point) <= LINEAR_RESOLUTION);
        assert!(thin.point_at(second).distance(point) <= LINEAR_RESOLUTION);
    }
}

#[test]
fn intersection_curves_support_every_curve_operation() {
    let (curve, range, [big, thin]) = marched();
    let period = curve.period().unwrap();
    assert!((period - range.length()).abs() < 1e-12);
    let step = 1e-6;
    for parameter in range.split(37).skip(1).take(35) {
        let exact = curve.evaluate(parameter);
        let ahead = curve.evaluate(parameter + step);
        let behind = curve.evaluate(parameter - step);
        let numeric = (ahead.point - behind.point) / (2.0 * step);
        assert!((numeric - exact.first).length() < 1e-5);
        assert!((exact.first.length() - 1.0).abs() < 0.05);
        assert!(curve.point(parameter + period).distance(exact.point) < 1e-9);
        let found = curve.closest_parameter(exact.point, range);
        assert!(curve.point(found).distance(exact.point) < 1e-9);
    }
    let bounds = curve.bounding_box(range);
    for parameter in range.split(400) {
        let point = curve.point(parameter);
        assert!(point.cmpge(bounds.min() - Vector3::splat(1e-12)).all());
        assert!(point.cmple(bounds.max() + Vector3::splat(1e-12)).all());
    }
    let samples = curve.sample(range, &SamplingTolerance::new(1e-3, 0.1).unwrap());
    let polyline: f64 = samples
        .windows(2)
        .map(|pair| pair[0].point.distance(pair[1].point))
        .sum();
    let length = curve.length(range);
    assert!(length >= polyline && length - polyline < 1e-2 * length);
    let reversed = curve.reversed();
    for parameter in range.split(20) {
        let back = curve.reversed_parameter(parameter);
        assert!(reversed.point(back).distance(curve.point(parameter)) < 1e-12);
    }
    let turn = RigidTransform::rotation_about(Point3::new(1.0, 2.0, 3.0), Vector3::Y, 0.7)
        .unwrap()
        .then(&RigidTransform::translation(Vector3::new(4.0, -1.0, 2.0)).unwrap());
    let moved = curve.transformed(&turn).unwrap();
    let (big_moved, thin_moved) = (
        big.transformed(&turn).unwrap(),
        thin.transformed(&turn).unwrap(),
    );
    for parameter in range.split(20) {
        let point = moved.point(parameter);
        assert!(point.distance(turn.apply_point(curve.point(parameter))) < 1e-9);
        assert!(big_moved.distance(point) <= LINEAR_RESOLUTION);
        assert!(thin_moved.distance(point) <= LINEAR_RESOLUTION);
    }
    let Curve::Intersection(intersection) = &curve else {
        panic!("expected an intersection curve");
    };
    let across = Interval::new(range.end() - 1.0, range.end() + 2.0).unwrap();
    let piece = intersection.trimmed(across).unwrap();
    assert!(!piece.is_closed());
    assert_eq!(piece.domain(), across);
    for parameter in across.split(60) {
        assert!(piece.point(parameter).distance(curve.point(parameter)) < 1e-12);
    }
    let whole = intersection.trimmed(range).unwrap();
    assert!(whole.is_closed());
    let found = curve.closest_parameter(curve.point(range.start() + 0.01), across);
    assert!((found - (range.end() + 0.01)).abs() < 1e-6);
}

#[test]
fn intersection_curves_reject_malformed_nodes() {
    let ring: Surface = Torus::new(frame(Point3::ZERO, Vector3::Z), 4.0, 1.0)
        .unwrap()
        .into();
    let node = |parameter: f64, point: Point3| IntersectionNode {
        parameter,
        point,
        derivative: Vector3::X,
        uv: [caditor_geometry::Point2::ZERO; 2],
    };
    let surfaces = || [ring.clone(), ring.clone()];
    assert!(IntersectionCurve::new(surfaces(), vec![node(0.0, Point3::ZERO)], false).is_err());
    assert_eq!(
        IntersectionCurve::new(
            surfaces(),
            vec![node(0.0, Point3::ZERO), node(0.0, Point3::X)],
            false
        ),
        Err(GeometryError::Knots)
    );
    assert!(
        IntersectionCurve::new(
            surfaces(),
            vec![node(0.0, Point3::ZERO), node(1.0, Point3::X)],
            true
        )
        .is_err()
    );
    let curve = IntersectionCurve::new(
        surfaces(),
        vec![node(0.0, Point3::ZERO), node(1.0, Point3::X)],
        false,
    )
    .unwrap();
    assert!(curve.point(0.5).distance(Point3::new(0.5, 0.0, 0.0)) < 1e-15);
    assert!(curve.point(7.0).distance(Point3::X) < 1e-15);
}

#[test]
fn an_intersection_curve_edge_gets_pcurves_on_both_surfaces() {
    let (curve, range, [big, thin]) = marched();
    let mut builder = crate::topology::SolidBuilder::new();
    let vertex = builder.vertex(curve.point(range.start())).unwrap();
    let edge = builder.edge(curve.clone(), range, vertex, vertex).unwrap();
    let shell = builder.shell().unwrap();
    for (surface, sense) in [
        (big, crate::sense::Sense::Same),
        (thin, crate::sense::Sense::Reversed),
    ] {
        let face = builder
            .face(shell, surface, crate::sense::Sense::Same)
            .unwrap();
        builder.add_loop(face, &[(edge, sense)]).unwrap();
    }
    let solid = builder.build_unchecked();
    assert_eq!(solid.coedges().count(), 2);
    for (id, coedge) in solid.coedges() {
        let surface = solid
            .face(solid.coedge_face(id).unwrap())
            .unwrap()
            .surface();
        for sample in coedge.pcurve().samples() {
            let expected = curve.point(sample.parameter);
            assert!(surface.point_at(sample.uv).distance(expected) <= LINEAR_RESOLUTION);
        }
    }
}
