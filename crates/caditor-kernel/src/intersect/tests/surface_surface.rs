use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, FRAC_PI_6, PI, TAU};

use caditor_geometry::{Plane, Point3, RigidTransform, Vector3};

use super::*;
use crate::{
    bspline::BSpline,
    curve::Curve,
    intersect::{IntersectionError, SurfaceIntersection, intersect_surfaces},
    sense::Sense,
    surface::{Cone, Cylinder, Extrusion, Revolution, Sphere, Torus},
};

fn cylinder(origin: Point3, axis: Vector3, radius: f64) -> Surface {
    Cylinder::new(frame(origin, axis), radius).unwrap().into()
}

fn sphere(center: Point3, radius: f64) -> Surface {
    Sphere::new(frame(center, Vector3::Z), radius)
        .unwrap()
        .into()
}

fn whole_ball(surface: &Surface) -> SurfacePatch<'_> {
    patch(surface, (0.0, TAU), (-FRAC_PI_2, FRAC_PI_2))
}

fn tilted_plane(angle: f64, offset: f64) -> Surface {
    plane(
        Point3::new(0.0, 0.0, offset),
        Vector3::new(angle.sin(), 0.0, angle.cos()),
    )
}

fn curves_of(result: &SurfaceIntersection) -> Vec<&Curve> {
    result
        .branches()
        .iter()
        .map(|branch| &branch.curve)
        .collect()
}

