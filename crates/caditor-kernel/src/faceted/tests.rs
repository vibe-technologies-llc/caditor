use std::f64::consts::TAU;

use caditor_geometry::{Point3, Vector3};

use super::*;
use crate::{Curve, tolerance::SamplingTolerance};

fn volume(solid: &Solid) -> f64 {
    solid
        .tessellate(&SamplingTolerance::new(1e-3, 0.05).unwrap())
        .unwrap()
        .mass_properties()
        .volume
}

fn cuboid(min: Point3, max: Point3) -> TriangleMesh {
    let corner = |x: bool, y: bool, z: bool| {
        Point3::new(
            if x { max.x } else { min.x },
            if y { max.y } else { min.y },
            if z { max.z } else { min.z },
        )
    };
    let positions = vec![
        corner(false, false, false),
        corner(true, false, false),
        corner(true, true, false),
        corner(false, true, false),
        corner(false, false, true),
        corner(true, false, true),
        corner(true, true, true),
        corner(false, true, true),
    ];
    let quads = [
        [0, 3, 2, 1],
        [4, 5, 6, 7],
        [0, 1, 5, 4],
        [1, 2, 6, 5],
        [2, 3, 7, 6],
        [3, 0, 4, 7],
    ];
    let triangles = quads
        .iter()
        .flat_map(|[a, b, c, d]| [[*a, *b, *c], [*a, *c, *d]])
        .collect();
    TriangleMesh {
        positions,
        triangles,
    }
}

fn cube() -> TriangleMesh {
    cuboid(Point3::ZERO, Point3::splat(10.0))
}

fn soup(mesh: &TriangleMesh) -> TriangleMesh {
    let mut positions = Vec::new();
    let mut triangles = Vec::new();
    for triangle in &mesh.triangles {
        let start = positions.len();
        positions.extend(triangle.iter().map(|corner| mesh.positions[*corner]));
        triangles.push([start, start + 1, start + 2]);
    }
    TriangleMesh {
        positions,
        triangles,
    }
}

fn counts(solid: &Solid) -> (usize, usize, usize) {
    (
        solid.faces().count(),
        solid.edges().count(),
        solid.vertices().count(),
    )
}

#[test]
fn a_triangulated_cube_becomes_six_square_faces() {
    let built = faceted_solids(&cube()).unwrap();

    let [solid] = built.solids.as_slice() else {
        panic!("expected one solid, found {}", built.solids.len());
    };
    assert_eq!(counts(solid), (6, 12, 8));
    assert_eq!(built.faces, 6);
    assert!((volume(solid) - 1000.0).abs() < 1e-6);
    assert_eq!(built.repairs, MeshRepairs::default());
}

#[test]
fn each_face_lists_the_triangles_it_was_made_of() {
    let mut mesh = soup(&cube());
    mesh.triangles.insert(0, [0, 0, 1]);

    let built = faceted_solids(&mesh).unwrap();

    let [solid] = built.solids.as_slice() else {
        panic!("expected one solid, found {}", built.solids.len());
    };
    let [sources] = built.sources.as_slice() else {
        panic!("expected sources for one solid");
    };
    assert_eq!(sources.len(), solid.faces().count());
    let mut every: Vec<usize> = sources.iter().flatten().copied().collect();
    every.sort_unstable();
    assert_eq!(every, (1..13).collect::<Vec<_>>());
    for ((_, face), made_of) in solid.faces().zip(sources) {
        assert_eq!(made_of.len(), 2);
        for source in made_of {
            for corner in mesh.triangles[*source] {
                let point = mesh.positions[corner];
                assert!(face.surface().distance(point) < 1e-9);
            }
        }
    }
}

#[test]
fn a_triangle_soup_is_welded_and_an_inside_out_one_turned_around() {
    let mut inverted = soup(&cube());
    for triangle in &mut inverted.triangles {
        triangle.swap(1, 2);
    }

    let built = faceted_solids(&inverted).unwrap();

    assert_eq!(built.repairs.welded, 36 - 8);
    assert_eq!(built.repairs.inverted_shells, 1);
    let [solid] = built.solids.as_slice() else {
        panic!("expected one solid");
    };
    assert!((volume(solid) - 1000.0).abs() < 1e-6);
}

#[test]
fn triangles_against_their_neighbours_are_flipped_and_degenerate_ones_dropped() {
    let mut mesh = cube();
    mesh.triangles[3].swap(0, 1);
    mesh.triangles.push([0, 0, 1]);
    mesh.triangles.push(mesh.triangles[0]);

    let built = faceted_solids(&mesh).unwrap();

    assert_eq!(built.repairs.flipped, 1);
    assert_eq!(built.repairs.degenerate, 1);
    assert_eq!(built.repairs.duplicate, 1);
    assert_eq!(counts(&built.solids[0]), (6, 12, 8));
}

