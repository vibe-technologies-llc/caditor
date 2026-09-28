use std::f64::consts::{PI, TAU};

use caditor_geometry::{Point3, Vector3};

use super::*;
use crate::{
    fixtures, interval::Interval, numeric::integrate, test_support::assert_watertight,
    topology::Solid,
};

struct Expected {
    name: &'static str,
    solid: Solid,
    volume: f64,
    area: f64,
    smallest_radius: f64,
    centroid: Option<Point3>,
}

fn profile_area_and_length() -> (f64, f64) {
    let profile = fixtures::spline_profile();
    let breaks: Vec<f64> = Interval::UNIT.split(64).collect();
    let area = integrate(&breaks, |parameter| {
        let derivatives = profile.evaluate(parameter);
        derivatives.point.x * derivatives.first.y
    });
    (area, profile.length(Interval::UNIT))
}

fn expectations() -> Vec<Expected> {
    let slant = 29f64.sqrt();
    let (profile_area, profile_length) = profile_area_and_length();
    vec![
        Expected {
            name: "cuboid",
            solid: fixtures::cuboid(Vector3::new(4.0, 3.0, 2.0)),
            volume: 24.0,
            area: 52.0,
            smallest_radius: f64::INFINITY,
            centroid: Some(Point3::new(2.0, 1.5, 1.0)),
        },
        Expected {
            name: "hollow cuboid",
            solid: fixtures::hollow_cuboid(10.0, 4.0),
            volume: 936.0,
            area: 696.0,
            smallest_radius: f64::INFINITY,
            centroid: Some(Point3::splat(5.0)),
        },
        Expected {
            name: "cylinder",
            solid: fixtures::cylinder(3.0, 5.0),
            volume: 45.0 * PI,
            area: 48.0 * PI,
            smallest_radius: 3.0,
            centroid: Some(Point3::new(0.0, 0.0, 2.5)),
        },
        Expected {
            name: "holed block",
            solid: fixtures::holed_block(10.0, 4.0, 2.5),
            volume: 400.0 - 25.0 * PI,
            area: 360.0 + 7.5 * PI,
            smallest_radius: 2.5,
            centroid: Some(Point3::new(5.0, 5.0, 2.0)),
        },
        Expected {
            name: "sphere",
            solid: fixtures::sphere(4.0),
            volume: 256.0 * PI / 3.0,
            area: 64.0 * PI,
            smallest_radius: 4.0,
            centroid: Some(Point3::ZERO),
        },
        Expected {
            name: "torus",
            solid: fixtures::torus(6.0, 2.0),
            volume: 48.0 * PI * PI,
            area: 48.0 * PI * PI,
            smallest_radius: 2.0,
            centroid: Some(Point3::ZERO),
        },
        Expected {
            name: "frustum",
            solid: fixtures::frustum(4.0, 2.0, 5.0),
            volume: 140.0 * PI / 3.0,
            area: 6.0 * PI * slant + 20.0 * PI,
            smallest_radius: 2.0,
            centroid: None,
        },
        Expected {
            name: "cone",
            solid: fixtures::cone(3.0, 4.0),
            volume: 12.0 * PI,
            area: 24.0 * PI,
            smallest_radius: 1.0,
            centroid: Some(Point3::new(0.0, 0.0, 1.0)),
        },
        Expected {
            name: "extruded spline",
            solid: fixtures::extruded_spline(5.0),
            volume: profile_area * 5.0,
            area: 2.0 * profile_area + 5.0 * (profile_length + 20.0),
            smallest_radius: 2.0,
            centroid: None,
        },
    ]
}

#[test]
fn every_fixture_tessellates_watertight() {
    for expected in expectations() {
        for chord in [0.5, 0.05, 0.002] {
            let tolerance = SamplingTolerance::new(chord, 0.3).unwrap();
            let mesh = expected.solid.tessellate(&tolerance).unwrap();
            assert_watertight(expected.name, &mesh);
        }
        let mesh = expected
            .solid
            .tessellate(&expected.solid.default_tolerance())
            .unwrap();
        assert_watertight(expected.name, &mesh);
    }
}

#[test]
fn mass_properties_match_closed_forms() {
    for expected in expectations() {
        let chord = 1e-3;
        let tolerance = SamplingTolerance::new(chord, 0.2).unwrap();
        let mesh = expected.solid.tessellate(&tolerance).unwrap();
        let properties = mesh.mass_properties();
        let volume_error = (properties.volume - expected.volume).abs();
        let area_error = (properties.area - expected.area).abs();
        assert!(
            volume_error <= 2.0 * chord * expected.area + 1e-9,
            "{}: volume {} vs {}",
            expected.name,
            properties.volume,
            expected.volume
        );
        let area_allowance = if expected.smallest_radius.is_finite() {
            2.0 * chord / expected.smallest_radius * expected.area
        } else {
            1e-9
        };
        assert!(
            area_error <= area_allowance,
            "{}: area {} vs {}",
            expected.name,
            properties.area,
            expected.area
        );
        if let Some(centroid) = expected.centroid {
            assert!(
                properties.centroid.distance(centroid) < 1e-2,
                "{}: centroid {} vs {}",
                expected.name,
                properties.centroid,
                centroid
            );
        }
    }
}

