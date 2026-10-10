use std::{
    collections::{BTreeMap, BTreeSet},
    f64::consts::{FRAC_PI_2, PI, TAU},
    time::{Duration, Instant},
};

use caditor_geometry::{Plane, Point2, Point3, Vector2, Vector3};

use super::*;
use crate::{
    build::plan::{Plan, PlanCoedge, PlanError, PlanFace},
    curve::{Circle, Curve},
    interval::Interval,
    naming::{EdgeName, FaceName},
    numeric::integrate,
    profile::{Profile, ProfileCurve, Region, RegionKey, Selection},
    surface::PlaneSurface,
    tessellation::Mesh,
    test_support::{
        Random, arc, assert_cancelled_anywhere, assert_watertight, circle, line, rectangle, spline,
    },
    tolerance::{MAX_SIZE, SamplingTolerance},
    topology::{BuildError, Solid},
};

const FEATURE: u64 = 7;
const CHORD: f64 = 1e-3;
const DENSE_SPLINE_TIME_LIMIT: Duration = Duration::from_secs(20);
const NEARLY_TOUCHING_LUMPS_TIME_LIMIT: Duration = Duration::from_secs(30);

fn regions(curves: &[ProfileCurve]) -> Vec<Region> {
    Profile::new(curves)
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap()
}

fn tilted() -> Plane {
    Plane::with_x_axis(
        Point3::new(1.0, 2.0, 3.0),
        Vector3::new(1.0, 1.0, 1.0),
        Vector3::new(1.0, -1.0, 0.0),
    )
    .unwrap()
}

fn fine_mesh(solid: &Solid) -> Mesh {
    solid
        .tessellate(&SamplingTolerance::new(CHORD, 0.2).unwrap())
        .unwrap()
}

fn assert_balanced(name: &str, mesh: &Mesh) {
    let mut directed: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    for [a, b, c] in mesh.position_triangles() {
        assert!(a != b && b != c && a != c, "{name}: degenerate triangle");
        for edge in [(a, b), (b, c), (c, a)] {
            *directed.entry(edge).or_default() += 1;
        }
    }
    for ((from, to), count) in &directed {
        assert_eq!(
            directed.get(&(*to, *from)),
            Some(count),
            "{name}: edge {from}->{to} is not matched"
        );
    }
}

fn check(name: &str, solid: &Solid, volume: f64, area: f64, smallest_radius: f64) {
    check_closed(
        name,
        solid,
        volume,
        area,
        smallest_radius,
        assert_watertight,
    );
}

fn check_pinched(name: &str, solid: &Solid, volume: f64, area: f64, smallest_radius: f64) {
    check_closed(name, solid, volume, area, smallest_radius, assert_balanced);
}

fn check_closed(
    name: &str,
    solid: &Solid,
    volume: f64,
    area: f64,
    smallest_radius: f64,
    closed: fn(&str, &Mesh),
) {
    assert_eq!(solid.validate(), Ok(()), "{name}");
    for tolerance in [
        SamplingTolerance::new(0.05, 0.3).unwrap(),
        solid.default_tolerance(),
    ] {
        closed(name, &solid.tessellate(&tolerance).unwrap());
    }
    let properties = fine_mesh(solid).mass_properties();
    assert!(
        (properties.volume - volume).abs() <= 2.0 * CHORD * area + 1e-9,
        "{name}: volume {} vs {volume}",
        properties.volume
    );
    let allowance = if smallest_radius.is_finite() {
        2.0 * CHORD / smallest_radius * area
    } else {
        1e-9
    };
    assert!(
        (properties.area - area).abs() <= allowance.max(1e-9),
        "{name}: area {} vs {area}",
        properties.area
    );
    let faces: BTreeSet<FaceName> = solid.faces().map(|(_, face)| face.name()).collect();
    assert_eq!(
        faces.len(),
        solid.faces().count(),
        "{name}: repeated face names"
    );
    assert!(!faces.contains(&FaceName::NONE), "{name}");
    assert!(solid.faces().all(|(_, face)| face.origin().is_some()));
    let edges: BTreeSet<EdgeName> = solid.edges().map(|(_, edge)| edge.name()).collect();
    assert_eq!(
        edges.len(),
        solid.edges().count(),
        "{name}: repeated edge names"
    );
    assert!(!edges.contains(&EdgeName::NONE), "{name}");
}

fn spline_length(region: &Region) -> f64 {
    region
        .pieces()
        .filter(|piece| matches!(piece.curve(), crate::curve2::Curve2::BSpline(_)))
        .map(|piece| piece.curve().length(piece.range()))
        .sum()
}

fn one_side(distance: f64) -> LinearExtent {
    LinearExtent::one_side(distance).unwrap()
}

#[test]
fn an_extruded_rectangle_is_a_box() {
    let profile = regions(&rectangle(1, (0.0, 0.0), (4.0, 3.0)));
    for plane in [Plane::XY, tilted()] {
        let solid = extrude(&plane, &profile, one_side(2.0), FEATURE).unwrap();
        check("box", &solid, 24.0, 52.0, f64::INFINITY);
        assert_eq!(solid.faces().count(), 6);
        assert_eq!(solid.shells().count(), 1);
    }
    let backwards = extrude(&Plane::XY, &profile, one_side(-2.0), FEATURE).unwrap();
    check("box backwards", &backwards, 24.0, 52.0, f64::INFINITY);
    let bounds = backwards.bounding_box().unwrap();
    assert!((bounds.min().z + 2.0).abs() < 1e-12);
}

#[test]
fn an_extruded_plate_keeps_its_hole() {
    let mut curves = rectangle(1, (0.0, 0.0), (10.0, 8.0));
    curves.push(circle(5, (4.0, 4.0), 2.0));
    let solid = extrude(&Plane::XY, &regions(&curves), one_side(3.0), FEATURE).unwrap();
    let face = 80.0 - 4.0 * PI;
    check(
        "plate",
        &solid,
        face * 3.0,
        2.0 * face + 36.0 * 3.0 + 4.0 * PI * 3.0,
        2.0,
    );
    assert_eq!(solid.faces().count(), 7);
}

#[test]
fn an_extruded_circle_is_a_cylinder() {
    let solid = extrude(
        &tilted(),
        &regions(&[circle(1, (1.0, -2.0), 3.0)]),
        one_side(5.0),
        FEATURE,
    )
    .unwrap();
    check("cylinder", &solid, 45.0 * PI, 48.0 * PI, 3.0);
    assert_eq!(solid.faces().count(), 3);
}

fn perimeter(region: &Region) -> f64 {
    region
        .pieces()
        .map(|piece| piece.curve().length(piece.range()))
        .sum()
}

#[test]
fn an_extruded_ellipse_is_an_elliptic_cylinder_on_an_extrusion_surface() {
    let (major, minor, height) = (5.0, 2.0, 3.0);
    let ellipse = ProfileCurve::ellipse(1, Point2::new(1.0, -2.0), Vector2::new(3.0, 4.0), minor);
    let profile = regions(&[ellipse]);
    assert_eq!(profile.len(), 1);
    assert!((profile[0].area() - PI * major * minor).abs() < 1e-6);
    let around = perimeter(&profile[0]);
    for plane in [Plane::XY, tilted()] {
        let solid = extrude(&plane, &profile, one_side(height), FEATURE).unwrap();
        check(
            "elliptic cylinder",
            &solid,
            PI * major * minor * height,
            2.0 * PI * major * minor + around * height,
            minor * minor / major,
        );
        assert_eq!(solid.faces().count(), 3);
        assert!(
            solid
                .faces()
                .any(|(_, face)| matches!(face.surface(), crate::surface::Surface::Extrusion(_)))
        );
    }
}

