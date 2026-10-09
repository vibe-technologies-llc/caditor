use std::f64::consts::{PI, TAU};

use caditor_geometry::{Point3, RigidTransform, Vector3};

use super::*;
use crate::{
    boolean::{BooleanOperation, boolean},
    fixtures,
    interval::Interval,
    numeric::integrate,
    test_support::{Random, assert_cancelled_anywhere, assert_watertight},
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
fn second_moments_give_the_inertia_of_a_box_and_a_cylinder() {
    let tolerance = SamplingTolerance::new(1e-3, 0.2).unwrap();
    let cuboid = fixtures::cuboid(Vector3::new(4.0, 3.0, 2.0))
        .tessellate(&tolerance)
        .unwrap()
        .mass_properties();
    let inertia = MassProperties::inertia(&cuboid.second_moment);
    let expected = [
        [24.0 * (9.0 + 4.0) / 12.0, 0.0, 0.0],
        [0.0, 24.0 * (16.0 + 4.0) / 12.0, 0.0],
        [0.0, 0.0, 24.0 * (16.0 + 9.0) / 12.0],
    ];
    for (row, expected_row) in inertia.iter().zip(expected) {
        for (entry, expected_entry) in row.iter().zip(expected_row) {
            assert!((entry - expected_entry).abs() < 1e-9, "{inertia:?}");
        }
    }
    let about_corner = MassProperties::inertia(&cuboid.second_moment_about(Point3::ZERO));
    assert!((about_corner[0][0] - (26.0 + 24.0 * (2.25 + 1.0))).abs() < 1e-9);
    assert!((about_corner[0][1] + 24.0 * 2.0 * 1.5).abs() < 1e-9);

    let cylinder = fixtures::cylinder(3.0, 5.0)
        .tessellate(&tolerance)
        .unwrap()
        .mass_properties();
    let volume = 45.0 * PI;
    let moments =
        MassProperties::principal_moments(&MassProperties::inertia(&cylinder.second_moment));
    let across = volume * (3.0 * 9.0 + 25.0) / 12.0;
    let along = volume * 9.0 / 2.0;
    let mut wanted = [across, across, along];
    wanted.sort_by(f64::total_cmp);
    for (moment, wanted) in moments.iter().zip(wanted) {
        assert!(
            (moment - wanted).abs() < 1e-3 * wanted,
            "{moments:?} vs {wanted}"
        );
    }
}

#[test]
fn principal_moments_of_a_turned_tensor_are_its_eigenvalues() {
    let turn = 0.4f64;
    let (sine, cosine) = turn.sin_cos();
    let diagonal = [2.0, 5.0, 11.0];
    let rotation = [[cosine, -sine, 0.0], [sine, cosine, 0.0], [0.0, 0.0, 1.0]];
    let mut turned = [[0.0; 3]; 3];
    for (row, out) in turned.iter_mut().enumerate() {
        for (column, entry) in out.iter_mut().enumerate() {
            *entry = (0..3)
                .map(|k| rotation[row][k] * diagonal[k] * rotation[column][k])
                .sum();
        }
    }
    let moments = MassProperties::principal_moments(&turned);
    for (moment, expected) in moments.iter().zip(diagonal) {
        assert!((moment - expected).abs() < 1e-9, "{moments:?}");
    }
    assert_eq!(
        MassProperties::principal_moments(&[[3.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 2.0]]),
        [1.0, 2.0, 3.0]
    );
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

#[test]
fn only_faces_whose_boundary_crosses_itself_are_meshed_finer() {
    use caditor_geometry::Plane;

    use crate::{
        build::{LinearExtent, extrude},
        profile::{Profile, Selection},
        test_support::circle,
    };

    let sweep = |curves: &[crate::profile::ProfileCurve]| {
        let regions = Profile::new(curves)
            .unwrap()
            .select(&Selection::EvenDepth)
            .unwrap();
        extrude(
            &Plane::XY,
            &regions,
            LinearExtent::one_side(1.0).unwrap(),
            1,
        )
        .unwrap()
    };
    let post = [circle(1, (30.0, 0.0), 3.0)];
    let mut curves = post.to_vec();
    curves.push(circle(5, (0.0, 0.0), 10.0));
    let (sin, cos) = 0.07_f64.sin_cos();
    curves.push(circle(6, (8.999 * cos, 8.999 * sin), 1.0));
    let both = sweep(&curves);
    let tolerance = SamplingTolerance::new(0.05, 0.5).unwrap();
    let mut tessellator =
        Tessellator::new(&both, &tolerance, MAX_POINTS, Keys::Skipped, 1).unwrap();
    let mut crossed = tessellator.first_pass().unwrap().faces().unwrap();
    let before = tessellator.triangles.clone();
    let mut refined_faces: Vec<FaceId> = Vec::new();
    while !crossed.is_empty() {
        assert!(tessellator.tolerances.refine(&crossed));
        refined_faces.extend(&crossed);
        crossed = tessellator.refine(&crossed).unwrap().faces;
    }
    let untouched: Vec<FaceId> = both
        .faces()
        .map(|(id, _)| id)
        .filter(|id| !refined_faces.contains(id) && tessellator.triangles.get(id) == before.get(id))
        .collect();
    assert!(untouched.len() >= 3, "{untouched:?}");
    let refined = tessellator.finish().into_mesh();
    let mut used = vec![false; refined.positions().len()];
    for position in refined.position_triangles().flatten().chain(
        refined
            .edges()
            .iter()
            .flat_map(|edge| edge.positions.iter().copied()),
    ) {
        used[position as usize] = true;
    }
    assert!(used.iter().all(|used| *used));
    assert_watertight("refined ring and post", &refined);
    let mesh = both.tessellate(&tolerance).unwrap();
    assert_watertight("ring and post", &mesh);
    let alone = sweep(&post).tessellate(&tolerance).unwrap();
    let counts = |mesh: &Mesh| {
        let mut counts: Vec<usize> = mesh
            .faces()
            .iter()
            .map(|face| face.triangles.len())
            .collect();
        counts.sort_unstable();
        counts
    };
    let found = counts(&mesh);
    assert!(counts(&alone).iter().all(|count| found.contains(count)));
}

#[test]
fn tessellation_cancelled_anywhere_stops_with_cancelled() {
    let ball = fixtures::sphere(5.0);
    let fine = SamplingTolerance::new(1e-3, 0.05).unwrap();

    let polls = assert_cancelled_anywhere(
        "ball",
        || ball.tessellate(&fine),
        |error| matches!(error, TessellationError::Cancelled(_)),
    );

    assert!(polls > 10, "only {polls} polls");
}

#[test]
fn a_mesh_needing_more_points_than_the_budget_is_refused() {
    let ball = fixtures::sphere(5.0);
    let fine = SamplingTolerance::new(1e-3, 0.05).unwrap();
    let points = ball.tessellate(&fine).unwrap().positions().len();

    assert!(tessellate_within(&ball, &fine, points).is_ok());
    assert_eq!(
        tessellate_within(&ball, &fine, points / 2),
        Err(TessellationError::TooLarge)
    );
    assert_eq!(
        tessellate_within(&ball, &fine, 10).unwrap_err().to_string(),
        format!("the mesh would need more than {MAX_POINTS} points")
    );
}

fn radial_distance(point: Point3, center: Point3) -> f64 {
    (point - center).truncate().length()
}

fn assert_circle_edges_follow(solid: &Solid, mesh: &Mesh, center: Point3, radius: f64) {
    let quality = MeshQuality::SMOOTH;
    let chord = solid.tolerance_for(&quality).chord();
    let circles: Vec<&EdgePolyline> = mesh
        .edges()
        .iter()
        .filter(|polyline| solid.edge(polyline.edge).unwrap().is_closed())
        .collect();

    assert!(!circles.is_empty());
    for polyline in circles {
        let points: Vec<Point3> = polyline
            .positions
            .iter()
            .map(|position| mesh.position(*position).unwrap())
            .collect();
        let edge = solid.edge(polyline.edge).unwrap();
        let faces: Vec<FaceId> = edge
            .coedges()
            .iter()
            .filter_map(|coedge| solid.coedge_face(*coedge))
            .collect();

        assert!(points.len() > (TAU / quality.angle()) as usize);
        assert_eq!(faces.len(), 2);
        for face in faces {
            let range = mesh
                .faces()
                .iter()
                .find(|found| found.face == face)
                .unwrap();
            let used: BTreeSet<u32> = mesh.triangles()[range.triangles.clone()]
                .iter()
                .flat_map(|triangle| mesh.triangle_positions(*triangle).unwrap())
                .collect();
            assert!(
                polyline
                    .positions
                    .iter()
                    .all(|position| used.contains(position)),
                "the outline of {face:?} leaves its triangles"
            );
        }
        for pair in points.windows(2) {
            let (from, to) = ((pair[0] - center).truncate(), (pair[1] - center).truncate());
            let step = from.angle_to(to).abs();
            let sagitta = radius * (1.0 - (step / 2.0).cos());
            assert!(step <= quality.angle() + 1e-9, "a {step} rad step");
            assert!(sagitta <= chord + 1e-12, "a sagitta of {sagitta}");
        }
    }
}

#[test]
fn a_displayed_cylinder_keeps_its_steps_within_the_angle_and_chord_and_shades_smoothly() {
    let (radius, height) = (10.0, 20.0);
    let solid = fixtures::cylinder(radius, height);
    let mesh = solid.display_mesh(&MeshQuality::SMOOTH).unwrap();
    let chord = solid.tolerance_for(&MeshQuality::SMOOTH).chord();
    let coarse = solid.tessellate(&solid.default_tolerance()).unwrap();

    assert_watertight("smooth cylinder", &mesh);
    assert!(mesh.triangles().len() > coarse.triangles().len());
    assert_circle_edges_follow(&solid, &mesh, Point3::ZERO, radius);

    for face in mesh.faces() {
        let curved = matches!(
            solid.face(face.face).unwrap().surface(),
            crate::surface::Surface::Cylinder(_)
        );
        for triangle in &mesh.triangles()[face.triangles.clone()] {
            let corners = mesh.corner_points(*triangle).unwrap();
            for (corner, point) in triangle.iter().zip(corners) {
                let normal = mesh.vertices()[*corner as usize].normal;
                let expected = if curved {
                    (point - Point3::new(0.0, 0.0, point.z)).normalize()
                } else if point.z > 0.5 * height {
                    Vector3::Z
                } else {
                    Vector3::NEG_Z
                };
                assert!(
                    normal.distance(expected) < 1e-9,
                    "{normal} at {point} should be {expected}"
                );
            }
            if curved {
                let [a, b, c] = corners;
                for inside in [
                    (a + b + c) / 3.0,
                    (a + b) * 0.5,
                    (b + c) * 0.5,
                    (c + a) * 0.5,
                ] {
                    let deviation = radius - radial_distance(inside, Point3::ZERO);
                    assert!(deviation <= chord + 1e-12, "{deviation} off the cylinder");
                }
            }
        }
    }
}

#[test]
fn a_small_hole_in_a_large_block_is_as_round_as_a_large_one() {
    let (side, radius) = (100.0, 2.0);
    let solid = fixtures::holed_block(side, 10.0, radius);
    let mesh = solid.display_mesh(&MeshQuality::SMOOTH).unwrap();

    assert_watertight("smooth holed block", &mesh);
    assert_circle_edges_follow(
        &solid,
        &mesh,
        Point3::new(0.5 * side, 0.5 * side, 0.0),
        radius,
    );
}

#[test]
fn a_display_mesh_over_its_budget_falls_back_to_the_coarse_quality() {
    let ball = fixtures::sphere(5.0);
    let extent = ball.bounding_box().unwrap().diagonal();
    let smooth = ball.display_mesh(&MeshQuality::SMOOTH).unwrap();
    let coarse = ball.tessellate(&ball.default_tolerance()).unwrap();
    let limit = smooth.positions().len() / 2;

    assert!(smooth.positions().len() > coarse.positions().len());
    assert_eq!(
        tessellate_for_display(&ball, extent, &MeshQuality::SMOOTH, limit, Keys::Skipped)
            .map(DisplayMesh::into_mesh),
        Ok(coarse.clone())
    );
    assert_eq!(
        tessellate_for_display(&ball, extent, &MeshQuality::COARSE, 10, Keys::Skipped)
            .map(DisplayMesh::into_mesh),
        Err(TessellationError::TooLarge)
    );
}

fn surface_deviation(solid: &Solid, tolerance: &SamplingTolerance) -> f64 {
    let mesh = solid.tessellate(tolerance).unwrap();
    let mut worst: f64 = 0.0;
    for face in mesh.faces() {
        let surface = solid.face(face.face).unwrap().surface();
        for triangle in &mesh.triangles()[face.triangles.clone()] {
            let [a, b, c] = mesh.corner_points(*triangle).unwrap();
            for point in [
                (a + b + c) / 3.0,
                (a + b) * 0.5,
                (b + c) * 0.5,
                (c + a) * 0.5,
            ] {
                let foot = surface.project(point, None);
                worst = worst.max(surface.point_at(foot).distance(point));
            }
        }
    }
    worst
}

#[test]
fn every_surface_kind_meshes_within_the_requested_chord() {
    let mut solids = fixtures::every_solid();
    solids.push(("fat torus", fixtures::torus(3.0, 2.5)));
    solids.push(("thin torus", fixtures::torus(20.0, 0.5)));
    solids.push(("very thin torus", fixtures::torus(60.0, 0.4)));
    for chord in [0.1, 0.02, 0.004] {
        let tolerance = SamplingTolerance::new(chord, 1.0).unwrap();
        for (name, solid) in &solids {
            let worst = surface_deviation(solid, &tolerance);
            assert!(
                worst <= chord * 1.05,
                "{name} deviates {worst} from its surface at a chord of {chord}"
            );
        }
    }
}

#[test]
fn a_long_cylinder_and_a_small_bump_mesh_with_few_triangles_within_the_chord() {
    let drill = fixtures::cylinder(0.2, 4.0)
        .transformed(
            &RigidTransform::rotation_about(Point3::ZERO, Vector3::X, PI / 2.0)
                .unwrap()
                .then(&RigidTransform::translation(Vector3::new(0.0, 2.0, 500.0)).unwrap()),
        )
        .unwrap();
    let long = fixtures::cylinder(0.5, 1000.0);
    let drilled = boolean(&long, &drill, BooleanOperation::Difference).unwrap();
    let bumped = fixtures::bumped_block(100.0, 10.0, 41, 2.0);

    for (name, solid, most) in [
        ("long cylinder", long, 1000),
        ("drilled long cylinder", drilled, 2000),
        ("bumped block", bumped, 5000),
    ] {
        let tolerance = solid.tolerance_for(&MeshQuality::SMOOTH);
        let mesh = solid.display_mesh(&MeshQuality::SMOOTH).unwrap();
        let deviation = surface_deviation(&solid, &tolerance);

        assert_watertight(name, &mesh);
        assert!(
            mesh.triangles().len() <= most,
            "{name} has {} triangles",
            mesh.triangles().len()
        );
        assert!(
            deviation <= tolerance.chord() * 1.05,
            "{name} deviates {deviation} at a chord of {}",
            tolerance.chord()
        );
    }
}

#[test]
fn indexed_containment_agrees_with_testing_every_triangle() {
    let mut random = Random::new(17);
    for (name, solid) in fixtures::every_solid() {
        let mesh = solid.tessellate(&solid.default_tolerance()).unwrap();
        let triangles: Vec<[Point3; 3]> = mesh.face_triangles(|_| true).collect();
        let index = TriangleIndex::new(&triangles);
        let bounds = solid.bounding_box().unwrap().expanded(1.0);

        let scattered = (0..64).map(|_| {
            let [x, y, z] = [random.unit(), random.unit(), random.unit()];
            bounds.min() + (bounds.max() - bounds.min()) * Vector3::new(x, y, z)
        });
        let on_mesh = triangles
            .iter()
            .step_by(triangles.len().div_ceil(64))
            .flat_map(|[a, b, c]| [*a, (*a + *b + *c) / 3.0, (*a + *b) / 2.0]);
        let probes: Vec<Point3> = scattered.chain(on_mesh).collect();

        for probe in probes {
            assert_eq!(
                index.contains(&triangles, probe),
                mesh.contains(probe, |_| true),
                "{name} at {probe}"
            );
        }
    }
}

fn perforated_plate(holes: usize, dimples: usize) -> Solid {
    let pitch = 15.0;
    let side = pitch * holes as f64;
    let placed = |solid: Solid, at: Vector3| {
        solid
            .transformed(&RigidTransform::translation(at).unwrap())
            .unwrap()
    };
    let mut plate = fixtures::cuboid(Vector3::new(side, side, 10.0));
    for row in 0..holes {
        for column in 0..holes {
            let at = Vector3::new(
                pitch * (column as f64 + 0.5),
                pitch * (row as f64 + 0.5),
                -1.0,
            );
            let drill = placed(fixtures::cylinder(3.0, 12.0), at);
            plate = boolean(&plate, &drill, BooleanOperation::Difference).unwrap();
        }
    }
    for row in 0..dimples {
        for column in 0..dimples {
            let at = Vector3::new(
                pitch * (column as f64 + 1.0),
                pitch * (row as f64 + 1.0),
                10.0,
            );
            let ball = placed(fixtures::sphere(4.0), at);
            plate = boolean(&plate, &ball, BooleanOperation::Difference).unwrap();
        }
    }
    plate
}

#[test]
#[ignore = "a timing benchmark: cargo test --release -p caditor-kernel display_mesh_costs -- --ignored --nocapture"]
fn display_mesh_costs() {
    let plate = perforated_plate(8, 7);
    let dimple = fixtures::sphere(2.0)
        .transformed(&RigidTransform::translation(Vector3::new(0.0, 60.0, 5.0)).unwrap())
        .unwrap();
    let dimpled = boolean(&plate, &dimple, BooleanOperation::Difference).unwrap();
    let quality = MeshQuality::SMOOTH;
    let tolerance = plate.tolerance_for(&quality);
    let earlier = plate.display_mesh_reusing(&quality, None).unwrap();
    let median = |mesh: &dyn Fn() -> usize| {
        let runs = 9;
        let mut times: Vec<std::time::Duration> = (0..runs)
            .map(|_| {
                let started = std::time::Instant::now();
                assert!(mesh() > 0);
                started.elapsed()
            })
            .collect();
        times.sort();
        times[runs / 2]
    };

    let one_thread = median(&|| {
        let meshed = mesh_faces(&plate, &tolerance, DISPLAY_POINTS, Keys::Skipped, 1);
        meshed.unwrap().mesh().triangles().len()
    });
    let every_thread = median(&|| plate.display_mesh(&quality).unwrap().triangles().len());
    let changed = median(&|| dimpled.display_mesh(&quality).unwrap().triangles().len());
    let reusing = median(&|| {
        let meshed = dimpled.display_mesh_reusing(&quality, Some(&earlier));
        meshed.unwrap().reused_faces()
    });
    let reused = dimpled.display_mesh_reusing(&quality, Some(&earlier));
    let small = perforated_plate(3, 2);
    let coarse = small.default_tolerance();
    let small_alone = median(&|| {
        let meshed = mesh_faces(&small, &coarse, MAX_POINTS, Keys::Skipped, 1);
        meshed.unwrap().mesh().triangles().len()
    });
    let small_shared = median(&|| small.tessellate(&coarse).unwrap().triangles().len());

    println!(
        "{} faces, {} triangles: one thread {one_thread:?}, every thread {every_thread:?}; \
         one face added: {changed:?} afresh, {reusing:?} reusing {} faces",
        plate.faces().count(),
        earlier.mesh().triangles().len(),
        reused.unwrap().reused_faces()
    );
    println!(
        "{} faces coarsely: one thread {small_alone:?}, every thread {small_shared:?}",
        small.faces().count()
    );
}

#[test]
fn faces_meshed_on_many_threads_make_the_mesh_of_one_thread() {
    let plate = perforated_plate(4, 3);
    let tolerance = plate.tolerance_for(&MeshQuality::SMOOTH);

    let alone = mesh_faces(&plate, &tolerance, MAX_POINTS, Keys::Skipped, 1).unwrap();
    let keyed = mesh_faces(
        &plate,
        &tolerance,
        MAX_POINTS,
        Keys::Kept { earlier: None },
        8,
    );

    assert_eq!(plate.faces().count(), 31);
    assert_eq!(keyed.unwrap().mesh(), alone.mesh());
    for threads in [2, 3, 4, 16] {
        let shared = mesh_faces(&plate, &tolerance, MAX_POINTS, Keys::Skipped, threads);
        assert_eq!(shared.unwrap(), alone, "on {threads} threads");
    }
}

#[test]
fn a_changed_body_reuses_the_mesh_of_every_face_it_kept() {
    let plate = perforated_plate(3, 2);
    let dimple = fixtures::sphere(2.0)
        .transformed(&RigidTransform::translation(Vector3::new(15.0, 3.0, 10.0)).unwrap())
        .unwrap();
    let dimpled = boolean(&plate, &dimple, BooleanOperation::Difference).unwrap();
    let quality = MeshQuality::SMOOTH;

    let earlier = plate.display_mesh_reusing(&quality, None).unwrap();
    let again = plate
        .display_mesh_reusing(&quality, Some(&earlier))
        .unwrap();
    let reusing = dimpled
        .display_mesh_reusing(&quality, Some(&earlier))
        .unwrap();
    let coarser = plate
        .display_mesh_reusing(&MeshQuality::COARSE, Some(&earlier))
        .unwrap();

    assert_eq!(earlier.reused_faces(), 0);
    assert_eq!(again.reused_faces(), plate.faces().count());
    assert_eq!(again.mesh(), earlier.mesh());
    assert_eq!(reusing.mesh(), &dimpled.display_mesh(&quality).unwrap());
    assert_eq!(dimpled.faces().count(), plate.faces().count() + 1);
    assert!(
        reusing.reused_faces() >= plate.faces().count() - 1,
        "{} of {} faces reused",
        reusing.reused_faces(),
        dimpled.faces().count()
    );
    assert_eq!(coarser.reused_faces(), 0);
    assert_eq!(
        coarser.mesh(),
        &plate.display_mesh(&MeshQuality::COARSE).unwrap()
    );
}

#[test]
fn meshing_faces_on_many_threads_cancelled_anywhere_stops_with_cancelled() {
    let plate = perforated_plate(4, 0);
    let tolerance = plate.default_tolerance();

    assert_cancelled_anywhere(
        "plate",
        || mesh_faces(&plate, &tolerance, MAX_POINTS, Keys::Skipped, 4),
        |error| matches!(error, TessellationError::Cancelled(_)),
    );
}