#[test]
fn finer_tolerances_converge_on_the_exact_volume() {
    let sphere = fixtures::sphere(4.0);
    let errors: Vec<f64> = [0.1, 0.01, 0.001]
        .into_iter()
        .map(|chord| {
            let tolerance = SamplingTolerance::new(chord, 0.5).unwrap();
            let volume = sphere
                .tessellate(&tolerance)
                .unwrap()
                .mass_properties()
                .volume;
            (volume - 256.0 * PI / 3.0).abs()
        })
        .collect();
    assert!(errors[1] < errors[0] && errors[2] < errors[1], "{errors:?}");
}

#[test]
fn normals_are_unit_and_face_outward() {
    for expected in expectations() {
        let mesh = expected
            .solid
            .tessellate(&SamplingTolerance::new(0.05, 0.3).unwrap())
            .unwrap();
        for triangle in mesh.triangles() {
            let [a, b, c] = mesh.corner_points(*triangle).unwrap();
            let geometric = (b - a).cross(c - a).normalize();
            for corner in triangle {
                let normal = mesh.vertices()[*corner as usize].normal;
                assert!((normal.length() - 1.0).abs() < 1e-9, "{}", expected.name);
                assert!(
                    normal.dot(geometric) > 0.5,
                    "{}: {normal} vs {geometric}",
                    expected.name
                );
            }
        }
    }
}

#[test]
fn faces_and_edges_are_reported_for_drawing_and_picking() {
    for expected in expectations() {
        let solid = &expected.solid;
        let mesh = solid.tessellate(&solid.default_tolerance()).unwrap();
        assert_eq!(mesh.faces().len(), solid.faces().count());
        let mut next = 0;
        for face in mesh.faces() {
            assert_eq!(face.triangles.start, next);
            assert!(
                !face.triangles.is_empty(),
                "{}: {:?} has no triangles",
                expected.name,
                face.face
            );
            next = face.triangles.end;
        }
        assert_eq!(next, mesh.triangles().len());
        assert_eq!(mesh.edges().len(), solid.edges().count());
        for polyline in mesh.edges() {
            let edge = solid.edge(polyline.edge).unwrap();
            let first = mesh.position(*polyline.positions.first().unwrap()).unwrap();
            let last = mesh.position(*polyline.positions.last().unwrap()).unwrap();
            assert_eq!(first, solid.vertex(edge.start()).unwrap().point());
            assert_eq!(last, solid.vertex(edge.end()).unwrap().point());
            assert!(polyline.positions.len() >= 2);
        }
    }
}

#[test]
fn tessellation_is_deterministic() {
    for expected in expectations() {
        let tolerance = expected.solid.default_tolerance();
        let first = expected.solid.tessellate(&tolerance).unwrap();
        let second = expected.solid.tessellate(&tolerance).unwrap();
        assert_eq!(first, second, "{}", expected.name);
    }
}

#[test]
fn a_void_shell_subtracts_and_lies_inside_the_outer_shell() {
    let solid = fixtures::hollow_cuboid(10.0, 4.0);
    let mesh = solid.tessellate(&solid.default_tolerance()).unwrap();
    let shell_volume = |shell: usize| {
        mesh.mass_properties_where(|face| solid.face(face).unwrap().shell().index() == shell)
            .volume
    };
    assert!((shell_volume(0) - 1000.0).abs() < 1e-9);
    assert!((shell_volume(1) + 64.0).abs() < 1e-9);
    let outer = |face: crate::topology::FaceId| solid.face(face).unwrap().shell().index() == 0;
    assert_eq!(mesh.contains(Point3::splat(5.0), outer), Some(true));
    assert_eq!(mesh.contains(Point3::splat(11.0), outer), Some(false));
    let _ = TAU;
}

#[test]
fn a_self_crossing_boundary_is_an_error_not_a_panic() {
    use caditor_geometry::Plane;

    use crate::{fixtures::Fixture, sense::Sense, surface::PlaneSurface};

    let mut fixture = Fixture::new();
    let corners = [
        Point3::ZERO,
        Point3::new(1.0, 1.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ]
    .map(|point| fixture.vertex(point));
    let coedges = fixture.polygon_loop(&corners);
    fixture.face(
        PlaneSurface::new(Plane::XY).unwrap(),
        Sense::Same,
        &[coedges],
    );
    let solid = fixture.build_unchecked();
    assert!(matches!(
        solid.tessellate(&SamplingTolerance::for_extent(1.0)),
        Err(TessellationError::SelfIntersectingBoundary(_))
    ));
    assert!(solid.validate().is_err());
}

#[test]
fn a_cone_meshes_without_slivers_at_its_apex_at_any_tolerance() {
    for (radius, height) in [(3.0, 4.0), (1.0, 20.0), (20.0, 1.0)] {
        let cone = fixtures::cone(radius, height);
        for chord in [0.1, 0.01] {
            let mesh = cone
                .tessellate(&SamplingTolerance::new(chord, 0.3).unwrap())
                .unwrap();
            assert_watertight("cone", &mesh);
            for triangle in mesh.triangles() {
                let [a, b, c] = mesh.corner_points(*triangle).unwrap();
                let longest = a.distance(b).max(b.distance(c)).max(c.distance(a));
                let area = (b - a).cross(c - a).length() / 2.0;
                assert!(
                    area > 1e-9 * longest * longest,
                    "a sliver {a} {b} {c} on a cone of radius {radius} and height {height} at \
                     chord {chord}"
                );
            }
        }
    }
}
