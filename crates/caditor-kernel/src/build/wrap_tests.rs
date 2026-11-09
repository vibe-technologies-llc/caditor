use std::f64::consts::{PI, TAU};

use caditor_geometry::{Maths, Plane, Point2, Point3, Vector3};

use super::*;
use crate::{
    boolean::{BooleanOperation, boolean, split_faces},
    measure::face_area,
    naming::{FaceName, FaceOrigin, SplitPiece},
    profile::{Profile, ProfileCurve, Region, Selection},
    surface::{Cylinder, Surface},
    test_support::{circle, line, rectangle, spline},
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
    let tool = wrap_regions(plane, &regions(curves), &Surface::Cylinder(cylinder), TOOL).unwrap();
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
        &Surface::Cylinder(cylinder),
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
        &Surface::Cylinder(cylinder),
        TOOL,
    );

    assert!(matches!(wrapped, Err(WrapError::BeyondFullTurn { turns }) if turns > 1.0));
}

#[test]
fn a_band_reaching_the_axis_or_not_a_length_is_refused() {
    let (_, cylinder) = wall_of(&rod());
    let surface = Surface::Cylinder(cylinder);
    let plane = tangent_plane(RADIUS, Vector3::X);
    let outline = regions(&rectangle(5, (10.0, -6.0), (20.0, 6.0)));

    let too_deep = WrapBand::new(RADIUS + 1.0, 1.0).unwrap();
    let reaching = wrap_regions_within(&plane, &outline, &surface, Some(too_deep), TOOL);

    assert_eq!(reaching.err(), Some(WrapError::ReachesAxis));
    assert_eq!(WrapBand::new(0.0, 1.0), Err(WrapError::UnusableBand));
    assert_eq!(WrapBand::new(1.0, f64::NAN), Err(WrapError::UnusableBand));
}

#[test]
fn a_band_wraps_into_a_valid_solid() {
    let (_, cylinder) = wall_of(&rod());
    let surface = Surface::Cylinder(cylinder);
    let plane = tangent_plane(RADIUS, Vector3::X);
    let outline = regions(&rectangle(5, (10.0, -6.0), (20.0, 6.0)));
    let band = WrapBand::new(0.25, 2.0).unwrap();

    let tool = wrap_regions_within(&plane, &outline, &surface, Some(band), TOOL).unwrap();

    assert_eq!(tool.validate(), Ok(()));
}

#[test]
fn a_point_of_the_outlines_stands_on_the_surface_looking_away_from_the_axis() {
    let (_, cylinder) = wall_of(&rod());
    let surface = Surface::Cylinder(cylinder);
    let plane = tangent_plane(RADIUS, Vector3::X);
    let outline = regions(&rectangle(5, (10.0, -6.0), (20.0, 6.0)));

    let (centre, _) = wrapped_stand(&plane, &outline, &surface, Point2::new(15.0, 0.0)).unwrap();
    let (point, outward) =
        wrapped_stand(&plane, &outline, &surface, Point2::new(15.0, 4.0)).unwrap();

    assert!((point.x.hypot_libm(point.y) - RADIUS).abs() < 1e-9);
    assert!((outward - Vector3::new(point.x, point.y, 0.0) / RADIUS).length() < 1e-9);
    assert!((centre.x - RADIUS).abs() < 1e-9);
}

const DRAFT: f64 = 10.0;

fn letter() -> Vec<ProfileCurve> {
    vec![
        line(10, (12.0, -6.0), (24.0, -6.0)),
        line(11, (24.0, -6.0), (24.0, -3.0)),
        line(12, (24.0, -3.0), (17.0, -3.0)),
        line(13, (17.0, -3.0), (17.0, -1.0)),
        line(14, (17.0, -1.0), (22.0, -1.0)),
        line(15, (22.0, -1.0), (22.0, 1.0)),
        line(16, (22.0, 1.0), (17.0, 1.0)),
        line(17, (17.0, 1.0), (17.0, 3.0)),
        line(18, (17.0, 3.0), (24.0, 3.0)),
        line(19, (24.0, 3.0), (24.0, 6.0)),
        spline(20, &[(24.0, 6.0), (18.0, 6.6), (12.0, 6.0)]),
        line(21, (12.0, 6.0), (12.0, -6.0)),
        circle(22, (14.5, 0.0), 1.5),
    ]
}

fn drafted_tool(curves: &[ProfileCurve], band: (f64, f64)) -> Solid {
    let (_, cylinder) = wall_of(&rod());
    let narrowing = if band.0 > band.1 {
        WrapNarrowing::Inward
    } else {
        WrapNarrowing::Outward
    };
    let band = WrapBand::new(band.0, band.1)
        .unwrap()
        .leaning(DRAFT.to_radians(), narrowing)
        .unwrap();

    wrap_regions_within(
        &tangent_plane(RADIUS, Vector3::X),
        &regions(curves),
        &Surface::Cylinder(cylinder),
        Some(band),
        TOOL,
    )
    .unwrap()
}

fn cap_area(tool: &Solid, wanted: fn(FaceOrigin) -> bool) -> f64 {
    let cap = tool
        .faces()
        .find(|(_, face)| face.origin().is_some_and(wanted))
        .map(|(id, _)| id)
        .unwrap();
    mesh_area(tool, cap)
}

fn is_start(origin: FaceOrigin) -> bool {
    matches!(origin, FaceOrigin::StartCap { .. })
}

