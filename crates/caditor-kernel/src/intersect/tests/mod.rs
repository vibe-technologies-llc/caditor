mod curve;
mod curve_curve;
mod curve_surface;
mod stress;
mod surface_surface;

use std::f64::consts::TAU;

use caditor_geometry::{Aabb2, Plane, Point2, Point3, Vector3};

use crate::{
    curve::Curve,
    intersect::{IntersectionBranch, SurfaceIntersection, SurfacePatch},
    interval::Interval,
    surface::{PlaneSurface, Surface},
    tolerance::LINEAR_RESOLUTION,
};

pub(super) fn uv_box(u: (f64, f64), v: (f64, f64)) -> Aabb2 {
    Aabb2::from_point(Point2::new(u.0, v.0)).including(Point2::new(u.1, v.1))
}

pub(super) fn patch(surface: &Surface, u: (f64, f64), v: (f64, f64)) -> SurfacePatch<'_> {
    SurfacePatch::new(surface, uv_box(u, v)).unwrap()
}

pub(super) fn around(surface: &Surface, v: (f64, f64)) -> SurfacePatch<'_> {
    patch(surface, (0.0, TAU), v)
}

pub(super) fn square(surface: &Surface, reach: f64) -> SurfacePatch<'_> {
    patch(surface, (-reach, reach), (-reach, reach))
}

pub(super) fn plane(origin: Point3, normal: Vector3) -> Surface {
    PlaneSurface::new(Plane::new(origin, normal).unwrap())
        .unwrap()
        .into()
}

pub(super) fn frame(origin: Point3, normal: Vector3) -> Plane {
    Plane::new(origin, normal).unwrap()
}

pub(super) fn distance(surface: &Surface, point: Point3) -> f64 {
    surface.distance(point)
}

pub(super) fn samples(curve: &Curve, range: Interval, count: usize) -> Vec<f64> {
    let mut parameters: Vec<f64> = range.split(count).collect();
    if let Curve::Intersection(intersection) = curve {
        parameters.extend(
            intersection
                .nodes()
                .windows(2)
                .map(|pair| 0.5 * (pair[0].parameter + pair[1].parameter))
                .filter(|parameter| range.contains(*parameter)),
        );
    }
    parameters
}

pub(super) fn check_branch(branch: &IntersectionBranch, first: &Surface, second: &Surface) {
    assert!(branch.range.length() > 0.0);
    for parameter in samples(&branch.curve, branch.range, 97) {
        let point = branch.curve.point(parameter);
        let (a, b) = (distance(first, point), distance(second, point));
        assert!(
            a <= LINEAR_RESOLUTION && b <= LINEAR_RESOLUTION,
            "point {point:?} at {parameter} lies {a} and {b} off the surfaces of {:?}",
            branch.curve
        );
    }
    if let Curve::Intersection(intersection) = &branch.curve {
        assert!(
            intersection
                .nodes()
                .windows(2)
                .all(|pair| pair[1].parameter > pair[0].parameter)
        );
    }
    let (start, end) = (
        branch.curve.point(branch.range.start()),
        branch.curve.point(branch.range.end()),
    );
    for (surface, uv, expected) in [
        (first, branch.start_uv[0], start),
        (second, branch.start_uv[1], start),
        (first, branch.end_uv[0], end),
        (second, branch.end_uv[1], end),
    ] {
        let point = surface.point_at(uv);
        assert!(
            point.distance(expected) <= 1e-5,
            "branch end uv {uv:?} maps to {point:?}, not {expected:?}"
        );
    }
}

pub(super) fn check_all(result: &SurfaceIntersection, first: &Surface, second: &Surface) {
    let branches = result.branches();
    for branch in branches {
        check_branch(branch, first, second);
    }
    for point in result.points() {
        assert!(distance(first, point.point) <= LINEAR_RESOLUTION);
        assert!(distance(second, point.point) <= LINEAR_RESOLUTION);
    }
    for (index, branch) in branches.iter().enumerate() {
        for other in branches.iter().skip(index + 1) {
            let shared = branch
                .range
                .split(40)
                .filter(|parameter| {
                    let point = branch.curve.point(*parameter);
                    let closest = other.curve.closest_parameter(point, other.range);
                    other.curve.point(closest).distance(point) <= 1e-5
                })
                .count();
            assert!(
                shared <= 3,
                "branches trace the same curve ({shared} shared samples)"
            );
        }
    }
}

pub(super) fn total_length(result: &SurfaceIntersection) -> f64 {
    result
        .branches()
        .iter()
        .map(|branch| branch.curve.length(branch.range))
        .sum()
}