#[test]
fn a_small_hole_is_closed_and_a_mesh_without_any_closed_part_is_refused() {
    let mut holed = cube();
    holed.triangles.remove(5);

    let built = faceted_solids(&holed).unwrap();

    assert_eq!(built.repairs.filled_holes, 1);
    assert!((volume(&built.solids[0]) - 1000.0).abs() < 1e-6);

    let sheet = TriangleMesh {
        positions: vec![
            Point3::ZERO,
            Point3::X,
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(2.0, 0.0, 0.0),
        ],
        triangles: vec![[0, 1, 2], [1, 3, 2]],
    };

    assert!(matches!(
        faceted_solids(&sheet),
        Err(FacetedError::NotClosed { .. })
    ));
    assert_eq!(
        faceted_solids(&TriangleMesh::default()),
        Err(FacetedError::NoTriangles)
    );
}

#[test]
fn a_point_splitting_a_straight_edge_is_left_out_of_the_solid() {
    let mut mesh = cube();
    let middle = mesh.positions.len();
    mesh.positions.push(Point3::new(5.0, 0.0, 0.0));
    mesh.triangles.retain(|triangle| {
        let mut sorted = *triangle;
        sorted.sort_unstable();
        sorted != [0, 2, 3] && sorted != [0, 1, 2] && sorted != [0, 1, 5] && sorted != [0, 4, 5]
    });
    mesh.triangles.extend([
        [0, 3, middle],
        [middle, 3, 2],
        [middle, 2, 1],
        [0, middle, 4],
        [middle, 5, 4],
        [middle, 1, 5],
    ]);

    let built = faceted_solids(&mesh).unwrap();

    assert_eq!(counts(&built.solids[0]), (6, 12, 8));
}

fn prism(sides: usize, radius: f64, height: f64, offset: Vector3) -> TriangleMesh {
    let mut positions = Vec::new();
    for level in [0.0, height] {
        for side in 0..sides {
            let angle = TAU * side as f64 / sides as f64;
            positions.push(Point3::new(radius * angle.cos(), radius * angle.sin(), level) + offset);
        }
    }
    let mut triangles = Vec::new();
    for side in 0..sides {
        let next = (side + 1) % sides;
        triangles.push([side, next, sides + next]);
        triangles.push([side, sides + next, sides + side]);
    }
    for side in 1..sides - 1 {
        triangles.push([0, side + 1, side]);
        triangles.push([sides, sides + side, sides + side + 1]);
    }
    TriangleMesh {
        positions,
        triangles,
    }
}

#[test]
fn a_faceted_cylinder_keeps_one_face_per_flat_side_and_two_caps() {
    let built = faceted_solids(&prism(32, 5.0, 8.0, Vector3::ZERO)).unwrap();

    let [solid] = built.solids.as_slice() else {
        panic!("expected one solid");
    };
    assert_eq!(counts(solid), (34, 96, 64));
    assert!(
        solid
            .edges()
            .all(|(_, edge)| matches!(edge.curve(), Curve::Line(_)))
    );
}

#[test]
fn separate_shells_become_separate_solids_and_a_sealed_hollow_is_left_out() {
    let mut two = cube();
    let other = cuboid(Point3::new(20.0, 0.0, 0.0), Point3::new(25.0, 5.0, 5.0));
    let shift = two.positions.len();
    two.positions.extend(other.positions);
    two.triangles.extend(
        other
            .triangles
            .iter()
            .map(|triangle| triangle.map(|corner| corner + shift)),
    );

    let built = faceted_solids(&two).unwrap();

    assert_eq!(built.solids.len(), 2);

    let mut hollow = cube();
    let mut void = cuboid(Point3::splat(3.0), Point3::splat(6.0));
    for triangle in &mut void.triangles {
        triangle.swap(1, 2);
    }
    let shift = hollow.positions.len();
    hollow.positions.extend(void.positions);
    hollow.triangles.extend(
        void.triangles
            .iter()
            .map(|triangle| triangle.map(|corner| corner + shift)),
    );

    let built = faceted_solids(&hollow).unwrap();

    assert_eq!(built.solids.len(), 1);
    assert_eq!(built.repairs.sealed_voids, 1);
}

#[test]
fn a_tilted_block_read_at_single_precision_is_snapped_onto_its_planes() {
    let block = cuboid(Point3::ZERO, Point3::new(37.0, 23.0, 11.0));
    let axis = Vector3::new(1.0, 2.0, 3.0).normalize();
    let turn = caditor_geometry::Rotation3::from_axis_angle(axis, 0.7);
    let rounded = TriangleMesh {
        positions: block
            .positions
            .iter()
            .map(|point| {
                let moved = turn * *point + Vector3::new(150.0, -80.0, 40.0);
                moved.as_vec3().as_dvec3()
            })
            .collect(),
        triangles: block.triangles,
    };

    let built = faceted_solids(&rounded).unwrap();

    let [solid] = built.solids.as_slice() else {
        panic!("expected one solid");
    };
    assert_eq!(counts(solid), (6, 12, 8));
    assert!((volume(solid) - 37.0 * 23.0 * 11.0).abs() < 1e-2);
    assert_eq!(built.repairs.kept_triangles, 0);
}

#[test]
fn a_mesh_of_thousands_of_faces_builds() {
    let built = faceted_solids(&prism(1500, 40.0, 10.0, Vector3::ZERO)).unwrap();

    assert_eq!(built.faces, 1502);
}