#[test]
fn half_an_ellipse_extrudes_and_revolves_into_half_a_spheroid() {
    let half = || {
        regions(&[
            ProfileCurve::elliptical_arc(
                1,
                Point2::ZERO,
                Vector2::new(0.0, 5.0),
                2.0,
                (Point2::new(0.0, -5.0), Point2::new(0.0, 5.0)),
            ),
            line(2, (0.0, 5.0), (0.0, -5.0)),
        ])
    };
    let profile = half();
    assert!((profile[0].area() - PI * 5.0).abs() < 1e-6);
    let solid = extrude(&Plane::XY, &profile, one_side(2.0), FEATURE).unwrap();
    let around = perimeter(&profile[0]);
    check(
        "half elliptic cylinder",
        &solid,
        PI * 5.0 * 2.0,
        2.0 * PI * 5.0 + around * 2.0,
        0.8,
    );

    let solid = revolve(&Plane::XY, &half(), y_axis(), full(), FEATURE).unwrap();
    let volume = 4.0 / 3.0 * PI * 2.0 * 2.0 * 5.0;
    let mesh = fine_mesh(&solid);
    assert_watertight("spheroid", &mesh);
    assert!((mesh.mass_properties().volume - volume).abs() < 1e-2 * volume);
}

#[test]
fn an_extruded_d_has_two_edges_between_the_same_faces() {
    let profile = regions(&[
        arc(1, (0.0, 0.0), (3.0, 0.0), (-3.0, 0.0)),
        line(2, (-3.0, 0.0), (3.0, 0.0)),
    ]);
    let solid = extrude(&Plane::XY, &profile, one_side(2.0), FEATURE).unwrap();
    check(
        "d",
        &solid,
        9.0 * PI,
        9.0 * PI + (3.0 * PI + 6.0) * 2.0,
        3.0,
    );
}

#[test]
fn an_extruded_spline_profile_uses_an_extrusion_surface() {
    let profile = regions(&[
        spline(1, &[(0.0, 0.0), (1.0, 2.0), (2.0, 0.0)]),
        line(2, (2.0, 0.0), (0.0, 0.0)),
    ]);
    let length = spline_length(&profile[0]);
    let solid = extrude(&Plane::XY, &profile, one_side(2.0), FEATURE).unwrap();
    check(
        "spline",
        &solid,
        8.0 / 3.0,
        8.0 / 3.0 + (length + 2.0) * 2.0,
        0.25,
    );
    let closed = regions(&[spline(
        3,
        &[(0.0, 0.0), (6.0, -1.0), (7.0, 5.0), (1.0, 6.0), (0.0, 0.0)],
    )]);
    assert_eq!(closed[0].pieces().count(), 1);
    let area = closed[0].area();
    let perimeter = spline_length(&closed[0]);
    let solid = extrude(&Plane::XY, &closed, one_side(1.5), FEATURE).unwrap();
    check(
        "closed spline",
        &solid,
        area * 1.5,
        2.0 * area + perimeter * 1.5,
        0.5,
    );
}

#[test]
fn separate_regions_become_separate_lumps() {
    let mut curves = rectangle(1, (0.0, 0.0), (2.0, 2.0));
    curves.extend(rectangle(5, (5.0, 0.0), (8.0, 1.0)));
    let solid = extrude(&Plane::XY, &regions(&curves), one_side(1.0), FEATURE).unwrap();
    check(
        "lumps",
        &solid,
        7.0,
        8.0 + 8.0 + 3.0 * 2.0 + 8.0,
        f64::INFINITY,
    );
    assert_eq!(solid.shells().count(), 2);
}

#[test]
fn hundreds_of_regions_sharing_their_x_become_as_many_lumps() {
    let curves: Vec<ProfileCurve> = (0..300_u32)
        .flat_map(|row| {
            let y = f64::from(row) * 3.0;
            rectangle(1 + u64::from(row) * 4, (0.0, y), (2.0, y + 2.0))
        })
        .collect();

    let solid = extrude(&Plane::XY, &regions(&curves), one_side(1.0), FEATURE).unwrap();

    assert_eq!(solid.shells().count(), 300);
    assert_eq!(solid.vertices().count(), 300 * 8);
}

#[test]
fn adjacent_regions_extrude_as_one_lump() {
    let mut curves = rectangle(1, (0.0, 0.0), (10.0, 4.0));
    curves.push(line(5, (4.0, -1.0), (4.0, 5.0)));
    let solid = extrude(&Plane::XY, &regions(&curves), one_side(1.0), FEATURE).unwrap();
    check("merged", &solid, 40.0, 80.0 + 28.0, f64::INFINITY);
    assert_eq!(solid.shells().count(), 1);
}

#[test]
fn extents_run_either_way_and_both_ways() {
    let profile = regions(&[circle(1, (0.0, 0.0), 1.0)]);
    for (name, extent, (low, high)) in [
        (
            "two sided",
            LinearExtent::two_sided(2.0, 1.0).unwrap(),
            (-1.0, 2.0),
        ),
        (
            "symmetric",
            LinearExtent::symmetric(4.0).unwrap(),
            (-2.0, 2.0),
        ),
        (
            "reversed",
            LinearExtent::new(3.0, -1.0).unwrap(),
            (-1.0, 3.0),
        ),
    ] {
        let solid = extrude(&Plane::XY, &profile, extent, FEATURE).unwrap();
        let total = high - low;
        check(name, &solid, PI * total, 2.0 * PI + TAU * total, 1.0);
        let bounds = solid.bounding_box().unwrap();
        assert!((bounds.min().z - low).abs() < 1e-9 && (bounds.max().z - high).abs() < 1e-9);
    }
    assert_eq!(LinearExtent::one_side(0.0), Err(SweepError::ZeroLength));
    assert_eq!(LinearExtent::new(f64::NAN, 1.0), Err(SweepError::NonFinite));
    assert_eq!(
        extrude(&Plane::XY, &[], one_side(1.0), FEATURE).unwrap_err(),
        SweepError::NoRegions
    );
}

fn y_axis() -> Axis2 {
    Axis2::new(Point2::ZERO, Vector2::Y).unwrap()
}

fn full() -> AngularExtent {
    AngularExtent::full()
}

#[test]
fn a_revolved_rectangle_off_the_axis_is_a_tube() {
    let profile = regions(&rectangle(1, (2.0, 0.0), (3.0, 4.0)));
    let solid = revolve(&Plane::XY, &profile, y_axis(), full(), FEATURE).unwrap();
    check("tube", &solid, 20.0 * PI, 50.0 * PI, 2.0);
    assert_eq!(solid.faces().count(), 4);
    let turned = Axis2::new(Point2::ZERO, Vector2::NEG_Y).unwrap();
    let solid = revolve(&tilted(), &profile, turned, full(), FEATURE).unwrap();
    check("tube tilted", &solid, 20.0 * PI, 50.0 * PI, 2.0);
}

#[test]
fn a_rectangle_on_the_axis_revolves_into_a_solid_cylinder() {
    let profile = regions(&rectangle(1, (0.0, 0.0), (3.0, 4.0)));
    let solid = revolve(&Plane::XY, &profile, y_axis(), full(), FEATURE).unwrap();
    check("solid cylinder", &solid, 36.0 * PI, 42.0 * PI, 3.0);
    assert_eq!(solid.faces().count(), 3);
}

#[test]
fn a_semicircle_on_the_axis_revolves_into_a_sphere() {
    let profile = regions(&[
        arc(1, (0.0, 0.0), (0.0, -2.0), (0.0, 2.0)),
        line(2, (0.0, 2.0), (0.0, -2.0)),
    ]);
    let solid = revolve(&Plane::XY, &profile, y_axis(), full(), FEATURE).unwrap();
    check("sphere", &solid, 32.0 * PI / 3.0, 16.0 * PI, 2.0);
    assert_eq!(solid.faces().count(), 1);
}

