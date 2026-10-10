use std::f64::consts::{PI, TAU};

use caditor_geometry::{Plane, Point2, Point3, Vector2, Vector3};

use super::{wrap_cut::wrap_chain, *};
use crate::{
    boolean::split_faces,
    measure::face_area,
    naming::{FaceName, SplitPiece},
    profile::{Profile, ProfileCurve, Region, Selection},
    surface::Surface,
    test_support::{arc, circle, line, rectangle},
    tolerance::SamplingTolerance,
    topology::{FaceId, Solid},
};

const BODY: u64 = 1;
const TOOL: u64 = 2;
const SPLIT: u64 = 3;
const RADIUS: f64 = 10.0;
const HEIGHT: f64 = 40.0;

fn regions(curves: &[ProfileCurve]) -> Vec<Region> {
    Profile::new(curves)
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap()
}

fn extruded(curves: &[ProfileCurve]) -> Solid {
    extrude(
        &Plane::XY,
        &regions(curves),
        LinearExtent::one_side(HEIGHT).unwrap(),
        BODY,
    )
    .unwrap()
}

fn rod() -> Solid {
    extruded(&[circle(1, (0.0, 0.0), RADIUS)])
}

fn half_rod() -> Solid {
    extruded(&[
        arc(1, (0.0, 0.0), (0.0, -RADIUS), (0.0, RADIUS)),
        line(2, (0.0, RADIUS), (0.0, -RADIUS)),
    ])
}

fn cone() -> Solid {
    revolve(
        &Plane::XY,
        &regions(&[
            line(1, (0.0, 0.0), (12.0, 0.0)),
            line(2, (12.0, 0.0), (6.0, 30.0)),
            line(3, (6.0, 30.0), (0.0, 30.0)),
            line(4, (0.0, 30.0), (0.0, 0.0)),
        ]),
        Axis2::new(Point2::ZERO, Vector2::Y).unwrap(),
        AngularExtent::full(),
        BODY,
    )
    .unwrap()
}

fn curved(solid: &Solid) -> (Vec<FaceId>, Surface) {
    let faces: Vec<(FaceId, Surface)> = solid
        .faces()
        .filter(|(_, face)| matches!(face.surface(), Surface::Cylinder(_) | Surface::Cone(_)))
        .map(|(id, face)| (id, face.surface().clone()))
        .collect();
    let surface = faces.first().unwrap().1.clone();
    (faces.into_iter().map(|(id, _)| id).collect(), surface)
}

fn side_plane() -> Plane {
    Plane::from_frame(Point3::new(RADIUS, 0.0, 0.0), Vector3::X, Vector3::Z).unwrap()
}

fn area(solid: &Solid, face: FaceId) -> f64 {
    face_area(solid, face).unwrap().unwrap_or_else(|| {
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
    })
}

struct Divided {
    inside: Vec<f64>,
    outside: Vec<f64>,
}

impl Divided {
    fn total(&self) -> f64 {
        self.inside.iter().chain(&self.outside).sum()
    }

    fn count(&self) -> usize {
        self.inside.len() + self.outside.len()
    }
}

fn divided(body: &Solid, plane: &Plane, curves: &[ProfileCurve]) -> Divided {
    let (faces, surface) = curved(body);
    let originals: Vec<FaceName> = faces
        .iter()
        .map(|face| body.face(*face).unwrap().name())
        .collect();
    let tool = wrap_chain(plane, curves, &surface, body, &faces, TOOL).unwrap();
    assert_eq!(tool.validate(), Ok(()));
    let split = split_faces(body, &faces, &tool, SPLIT).unwrap();
    assert_eq!(split.validate(), Ok(()));
    let areas = |piece: SplitPiece| -> Vec<f64> {
        let names: Vec<FaceName> = originals
            .iter()
            .map(|original| FaceName::split(SPLIT, *original, piece))
            .collect();
        split
            .faces()
            .filter(|(_, face)| names.contains(&face.name()))
            .map(|(id, _)| area(&split, id))
            .collect()
    };
    Divided {
        inside: areas(SplitPiece::Inside),
        outside: areas(SplitPiece::Outside),
    }
}

fn round_line(from: (f64, f64), to: (f64, f64)) -> ProfileCurve {
    line(7, (from.0, -from.1), (to.0, -to.1))
}

#[test]
fn a_chain_along_the_axis_parts_the_tube_between_the_seam_and_the_curve() {
    let wall = TAU * RADIUS * HEIGHT;

    let parts = divided(
        &rod(),
        &side_plane(),
        &[round_line((-5.0, 5.0), (45.0, 5.0))],
    );

    assert_eq!(parts.count(), 2);
    assert!((parts.total() - wall).abs() < 1e-2, "{}", parts.total());
}

#[test]
fn a_helix_running_round_three_times_parts_the_tube_into_bands_at_the_seam() {
    let round = TAU * RADIUS;
    let rise = 3.0 * round / HEIGHT;
    let start = 0.5 * round;

    let parts = divided(
        &rod(),
        &side_plane(),
        &[round_line(
            (-10.0, start - 10.0 * rise),
            (50.0, start + 50.0 * rise),
        )],
    );

    assert_eq!(parts.count(), 5);
    assert_eq!(parts.inside.len().abs_diff(parts.outside.len()), 1);
    assert!(
        (parts.total() - round * HEIGHT).abs() < 1e-1,
        "{}",
        parts.total()
    );
}

