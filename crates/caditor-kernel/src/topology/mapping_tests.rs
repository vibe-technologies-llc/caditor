use std::{
    collections::BTreeSet,
    f64::consts::{FRAC_PI_2, PI},
};

use caditor_geometry::{Plane, Point3, RigidTransform, Similarity, Vector3};

use super::*;
use crate::{
    boolean::{BooleanOperation, boolean},
    curve::Curve,
    fixtures::{self, cylinder, holed_block},
    naming::EdgeName,
    test_support::assert_watertight,
};

fn volume(solid: &Solid) -> f64 {
    solid
        .tessellate(&SamplingTolerance::new(1e-3, 0.05).unwrap())
        .unwrap()
        .mass_properties()
        .volume
}

fn face_names(solid: &Solid) -> BTreeSet<FaceName> {
    solid.faces().map(|(_, face)| face.name()).collect()
}

fn edge_names(solid: &Solid) -> BTreeSet<EdgeName> {
    solid.edges().map(|(_, edge)| edge.name()).collect()
}

fn tilted_mirror() -> Similarity {
    Similarity::reflection(
        &Plane::new(Point3::new(1.0, -2.0, 0.5), Vector3::new(0.3, -1.0, 0.7)).unwrap(),
    )
    .unwrap()
}

fn mirrors() -> Vec<(&'static str, Similarity)> {
    vec![
        ("YZ", Similarity::reflection(&Plane::YZ).unwrap()),
        ("XY", Similarity::reflection(&Plane::XY).unwrap()),
        ("tilted", tilted_mirror()),
    ]
}

fn crossing_cylinders() -> Solid {
    let upright = cylinder(3.0, 10.0);
    let across = cylinder(1.5, 12.0)
        .transformed(
            &RigidTransform::rotation_about(Point3::ZERO, Vector3::X, -FRAC_PI_2)
                .unwrap()
                .then(&RigidTransform::translation(Vector3::new(0.5, -6.0, 5.0)).unwrap()),
        )
        .unwrap();
    boolean(&upright, &across, BooleanOperation::Union).unwrap()
}

fn check(name: &str, solid: &Solid, expected_volume: f64) {
    assert_eq!(solid.validate(), Ok(()), "{name}");
    assert_watertight(name, &solid.tessellate(&solid.default_tolerance()).unwrap());
    let found = volume(solid);
    assert!(
        (found - expected_volume).abs() <= 2e-3 * expected_volume.abs().max(1.0),
        "{name}: volume {found}, expected {expected_volume}"
    );
}

#[test]
fn every_fixture_mirrors_into_a_valid_solid_of_the_same_volume_and_names() {
    for (name, solid) in fixtures::every_solid() {
        let original = volume(&solid);
        for (plane, mirror) in mirrors() {
            let mirrored = solid.mapped(&mirror).unwrap();
            let label = format!("{name} across {plane}");

            check(&label, &mirrored, original);
            assert_eq!(face_names(&mirrored), face_names(&solid), "{label}");
            assert_eq!(edge_names(&mirrored), edge_names(&solid), "{label}");
        }
    }
}

#[test]
fn mirroring_puts_every_vertex_at_its_image_and_twice_puts_it_back() {
    let mirror = tilted_mirror();
    for (name, solid) in fixtures::every_solid() {
        let once = solid.mapped(&mirror).unwrap();
        let twice = once.mapped(&mirror).unwrap();

        for ((_, vertex), (_, image)) in solid.vertices().zip(once.vertices()) {
            assert!(
                image.point().distance(mirror.apply_point(vertex.point())) < 1e-12,
                "{name}"
            );
        }
        for ((_, vertex), (_, back)) in solid.vertices().zip(twice.vertices()) {
            assert!(back.point().distance(vertex.point()) < 1e-9, "{name}");
        }
        assert_eq!(twice.validate(), Ok(()), "{name}");
    }
}

#[test]
fn every_fixture_scales_into_a_valid_solid_of_the_scaled_volume() {
    let center = Point3::new(2.0, -1.0, 3.0);
    for (name, solid) in fixtures::every_solid() {
        let original = volume(&solid);
        for factor in [0.1, 0.5, 2.0, 25.4] {
            let scaling = Similarity::scaling(center, factor).unwrap();
            let scaled = solid.mapped(&scaling).unwrap();
            let label = format!("{name} by {factor}");

            check(&label, &scaled, original * factor.powi(3));
            assert_eq!(face_names(&scaled), face_names(&solid), "{label}");
            assert_eq!(edge_names(&scaled), edge_names(&solid), "{label}");
        }
    }
}

#[test]
fn intersection_edges_are_traced_again_when_enlarged() {
    let solid = crossing_cylinders();
    let original = volume(&solid);
    assert!(
        solid
            .edges()
            .any(|(_, edge)| matches!(edge.curve(), Curve::Intersection(_)))
    );

    for factor in [0.25, 25.4, 400.0] {
        let scaling = Similarity::scaling(Point3::ZERO, factor).unwrap();
        let mirrored_and_scaled = tilted_mirror().then(&scaling);
        for (label, similarity) in [("scaled", scaling), ("mirrored", mirrored_and_scaled)] {
            let mapped = solid.mapped(&similarity).unwrap();
            let label = format!("{label} by {factor}");

            check(&label, &mapped, original * factor.powi(3));
            assert_eq!(edge_names(&mapped), edge_names(&solid), "{label}");
        }
    }
}

#[test]
fn a_mirrored_copy_joins_its_original() {
    let block = holed_block(10.0, 4.0, 2.5)
        .transformed(&RigidTransform::translation(Vector3::new(-1.0, 0.0, 0.0)).unwrap())
        .unwrap();
    let mirrored = block
        .mapped(&Similarity::reflection(&Plane::YZ).unwrap())
        .unwrap();
    let joined = boolean(&block, &mirrored, BooleanOperation::Union).unwrap();
    let hole = PI * 2.5 * 2.5 * 4.0;

    check("joined", &joined, 2.0 * (400.0 - hole) - 4.0 * 2.0 * 10.0);
}

#[test]
fn a_scale_beyond_the_largest_size_or_below_the_resolution_is_refused() {
    let solid = fixtures::cuboid(Vector3::new(0.5, 0.3, 0.2));
    let enlarged = Similarity::scaling(Point3::new(-2.0, 0.0, 0.0), 1e6).unwrap();
    let shrunk = Similarity::scaling(Point3::ZERO, 1e-6).unwrap();

    assert!(matches!(
        solid.mapped(&enlarged),
        Err(TransformError::Geometry(GeometryError::BeyondMaximum(_)))
    ));
    assert!(matches!(
        solid.mapped(&shrunk),
        Err(TransformError::Geometry(GeometryError::BelowResolution(_)))
    ));
}
