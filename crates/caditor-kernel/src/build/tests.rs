use std::{
    collections::{BTreeMap, BTreeSet},
    f64::consts::{FRAC_PI_2, PI, TAU},
};

use caditor_geometry::{Plane, Point2, Point3, Vector2, Vector3};

use super::*;
use crate::{
    interval::Interval,
    naming::{EdgeName, FaceName},
    numeric::integrate,
    profile::{Profile, ProfileCurve, Region, Selection},
    tessellation::Mesh,
    test_support::{Random, arc, assert_watertight, circle, line, rectangle, spline},
    tolerance::{MAX_SIZE, SamplingTolerance},
    topology::Solid,
};

const FEATURE: u64 = 7;
const CHORD: f64 = 1e-3;

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
    for (name, extent, total) in [
        ("two sided", LinearExtent::two_sided(2.0, 1.0).unwrap(), 3.0),
        ("symmetric", LinearExtent::symmetric(4.0).unwrap(), 4.0),
        ("reversed", LinearExtent::new(3.0, -1.0).unwrap(), 4.0),
    ] {
        let solid = extrude(&Plane::XY, &profile, extent, FEATURE).unwrap();
        check(name, &solid, PI * total, 2.0 * PI + TAU * total, 1.0);
        let bounds = solid.bounding_box().unwrap();
        let (low, high) = (
            extent.start().min(extent.end()),
            extent.start().max(extent.end()),
        );
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
