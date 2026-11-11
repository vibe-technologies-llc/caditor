use std::f64::consts::{FRAC_PI_2, TAU};

use caditor_geometry::{Maths, Plane, Point2, Point3, Vector3};

use super::*;
use crate::{
    fixtures::bumped_sheet,
    intersect::{SurfaceIntersection, intersect_surfaces},
    surface::{Cylinder, Sphere, Torus},
};

const ANGLE: f64 = 0.9;

fn cylinder(origin: Point3, axis: Vector3, radius: f64) -> Surface {
    Cylinder::new(frame(origin, axis), radius).unwrap().into()
}

fn sphere(center: Point3, radius: f64) -> Surface {
    Sphere::new(frame(center, Vector3::Z), radius)
        .unwrap()
        .into()
}

fn ring(major: f64, minor: f64) -> Surface {
    Torus::new(Plane::XY, major, minor).unwrap().into()
}

fn at(radius: f64, height: f64) -> Point3 {
    Point3::new(radius * ANGLE.cos_libm(), radius * ANGLE.sin_libm(), height)
}

fn across() -> Vector3 {
    Vector3::new(-ANGLE.sin_libm(), ANGLE.cos_libm(), 0.0)
}

struct Touching {
    name: &'static str,
    first: Surface,
    second: Surface,
    point: Point3,
}

fn touching(gap: f64) -> Vec<Touching> {
    let (wall, ball) = (12.0, 1.5);
    vec![
        Touching {
            name: "a ball on a cylinder",
            first: cylinder(Point3::ZERO, Vector3::Z, wall),
            second: sphere(at(wall + ball + gap, 0.4), ball),
            point: at(wall + 0.5 * gap, 0.4),
        },
        Touching {
            name: "a ball in a bore",
            first: cylinder(Point3::ZERO, Vector3::Z, wall),
            second: sphere(at(wall - ball - gap, 0.4), ball),
            point: at(wall - 0.5 * gap, 0.4),
        },
        Touching {
            name: "a ball on a torus",
            first: ring(wall, ball),
            second: sphere(at(wall + 2.0 * ball + gap, 0.0), ball),
            point: at(wall + ball + 0.5 * gap, 0.0),
        },
        Touching {
            name: "a torus against a cylinder across it",
            first: ring(wall, ball),
            second: cylinder(at(wall + 2.0 * ball + gap, 0.0), across(), ball),
            point: at(wall + ball + 0.5 * gap, 0.0),
        },
        Touching {
            name: "a torus against a post",
            first: ring(wall, ball),
            second: cylinder(at(wall + 2.0 * ball + gap, 0.0), Vector3::Z, ball),
            point: at(wall + ball + 0.5 * gap, 0.0),
        },
    ]
}

fn whole(surface: &Surface) -> SurfacePatch<'_> {
    match surface {
        Surface::Torus(_) => patch(surface, (0.0, TAU), (0.0, TAU)),
        Surface::Sphere(_) => patch(surface, (0.0, TAU), (-FRAC_PI_2, FRAC_PI_2)),
        _ => around(surface, (-3.0, 3.0)),
    }
}

fn met(case: &Touching) -> SurfaceIntersection {
    intersect_surfaces(&whole(&case.first), &whole(&case.second)).unwrap()
}

fn assert_one_touch(name: &str, result: &SurfaceIntersection, expected: Point3) {
    assert!(result.branches().is_empty(), "{name}: {result:?}");
    let [point] = result.points() else {
        panic!("{name}: {result:?}");
    };
    assert!(point.tangent, "{name}");
    assert!(
        point.point.distance(expected) <= 1e-9,
        "{name}: {:?} is {} from {expected:?}",
        point.point,
        point.point.distance(expected)
    );
}

#[test]
fn surfaces_touching_at_a_point_from_outside_each_other_meet_there_alone() {
    for case in touching(0.0) {
        let result = met(&case);

        assert_one_touch(case.name, &result, case.point);
    }
}

#[test]
fn surfaces_touching_within_the_resolution_meet_at_one_point() {
    for gap in [4e-7, -4e-7] {
        for case in touching(gap) {
            let result = met(&case);

            assert_one_touch(case.name, &result, case.point);
        }
    }
}

#[test]
fn surfaces_overlapping_beyond_the_resolution_still_meet_in_a_loop() {
    for case in touching(-1e-5) {
        let result = met(&case);

        check_all(&result, &case.first, &case.second);
        assert!(result.points().is_empty(), "{}: {result:?}", case.name);
        let [branch] = result.branches() else {
            panic!("{}: {result:?}", case.name);
        };
        assert!(branch.closed, "{}", case.name);
    }
}

#[test]
fn a_spline_bump_touching_a_plane_meets_it_at_its_peak() {
    let sheet: Surface = bumped_sheet(4.0, 1.0, 7, 0.5).into();
    let peak = sheet.point_at(Point2::new(0.5, 0.5));
    let lid = plane(peak, Vector3::Z);

    let result =
        intersect_surfaces(&patch(&sheet, (0.0, 1.0), (0.0, 1.0)), &square(&lid, 5.0)).unwrap();

    assert_one_touch("a bump under a plane", &result, peak);
}