#[test]
fn an_off_axis_circle_revolves_into_a_torus() {
    let profile = regions(&[circle(1, (5.0, 0.0), 1.0)]);
    let solid = revolve(&Plane::XY, &profile, y_axis(), full(), FEATURE).unwrap();
    check("torus", &solid, 10.0 * PI * PI, 20.0 * PI * PI, 1.0);
}

#[test]
fn a_triangle_on_the_axis_revolves_into_a_cone() {
    let profile = regions(&[
        line(1, (0.0, 0.0), (3.0, 0.0)),
        line(2, (3.0, 0.0), (0.0, 4.0)),
        line(3, (0.0, 4.0), (0.0, 0.0)),
    ]);
    let solid = revolve(&Plane::XY, &profile, y_axis(), full(), FEATURE).unwrap();
    check("cone", &solid, 12.0 * PI, 24.0 * PI, 1.0);
}

#[test]
fn partial_revolutions_get_planar_caps() {
    let quarter = AngularExtent::one_side(FRAC_PI_2).unwrap();
    let tube = regions(&rectangle(1, (2.0, 0.0), (3.0, 4.0)));
    let solid = revolve(&Plane::XY, &tube, y_axis(), quarter, FEATURE).unwrap();
    check("quarter tube", &solid, 5.0 * PI, 12.5 * PI + 8.0, 2.0);
    let backwards = AngularExtent::new(0.0, -FRAC_PI_2).unwrap();
    let solid = revolve(&Plane::XY, &tube, y_axis(), backwards, FEATURE).unwrap();
    check(
        "quarter tube backwards",
        &solid,
        5.0 * PI,
        12.5 * PI + 8.0,
        2.0,
    );
    let ball = regions(&[
        arc(1, (0.0, 0.0), (0.0, -2.0), (0.0, 2.0)),
        line(2, (0.0, 2.0), (0.0, -2.0)),
    ]);
    let solid = revolve(&Plane::XY, &ball, y_axis(), quarter, FEATURE).unwrap();
    check(
        "quarter sphere",
        &solid,
        8.0 * PI / 3.0,
        4.0 * PI + 4.0 * PI,
        2.0,
    );
    let ring = regions(&[circle(1, (5.0, 0.0), 1.0)]);
    let half = AngularExtent::symmetric(PI).unwrap();
    let solid = revolve(&Plane::XY, &ring, y_axis(), half, FEATURE).unwrap();
    check(
        "half torus",
        &solid,
        5.0 * PI * PI,
        10.0 * PI * PI + TAU,
        1.0,
    );
    let cylinder = regions(&rectangle(1, (0.0, 0.0), (3.0, 4.0)));
    let solid = revolve(&Plane::XY, &cylinder, y_axis(), quarter, FEATURE).unwrap();
    check(
        "quarter cylinder",
        &solid,
        9.0 * PI,
        4.5 * PI + 6.0 * PI + 24.0,
        3.0,
    );
}

fn pappus(region: &Region) -> f64 {
    region
        .pieces()
        .map(|piece| {
            let range = piece.range();
            let breaks: Vec<f64> = range.split(64).collect();
            let integral = integrate(&breaks, |parameter| {
                let derivatives = piece.curve().evaluate(parameter);
                derivatives.point.x * derivatives.point.x * derivatives.first.y
            });
            if piece.is_reversed() {
                -integral
            } else {
                integral
            }
        })
        .sum::<f64>()
        * PI
}

#[test]
fn a_revolved_spline_profile_matches_pappus() {
    let profile = regions(&[
        spline(1, &[(1.0, 0.0), (3.0, 1.0), (3.0, 2.0), (1.0, 3.0)]),
        line(2, (1.0, 3.0), (1.0, 0.0)),
    ]);
    let volume = pappus(&profile[0]);
    let solid = revolve(&Plane::XY, &profile, y_axis(), full(), FEATURE).unwrap();
    let mesh = fine_mesh(&solid);
    assert_watertight("revolved spline", &mesh);
    let properties = mesh.mass_properties();
    assert!(
        (properties.volume - volume).abs() < 1e-2 * volume,
        "{} vs {volume}",
        properties.volume
    );
    let dome = regions(&[
        spline(1, &[(0.0, -2.0), (2.5, -2.0), (2.5, 2.0), (0.0, 2.0)]),
        line(2, (0.0, 2.0), (0.0, -2.0)),
    ]);
    let volume = pappus(&dome[0]);
    let solid = revolve(&Plane::XY, &dome, y_axis(), full(), FEATURE).unwrap();
    let mesh = fine_mesh(&solid);
    assert_watertight("dome", &mesh);
    assert!((mesh.mass_properties().volume - volume).abs() < 1e-2 * volume);
    let solid = revolve(
        &Plane::XY,
        &dome,
        y_axis(),
        AngularExtent::one_side(1.0).unwrap(),
        FEATURE,
    )
    .unwrap();
    let mesh = fine_mesh(&solid);
    assert_watertight("dome wedge", &mesh);
    assert!((mesh.mass_properties().volume - volume / TAU).abs() < 1e-2 * volume);
}

#[test]
fn an_arc_reaching_the_axis_revolves_through_a_revolution_surface() {
    let profile = regions(&[
        arc(1, (1.0, 0.0), (0.0, -3f64.sqrt()), (0.0, 3f64.sqrt())),
        line(2, (0.0, 3f64.sqrt()), (0.0, -3f64.sqrt())),
    ]);
    let volume = pappus(&profile[0]);
    let solid = revolve(&Plane::XY, &profile, y_axis(), full(), FEATURE).unwrap();
    let mesh = fine_mesh(&solid);
    assert_watertight("lemon", &mesh);
    assert!((mesh.mass_properties().volume - volume).abs() < 1e-2 * volume);
}

#[test]
fn revolutions_refuse_profiles_across_the_axis() {
    let straddling = regions(&rectangle(1, (-1.0, 0.0), (2.0, 1.0)));
    assert_eq!(
        revolve(&Plane::XY, &straddling, y_axis(), full(), FEATURE).unwrap_err(),
        SweepError::CrossesAxis {
            entities: vec![1, 3]
        }
    );
    let mut curves = rectangle(1, (-3.0, 0.0), (-2.0, 1.0));
    curves.extend(rectangle(5, (2.0, 0.0), (3.0, 1.0)));
    let error = revolve(&Plane::XY, &regions(&curves), y_axis(), full(), FEATURE).unwrap_err();
    assert!(
        matches!(error, SweepError::BothSidesOfAxis { .. }),
        "{error}"
    );
    assert_eq!(error.entities().len(), 8);
    assert_eq!(AngularExtent::new(1.0, 1.0), Err(SweepError::ZeroAngle));
    assert_eq!(
        AngularExtent::new(0.0, 7.0),
        Err(SweepError::BeyondFullTurn)
    );
    assert_eq!(
        Axis2::new(Point2::ZERO, Vector2::ZERO),
        Err(SweepError::DegenerateAxis)
    );
    assert_eq!(
        SweepError::CrossesAxis {
            entities: vec![1, 3, 4]
        }
        .to_string(),
        "curves 1, 3 and 4 cross the revolution axis"
    );
    let _ = Interval::UNIT;
}

fn random_curves(random: &mut Random) -> Vec<ProfileCurve> {
    let mut curves = Vec::new();
    let count = 2 + (random.unit() * 5.0) as usize;
    for index in 0..count {
        let entity = index as u64 + 1;
        let kind = random.unit();
        let mut point = |reach: f64| (random.between(-reach, reach), random.between(-reach, reach));
        if kind < 0.4 {
            let center = point(5.0);
            curves.push(circle(entity, center, 0.5 + 3.5 * random.unit()));
        } else if kind < 0.7 {
            let corner = point(5.0);
            let size = (1.0 + 5.0 * random.unit(), 1.0 + 5.0 * random.unit());
            curves.extend(rectangle(
                entity * 10,
                corner,
                (corner.0 + size.0, corner.1 + size.1),
            ));
        } else if kind < 0.85 {
            let (from, to) = (point(6.0), point(6.0));
            curves.push(line(entity, from, to));
        } else {
            let points: Vec<(f64, f64)> = (0..5).map(|_| point(5.0)).collect();
            curves.push(spline(entity, &points));
        }
    }
    curves
}

