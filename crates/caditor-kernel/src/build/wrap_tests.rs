use std::f64::consts::{PI, TAU};

use caditor_geometry::{Plane, Point3, Vector3};

use super::*;
use crate::{
    boolean::split_faces,
    measure::face_area,
    naming::{FaceName, SplitPiece},
    profile::{Profile, ProfileCurve, Region, Selection},
    surface::{Cylinder, Surface},
    test_support::{circle, line, rectangle},
    tolerance::SamplingTolerance,
    topology::{FaceId, Solid},
};

const BODY: u64 = 1;
const TOOL: u64 = 2;
const SPLIT: u64 = 3;
const RADIUS: f64 = 10.0;

fn rod() -> Solid {
    let regions = Profile::new(&[circle(1, (0.0, 0.0), RADIUS)])
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap();
    extrude(
        &Plane::XY,
        &regions,
        LinearExtent::one_side(40.0).unwrap(),
        BODY,
    )
    .unwrap()
}

fn wall_of(solid: &Solid) -> (FaceId, Cylinder) {
    solid
        .faces()
        .find_map(|(id, face)| match face.surface() {
            Surface::Cylinder(cylinder) => Some((id, *cylinder)),
            _ => None,
        })
        .unwrap()
}

fn tangent_plane(offset: f64, normal: Vector3) -> Plane {
    Plane::from_frame(Point3::ZERO + normal.abs() * offset, normal, Vector3::Z).unwrap()
}

fn regions(curves: &[ProfileCurve]) -> Vec<Region> {
    Profile::new(curves)
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap()
}

fn mesh_area(solid: &Solid, face: FaceId) -> f64 {
    let mesh = solid
        .tessellate(&SamplingTolerance::new(1e-4, 0.01).unwrap())
        .unwrap();
    let range = mesh
        .faces()
        .iter()
        .find(|triangles| triangles.face == face)
        .unwrap()
        .triangles
        .clone();
    mesh.triangles()[range]
        .iter()
        .map(|triangle| {
            let [a, b, c] = mesh
                .triangle_positions(*triangle)
                .unwrap()
                .map(|index| mesh.position(index).unwrap());
            0.5 * (b - a).cross(c - a).length()
        })
        .sum()
}

fn wrapped_patch(plane: &Plane, curves: &[ProfileCurve], pieces: usize) -> f64 {
    let body = rod();
    let (wall, cylinder) = wall_of(&body);
    let original = body.face(wall).unwrap().name();
    let tool = wrap_regions(plane, &regions(curves), &cylinder, TOOL).unwrap();
    assert_eq!(tool.validate(), Ok(()));

    let split = split_faces(&body, &[wall], &tool, SPLIT).unwrap();
    let inside = FaceName::split(SPLIT, original, SplitPiece::Inside);
    let patches: Vec<FaceId> = split
        .faces()
        .filter(|(_, face)| face.name() == inside)
        .map(|(id, _)| id)
        .collect();

    assert_eq!(split.validate(), Ok(()));
    assert_eq!(patches.len(), pieces);
    patches
        .iter()
        .map(|patch| {
            face_area(&split, *patch)
                .unwrap()
                .unwrap_or_else(|| mesh_area(&split, *patch))
        })
        .sum()
}

#[test]
fn a_rectangle_wraps_onto_a_cylinder_keeping_its_area_across_the_seam() {
    let area = wrapped_patch(
        &tangent_plane(RADIUS, Vector3::X),
        &rectangle(5, (10.0, -6.0), (20.0, 6.0)),
        2,
    );

    assert!((area - 120.0).abs() < 1e-3, "{area}");
}

#[test]
fn a_slanted_outline_wraps_into_helical_edges() {
    let curves = [
        line(5, (10.0, -8.0), (14.0, -8.0)),
        line(6, (14.0, -8.0), (22.0, 8.0)),
        line(7, (22.0, 8.0), (18.0, 8.0)),
        line(8, (18.0, 8.0), (10.0, -8.0)),
    ];

    let area = wrapped_patch(&tangent_plane(RADIUS, Vector3::Y), &curves, 1);

    assert!((area - 64.0).abs() < 1e-2, "{area}");
}

#[test]
fn a_circle_from_a_plane_apart_and_facing_the_axis_wraps_into_a_disc() {
    let area = wrapped_patch(
        &tangent_plane(25.0, -Vector3::X),
        &[circle(5, (20.0, 3.0), 4.0)],
        2,
    );

    assert!((area - PI * 16.0).abs() < 1e-3, "{area}");
}

#[test]
fn a_plane_across_the_axis_cannot_be_wrapped() {
    let (_, cylinder) = wall_of(&rod());

    let wrapped = wrap_regions(
        &Plane::XY,
        &regions(&rectangle(5, (1.0, 1.0), (2.0, 2.0))),
        &cylinder,
        TOOL,
    );

    assert_eq!(wrapped.err(), Some(WrapError::AcrossAxis));
}

#[test]
fn an_outline_longer_than_once_round_is_refused() {
    let (_, cylinder) = wall_of(&rod());
    let round = TAU * RADIUS;

    let wrapped = wrap_regions(
        &tangent_plane(RADIUS, Vector3::X),
        &regions(&rectangle(
            5,
            (10.0, -0.5 * round),
            (20.0, 0.5 * round + 1.0),
        )),
        &cylinder,
        TOOL,
    );

    assert!(matches!(
        wrapped,
        Err(WrapError::BeyondFullTurn { circumference, .. }) if (circumference - round).abs() < 1e-9
    ));
}
