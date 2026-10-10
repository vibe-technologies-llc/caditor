use std::f64::consts::PI;

use caditor_geometry::{Plane, Point3, Vector3};

use super::*;
use crate::{
    boolean::{BooleanOperation, boolean},
    build::{LinearExtent, extrude},
    fixtures::{cuboid, holed_block},
    profile::{Profile, Selection},
    test_support::{assert_watertight, rectangle},
    tolerance::SamplingTolerance,
};

const FEATURE: u64 = 40;
const TOOL: u64 = 2;

fn block(min: (f64, f64), max: (f64, f64), base: f64, height: f64, feature: u64) -> Solid {
    let regions = Profile::new(&rectangle(1, min, max))
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap();
    let plane = Plane::from_frame(Point3::new(0.0, 0.0, base), Vector3::Z, Vector3::X).unwrap();
    extrude(
        &plane,
        &regions,
        LinearExtent::one_side(height).unwrap(),
        feature,
    )
    .unwrap()
}

fn volume(solid: &Solid) -> f64 {
    solid
        .tessellate(&SamplingTolerance::new(1e-3, 0.05).unwrap())
        .unwrap()
        .mass_properties()
        .volume
}

fn faces_of(solid: &Solid, feature: u64) -> Vec<FaceId> {
    solid
        .faces()
        .filter(|(_, face)| {
            face.origin()
                .is_some_and(|origin| origin.feature() == feature)
        })
        .map(|(id, _)| id)
        .collect()
}

fn check(enclosure: &Enclosure, bounds: Bounds, expected: f64) {
    assert_eq!(enclosure.bounds, bounds);
    assert_eq!(enclosure.solid.validate(), Ok(()));
    let tolerance = enclosure.solid.default_tolerance();
    assert_watertight(
        "enclosure",
        &enclosure.solid.tessellate(&tolerance).unwrap(),
    );
    let found = volume(&enclosure.solid);
    assert!(
        (found - expected).abs() <= 2e-3 * expected,
        "volume {found} instead of {expected}"
    );
}

#[test]
fn a_pocket_s_walls_and_floor_close_into_the_cavity_they_bound() {
    let body = block((0.0, 0.0), (10.0, 10.0), 0.0, 5.0, 1);
    let tool = block((3.0, 3.0), (7.0, 7.0), 2.0, 4.0, TOOL);
    let pocketed = boolean(&body, &tool, BooleanOperation::Difference).unwrap();

    let pocket = faces_of(&pocketed, TOOL);
    let enclosure = enclose_faces(&pocketed, &pocket, FEATURE).unwrap();

    assert_eq!(pocket.len(), 5);
    check(&enclosure, Bounds::Cavity, 48.0);
    assert_eq!(enclosure.solid.faces().count(), 6);
    assert!(
        enclosure
            .solid
            .faces()
            .any(|(_, face)| face.origin().is_some_and(|origin| origin.feature() == 1))
    );
}

#[test]
fn a_boss_s_walls_and_top_close_into_the_material_they_bound() {
    let body = block((0.0, 0.0), (10.0, 10.0), 0.0, 5.0, 1);
    let tool = block((3.0, 3.0), (7.0, 7.0), 5.0, 3.0, TOOL);
    let bossed = boolean(&body, &tool, BooleanOperation::Union).unwrap();

    let boss = faces_of(&bossed, TOOL);
    let enclosure = enclose_faces(&bossed, &boss, FEATURE).unwrap();

    assert_eq!(boss.len(), 5);
    check(&enclosure, Bounds::Material, 48.0);
}

#[test]
fn a_through_hole_s_wall_closes_at_both_ends() {
    let body = holed_block(10.0, 5.0, 2.0);

    let wall: Vec<FaceId> = body
        .faces()
        .filter(|(_, face)| matches!(face.surface(), Surface::Cylinder(_)))
        .map(|(id, _)| id)
        .collect();
    let enclosure = enclose_faces(&body, &wall, FEATURE).unwrap();

    assert_eq!(wall.len(), 1);
    check(&enclosure, Bounds::Cavity, PI * 4.0 * 5.0);
}

#[test]
fn faces_whose_opening_is_not_flat_are_refused() {
    let body = cuboid(Vector3::new(2.0, 3.0, 4.0));

    let two: Vec<FaceId> = body
        .faces()
        .filter(|(_, face)| match face.surface() {
            Surface::Plane(plane) => {
                plane.frame().normal().z > 0.5 || plane.frame().normal().x > 0.5
            }
            _ => false,
        })
        .map(|(id, _)| id)
        .collect();

    assert_eq!(two.len(), 2);
    assert!(matches!(
        enclose_faces(&body, &two, FEATURE),
        Err(EnclosureError::NotFlat { .. })
    ));
}

#[test]
fn a_lone_flat_face_closes_on_itself_into_nothing() {
    let body = cuboid(Vector3::new(2.0, 3.0, 4.0));

    let top: Vec<FaceId> = body
        .faces()
        .filter(|(_, face)| match face.surface() {
            Surface::Plane(plane) => plane.frame().normal().z > 0.5,
            _ => false,
        })
        .map(|(id, _)| id)
        .collect();

    assert!(matches!(
        enclose_faces(&body, &top, FEATURE),
        Err(EnclosureError::Build(_))
    ));
    assert_eq!(
        enclose_faces(&body, &[], FEATURE),
        Err(EnclosureError::NoFaces)
    );
}