#[test]
fn planes_meet_in_a_line_and_coincide_with_a_sense() {
    let first = plane(Point3::ZERO, Vector3::Z);
    let second = plane(Point3::new(1.0, 0.0, 0.0), Vector3::new(1.0, 1.0, 0.0));
    let result = intersect_surfaces(&square(&first, 10.0), &square(&second, 10.0)).unwrap();
    check_all(&result, &first, &second);
    assert_eq!(result.branches().len(), 1);
    assert!(matches!(result.branches()[0].curve, Curve::Line(_)));
    let flipped = plane(Point3::new(0.0, 0.0, 1e-9), Vector3::NEG_Z);
    assert_eq!(
        intersect_surfaces(&square(&first, 1.0), &square(&flipped, 1.0)).unwrap(),
        SurfaceIntersection::Coincident(Sense::Reversed)
    );
    let parallel = plane(Point3::new(0.0, 0.0, 1.0), Vector3::Z);
    assert!(
        intersect_surfaces(&square(&first, 1.0), &square(&parallel, 1.0))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_plane_cuts_a_cylinder_in_circles_ellipses_and_lines() {
    let radius = 3.0;
    let tube = cylinder(Point3::ZERO, Vector3::Z, radius);
    let side = around(&tube, (-20.0, 20.0));
    for angle in [0.0, FRAC_PI_6, 1.2] {
        let cut = tilted_plane(angle, 1.0);
        let result = intersect_surfaces(&square(&cut, 60.0), &side).unwrap();
        check_all(&result, &cut, &tube);
        assert_eq!(result.branches().len(), 1, "angle {angle}");
        let branch = &result.branches()[0];
        assert!(branch.closed);
        match &branch.curve {
            Curve::Circle(circle) => {
                assert_eq!(angle, 0.0);
                assert!((circle.radius() - radius).abs() < 1e-12);
            }
            Curve::Ellipse(ellipse) => {
                assert!((ellipse.major_radius() - radius / angle.cos()).abs() < 1e-9);
                assert!((ellipse.minor_radius() - radius).abs() < 1e-12);
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    let steep = tilted_plane((89.9f64).to_radians(), 0.0);
    let result = intersect_surfaces(&square(&steep, 60.0), &side).unwrap();
    check_all(&result, &steep, &tube);
    assert_eq!(result.branches().len(), 2);
    assert!(
        result
            .branches()
            .iter()
            .all(|branch| matches!(branch.curve, Curve::Ellipse(_)) && !branch.closed)
    );
    let along = plane(Point3::new(1.0, 0.0, 0.0), Vector3::X);
    let result = intersect_surfaces(&square(&along, 60.0), &side).unwrap();
    check_all(&result, &along, &tube);
    assert_eq!(result.branches().len(), 2);
    assert!(
        result
            .branches()
            .iter()
            .all(|branch| matches!(branch.curve, Curve::Line(_)) && !branch.tangent)
    );
    assert!((total_length(&result) - 80.0).abs() < 1e-6);
    let touching = plane(Point3::new(radius, 0.0, 0.0), Vector3::X);
    let result = intersect_surfaces(&square(&touching, 60.0), &side).unwrap();
    check_all(&result, &touching, &tube);
    assert_eq!(result.branches().len(), 1);
    assert!(result.branches()[0].tangent);
    let missing = plane(Point3::new(radius + 1e-3, 0.0, 0.0), Vector3::X);
    assert!(
        intersect_surfaces(&square(&missing, 60.0), &side)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn cylinder_patches_clip_the_intersection() {
    let tube = cylinder(Point3::ZERO, Vector3::Z, 2.0);
    let cut = tilted_plane(0.4, 0.0);
    let half = patch(&tube, (-FRAC_PI_2, FRAC_PI_2), (-5.0, 5.0));
    let result = intersect_surfaces(&square(&cut, 20.0), &half).unwrap();
    check_all(&result, &cut, &tube);
    assert_eq!(result.branches().len(), 1);
    let branch = &result.branches()[0];
    assert!(!branch.closed);
    assert!((branch.range.length() - PI).abs() < 1e-9);
    for uv in [branch.start_uv[1], branch.end_uv[1]] {
        assert!((uv.x.abs() - FRAC_PI_2).abs() < 1e-9, "{uv:?}");
    }
}

#[test]
fn a_plane_cuts_a_cone_in_every_conic() {
    let half_angle = FRAC_PI_6;
    let cone: Surface = Cone::new(Plane::XY, 2.0, half_angle).unwrap().into();
    let apex = match &cone {
        Surface::Cone(cone) => cone.apex(),
        _ => panic!("expected a cone"),
    };
    let side = around(&cone, (-4.0, 30.0));
    let circle = plane(Point3::new(0.0, 0.0, 5.0), Vector3::Z);
    let result = intersect_surfaces(&square(&circle, 40.0), &side).unwrap();
    check_all(&result, &circle, &cone);
    assert!(matches!(curves_of(&result)[..], [Curve::Circle(_)]));
    let ellipse = tilted_plane(0.3, 5.0);
    let result = intersect_surfaces(&square(&ellipse, 40.0), &side).unwrap();
    check_all(&result, &ellipse, &cone);
    assert!(matches!(curves_of(&result)[..], [Curve::Ellipse(_)]));
    assert!(result.branches()[0].closed);
    let slope = FRAC_PI_2 - half_angle;
    let parabola = plane(
        Point3::new(0.0, 0.0, 5.0),
        Vector3::new(slope.sin(), 0.0, slope.cos()),
    );
    let result = intersect_surfaces(&square(&parabola, 40.0), &side).unwrap();
    check_all(&result, &parabola, &cone);
    assert_eq!(result.branches().len(), 1);
    assert!(total_length(&result) > 10.0);
    let hyperbola = plane(Point3::new(1.0, 0.0, 0.0), Vector3::X);
    let result = intersect_surfaces(&square(&hyperbola, 40.0), &side).unwrap();
    check_all(&result, &hyperbola, &cone);
    assert_eq!(result.branches().len(), 1);
    let through = plane(apex, Vector3::Y);
    let result = intersect_surfaces(&square(&through, 40.0), &side).unwrap();
    check_all(&result, &through, &cone);
    assert_eq!(result.branches().len(), 2);
    assert!(
        result
            .branches()
            .iter()
            .all(|branch| matches!(branch.curve, Curve::Line(_)))
    );
    let along_ruling = plane(apex, Vector3::new(half_angle.cos(), 0.0, -half_angle.sin()));
    let result = intersect_surfaces(&square(&along_ruling, 40.0), &side).unwrap();
    check_all(&result, &along_ruling, &cone);
    assert_eq!(result.branches().len(), 1);
    assert!(result.branches()[0].tangent);
    let only_apex = plane(apex, Vector3::new(0.1, 0.0, 1.0));
    let result = intersect_surfaces(&square(&only_apex, 40.0), &side).unwrap();
    assert!(result.branches().is_empty());
    assert_eq!(result.points().len(), 1);
    assert!(result.points()[0].point.distance(apex) < 1e-9);
    assert!(result.points()[0].tangent);
}

#[test]
fn spheres_meet_in_circles_or_touch() {
    let first = sphere(Point3::ZERO, 5.0);
    let second = sphere(Point3::new(3.0, 4.0, 0.0), 4.0);
    let result = intersect_surfaces(&whole_ball(&first), &whole_ball(&second)).unwrap();
    check_all(&result, &first, &second);
    assert!(matches!(curves_of(&result)[..], [Curve::Circle(_)]));
    assert!((total_length(&result) - TAU * (25.0f64 - 3.4 * 3.4).sqrt()).abs() < 1e-9);
    for (center, radius) in [
        (Point3::new(0.0, 0.0, 8.0), 3.0),
        (Point3::new(0.0, 3.0, 0.0), 2.0),
    ] {
        let other = sphere(center, radius);
        let result = intersect_surfaces(&whole_ball(&first), &whole_ball(&other)).unwrap();
        assert!(result.branches().is_empty(), "{center:?}");
        assert_eq!(result.points().len(), 1);
        assert!(result.points()[0].tangent);
    }
    let apart = sphere(Point3::new(0.0, 0.0, 9.0), 3.0);
    assert!(
        intersect_surfaces(&whole_ball(&first), &whole_ball(&apart))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_small_sphere_barely_poking_through_a_plane_is_found() {
    let ball = sphere(Point3::new(1.0, 2.0, -0.999_99), 1.0);
    let floor = plane(Point3::ZERO, Vector3::Z);
    let result = intersect_surfaces(
        &square(&floor, 10.0),
        &patch(&ball, (0.0, TAU), (-FRAC_PI_2, FRAC_PI_2)),
    )
    .unwrap();
    check_all(&result, &floor, &ball);
    let [branch] = result.branches() else {
        panic!("{result:?}");
    };
    let expected = (1.0f64 - 0.999_99f64 * 0.999_99).sqrt();
    match &branch.curve {
        Curve::Circle(circle) => assert!((circle.radius() - expected).abs() < 1e-9),
        other => panic!("{other:?}"),
    }
}

#[test]
fn coaxial_surfaces_of_revolution_meet_in_circles() {
    let tube = cylinder(Point3::ZERO, Vector3::Z, 2.0);
    let cone: Surface = Cone::new(Plane::XY, 1.0, 0.4).unwrap().into();
    let result =
        intersect_surfaces(&around(&tube, (-10.0, 10.0)), &around(&cone, (-2.0, 10.0))).unwrap();
    check_all(&result, &tube, &cone);
    assert!(matches!(curves_of(&result)[..], [Curve::Circle(_)]));
    let ball = sphere(Point3::new(0.0, 0.0, 1.0), 3.0);
    let result = intersect_surfaces(
        &around(&tube, (-10.0, 10.0)),
        &patch(&ball, (0.0, TAU), (-FRAC_PI_2, FRAC_PI_2)),
    )
    .unwrap();
    check_all(&result, &tube, &ball);
    assert_eq!(result.branches().len(), 2);
    let ring: Surface = Torus::new(Plane::XY, 5.0, 1.0).unwrap().into();
    let hugging = cylinder(Point3::ZERO, Vector3::Z, 6.0);
    let result = intersect_surfaces(
        &around(&hugging, (-10.0, 10.0)),
        &patch(&ring, (0.0, TAU), (0.0, TAU)),
    )
    .unwrap();
    check_all(&result, &hugging, &ring);
    assert_eq!(result.branches().len(), 1);
    assert!(result.branches()[0].tangent);
    let profile: Curve = BSpline::clamped_uniform(
        3,
        vec![
            Point3::new(1.0, 0.0, -3.0),
            Point3::new(3.0, 0.0, -1.0),
            Point3::new(2.0, 0.0, 1.0),
            Point3::new(3.0, 0.0, 3.0),
        ],
    )
    .unwrap()
    .into();
    let vase: Surface = Revolution::new(profile, Point3::ZERO, Vector3::Z)
        .unwrap()
        .into();
    let level = plane(Point3::new(0.0, 0.0, 0.5), Vector3::Z);
    let result =
        intersect_surfaces(&square(&level, 10.0), &patch(&vase, (0.0, TAU), (0.0, 1.0))).unwrap();
    check_all(&result, &level, &vase);
    assert!(matches!(curves_of(&result)[..], [Curve::Circle(_)]));
}

#[test]
fn a_plane_meets_a_torus_through_along_and_tangent_to_the_tube() {
    let ring: Surface = Torus::new(Plane::XY, 6.0, 2.0).unwrap().into();
    let whole = patch(&ring, (0.0, TAU), (0.0, TAU));
    let through = plane(Point3::new(0.0, 0.0, 1.0), Vector3::Z);
    let result = intersect_surfaces(&square(&through, 20.0), &whole).unwrap();
    check_all(&result, &through, &ring);
    assert_eq!(result.branches().len(), 2);
    let top = plane(Point3::new(0.0, 0.0, 2.0), Vector3::Z);
    let result = intersect_surfaces(&square(&top, 20.0), &whole).unwrap();
    check_all(&result, &top, &ring);
    assert_eq!(result.branches().len(), 1);
    assert!(result.branches()[0].tangent);
    match &result.branches()[0].curve {
        Curve::Circle(circle) => assert!((circle.radius() - 6.0).abs() < 1e-9),
        other => panic!("{other:?}"),
    }
    let meridian = plane(Point3::ZERO, Vector3::Y);
    let result = intersect_surfaces(&square(&meridian, 20.0), &whole).unwrap();
    check_all(&result, &meridian, &ring);
    assert_eq!(result.branches().len(), 2);
    let near_top = Plane::new(Point3::new(6.0, 0.0, 1.98), Vector3::new(0.05, 0.02, 1.0)).unwrap();
    let cap: Surface = crate::surface::PlaneSurface::new(near_top).unwrap().into();
    let result = intersect_surfaces(&square(&cap, 20.0), &whole).unwrap();
    check_all(&result, &cap, &ring);
    assert_eq!(result.branches().len(), 1, "{result:?}");
    assert!(result.branches()[0].closed);
    let outer = Plane::new(Point3::new(7.99, 0.0, 0.0), Vector3::new(1.0, 0.03, 0.02)).unwrap();
    let skim: Surface = crate::surface::PlaneSurface::new(outer).unwrap().into();
    let result = intersect_surfaces(&square(&skim, 20.0), &whole).unwrap();
    check_all(&result, &skim, &ring);
    assert_eq!(result.branches().len(), 1, "{result:?}");
    assert!(result.branches()[0].closed);
    assert!(total_length(&result) < 3.0, "{}", total_length(&result));
    let barely = Plane::new(
        Point3::new(7.999_99, 0.0, 0.0),
        Vector3::new(1.0, 0.03, 0.02),
    )
    .unwrap();
    let barely: Surface = crate::surface::PlaneSurface::new(barely).unwrap().into();
    let result = intersect_surfaces(&square(&barely, 20.0), &whole).unwrap();
    check_all(&result, &barely, &ring);
    assert_eq!(result.branches().len(), 1, "{result:?}");
    assert!(result.branches()[0].closed);
}

#[test]
fn crossing_cylinders_of_equal_and_unequal_radius() {
    let first = cylinder(Point3::ZERO, Vector3::Z, 2.0);
    let second = cylinder(Point3::ZERO, Vector3::X, 2.0);
    let a = around(&first, (-6.0, 6.0));
    let b = around(&second, (-6.0, 6.0));
    let result = intersect_surfaces(&a, &b).unwrap();
    check_all(&result, &first, &second);
    assert_eq!(result.branches().len(), 2);
    assert!(
        result
            .branches()
            .iter()
            .all(|branch| matches!(branch.curve, Curve::Ellipse(_)) && branch.closed)
    );
    assert_eq!(result.points().len(), 2);
    assert!(result.points().iter().all(|point| point.tangent));
    let slanted = cylinder(Point3::ZERO, Vector3::new(1.0, 0.0, 1.0), 2.0);
    let result = intersect_surfaces(&a, &around(&slanted, (-8.0, 8.0))).unwrap();
    check_all(&result, &first, &slanted);
    assert_eq!(result.branches().len(), 2);
    let thin = cylinder(Point3::ZERO, Vector3::X, 1.0);
    let result = intersect_surfaces(&a, &around(&thin, (-6.0, 6.0))).unwrap();
    check_all(&result, &first, &thin);
    assert_eq!(result.branches().len(), 2, "{result:?}");
    assert!(result.branches().iter().all(|branch| branch.closed));
    let offset = cylinder(Point3::new(0.0, 0.5, 0.0), Vector3::X, 1.0);
    let result = intersect_surfaces(&a, &around(&offset, (-6.0, 6.0))).unwrap();
    check_all(&result, &first, &offset);
    assert_eq!(result.branches().len(), 2);
    let parallel = cylinder(Point3::new(3.0, 0.0, 0.0), Vector3::Z, 1.5);
    let result = intersect_surfaces(&a, &around(&parallel, (-6.0, 6.0))).unwrap();
    check_all(&result, &first, &parallel);
    assert_eq!(result.branches().len(), 2);
    assert!(
        result
            .branches()
            .iter()
            .all(|branch| matches!(branch.curve, Curve::Line(_)))
    );
    let touching = cylinder(Point3::new(4.0, 0.0, 0.0), Vector3::Z, 2.0);
    let result = intersect_surfaces(&a, &around(&touching, (-6.0, 6.0))).unwrap();
    assert_eq!(result.branches().len(), 1);
    assert!(result.branches()[0].tangent);
}

#[test]
fn a_sphere_off_a_cylinder_axis_is_marched() {
    let tube = cylinder(Point3::new(1.0, 0.0, 0.0), Vector3::Z, 1.0);
    let ball = sphere(Point3::ZERO, 2.0);
    let result = intersect_surfaces(
        &around(&tube, (-5.0, 5.0)),
        &patch(&ball, (0.0, TAU), (-FRAC_PI_2, FRAC_PI_2)),
    )
    .unwrap();
    check_all(&result, &tube, &ball);
    assert!(!result.branches().is_empty());
    let pierced = cylinder(Point3::new(0.5, 0.0, 0.0), Vector3::Z, 0.7);
    let result = intersect_surfaces(
        &around(&pierced, (-5.0, 5.0)),
        &patch(&ball, (0.0, TAU), (-FRAC_PI_2, FRAC_PI_2)),
    )
    .unwrap();
    check_all(&result, &pierced, &ball);
    assert_eq!(result.branches().len(), 2, "{result:?}");
    assert!(result.branches().iter().all(|branch| branch.closed));
}

#[test]
fn a_marched_intersection_stops_when_interrupted() {
    let tube = cylinder(Point3::new(1.0, 0.0, 0.0), Vector3::Z, 1.0);
    let ball = sphere(Point3::ZERO, 2.0);
    let (tube_patch, ball_patch) = (around(&tube, (-5.0, 5.0)), whole_ball(&ball));
    let polls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = std::sync::Arc::clone(&polls);
    let stop = std::sync::Arc::new(move || {
        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= 20
    });

    let stopped = crate::interruptible(stop, || intersect_surfaces(&tube_patch, &ball_patch));

    assert!(
        matches!(stopped, Err(IntersectionError::Cancelled(_))),
        "{stopped:?}"
    );
    assert_eq!(polls.load(std::sync::atomic::Ordering::SeqCst), 21);
}

fn spline_extrusion() -> Surface {
    let profile: Curve = BSpline::clamped_uniform(
        3,
        vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(2.0, 3.0, 0.0),
            Point3::new(5.0, -1.0, 0.0),
            Point3::new(8.0, 2.0, 0.0),
            Point3::new(10.0, 0.0, 0.0),
        ],
    )
    .unwrap()
    .into();
    Extrusion::new(profile, Vector3::new(0.1, 0.2, 1.0))
        .unwrap()
        .into()
}

#[test]
fn a_plane_cuts_a_spline_extrusion_exactly() {
    let wall = spline_extrusion();
    let sheet = patch(&wall, (0.0, 1.0), (-5.0, 5.0));
    let cut = plane(Point3::new(0.0, 0.0, 1.5), Vector3::new(0.2, -0.1, 1.0));
    let result = intersect_surfaces(&square(&cut, 30.0), &sheet).unwrap();
    check_all(&result, &cut, &wall);
    assert!(matches!(curves_of(&result)[..], [Curve::BSpline(_)]));
    let upright = plane(Point3::new(4.0, 0.0, 0.0), Vector3::new(1.0, 0.0, -0.1));
    let result = intersect_surfaces(&square(&upright, 30.0), &sheet).unwrap();
    check_all(&result, &upright, &wall);
    assert!(!result.branches().is_empty());
    assert!(
        result
            .branches()
            .iter()
            .all(|branch| matches!(branch.curve, Curve::Line(_)))
    );
}

#[test]
fn identical_and_moved_surfaces_are_coincident() {
    let ring: Surface = Torus::new(Plane::XY, 4.0, 1.0).unwrap().into();
    let turn = RigidTransform::rotation_about(Point3::ZERO, Vector3::Z, FRAC_PI_4).unwrap();
    let turned = ring.transformed(&turn).unwrap();
    assert_eq!(
        intersect_surfaces(
            &patch(&ring, (0.0, TAU), (0.0, TAU)),
            &patch(&turned, (0.0, TAU), (0.0, TAU))
        )
        .unwrap(),
        SurfaceIntersection::Coincident(Sense::Same)
    );
}

#[test]
fn swept_surfaces_are_marched_against_other_surfaces() {
    let wall = spline_extrusion();
    let sheet = patch(&wall, (0.0, 1.0), (-5.0, 5.0));
    let post = cylinder(Point3::new(5.0, 0.5, 0.0), Vector3::new(0.3, 1.0, 0.2), 1.0);
    let result = intersect_surfaces(&sheet, &around(&post, (-6.0, 6.0))).unwrap();
    check_all(&result, &wall, &post);
    assert!(!result.branches().is_empty());
    let profile: Curve = BSpline::clamped_uniform(
        3,
        vec![
            Point3::new(1.0, 0.0, -3.0),
            Point3::new(3.0, 0.0, -1.0),
            Point3::new(2.0, 0.0, 1.0),
            Point3::new(3.0, 0.0, 3.0),
        ],
    )
    .unwrap()
    .into();
    let vase: Surface = Revolution::new(profile, Point3::ZERO, Vector3::Z)
        .unwrap()
        .into();
    let slant = plane(Point3::new(0.0, 0.0, 0.3), Vector3::new(0.3, 0.1, 1.0));
    let result =
        intersect_surfaces(&square(&slant, 10.0), &patch(&vase, (0.0, TAU), (0.0, 1.0))).unwrap();
    check_all(&result, &slant, &vase);
    assert_eq!(result.branches().len(), 1, "{result:?}");
    assert!(result.branches()[0].closed);
}

fn flat_spline(corners: [Point3; 4]) -> Surface {
    let [a, b, c, d] = corners;
    crate::surface::BSplineSurface::new(
        1,
        1,
        vec![0.0, 0.0, 1.0, 1.0],
        vec![0.0, 0.0, 1.0, 1.0],
        2,
        vec![a, b, c, d],
        None,
    )
    .unwrap()
    .into()
}

#[test]
fn spline_patches_that_share_only_part_of_their_extent_are_coincident() {
    let unit = (0.0, 1.0);
    let left = flat_spline([
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(10.0, 0.0, 0.0),
        Point3::new(0.0, 10.0, 0.0),
        Point3::new(10.0, 10.0, 0.0),
    ]);
    let right = flat_spline([
        Point3::new(5.0, 2.0, 0.0),
        Point3::new(15.0, 2.0, 0.0),
        Point3::new(5.0, 12.0, 0.0),
        Point3::new(15.0, 12.0, 0.0),
    ]);
    let flipped = flat_spline([
        Point3::new(5.0, 2.0, 0.0),
        Point3::new(5.0, 12.0, 0.0),
        Point3::new(15.0, 2.0, 0.0),
        Point3::new(15.0, 12.0, 0.0),
    ]);
    let tilted = flat_spline([
        Point3::new(5.0, 2.0, -1.0),
        Point3::new(15.0, 2.0, 3.0),
        Point3::new(5.0, 12.0, -1.0),
        Point3::new(15.0, 12.0, 3.0),
    ]);
    let found = |first: &Surface, second: &Surface| {
        intersect_surfaces(&patch(first, unit, unit), &patch(second, unit, unit)).unwrap()
    };
    assert_eq!(
        found(&left, &right),
        SurfaceIntersection::Coincident(Sense::Same)
    );
    assert_eq!(
        found(&right, &left),
        SurfaceIntersection::Coincident(Sense::Same)
    );
    assert_eq!(
        found(&left, &flipped),
        SurfaceIntersection::Coincident(Sense::Reversed)
    );
    let SurfaceIntersection::Branches { branches, .. } = found(&left, &tilted) else {
        panic!("a crossing spline is not coincident");
    };
    let [branch] = branches.as_slice() else {
        panic!("expected one crossing line, found {branches:?}");
    };
    for fraction in [0.0, 0.5, 1.0] {
        let point = branch.curve.point(branch.range.at(fraction));
        assert!(
            (point.x - 7.5).abs() < 1e-6 && point.z.abs() < 1e-6,
            "{point}"
        );
    }
}
