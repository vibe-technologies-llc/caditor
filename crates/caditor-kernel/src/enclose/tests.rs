use std::f64::consts::PI;

use caditor_geometry::{Plane, Point3, RigidTransform, Similarity, Vector3};

use super::*;
use crate::{
    boolean::{BooleanOperation, boolean},
    build::{LinearExtent, extrude},
    fixtures::{cuboid, cylinder, holed_block},
    measure::mass_properties,
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

const RADIUS: f64 = 5.0;
const SLOT_DEPTH: f64 = 3.0;
const SLOT_FLOOR: f64 = 3.0;
const BOSS_TOP: f64 = 7.0;
const CYLINDER_HEIGHT: f64 = 10.0;

fn cylinder_volume() -> f64 {
    PI * RADIUS * RADIUS * CYLINDER_HEIGHT
}

fn under_arc() -> f64 {
    (RADIUS * RADIUS - 1.0).sqrt() + RADIUS * RADIUS * (1.0 / RADIUS).asin()
}

fn round_faces(enclosure: &Enclosure) -> usize {
    enclosure
        .solid
        .faces()
        .filter(|(_, face)| matches!(face.surface(), Surface::Cylinder(_)))
        .count()
}

fn enclosed_tool(
    min: (f64, f64),
    max: (f64, f64),
    operation: BooleanOperation,
) -> (Solid, Enclosure) {
    let body = cylinder(RADIUS, CYLINDER_HEIGHT);
    let tool = block(min, max, 3.0, SLOT_DEPTH, TOOL);
    let shaped = boolean(&body, &tool, operation).unwrap();

    let chosen = faces_of(&shaped, TOOL);
    let enclosure = enclose_faces(&shaped, &chosen, FEATURE).unwrap();

    assert_eq!(chosen.len(), 5);
    (shaped, enclosure)
}

fn exact_volume(solid: &Solid) -> f64 {
    let mesh = solid.tessellate(&solid.default_tolerance()).unwrap();
    let mass = mass_properties(solid, &mesh).unwrap();

    assert!(mass.meshed_faces.is_empty());
    mass.properties.volume
}

fn mirrored_across_xz(body: &Solid, enclosure: &Enclosure) -> f64 {
    let across = Similarity::reflection(&Plane::new(Point3::ZERO, Vector3::Y).unwrap()).unwrap();
    let image = enclosure.solid.mapped(&across).unwrap();
    let operation = match enclosure.bounds {
        Bounds::Cavity => BooleanOperation::Difference,
        Bounds::Material => BooleanOperation::Union,
    };

    let mirrored = boolean(body, &image, operation).unwrap();

    assert_eq!(mirrored.validate(), Ok(()));
    exact_volume(&mirrored)
}

#[test]
fn a_pocket_in_a_cylinder_s_wall_closes_with_a_patch_of_the_wall_and_mirrors() {
    let (pocketed, enclosure) = enclosed_tool(
        (-1.0, SLOT_FLOOR),
        (1.0, BOSS_TOP),
        BooleanOperation::Difference,
    );
    let pocket = SLOT_DEPTH * (under_arc() - 2.0 * SLOT_FLOOR);

    let mirrored = mirrored_across_xz(&pocketed, &enclosure);

    check(&enclosure, Bounds::Cavity, pocket);
    assert_eq!(enclosure.solid.faces().count(), 6);
    assert_eq!(round_faces(&enclosure), 1);
    assert!(
        (mirrored - (cylinder_volume() - 2.0 * pocket)).abs() < 1e-6,
        "{mirrored}"
    );
}

#[test]
fn a_boss_on_a_cylinder_closes_with_a_patch_of_the_wall_and_mirrors() {
    let (bossed, enclosure) =
        enclosed_tool((-1.0, SLOT_FLOOR), (1.0, BOSS_TOP), BooleanOperation::Union);
    let boss = SLOT_DEPTH * (2.0 * BOSS_TOP - under_arc());

    let mirrored = mirrored_across_xz(&bossed, &enclosure);

    check(&enclosure, Bounds::Material, boss);
    assert_eq!(enclosure.solid.faces().count(), 6);
    assert_eq!(round_faces(&enclosure), 1);
    assert!(
        (mirrored - (cylinder_volume() + 2.0 * boss)).abs() < 1e-6,
        "{mirrored}"
    );
}

#[test]
fn a_pocket_across_a_cylinder_s_seam_closes_with_a_patch_of_the_wall() {
    let (_, enclosure) = enclosed_tool(
        (SLOT_FLOOR, -1.0),
        (BOSS_TOP, 1.0),
        BooleanOperation::Difference,
    );

    check(
        &enclosure,
        Bounds::Cavity,
        SLOT_DEPTH * (under_arc() - 2.0 * SLOT_FLOOR),
    );
    assert_eq!(round_faces(&enclosure), 1);
}

#[test]
fn a_collar_whose_openings_run_around_the_cylinder_is_refused() {
    let shift = RigidTransform::translation(Vector3::new(0.0, 0.0, 4.0)).unwrap();
    let collar = cylinder(7.0, 2.0).transformed(&shift).unwrap();
    let collared = boolean(
        &cylinder(RADIUS, CYLINDER_HEIGHT),
        &collar,
        BooleanOperation::Union,
    )
    .unwrap();

    let chosen: Vec<FaceId> = collared
        .faces()
        .filter(|(_, face)| match face.surface() {
            Surface::Cylinder(round) => round.radius() > RADIUS + 1.0,
            Surface::Plane(plane) => (4.0..=6.0).contains(&plane.frame().origin().z),
            _ => false,
        })
        .map(|(id, _)| id)
        .collect();

    assert_eq!(chosen.len(), 3);
    assert!(matches!(
        enclose_faces(&collared, &chosen, FEATURE),
        Err(EnclosureError::AroundSurface { .. })
    ));
}

#[test]
fn a_round_boss_on_a_cylinder_closes_with_a_patch_of_the_wall() {
    let lay = RigidTransform::rotation_about(Point3::ZERO, Vector3::X, -PI / 2.0).unwrap();
    let raise = RigidTransform::translation(Vector3::new(0.0, 0.0, 5.0)).unwrap();
    let peg = cylinder(1.0, 7.0).transformed(&lay.then(&raise)).unwrap();
    let pegged = boolean(
        &cylinder(RADIUS, CYLINDER_HEIGHT),
        &peg,
        BooleanOperation::Union,
    )
    .unwrap();

    let chosen: Vec<FaceId> = pegged
        .faces()
        .filter(|(_, face)| match face.surface() {
            Surface::Cylinder(round) => round.radius() < RADIUS - 1.0,
            Surface::Plane(plane) => plane.frame().origin().y > RADIUS,
            _ => false,
        })
        .map(|(id, _)| id)
        .collect();
    let enclosure = enclose_faces(&pegged, &chosen, FEATURE).unwrap();
    let mirrored = mirrored_across_xz(&pegged, &enclosure);

    assert_eq!(chosen.len(), 2);
    assert_eq!(enclosure.bounds, Bounds::Material);
    assert_eq!(round_faces(&enclosure), 2);
    assert!(
        (mirrored - (2.0 * exact_volume(&pegged) - cylinder_volume())).abs() < 1e-5,
        "{mirrored}"
    );
}