#[test]
fn random_sketches_give_closed_regions_and_valid_solids() {
    let mut random = Random::new(11);
    let axis = Axis2::new(Point2::new(-30.0, 0.0), Vector2::new(0.3, 1.0)).unwrap();
    let wedge = AngularExtent::new(0.3, 2.0).unwrap();
    for round in 0..24 {
        let curves = random_curves(&mut random);
        let profile = Profile::new(&curves).unwrap();
        if profile.regions().is_empty() {
            continue;
        }
        let total: f64 = profile.regions().iter().map(Region::area).sum();
        let everything = profile
            .select(&Selection::Regions(
                profile.regions().iter().map(Region::key).collect(),
            ))
            .unwrap();
        let merged: f64 = everything.iter().map(Region::area).sum();
        assert!((total - merged).abs() < 1e-6 * total, "round {round}");
        let chosen = profile.select(&Selection::EvenDepth).unwrap();
        extrude(&Plane::XY, &chosen, one_side(1.0), FEATURE).unwrap();
        for extent in [full(), wedge] {
            revolve(&Plane::XY, &chosen, axis, extent, FEATURE).unwrap();
        }
    }
}

#[test]
fn a_hole_close_to_the_outline_still_validates_and_tessellates() {
    let (outer, inner) = (3.485_953_704_005_785, 0.759_569_104_931_942_6);
    let curves = vec![
        circle(4, (-1.179_075_879_149_484_9, 1.515_052_662_422_505), outer),
        circle(
            1,
            (-2.279_326_463_059_322_6, -0.951_200_520_773_594_1),
            inner,
        ),
    ];
    let solid = extrude(&Plane::XY, &regions(&curves), one_side(1.0), FEATURE).unwrap();
    let face = PI * (outer * outer - inner * inner);
    check(
        "hole near outline",
        &solid,
        face,
        2.0 * face + TAU * (outer + inner),
        inner,
    );
}

#[test]
fn extents_reaching_beyond_the_largest_size_are_refused() {
    assert_eq!(
        LinearExtent::one_side(-2.0 * MAX_SIZE),
        Err(SweepError::TooLong)
    );
    assert_eq!(
        LinearExtent::two_sided(1.0, 2.0 * MAX_SIZE),
        Err(SweepError::TooLong)
    );
    assert!(LinearExtent::symmetric(2.0 * MAX_SIZE).is_ok());
    assert!(LinearExtent::one_side(MAX_SIZE).is_ok());
}

fn corner_to_corner(from: (f64, f64)) -> Vec<ProfileCurve> {
    let (x, y) = from;
    let mut curves = rectangle(1, (x, y), (x + 2.0, y + 2.0));
    curves.extend(rectangle(5, (x + 2.0, y + 2.0), (x + 4.0, y + 4.0)));
    curves
}

fn hole_touching_the_bottom(from: (f64, f64), size: f64, radius: f64) -> Vec<Region> {
    let (x, y) = from;
    let mut curves = rectangle(1, (x, y), (x + size, y + size));
    curves.push(circle(5, (x + size / 2.0, y + radius), radius));
    let profile = Profile::new(&curves).unwrap();
    let around = profile
        .regions()
        .iter()
        .max_by(|a, b| a.area().total_cmp(&b.area()))
        .unwrap()
        .key();
    profile.select(&Selection::Regions(vec![around])).unwrap()
}

#[test]
fn squares_sharing_a_corner_sweep_into_two_lumps() {
    let extruded = extrude(
        &Plane::XY,
        &regions(&corner_to_corner((0.0, 0.0))),
        one_side(1.0),
        FEATURE,
    )
    .unwrap();
    check("corner extrude", &extruded, 8.0, 32.0, f64::INFINITY);
    assert_eq!(extruded.shells().count(), 2);

    let revolved = revolve(
        &Plane::XY,
        &regions(&corner_to_corner((2.0, 0.0))),
        y_axis(),
        full(),
        FEATURE,
    )
    .unwrap();
    check("corner revolve", &revolved, 64.0 * PI, 128.0 * PI, 2.0);
    assert_eq!(revolved.shells().count(), 2);
}

#[test]
fn a_hole_tangent_to_its_outline_sweeps_into_one_pinched_lump() {
    let flat = hole_touching_the_bottom((0.0, 0.0), 10.0, 3.0);
    assert_eq!(flat.len(), 1);
    let face = 100.0 - 9.0 * PI;
    let extruded = extrude(&Plane::XY, &flat, one_side(1.0), FEATURE).unwrap();
    check_pinched(
        "tangent hole extrude",
        &extruded,
        face,
        2.0 * face + 40.0 + 6.0 * PI,
        3.0,
    );
    assert_eq!(extruded.shells().count(), 1);

    let turned = hole_touching_the_bottom((2.0, 0.0), 6.0, 2.0);
    let revolved = revolve(&Plane::XY, &turned, y_axis(), full(), FEATURE).unwrap();
    check_pinched(
        "tangent hole revolve",
        &revolved,
        10.0 * PI * (36.0 - 4.0 * PI),
        240.0 * PI + 40.0 * PI * PI,
        2.0,
    );
    assert_eq!(revolved.shells().count(), 1);
}

#[test]
fn a_circle_tangent_to_the_axis_revolves_into_a_horn_torus() {
    let profile = regions(&[circle(1, (2.0, 0.0), 2.0)]);
    let whole = revolve(&Plane::XY, &profile, y_axis(), full(), FEATURE).unwrap();
    check("horn torus", &whole, 16.0 * PI * PI, 16.0 * PI * PI, 2.0);
    assert_eq!(whole.shells().count(), 1);

    let quarter = AngularExtent::one_side(FRAC_PI_2).unwrap();
    let part = revolve(&Plane::XY, &profile, y_axis(), quarter, FEATURE).unwrap();
    check(
        "quarter horn torus",
        &part,
        4.0 * PI * PI,
        4.0 * PI * PI + 8.0 * PI,
        2.0,
    );
}

#[test]
fn sweeps_cancelled_anywhere_stop_with_cancelled() {
    let mut curves = rectangle(1, (2.0, 0.0), (10.0, 8.0));
    curves.push(circle(5, (6.0, 4.0), 2.0));
    let profile = regions(&curves);
    let cancelled = |error: &SweepError| matches!(error, SweepError::Cancelled(_));

    assert_cancelled_anywhere(
        "extrude",
        || extrude(&Plane::XY, &profile, one_side(3.0), FEATURE),
        cancelled,
    );
    assert_cancelled_anywhere(
        "revolve",
        || revolve(&Plane::XY, &profile, y_axis(), full(), FEATURE),
        cancelled,
    );
    let tilted_top = plane_through((0.0, 0.0, 3.0), (0.2, 0.1, 1.0));
    assert_cancelled_anywhere(
        "tilted extrude",
        || extrude(&Plane::XY, &profile, up_to(tilted_top), FEATURE),
        cancelled,
    );
    let body = slab(5.0, 8.0, (0.0, 0.0), (12.0, 10.0));
    assert_cancelled_anywhere(
        "next face",
        || next_face(&body, &Plane::XY, &profile, false),
        |error| matches!(error, ReachError::Cancelled(_)),
    );
}

fn plane_through(point: (f64, f64, f64), normal: (f64, f64, f64)) -> Plane {
    Plane::new(
        Point3::new(point.0, point.1, point.2),
        Vector3::new(normal.0, normal.1, normal.2),
    )
    .unwrap()
}

fn bounded(start: LinearBound, end: LinearBound) -> LinearExtent {
    LinearExtent::between(start, end).unwrap()
}

