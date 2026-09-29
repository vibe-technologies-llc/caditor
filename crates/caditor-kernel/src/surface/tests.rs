use std::f64::consts::{FRAC_PI_2, PI, TAU};

use caditor_geometry::{Plane, Point2, Point3, RigidTransform, Vector3};

use super::*;
use crate::{
    bspline::BSpline,
    curve::{Circle, Curve, Line},
    test_support::Random,
    tolerance::{LINEAR_RESOLUTION, MAX_SIZE},
};

fn tilted() -> Plane {
    Plane::with_x_axis(
        Point3::new(2.0, -1.0, 3.0),
        Vector3::new(0.3, -0.5, 1.0),
        Vector3::new(1.0, 1.0, 0.0),
    )
    .unwrap()
}

fn profile_spline() -> Curve {
    BSpline::clamped_uniform(
        3,
        vec![
            Point3::new(2.0, 0.0, -3.0),
            Point3::new(4.0, 0.0, -1.0),
            Point3::new(3.0, 0.0, 1.0),
            Point3::new(5.0, 0.0, 2.5),
            Point3::new(4.0, 0.0, 4.0),
        ],
    )
    .unwrap()
    .into()
}

fn extrusion_profile() -> Curve {
    BSpline::clamped_uniform(
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
    .into()
}

fn dome_profile() -> Curve {
    let frame = Plane::from_frame(Point3::ZERO, Vector3::NEG_Y, Vector3::X).unwrap();
    Circle::new(frame, 3.0).unwrap().into()
}

fn surfaces() -> Vec<Surface> {
    vec![
        PlaneSurface::new(tilted()).unwrap().into(),
        Cylinder::new(tilted(), 3.0).unwrap().into(),
        Cone::new(tilted(), 2.0, 0.4).unwrap().into(),
        Cone::new(tilted(), 2.0, -0.3).unwrap().into(),
        Sphere::new(tilted(), 4.0).unwrap().into(),
        Torus::new(tilted(), 5.0, 1.5).unwrap().into(),
        Extrusion::new(extrusion_profile(), Vector3::new(0.2, 0.1, 1.0))
            .unwrap()
            .into(),
        Revolution::new(profile_spline(), Point3::new(0.0, 0.0, 1.0), Vector3::Z)
            .unwrap()
            .into(),
    ]
}

fn interior_uv(surface: &Surface, random: &mut Random) -> Point2 {
    let u = surface
        .u_domain()
        .clipped(3.0)
        .at(random.between(0.05, 0.95));
    let v = surface
        .v_domain()
        .clipped(3.0)
        .at(random.between(0.05, 0.95));
    Point2::new(u, v)
}

#[test]
fn derivatives_match_finite_differences() {
    let mut random = Random::new(5);
    let step = 1e-6;
    for surface in surfaces() {
        for _ in 0..40 {
            let uv = interior_uv(&surface, &mut random);
            let exact = surface.evaluate(uv.x, uv.y);
            let u_ahead = surface.evaluate(uv.x + step, uv.y);
            let u_behind = surface.evaluate(uv.x - step, uv.y);
            let v_ahead = surface.evaluate(uv.x, uv.y + step);
            let v_behind = surface.evaluate(uv.x, uv.y - step);
            let close = |numeric: Vector3, analytic: Vector3| {
                (numeric - analytic).length() < 1e-5 * (1.0 + analytic.length())
            };
            assert!(
                close((u_ahead.point - u_behind.point) / (2.0 * step), exact.du),
                "{surface:?}"
            );
            assert!(
                close((v_ahead.point - v_behind.point) / (2.0 * step), exact.dv),
                "{surface:?}"
            );
            assert!(
                close((u_ahead.du - u_behind.du) / (2.0 * step), exact.duu),
                "{surface:?}"
            );
            assert!(
                close((v_ahead.du - v_behind.du) / (2.0 * step), exact.duv),
                "{surface:?}"
            );
            assert!(
                close((u_ahead.dv - u_behind.dv) / (2.0 * step), exact.duv),
                "{surface:?}"
            );
            assert!(
                close((v_ahead.dv - v_behind.dv) / (2.0 * step), exact.dvv),
                "{surface:?}"
            );
        }
    }
}

#[test]
fn normals_follow_the_parametrization() {
    let mut random = Random::new(9);
    for surface in surfaces() {
        for _ in 0..40 {
            let uv = interior_uv(&surface, &mut random);
            let parametric = surface.evaluate(uv.x, uv.y).normal().unwrap();
            let normal = surface.normal(uv.x, uv.y).unwrap();
            assert!((normal - parametric).length() < 1e-9, "{surface:?} at {uv}");
        }
    }
}

#[test]
fn projection_round_trips_points_on_and_near_the_surface() {
    let mut random = Random::new(13);
    for surface in surfaces() {
        for _ in 0..100 {
            let uv = interior_uv(&surface, &mut random);
            let point = surface.point_at(uv);
            let found = surface.project(point, None);
            assert!(
                surface.point_at(found).distance(point) < 1e-9,
                "{surface:?} at {uv}"
            );

            let normal = surface.normal(uv.x, uv.y).unwrap();
            let lifted = point + normal * 0.05;
            let found = surface.project(lifted, Some(uv));
            assert!(
                surface.point_at(found).distance(point) < 1e-7,
                "{surface:?} at {uv}: {found}"
            );
            assert!((found - uv).length() < 1e-6, "{surface:?} at {uv}: {found}");
        }
    }
}

#[test]
fn projection_picks_the_periodic_branch_nearest_the_hint() {
    let mut random = Random::new(17);
    for surface in surfaces() {
        let (Some(u_period), v_period) = (surface.u_period(), surface.v_period()) else {
            continue;
        };
        for _ in 0..20 {
            let uv = interior_uv(&surface, &mut random);
            let point = surface.point_at(uv);
            let shifted = Point2::new(uv.x + u_period, uv.y + v_period.unwrap_or(0.0));
            let found = surface.project(point, Some(shifted + Point2::splat(0.1)));
            assert!(
                (found - shifted).length() < 1e-7,
                "{surface:?}: {found} vs {shifted}"
            );
            let principal = surface.project(point, None);
            assert!((0.0..u_period).contains(&principal.x));
        }
    }
}

#[test]
fn poles_sit_where_the_u_derivative_vanishes() {
    for surface in surfaces() {
        for pole in surface.poles() {
            for u in [0.0, 1.0, 4.0] {
                let at = surface.evaluate(u, pole.v);
                assert!(at.point.distance(pole.point) < 1e-9);
                assert!(at.du.length() < 1e-9);
                assert!(surface.pole_at(Point2::new(u, pole.v)).is_some());
            }
        }
    }
    let sphere = Surface::from(Sphere::new(Plane::XY, 2.0).unwrap());
    assert_eq!(sphere.poles().len(), 2);
    let north = sphere.project(Point3::new(0.0, 0.0, 5.0), Some(Point2::new(1.25, 0.0)));
    assert_eq!(north, Point2::new(1.25, FRAC_PI_2));
    assert!(sphere.normal(3.0, FRAC_PI_2).unwrap().distance(Vector3::Z) < 1e-15);

    let cone = Surface::from(Cone::new(Plane::XY, 2.0, 0.5).unwrap());
    let apex = cone.poles()[0];
    assert!(
        apex.point
            .distance(Point3::new(0.0, 0.0, -2.0 / 0.5f64.tan()))
            < 1e-12
    );
    let below = cone.project(Point3::new(0.1, 0.0, -20.0), Some(Point2::new(2.0, 0.0)));
    assert_eq!(below, Point2::new(2.0, apex.v));
    assert!(cone.normal(1.0, apex.v).is_some());

    let dome = Surface::from(Revolution::new(dome_profile(), Point3::ZERO, Vector3::Z).unwrap());
    assert!(dome.poles().is_empty());
    let capped = Revolution::new(
        BSpline::clamped_uniform(
            1,
            vec![
                Point3::new(0.0, 0.0, 3.0),
                Point3::new(3.0, 0.0, 3.0),
                Point3::new(3.0, 0.0, 0.0),
            ],
        )
        .unwrap()
        .into(),
        Point3::ZERO,
        Vector3::Z,
    )
    .unwrap();
    let capped = Surface::from(capped);
    let poles = capped.poles();
    assert_eq!(poles.len(), 1);
    assert_eq!(poles[0].v, 0.0);
    assert!(capped.normal(0.3, 0.0).unwrap().distance(Vector3::NEG_Z) < 1e-6);
    assert_eq!(dome.u_period(), Some(TAU));
    assert_eq!(dome.v_period(), Some(TAU));
}

#[test]
fn transforms_move_every_point() {
    let transform = RigidTransform::rotation_about(
        Point3::new(1.0, 2.0, 0.0),
        Vector3::new(1.0, 0.3, 0.2),
        0.9,
    )
    .unwrap()
    .then(&RigidTransform::translation(Vector3::new(-4.0, 6.0, 1.0)).unwrap());
    let mut random = Random::new(19);
    for surface in surfaces() {
        let moved = surface.transformed(&transform).unwrap();
        for _ in 0..10 {
            let uv = interior_uv(&surface, &mut random);
            let expected = transform.apply_point(surface.point_at(uv));
            assert!(moved.point_at(uv).distance(expected) < 1e-9, "{surface:?}");
        }
    }
}

#[test]
fn every_surface_is_the_same_as_itself_and_its_transformed_copy_is_not() {
    let shift = RigidTransform::translation(Vector3::new(0.0, 0.0, 0.01))
        .unwrap()
        .then(&RigidTransform::rotation_about(Point3::ZERO, Vector3::X, 0.01).unwrap());
    for surface in surfaces() {
        assert_eq!(
            surface.same_surface(&surface),
            Some(Sense::Same),
            "{surface:?}"
        );
        let moved = surface.transformed(&shift).unwrap();
        assert_eq!(surface.same_surface(&moved), None, "{surface:?}");
    }
}

#[test]
fn planes_match_regardless_of_frame_and_report_flipped_normals() {
    let base = Surface::from(PlaneSurface::new(tilted()).unwrap());
    let origin = tilted().to_world(Point2::new(7.0, -3.0));
    let turned = Plane::with_x_axis(origin, tilted().normal(), tilted().y_axis()).unwrap();
    assert_eq!(
        base.same_surface(&PlaneSurface::new(turned).unwrap().into()),
        Some(Sense::Same)
    );
    let flipped = PlaneSurface::new(turned.flipped()).unwrap();
    assert_eq!(base.same_surface(&flipped.into()), Some(Sense::Reversed));
    let parallel = Plane::from_frame(
        origin + tilted().normal() * 0.01,
        tilted().normal(),
        tilted().x_axis(),
    )
    .unwrap();
    assert_eq!(
        base.same_surface(&PlaneSurface::new(parallel).unwrap().into()),
        None
    );
}

#[test]
fn cylinders_match_with_different_seams_and_axis_directions() {
    let base = Surface::from(Cylinder::new(Plane::XY, 3.0).unwrap());
    let reseamed = Plane::with_x_axis(
        Point3::new(0.0, 0.0, 17.0),
        Vector3::NEG_Z,
        Vector3::new(1.0, 1.0, 0.0),
    )
    .unwrap();
    let other = Surface::from(Cylinder::new(reseamed, 3.0).unwrap());
    assert_eq!(base.same_surface(&other), Some(Sense::Same));
    assert_eq!(other.same_surface(&base), Some(Sense::Same));
    let wider = Surface::from(Cylinder::new(reseamed, 3.1).unwrap());
    assert_eq!(base.same_surface(&wider), None);
    let beside = Plane::from_frame(Point3::new(0.1, 0.0, 0.0), Vector3::Z, Vector3::X).unwrap();
    assert_eq!(
        base.same_surface(&Cylinder::new(beside, 3.0).unwrap().into()),
        None
    );

    let circle: Curve = Circle::new(
        Plane::from_frame(Point3::new(0.0, 0.0, 4.0), Vector3::Z, Vector3::Y).unwrap(),
        3.0,
    )
    .unwrap()
    .into();
    let outward = Extrusion::new(circle.clone(), Vector3::Z).unwrap();
    assert_eq!(base.same_surface(&outward.into()), Some(Sense::Same));
    let inward = Extrusion::new(circle, Vector3::NEG_Z).unwrap();
    assert_eq!(base.same_surface(&inward.into()), Some(Sense::Reversed));
    let revolved_line = Revolution::new(
        Line::new(Point3::new(0.0, 3.0, 0.0), Vector3::Z)
            .unwrap()
            .into(),
        Point3::ZERO,
        Vector3::Z,
    )
    .unwrap();
    assert_eq!(base.same_surface(&revolved_line.into()), Some(Sense::Same));
}

#[test]
fn spheres_cones_and_tori_match_by_shape_not_frame() {
    let frame = Plane::with_x_axis(Point3::new(1.0, 2.0, 3.0), Vector3::Y, Vector3::Z).unwrap();
    let flipped = Plane::with_x_axis(frame.origin(), Vector3::NEG_Y, Vector3::X).unwrap();
    let sphere = Surface::from(Sphere::new(frame, 2.0).unwrap());
    assert_eq!(
        sphere.same_surface(&Sphere::new(flipped, 2.0).unwrap().into()),
        Some(Sense::Same)
    );
    let torus = Surface::from(Torus::new(frame, 5.0, 1.0).unwrap());
    assert_eq!(
        torus.same_surface(&Torus::new(flipped, 5.0, 1.0).unwrap().into()),
        Some(Sense::Same)
    );
    assert_eq!(
        torus.same_surface(&Torus::new(flipped, 5.0, 1.1).unwrap().into()),
        None
    );

    let cone = Cone::new(frame, 2.0, 0.4).unwrap();
    let apex = cone.apex();
    let from_apex = Plane::with_x_axis(apex, Vector3::NEG_Y, Vector3::X).unwrap();
    let opposite = Cone::new(from_apex, 0.0, -0.4).unwrap();
    let cone = Surface::from(cone);
    assert_eq!(cone.same_surface(&opposite.into()), Some(Sense::Same));
    let mirrored = Cone::new(from_apex, 0.0, 0.4).unwrap();
    assert_eq!(cone.same_surface(&mirrored.into()), None);
    assert_eq!(cone.same_surface(&sphere), None);
}

#[test]
fn swept_surfaces_match_shifted_and_reversed_copies() {
    let direction = Vector3::new(0.2, 0.1, 1.0);
    let base = Surface::from(Extrusion::new(extrusion_profile(), direction).unwrap());
    let shifted_profile = extrusion_profile()
        .transformed(&RigidTransform::translation(direction * 2.5).unwrap())
        .unwrap();
    let shifted = Surface::from(Extrusion::new(shifted_profile, -direction).unwrap());
    assert_eq!(base.same_surface(&shifted), Some(Sense::Reversed));
    let reversed =
        Surface::from(Extrusion::new(extrusion_profile().reversed(), -direction).unwrap());
    assert_eq!(base.same_surface(&reversed), Some(Sense::Same));

    let revolved = Surface::from(
        Revolution::new(profile_spline(), Point3::new(0.0, 0.0, 1.0), Vector3::Z).unwrap(),
    );
    let turned_profile = profile_spline()
        .transformed(&RigidTransform::rotation_about(Point3::ZERO, Vector3::Z, 2.0).unwrap())
        .unwrap();
    let turned = Surface::from(
        Revolution::new(turned_profile, Point3::new(0.0, 0.0, -4.0), Vector3::NEG_Z).unwrap(),
    );
    assert_eq!(revolved.same_surface(&turned), Some(Sense::Reversed));
}

#[test]
fn projecting_a_non_finite_point_stays_finite() {
    for surface in surfaces() {
        let found = surface.project(Point3::new(f64::NAN, 0.0, 1.0), None);
        assert!(found.is_finite(), "{surface:?}");
        let hint = Point2::new(0.5, 0.25);
        assert_eq!(
            surface.project(Point3::splat(f64::INFINITY), Some(hint)),
            hint
        );
    }
}

#[test]
fn constructors_reject_degenerate_surfaces() {
    assert!(matches!(
        Cylinder::new(Plane::XY, 0.0),
        Err(GeometryError::NonPositive(_))
    ));
    assert!(matches!(
        Cone::new(Plane::XY, 1.0, 0.0),
        Err(GeometryError::ConeAngle(_))
    ));
    assert!(matches!(
        Cone::new(Plane::XY, 1.0, FRAC_PI_2),
        Err(GeometryError::ConeAngle(_))
    ));
    assert!(matches!(
        Cone::new(Plane::XY, -1.0, 0.3),
        Err(GeometryError::NonPositive(_))
    ));
    assert!(matches!(
        Cone::new(Plane::XY, 2.0 * MAX_SIZE, 0.3),
        Err(GeometryError::BeyondMaximum(_))
    ));
    assert!(matches!(
        Sphere::new(Plane::XY, 0.1 * LINEAR_RESOLUTION),
        Err(GeometryError::BelowResolution(_))
    ));
    assert!(matches!(
        Torus::new(Plane::XY, 2.0 * MAX_SIZE, 1.0),
        Err(GeometryError::BeyondMaximum(_))
    ));
    assert!(Cylinder::new(Plane::XY, MAX_SIZE).is_ok());
    assert!(Cylinder::new(Plane::XY, LINEAR_RESOLUTION).is_ok());
    assert!(matches!(
        Torus::new(Plane::XY, 1.0, 1.0),
        Err(GeometryError::SelfIntersectingTorus { .. })
    ));
    assert!(matches!(
        Sphere::new(Plane::XY, f64::NAN),
        Err(GeometryError::NonFinite)
    ));
    let vertical = Curve::from(Line::new(Point3::ZERO, Vector3::Z).unwrap());
    assert_eq!(
        Extrusion::new(vertical.clone(), Vector3::Z),
        Err(GeometryError::DegenerateSurface)
    );
    assert_eq!(
        Revolution::new(vertical, Point3::ZERO, Vector3::Z),
        Err(GeometryError::DegenerateSurface)
    );
    assert_eq!(
        Revolution::new(profile_spline(), Point3::ZERO, Vector3::ZERO),
        Err(GeometryError::ZeroDirection)
    );
    let _ = PI;
}

#[test]
fn a_line_extruded_almost_along_itself_is_refused_and_a_steep_one_projects_exactly() {
    let along = Line::new(Point3::ZERO, Vector3::new(1e-8, 0.0, 1.0)).unwrap();
    assert_eq!(
        Extrusion::new(along.into(), Vector3::Z),
        Err(GeometryError::DegenerateSurface)
    );

    let steep = Line::new(Point3::new(3.0, -2.0, 1.0), Vector3::new(1e-4, 0.0, 1.0)).unwrap();
    let surface: Surface = Extrusion::new(steep.into(), Vector3::Z).unwrap().into();
    for (u, v) in [(0.0, 0.0), (250.0, -40.0), (-3_000.0, 7_000.0)] {
        let point = surface.evaluate(u, v).point;
        let found = surface.project(point, None);
        let back = surface.evaluate(found.x, found.y).point;
        assert!(back.distance(point) < 1e-9, "{u} {v}: {found:?}");
        assert!(
            (found.x - u).abs() < 1e-6 * (1.0 + u.abs()),
            "{u}: {found:?}"
        );
    }
}
