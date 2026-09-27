use std::{
    f64::consts::{FRAC_PI_2, TAU},
    time::{Duration, Instant},
};

use caditor_geometry::{Plane, Point3, Vector3};

use super::*;
use crate::{
    intersect::{SurfaceIntersection, intersect_surfaces},
    surface::{Cone, Cylinder, Sphere, Torus},
    test_support::Random,
};

const ROUNDS: usize = 60;
const TIME_LIMIT: Duration = Duration::from_secs(120);

fn random_frame(random: &mut Random) -> Plane {
    let mut normal = random.point(1.0);
    if normal.length() < 0.1 {
        normal = Vector3::Z;
    }
    if random.unit() < 0.3 {
        normal = [Vector3::X, Vector3::Y, Vector3::Z][(random.unit() * 3.0) as usize % 3];
    }
    let origin = if random.unit() < 0.2 {
        Point3::ZERO
    } else {
        random.point(2.0)
    };
    Plane::with_x_axis(
        origin,
        normal,
        random.point(1.0) + Vector3::new(0.3, 0.1, 0.0),
    )
    .unwrap_or(Plane::new(origin, normal).unwrap())
}

fn random_surface(random: &mut Random) -> (Surface, (f64, f64), (f64, f64)) {
    let frame = random_frame(random);
    let kind = random.unit();
    if kind < 0.2 {
        (
            crate::surface::PlaneSurface::new(frame).unwrap().into(),
            (-8.0, 8.0),
            (-8.0, 8.0),
        )
    } else if kind < 0.45 {
        (
            Cylinder::new(frame, random.between(0.5, 3.0))
                .unwrap()
                .into(),
            (0.0, TAU),
            (-6.0, 6.0),
        )
    } else if kind < 0.65 {
        let half_angle = random.between(0.2, 1.0) * if random.unit() < 0.5 { 1.0 } else { -1.0 };
        let cone = Cone::new(frame, random.between(0.0, 2.0), half_angle).unwrap();
        let apex = cone.apex_parameter();
        let v = if half_angle > 0.0 {
            (apex, apex + 8.0)
        } else {
            (apex - 8.0, apex)
        };
        (cone.into(), (0.0, TAU), v)
    } else if kind < 0.85 {
        (
            Sphere::new(frame, random.between(0.5, 3.0)).unwrap().into(),
            (0.0, TAU),
            (-FRAC_PI_2, FRAC_PI_2),
        )
    } else {
        let major = random.between(1.5, 3.0);
        (
            Torus::new(frame, major, random.between(0.3, 0.9) * major * 0.5)
                .unwrap()
                .into(),
            (0.0, TAU),
            (0.0, TAU),
        )
    }
}

#[test]
fn random_elementary_pairs_intersect_on_both_surfaces_in_bounded_time() {
    let mut random = Random::new(41);
    let clock = Instant::now();
    let mut meeting = 0;
    for round in 0..ROUNDS {
        let (first, first_u, first_v) = random_surface(&mut random);
        let (second, second_u, second_v) = random_surface(&mut random);
        let a = patch(&first, first_u, first_v);
        let b = patch(&second, second_u, second_v);
        let result = intersect_surfaces(&a, &b)
            .unwrap_or_else(|error| panic!("round {round}: {error} for {first:?} and {second:?}"));
        if let SurfaceIntersection::Branches { .. } = &result {
            check_all(&result, &first, &second);
            if !result.is_empty() {
                meeting += 1;
            }
        }
        assert!(
            clock.elapsed() < TIME_LIMIT,
            "round {round} ran out of time"
        );
    }
    assert!(meeting > ROUNDS / 3, "{meeting}");
}

#[test]
fn nearly_tangent_and_nearly_parallel_pairs_stay_on_both_surfaces() {
    let ball: Surface = Sphere::new(Plane::XY, 2.0).unwrap().into();
    let whole = patch(&ball, (0.0, TAU), (-FRAC_PI_2, FRAC_PI_2));
    for gap in [1e-3, 1e-5, 1e-7, 0.0, -1e-7, -1e-5] {
        let toward = Vector3::new(1.0, 0.2, 0.1).normalize();
        let other: Surface = Sphere::new(frame(toward * (3.5 - gap), Vector3::Z), 1.5)
            .unwrap()
            .into();
        let result =
            intersect_surfaces(&whole, &patch(&other, (0.0, TAU), (-FRAC_PI_2, FRAC_PI_2)))
                .unwrap();
        check_all(&result, &ball, &other);
        let found = result.branches().len() + result.points().len();
        if gap.abs() <= 1e-6 {
            assert_eq!(found, 1, "gap {gap}: {result:?}");
        }
    }
    let tube: Surface = Cylinder::new(Plane::XY, 2.0).unwrap().into();
    for tilt in [1e-3, 1e-6, 1e-9] {
        let skew: Surface = Cylinder::new(
            frame(Point3::new(3.0, 0.0, 0.0), Vector3::new(tilt, 0.0, 1.0)),
            1.5,
        )
        .unwrap()
        .into();
        let result =
            intersect_surfaces(&around(&tube, (-5.0, 5.0)), &around(&skew, (-5.0, 5.0))).unwrap();
        check_all(&result, &tube, &skew);
        assert!(!result.branches().is_empty(), "tilt {tilt}");
    }
}