fn up_to(plane: Plane) -> LinearExtent {
    bounded(LinearBound::Offset(0.0), LinearBound::Plane(plane))
}

fn cap_plane(solid: &Solid, name: FaceName) -> Plane {
    let (_, face) = solid.faces().find(|(_, face)| face.name() == name).unwrap();
    let crate::surface::Surface::Plane(surface) = face.surface() else {
        panic!("a cap is flat");
    };
    *surface.frame()
}

fn mesh_volume(solid: &Solid) -> f64 {
    fine_mesh(solid).mass_properties().volume
}

#[test]
fn an_extrusion_up_to_a_tilted_plane_ends_on_it() {
    let profile = regions(&rectangle(1, (0.0, 0.0), (4.0, 3.0)));
    let tilted_top = plane_through((0.0, 0.0, 2.0), (-0.5, 0.0, 1.0));

    let solid = extrude(&Plane::XY, &profile, up_to(tilted_top), FEATURE).unwrap();

    let top = 12.0 * 1.25_f64.sqrt();
    check(
        "tilted top",
        &solid,
        36.0,
        12.0 + top + 12.0 + 12.0 + 6.0 + 12.0,
        f64::INFINITY,
    );
    assert_eq!(solid.faces().count(), 6);
    let end = cap_plane(&solid, FaceName::end_cap(FEATURE, profile[0].key()));
    assert!(end.normal().dot(tilted_top.normal()).abs() > 1.0 - 1e-12);
    assert!(tilted_top.signed_distance(end.origin()).abs() < 1e-9);
    let start = cap_plane(&solid, FaceName::start_cap(FEATURE, profile[0].key()));
    assert!(start.origin().z.abs() < 1e-12);
}

#[test]
fn a_circle_up_to_a_tilted_plane_ends_in_an_ellipse() {
    let profile = regions(&[circle(1, (0.0, 0.0), 1.0)]);
    let tilted_top = plane_through((0.0, 0.0, 3.0), (-0.5, 0.0, 1.0));

    let solid = extrude(&Plane::XY, &profile, up_to(tilted_top), FEATURE).unwrap();

    check(
        "tilted cylinder",
        &solid,
        3.0 * PI,
        PI + PI * 1.25_f64.sqrt() + 6.0 * PI,
        1.0,
    );
    assert!(
        solid
            .edges()
            .any(|(_, edge)| matches!(edge.curve(), crate::curve::Curve::Ellipse(_)))
    );

    let half = regions(&[
        arc(1, (0.0, 0.0), (1.0, 0.0), (-1.0, 0.0)),
        line(2, (-1.0, 0.0), (1.0, 0.0)),
    ]);
    let tilted_across = plane_through((0.0, 0.0, 3.0), (0.3, -0.5, 1.0));
    let solid = extrude(&Plane::XY, &half, up_to(tilted_across), FEATURE).unwrap();
    assert_eq!(solid.validate(), Ok(()));
    assert_watertight("tilted half cylinder", &fine_mesh(&solid));
    let centroid_y = 4.0 / (3.0 * PI);
    let expected = 0.5 * PI * (3.0 + 0.5 * centroid_y);
    let volume = mesh_volume(&solid);
    assert!((volume - expected).abs() < 1e-2, "{volume} vs {expected}");
}

#[test]
fn an_ellipse_up_to_a_tilted_plane_ends_in_another_ellipse() {
    let profile = regions(&[ProfileCurve::ellipse(
        1,
        Point2::ZERO,
        Vector2::new(2.0, 1.0),
        1.0,
    )]);
    let tilted_top = plane_through((0.0, 0.0, 3.0), (-0.4, 0.3, 1.0));

    let solid = extrude(&Plane::XY, &profile, up_to(tilted_top), FEATURE).unwrap();

    assert_eq!(solid.validate(), Ok(()));
    assert_watertight("tilted elliptic cylinder", &fine_mesh(&solid));
    let expected = PI * 5f64.sqrt() * 3.0;
    let volume = mesh_volume(&solid);
    assert!((volume - expected).abs() < 1e-2, "{volume} vs {expected}");
    assert_eq!(
        solid
            .edges()
            .filter(|(_, edge)| matches!(edge.curve(), crate::curve::Curve::Ellipse(_)))
            .count(),
        2
    );
}

#[test]
fn a_spline_profile_up_to_a_tilted_plane_is_valid() {
    let profile = regions(&[spline(
        3,
        &[(0.0, 0.0), (6.0, -1.0), (7.0, 5.0), (1.0, 6.0), (0.0, 0.0)],
    )]);
    let tilted_top = plane_through((0.0, 0.0, 4.0), (0.2, -0.3, 1.0));
    let level = |x: f64, y: f64| 4.0 - 0.2 * x + 0.3 * y;
    let polygon = profile[0].polygons(&SamplingTolerance::new(1e-4, 0.01).unwrap());
    let outline = &polygon[0];
    let expected: f64 = outline
        .iter()
        .zip(outline.iter().cycle().skip(1))
        .map(|(a, b)| {
            let cross = a.x * b.y - b.x * a.y;
            0.5 * cross * level((a.x + b.x) / 3.0, (a.y + b.y) / 3.0)
        })
        .sum::<f64>()
        .abs();

    let solid = extrude(&Plane::XY, &profile, up_to(tilted_top), FEATURE).unwrap();

    assert_eq!(solid.validate(), Ok(()));
    let mesh = fine_mesh(&solid);
    assert_watertight("tilted spline", &mesh);
    let properties = mesh.mass_properties();
    let allowance = 2.0 * CHORD * properties.area;
    assert!(
        (properties.volume - expected).abs() <= allowance,
        "{} vs {expected}",
        properties.volume
    );
}

#[test]
fn two_tilted_ends_bound_an_extrusion_both_ways() {
    let profile = regions(&rectangle(1, (0.0, 0.0), (4.0, 3.0)));
    let below = plane_through((0.0, 0.0, -1.0), (0.0, 0.25, 1.0));
    let above = plane_through((0.0, 0.0, 2.0), (-0.5, 0.0, 1.0));

    let solid = extrude(
        &Plane::XY,
        &profile,
        bounded(LinearBound::Plane(below), LinearBound::Plane(above)),
        FEATURE,
    )
    .unwrap();

    assert_eq!(solid.validate(), Ok(()));
    let volume = mesh_volume(&solid);
    assert!((volume - 52.5).abs() < 1e-6, "{volume}");
    let start = cap_plane(&solid, FaceName::start_cap(FEATURE, profile[0].key()));
    assert!(below.signed_distance(start.origin()).abs() < 1e-9);
}

#[test]
fn a_plane_below_the_sketch_ends_a_reversed_extrusion() {
    let profile = regions(&rectangle(1, (0.0, 0.0), (4.0, 3.0)));
    let below = plane_through((0.0, 0.0, -2.0), (0.5, 0.0, 1.0));

    let solid = extrude(&Plane::XY, &profile, up_to(below), FEATURE).unwrap();

    assert_eq!(solid.validate(), Ok(()));
    let volume = mesh_volume(&solid);
    assert!((volume - 36.0).abs() < 1e-6, "{volume}");
    let start = cap_plane(&solid, FaceName::start_cap(FEATURE, profile[0].key()));
    assert!(start.origin().z.abs() < 1e-12);
    assert!(solid.bounding_box().unwrap().max().z.abs() < 1e-9);
}

#[test]
fn a_parallel_plane_ends_an_extrusion_like_a_distance() {
    let profile = regions(&[
        spline(1, &[(0.0, 0.0), (1.0, 2.0), (2.0, 0.0)]),
        line(2, (2.0, 0.0), (0.0, 0.0)),
    ]);
    let level = plane_through((5.0, -3.0, 2.0), (0.0, 0.0, 1.0));

    let through_plane = extrude(&Plane::XY, &profile, up_to(level), FEATURE).unwrap();
    let by_distance = extrude(&Plane::XY, &profile, one_side(2.0), FEATURE).unwrap();

    assert_eq!(through_plane, by_distance);
}