#[test]
fn a_helix_crosses_a_half_cylinder_once_each_turn() {
    let angle_at = |height: f64| -0.75 * PI + height * PI / 10.0;

    let parts = divided(
        &half_rod(),
        &side_plane(),
        &[round_line(
            (-5.0, RADIUS * angle_at(-5.0)),
            (45.0, RADIUS * angle_at(45.0)),
        )],
    );

    assert_eq!(parts.count(), 3);
    assert!(
        (parts.total() - PI * RADIUS * HEIGHT).abs() < 1e-1,
        "{}",
        parts.total()
    );
}

#[test]
fn a_slanted_chain_ending_inside_a_half_cylinder_is_carried_on_past_its_ends() {
    let parts = divided(
        &half_rod(),
        &side_plane(),
        &[round_line((15.0, -3.0), (25.0, 3.0))],
    );

    assert_eq!(parts.count(), 2);
    assert!(
        (parts.total() - PI * RADIUS * HEIGHT).abs() < 1e-1,
        "{}",
        parts.total()
    );
}

#[test]
fn a_chain_turning_back_across_the_seam_cuts_out_its_own_side() {
    let parts = divided(
        &rod(),
        &side_plane(),
        &[
            round_line((-5.0, -8.0), (20.0, 0.0)),
            round_line((20.0, 0.0), (-5.0, 8.0)),
        ],
    );

    assert!(parts.count() >= 2);
    assert!(parts.inside.len() <= 2);
    assert!(
        (parts.total() - TAU * RADIUS * HEIGHT).abs() < 1e-1,
        "{}",
        parts.total()
    );
}

#[test]
fn a_chain_round_the_axis_never_leaves_the_tube() {
    let (faces, surface) = curved(&rod());

    let wrapped = wrap_chain(
        &side_plane(),
        &[round_line((20.0, -5.0), (20.0, 5.0))],
        &surface,
        &rod(),
        &faces,
        TOOL,
    );

    assert_eq!(wrapped.err(), Some(WrapError::EndRunsRound));
}

#[test]
fn a_closed_outline_is_no_chain_to_cut_along() {
    let (faces, surface) = curved(&rod());

    let wrapped = wrap_chain(
        &side_plane(),
        &rectangle(5, (10.0, -3.0), (20.0, 3.0)),
        &surface,
        &rod(),
        &faces,
        TOOL,
    );

    assert_eq!(wrapped.err(), Some(WrapError::NotOneChain));
}

#[test]
fn a_rectangle_wraps_onto_a_cone_keeping_its_area() {
    let body = cone();
    let (faces, surface) = curved(&body);
    let plane = Plane::from_frame(Point3::new(9.0, 0.0, 0.0), Vector3::X, Vector3::Y).unwrap();
    let original = body.face(faces[0]).unwrap().name();

    let tool = wrap_regions(
        &plane,
        &regions(&rectangle(5, (10.0, -6.0), (20.0, 6.0))),
        &surface,
        TOOL,
    )
    .unwrap();
    let split = split_faces(&body, &faces, &tool, SPLIT).unwrap();
    let inside = FaceName::split(SPLIT, original, SplitPiece::Inside);
    let patch: f64 = split
        .faces()
        .filter(|(_, face)| face.name() == inside)
        .map(|(id, _)| area(&split, id))
        .sum();

    assert_eq!(tool.validate(), Ok(()));
    assert_eq!(split.validate(), Ok(()));
    assert!((patch - 120.0).abs() < 1e-2, "{patch}");
}

#[test]
fn a_chain_up_a_cone_parts_it_along_a_generator() {
    let body = cone();
    let plane = Plane::from_frame(Point3::new(9.0, 0.0, 0.0), Vector3::X, Vector3::Y).unwrap();

    let parts = divided(&body, &plane, &[line(7, (-5.0, 4.0), (35.0, 4.0))]);

    assert_eq!(parts.count(), 2);
}

#[test]
fn a_helix_winds_twice_round_a_cone() {
    let body = cone();
    let plane = Plane::from_frame(Point3::new(9.0, 0.0, 0.0), Vector3::X, Vector3::Y).unwrap();

    let parts = divided(
        &body,
        &plane,
        &[line(7, (-5.0, 0.5), (35.0, 0.5 + 2.0 * TAU * 9.0))],
    );

    assert!(parts.count() >= 3, "{}", parts.count());
}

#[test]
fn a_sphere_cannot_be_unrolled() {
    let sphere = revolve(
        &Plane::XY,
        &regions(&[
            arc(1, (0.0, 0.0), (0.0, -5.0), (0.0, 5.0)),
            line(2, (0.0, 5.0), (0.0, -5.0)),
        ]),
        Axis2::new(Point2::ZERO, Vector2::Y).unwrap(),
        AngularExtent::full(),
        BODY,
    )
    .unwrap();
    let (face, surface) = sphere
        .faces()
        .next()
        .map(|(id, face)| (id, face.surface().clone()))
        .unwrap();

    let outline = wrap_regions(
        &side_plane(),
        &regions(&rectangle(5, (1.0, -1.0), (2.0, 1.0))),
        &surface,
        TOOL,
    );
    let chain = wrap_chain(
        &side_plane(),
        &[line(7, (0.0, 0.0), (3.0, 0.0))],
        &surface,
        &sphere,
        &[face],
        TOOL,
    );

    assert_eq!(outline.err(), Some(WrapError::NotUnrollable));
    assert_eq!(chain.err(), Some(WrapError::NotUnrollable));
}