fn is_end(origin: FaceOrigin) -> bool {
    matches!(origin, FaceOrigin::EndCap { .. })
}

fn drafted_cap(offset: f64, narrowing: f64) -> f64 {
    let lean = DRAFT.to_radians().tan_libm() * offset * narrowing;
    (10.0 - 2.0 * lean) * (12.0 - 2.0 * lean) * (RADIUS + offset) / RADIUS
}

fn wall_tilts(tool: &Solid) -> Vec<(f64, f64)> {
    tool.faces()
        .filter_map(|(_, face)| match face.surface() {
            Surface::BSpline(_) => Some(face.surface()),
            _ => None,
        })
        .map(|surface| {
            let (u, v) = (surface.u_domain(), 0.5);
            let at = 0.5 * (u.start() + u.end());
            let point = surface.point(at, v);
            let normal = surface.normal(at, v).unwrap();
            let radial = Vector3::new(point.x, point.y, 0.0).normalize();
            (normal.dot(Vector3::Z).abs(), normal.dot(radial).abs())
        })
        .collect()
}

#[test]
fn a_drafted_rectangle_wraps_into_a_valid_solid_with_leaning_caps() {
    let outline = rectangle(5, (10.0, -6.0), (20.0, 6.0));
    let raised = drafted_tool(&outline, (0.25, 2.0));
    let recessed = drafted_tool(&outline, (2.0, 0.25));

    assert_eq!(raised.validate(), Ok(()));
    assert_eq!(recessed.validate(), Ok(()));

    let top = cap_area(&raised, is_end);
    let foot = cap_area(&raised, is_start);
    let floor = cap_area(&recessed, is_start);
    let rim = cap_area(&recessed, is_end);

    assert!((top / drafted_cap(2.0, 1.0) - 1.0).abs() < 1e-3, "{top}");
    assert!(
        (foot / drafted_cap(-0.25, 1.0) - 1.0).abs() < 1e-3,
        "{foot}"
    );
    assert!(
        (floor / drafted_cap(-2.0, -1.0) - 1.0).abs() < 1e-3,
        "{floor}"
    );
    assert!((rim / drafted_cap(0.25, -1.0) - 1.0).abs() < 1e-3, "{rim}");
}

#[test]
fn the_walls_of_a_drafted_rectangle_lean_by_the_draft() {
    let outline = rectangle(5, (10.0, -6.0), (20.0, 6.0));
    let (sin, cos) = DRAFT.to_radians().sin_cos_libm();

    for band in [(0.25, 2.0), (2.0, 0.25)] {
        let tilts = wall_tilts(&drafted_tool(&outline, band));
        let (ends, sides): (Vec<_>, Vec<_>) = tilts.iter().partition(|(axial, _)| *axial > 0.5);

        assert_eq!(ends.len(), 2, "{tilts:?}");
        assert_eq!(sides.len(), 2, "{tilts:?}");
        assert!(
            ends.iter().all(|(axial, radial)| {
                (axial - cos).abs() < 1e-3 && (radial - sin).abs() < 1e-3
            })
        );
        assert!(
            sides
                .iter()
                .all(|(_, radial)| (0.8 * sin..1.25 * sin).contains(radial))
        );
    }
}

#[test]
fn a_drafted_letter_with_a_hole_wraps_into_a_valid_solid_both_ways() {
    let curves = letter();
    let raised = drafted_tool(&curves, (0.25, 2.0));
    let recessed = drafted_tool(&curves, (2.0, 0.25));

    assert_eq!(raised.validate(), Ok(()));
    assert_eq!(recessed.validate(), Ok(()));
    assert!(cap_area(&raised, is_end) < cap_area(&raised, is_start));
    assert!(cap_area(&recessed, is_start) < cap_area(&recessed, is_end));
    assert!(wall_tilts(&raised).iter().all(|(_, radial)| *radial > 0.1));
}

#[test]
fn a_drafted_letter_joins_and_cuts_a_rod_into_valid_solids() {
    let body = rod();
    let raised = boolean(
        &body,
        &drafted_tool(&letter(), (0.01, 1.0)),
        BooleanOperation::Union,
    )
    .unwrap();
    let cut = boolean(
        &body,
        &drafted_tool(&letter(), (1.0, 0.01)),
        BooleanOperation::Difference,
    )
    .unwrap();

    assert_eq!(raised.validate(), Ok(()));
    assert_eq!(cut.validate(), Ok(()));
}

#[test]
fn a_draft_closing_the_outline_or_out_of_range_is_refused() {
    let (_, cylinder) = wall_of(&rod());
    let surface = Surface::Cylinder(cylinder);
    let plane = tangent_plane(RADIUS, Vector3::X);
    let outline = regions(&rectangle(5, (10.0, -6.0), (20.0, 6.0)));

    let steep = WrapBand::new(0.25, 8.0)
        .unwrap()
        .leaning(40.0_f64.to_radians(), WrapNarrowing::Outward)
        .unwrap();
    let closed = wrap_regions_within(&plane, &outline, &surface, Some(steep), TOOL);
    let band = WrapBand::new(0.25, 2.0).unwrap();

    assert!(matches!(closed, Err(WrapError::DraftCloses { .. })));
    for angle in [-0.1, f64::NAN, 90.0_f64.to_radians()] {
        assert_eq!(
            band.leaning(angle, WrapNarrowing::Outward),
            Err(WrapError::DraftOutOfRange)
        );
    }
}