#[test]
fn end_planes_along_the_direction_or_across_the_profile_are_refused() {
    let profile = regions(&rectangle(1, (0.0, 0.0), (4.0, 3.0)));
    let along = plane_through((0.0, 0.0, 0.0), (1.0, 0.0, 0.0));
    let across = plane_through((2.0, 0.0, 0.0), (-1.0, 0.0, 1.0));
    let far = plane_through((0.0, 0.0, 2.0 * MAX_SIZE), (0.0, 0.0, 1.0));

    assert_eq!(
        extrude(&Plane::XY, &profile, up_to(along), FEATURE).unwrap_err(),
        SweepError::EndAlongDirection
    );
    assert_eq!(
        extrude(&Plane::XY, &profile, up_to(across), FEATURE).unwrap_err(),
        SweepError::EndsCross
    );
    assert_eq!(
        extrude(&Plane::XY, &profile, up_to(far), FEATURE).unwrap_err(),
        SweepError::TooLong
    );
    assert_eq!(
        LinearExtent::between(LinearBound::Offset(f64::NAN), LinearBound::Plane(far)),
        Err(SweepError::NonFinite)
    );
}

#[test]
fn heights_of_a_plane_over_the_profile_span_its_extremes() {
    let mut curves = rectangle(1, (0.0, 0.0), (4.0, 3.0));
    curves.push(circle(5, (10.0, 0.0), 1.0));
    let profile = regions(&curves);
    let target = plane_through((0.0, 0.0, 2.0), (-0.5, 0.0, 1.0));
    let sideways = plane_through((0.0, 0.0, 0.0), (0.0, 1.0, 0.0));

    let found = heights(&Plane::XY, &profile, &target).unwrap();

    assert!((found.least - 2.0).abs() < 1e-12);
    assert!((found.most - 7.5).abs() < 1e-12);
    assert_eq!(
        heights(&Plane::XY, &profile, &sideways),
        Err(SweepError::EndAlongDirection)
    );
}

fn slab(bottom: f64, top: f64, min: (f64, f64), max: (f64, f64)) -> Solid {
    extrude(
        &sketch_at(bottom),
        &regions(&rectangle(1, min, max)),
        one_side(top - bottom),
        1,
    )
    .unwrap()
}

fn sketch_at(height: f64) -> Plane {
    Plane::from_frame(Point3::new(0.0, 0.0, height), Vector3::Z, Vector3::X).unwrap()
}

fn face_height(solid: &Solid, face: crate::topology::FaceId) -> f64 {
    let crate::surface::Surface::Plane(surface) = solid.face(face).unwrap().surface() else {
        panic!("expected a flat face");
    };
    surface.frame().origin().z
}

#[test]
fn the_next_face_is_where_the_profile_first_meets_the_body() {
    let body = slab(5.0, 8.0, (0.0, 0.0), (10.0, 10.0));
    let inside = regions(&rectangle(1, (2.0, 2.0), (6.0, 5.0)));

    let entering = next_face(&body, &Plane::XY, &inside, false).unwrap();
    let leaving = next_face(&body, &sketch_at(8.0), &inside, true).unwrap();
    let from_inside = next_face(&body, &sketch_at(6.0), &inside, false).unwrap();

    assert!(entering.entering);
    assert!((face_height(&body, entering.face) - 5.0).abs() < 1e-12);
    assert!(entering.plane.normal().dot(Vector3::Z) < 0.0);
    assert!(!leaving.entering);
    assert!((face_height(&body, leaving.face) - 5.0).abs() < 1e-12);
    assert!(!from_inside.entering);
    assert!((face_height(&body, from_inside.face) - 8.0).abs() < 1e-12);
    assert_eq!(
        next_face(&body, &Plane::XY, &inside, true),
        Err(ReachError::Nothing)
    );
}

#[test]
fn a_profile_that_misses_the_next_face_in_part_or_meets_several_is_refused() {
    let body = slab(5.0, 8.0, (0.0, 0.0), (10.0, 10.0));
    let wider = regions(&rectangle(1, (5.0, 2.0), (15.0, 5.0)));
    let step = slab(3.0, 8.0, (10.0, 0.0), (20.0, 10.0));
    let stepped =
        crate::boolean::boolean(&body, &step, crate::boolean::BooleanOperation::Union).unwrap();

    let partly = next_face(&body, &Plane::XY, &wider, false);
    let several = next_face(&stepped, &Plane::XY, &wider, false);

    assert_eq!(partly, Err(ReachError::Partly));
    let Err(ReachError::SeveralFaces(faces)) = several else {
        panic!("expected several faces, found {several:?}");
    };
    let mut met: Vec<f64> = faces
        .iter()
        .map(|face| face_height(&stepped, *face))
        .collect();
    met.sort_by(f64::total_cmp);
    assert_eq!(met.len(), 2);
    assert!((met[0] - 3.0).abs() < 1e-9 && (met[1] - 5.0).abs() < 1e-9);
}

#[test]
fn a_small_boss_between_the_sampled_rays_still_makes_the_next_face_several() {
    let plate = slab(10.0, 20.0, (0.0, 0.0), (100.0, 100.0));
    let boss = slab(5.0, 10.0, (50.3, 50.3), (50.8, 50.8));
    let body =
        crate::boolean::boolean(&plate, &boss, crate::boolean::BooleanOperation::Union).unwrap();
    let everything = regions(&rectangle(1, (0.0, 0.0), (100.0, 100.0)));

    let found = next_face(&body, &Plane::XY, &everything, false);

    let Err(ReachError::SeveralFaces(faces)) = found else {
        panic!("expected several faces, found {found:?}");
    };
    let mut met: Vec<f64> = faces.iter().map(|face| face_height(&body, *face)).collect();
    met.sort_by(f64::total_cmp);
    assert_eq!(met.len(), 2);
    assert!((met[0] - 5.0).abs() < 1e-9 && (met[1] - 10.0).abs() < 1e-9);
}

#[test]
fn a_curved_next_face_is_refused() {
    let yz = Plane::from_frame(Point3::new(-10.0, 0.0, 10.0), Vector3::X, Vector3::Y).unwrap();
    let rod = extrude(
        &yz,
        &regions(&[circle(1, (0.0, 0.0), 5.0)]),
        one_side(20.0),
        1,
    )
    .unwrap();
    let under = regions(&rectangle(1, (1.0, -1.0), (2.0, 1.0)));

    let refused = next_face(&rod, &Plane::XY, &under, false);

    assert!(matches!(refused, Err(ReachError::Curved(_))), "{refused:?}");
}

#[test]
fn a_tall_extrusion_of_a_thin_lens_is_valid() {
    let half_width = 0.5;
    let sagitta = 0.03;
    let radius = (half_width * half_width + sagitta * sagitta) / (2.0 * sagitta);
    let offset = radius - sagitta;
    let lens = [
        arc(1, (0.0, -offset), (half_width, 0.0), (-half_width, 0.0)),
        arc(2, (0.0, offset), (-half_width, 0.0), (half_width, 0.0)),
    ];

    let solid = extrude(
        &Plane::XY,
        &regions(&lens),
        LinearExtent::one_side(1000.0).unwrap(),
        FEATURE,
    )
    .unwrap();

    assert_eq!(solid.validate(), Ok(()));
}

#[test]
fn a_lens_of_a_tenth_of_a_square_millimetre_is_valid_and_has_volume() {
    let sag = 0.01;
    let radius = (25.0 + sag * sag) / (2.0 * sag);
    let curves = [
        arc(1, (5.0, sag - radius), (10.0, 0.0), (0.0, 0.0)),
        arc(2, (5.0, radius - sag), (0.0, 0.0), (10.0, 0.0)),
    ];

    let solid = extrude(
        &Plane::XY,
        &regions(&curves),
        LinearExtent::one_side(1.0).unwrap(),
        FEATURE,
    )
    .unwrap();

    assert_eq!(solid.validate(), Ok(()));
    let volume = fine_mesh(&solid).mass_properties().volume;
    assert!(
        (volume - 4.0 / 3.0 * 10.0 * sag).abs() < 0.1 * volume,
        "{volume}"
    );
}

fn crescent(center: Point2, tip: f64) -> Vec<Region> {
    let inner = center + Vector2::from_angle(tip);
    regions(&[
        circle(1, (center.x, center.y), 3.0),
        circle(2, (inner.x, inner.y), 2.0),
    ])
}

#[test]
fn a_crescent_extrudes_wherever_its_tip_lies() {
    for degrees in [0.0, 30.0, 45.0, 137.0, 200.0, 311.0] {
        let tip = f64::to_radians(degrees);

        let solid = extrude(
            &Plane::XY,
            &crescent(Point2::ZERO, tip),
            one_side(1.0),
            FEATURE,
        )
        .unwrap();

        check_pinched(
            &format!("crescent tipped at {degrees}°"),
            &solid,
            5.0 * PI,
            20.0 * PI,
            2.0,
        );
    }
}

#[test]
fn a_crescent_revolves_part_way_wherever_its_tip_lies() {
    let quarter = AngularExtent::one_side(FRAC_PI_2).unwrap();
    for degrees in [0.0, 30.0, 45.0, 200.0, 311.0] {
        let tip = f64::to_radians(degrees);
        let centroid = 6.0 - 0.8 * tip.cos();

        let solid = revolve(
            &Plane::XY,
            &crescent(Point2::new(6.0, 0.0), tip),
            y_axis(),
            quarter,
            FEATURE,
        )
        .unwrap();
        let volume = fine_mesh(&solid).mass_properties().volume;

        assert_eq!(solid.validate(), Ok(()), "{degrees}°");
        assert_balanced(
            &format!("{degrees}°"),
            &solid.tessellate(&solid.default_tolerance()).unwrap(),
        );
        assert!(
            (volume - FRAC_PI_2 * centroid * 5.0 * PI).abs() <= 2e-3 * volume,
            "{degrees}°: volume {volume}"
        );
    }
}

#[test]
fn a_crescent_tipped_on_a_slanted_axis_revolves_into_nested_horn_tori() {
    let along = Vector2::new(1.0, 1.0).normalize();
    let outward = -along.perp();
    let outer = outward * 3.0;
    let inner = outward * 2.0;
    let chosen = regions(&[
        circle(1, (outer.x, outer.y), 3.0),
        circle(2, (inner.x, inner.y), 2.0),
    ]);

    let solid = revolve(
        &Plane::XY,
        &chosen,
        Axis2::new(Point2::ZERO, along).unwrap(),
        full(),
        FEATURE,
    )
    .unwrap();

    check_pinched(
        "slanted horn crescent",
        &solid,
        38.0 * PI * PI,
        52.0 * PI * PI,
        2.0,
    );
}

fn touching_the_axis(axis: Axis2) -> Vec<(&'static str, Vec<ProfileCurve>)> {
    let along = axis.direction();
    let base = axis.origin() + along * 1.3;
    let at = |distance: f64, sideways: f64| {
        let point = base - along.perp() * distance + along * sideways;
        (point.x, point.y)
    };
    vec![
        ("circle", vec![circle(1, at(2.0, 0.0), 2.0)]),
        (
            "crescent",
            vec![circle(1, at(3.0, 0.0), 3.0), circle(2, at(2.0, 0.0), 2.0)],
        ),
        (
            "half disc on a block",
            vec![
                arc(1, at(2.0, 0.0), at(2.0, 2.0), at(2.0, -2.0)),
                line(2, at(2.0, -2.0), at(6.0, -2.0)),
                line(3, at(6.0, -2.0), at(6.0, 2.0)),
                line(4, at(6.0, 2.0), at(2.0, 2.0)),
            ],
        ),
    ]
}

#[test]
fn profiles_touching_a_slanted_axis_revolve_into_valid_solids() {
    for degrees in [3.0, 73.0, 123.0, 163.0, 193.0, 343.0] {
        let slant = f64::to_radians(degrees);
        let axis = Axis2::new(Point2::new(0.3, -0.7), Vector2::from_angle(slant)).unwrap();
        for (name, curves) in touching_the_axis(axis) {
            let chosen = regions(&curves);
            for extent in [AngularExtent::one_side(FRAC_PI_2).unwrap(), full()] {
                let solid = revolve(&Plane::XY, &chosen, axis, extent, FEATURE);

                assert!(
                    solid.is_ok(),
                    "{name} about an axis at {degrees}°: {solid:?}"
                );
            }
        }
    }
}

#[test]
fn a_dense_spline_profile_extrudes_validates_and_meshes_in_bounded_time() {
    let count = 320;
    let points: Vec<(f64, f64)> = (0..=count)
        .map(|index| {
            let angle = TAU * (index % count) as f64 / count as f64;
            let radius = 20.0 + 2.0 * (7.0 * angle).sin();
            (radius * angle.cos(), radius * angle.sin())
        })
        .collect();
    let profile = regions(&[spline(1, &points)]);
    let area = profile[0].area();
    let clock = Instant::now();

    let solid = extrude(&Plane::XY, &profile, one_side(5.0), FEATURE).unwrap();
    let mesh = solid.tessellate(&solid.default_tolerance()).unwrap();

    assert_eq!(solid.validate(), Ok(()));
    assert_watertight("dense spline", &mesh);
    assert!(
        clock.elapsed() < DENSE_SPLINE_TIME_LIMIT,
        "{:?}",
        clock.elapsed()
    );
    let volume = mesh.mass_properties().volume;
    assert!(
        (volume - 5.0 * area).abs() < 2e-3 * volume,
        "{volume} vs {}",
        5.0 * area
    );
}

fn disc_edge(plan: &mut Plan, height: f64) -> (usize, Plane) {
    let frame = Plane::with_x_axis(Point3::new(0.0, 0.0, height), Vector3::Z, Vector3::X).unwrap();
    let circle = Curve::from(Circle::new(frame, 2.0).unwrap());
    let vertex = plan.vertex(circle.point(0.0));
    let edge = plan.edge(
        circle,
        Interval::new(0.0, TAU).unwrap(),
        (vertex, vertex),
        EdgeName::NONE,
    );
    (edge, frame)
}

fn disc_face((edge, frame): (usize, Plane)) -> PlanFace {
    PlanFace {
        surface: PlaneSurface::new(frame).unwrap().into(),
        sense: Sense::Same,
        name: FaceName::NONE,
        origin: None,
        loops: vec![vec![PlanCoedge::new(edge, Sense::Same)]],
    }
}

#[test]
fn a_plan_names_the_labels_of_the_faces_validation_blames() {
    let mut plan = Plan::default();
    let blamed = disc_edge(&mut plan, 5.0);
    let other = disc_edge(&mut plan, 0.0);
    plan.label(vec![11]);
    plan.face(disc_face(other));
    plan.label(vec![22]);
    plan.face(disc_face(blamed));

    let error = plan.build().unwrap_err();

    assert!(
        matches!(
            &error,
            PlanError::Labelled { error: BuildError::Invalid(_), labels } if labels == &[22]
        ),
        "{error:?}"
    );
}

#[test]
fn a_swept_solid_that_fails_validation_names_the_curves_of_its_region() {
    let mut plan = Plan::default();
    let edge = disc_edge(&mut plan, 0.0);
    plan.label(vec![3, 8]);
    plan.face(disc_face(edge));

    let error = SweepError::from(plan.build().unwrap_err());

    assert!(matches!(&error, SweepError::Invalid { .. }), "{error:?}");
    assert_eq!(error.entities(), vec![3, 8]);
}

#[test]
fn a_plan_without_labels_names_no_curves() {
    let mut plan = Plan::default();
    let edge = disc_edge(&mut plan, 0.0);
    plan.face(disc_face(edge));

    let error = SweepError::from(plan.build().unwrap_err());

    assert!(matches!(&error, SweepError::Invalid { .. }), "{error:?}");
    assert_eq!(error.entities(), Vec::<u64>::new());
}

fn rod() -> Solid {
    let yz = Plane::from_frame(Point3::new(-10.0, 0.0, 10.0), Vector3::X, Vector3::Y).unwrap();
    extrude(
        &yz,
        &regions(&[circle(1, (0.0, 0.0), 5.0)]),
        one_side(20.0),
        1,
    )
    .unwrap()
}

fn under_the_rod(width: f64) -> f64 {
    let half = width / 2.0;
    2.0 * (half / 2.0 * (25.0 - half * half).sqrt() + 12.5 * (half / 5.0).asin())
}

#[test]
fn a_sweep_stops_where_it_first_meets_a_curved_body_from_outside_or_from_inside() {
    let rod = rod();
    let strip = regions(&rectangle(2, (1.0, -1.0), (2.0, 1.0)));
    let from_below = extrude(&Plane::XY, &strip, one_side(30.0), 2).unwrap();
    let from_axis = extrude(&sketch_at(10.0), &strip, one_side(30.0), 2).unwrap();

    let below = stop_at_body(&from_below, &rod, &Plane::XY, &strip, false, 30.0).unwrap();
    let inside = stop_at_body(&from_axis, &rod, &sketch_at(10.0), &strip, false, 30.0).unwrap();

    assert!(below.entering);
    below.solid.validate().unwrap();
    assert_eq!(below.solid.shells().count(), 1);
    let expected = 20.0 - under_the_rod(2.0);
    let found = fine_mesh(&below.solid).mass_properties().volume;
    assert!(
        (found - expected).abs() < 1e-3 * expected,
        "{found} {expected}"
    );
    assert!(!inside.entering);
    inside.solid.validate().unwrap();
    let expected = under_the_rod(2.0);
    let found = fine_mesh(&inside.solid).mass_properties().volume;
    assert!(
        (found - expected).abs() < 1e-3 * expected,
        "{found} {expected}"
    );
}

#[test]
fn a_sweep_passing_beside_a_curved_body_or_starting_half_inside_it_does_not_stop() {
    let rod = rod();
    let wide = regions(&rectangle(2, (1.0, -8.0), (2.0, 8.0)));
    let wide_tool = extrude(&Plane::XY, &wide, one_side(30.0), 2).unwrap();
    let straddling = regions(&rectangle(2, (1.0, 3.0), (2.0, 7.0)));
    let straddling_tool = extrude(&sketch_at(10.0), &straddling, one_side(30.0), 2).unwrap();

    let beside = stop_at_body(&wide_tool, &rod, &Plane::XY, &wide, false, 30.0);
    let half_inside = stop_at_body(
        &straddling_tool,
        &rod,
        &sketch_at(10.0),
        &straddling,
        false,
        30.0,
    );

    assert_eq!(beside, Err(StopError::PassesBeside));
    assert_eq!(half_inside, Err(StopError::Straddles));
}

#[test]
fn an_extrusion_along_a_slanted_direction_keeps_its_volume_and_shifts_its_end() {
    let square = regions(&rectangle(1, (0.0, 0.0), (10.0, 10.0)));
    let disc = regions(&[circle(1, (0.0, 0.0), 5.0)]);
    let slant = Vector3::new(1.0, 0.0, 1.0);

    let leaning = extrude_along(&Plane::XY, &square, one_side(10.0), slant, FEATURE).unwrap();
    let rod = extrude_along(
        &Plane::XY,
        &disc,
        one_side(8.0),
        Vector3::new(0.0, 1.0, 2.0),
        FEATURE,
    )
    .unwrap();
    let roof = Plane::with_x_axis(
        Point3::new(0.0, 0.0, 5.0),
        Vector3::new(-0.25, 0.0, 1.0),
        Vector3::X,
    )
    .unwrap();
    let up_to_roof = extrude_along(
        &Plane::XY,
        &square,
        LinearExtent::between(LinearBound::Offset(0.0), LinearBound::Plane(roof)).unwrap(),
        slant,
        FEATURE,
    )
    .unwrap();
    let flat = extrude_along(&Plane::XY, &square, one_side(10.0), Vector3::X, FEATURE);

    leaning.validate().unwrap();
    let volume = fine_mesh(&leaning).mass_properties().volume;
    assert!((volume - 1_000.0).abs() < 1e-6, "{volume}");
    let bounds = leaning.bounding_box().unwrap();
    assert!((bounds.max().x - 20.0).abs() < 1e-9 && (bounds.max().z - 10.0).abs() < 1e-9);
    rod.validate().unwrap();
    let volume = fine_mesh(&rod).mass_properties().volume;
    let expected = 25.0 * std::f64::consts::PI * 8.0;
    assert!((volume - expected).abs() < 1e-3 * expected, "{volume}");
    up_to_roof.validate().unwrap();
    let volume = fine_mesh(&up_to_roof).mass_properties().volume;
    let expected = 10.0 * (50.0 + 12.5) / 0.75;
    assert!((volume - expected).abs() < 1e-6 * expected, "{volume}");
    assert_eq!(flat, Err(SweepError::DirectionAlongSketch));
}

#[test]
fn two_nearly_touching_lumps_of_a_slanted_revolution_validate_in_bounded_time() {
    let curves = vec![
        circle(1, (6.0, 3.499_999), 3.0),
        circle(2, (-2.5, 11.5), 6.000_1),
        arc(
            3,
            (-8.5, -8.000_001),
            (-4.749_935_048_282_216, -1.504_847_971_941_468_5),
            (-6.558_857_161_731_094, -0.755_557_302_831_986_6),
        ),
        circle(4, (-8.500_001, -9.999_999), 1.500_1),
        line(5, (-4.0, 0.5), (3.0, 0.5)),
        line(6, (3.0, 0.5), (3.0, 8.5)),
        line(7, (3.0, 8.5), (-4.0, 8.5)),
        line(8, (-4.0, 8.5), (-4.0, 0.5)),
        line(9, (-2.5, 6.500_1), (-11.999_999, -0.499_999)),
    ];
    let chosen = Selection::Regions(vec![
        RegionKey::from_digest(96_685_686_626_783_132_505_241_604_872_028_554_191),
        RegionKey::from_digest(217_915_268_302_919_612_151_948_420_403_395_607_601),
    ]);
    let profile = Profile::new(&curves).unwrap().select(&chosen).unwrap();
    let plane = Plane::from_frame(
        Point3::new(5.0, -5.5, -12.0),
        Vector3::new(
            4.472_135_950_527_444e-5,
            -0.894_427_190_105_488_8,
            0.447_213_595_052_744_4,
        ),
        Vector3::new(
            4.472_223_599_884_365_6e-7,
            -0.447_213_595_482_024_33,
            -0.894_427_191_008_770_8,
        ),
    )
    .unwrap();
    let axis = Axis2::new(
        Point2::new(-9.999_999, -9.000_001),
        Vector2::new(0.948_683_234_804_903_7, 0.316_227_955_753_604_9),
    )
    .unwrap();
    let clock = Instant::now();

    let solid = revolve(&plane, &profile, axis, full(), FEATURE).unwrap();

    assert_eq!(profile.len(), 2);
    assert_eq!(solid.validate(), Ok(()));
    assert_eq!(solid.shells().count(), 2);
    assert!(
        clock.elapsed() < NEARLY_TOUCHING_LUMPS_TIME_LIMIT,
        "{:?}",
        clock.elapsed()
    );
}
