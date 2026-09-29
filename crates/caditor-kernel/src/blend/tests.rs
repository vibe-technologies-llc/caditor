use std::f64::consts::PI;

use caditor_geometry::{Plane, Point3, RigidTransform, Vector3};

use super::*;
use crate::{
    build::{LinearExtent, extrude},
    fixtures::{cuboid, cylinder, hollow_cuboid},
    naming::FaceOrigin,
    profile::{Profile, ProfileCurve, Selection},
    test_support::{arc, assert_cancelled_anywhere, assert_watertight, line},
    tolerance::SamplingTolerance,
};

const SPANDREL_CENTROID: f64 = (10.0 - 3.0 * PI) / (12.0 - 3.0 * PI);

fn spandrel(radius: f64) -> f64 {
    (1.0 - PI / 4.0) * radius * radius
}

fn moved(solid: Solid, offset: (f64, f64, f64)) -> Solid {
    let transform =
        RigidTransform::translation(Vector3::new(offset.0, offset.1, offset.2)).unwrap();
    solid.transformed(&transform).unwrap()
}

fn swept(plane: Plane, curves: &[ProfileCurve], height: f64) -> Solid {
    let regions = Profile::new(curves)
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap();
    extrude(&plane, &regions, LinearExtent::one_side(height).unwrap(), 1).unwrap()
}

fn polygon(points: &[(f64, f64)]) -> Vec<ProfileCurve> {
    (0..points.len())
        .map(|index| {
            line(
                index as u64 + 1,
                points[index],
                points[(index + 1) % points.len()],
            )
        })
        .collect()
}

fn volume(solid: &Solid) -> f64 {
    solid
        .tessellate(&SamplingTolerance::new(1e-3, 0.05).unwrap())
        .unwrap()
        .mass_properties()
        .volume
}

fn check(name: &str, solid: &Solid, expected: f64) {
    assert_eq!(solid.validate(), Ok(()), "{name}");
    assert_watertight(name, &solid.tessellate(&solid.default_tolerance()).unwrap());
    let found = volume(solid);
    assert!(
        (found - expected).abs() <= 2e-3 * expected.abs().max(1.0),
        "{name}: volume {found} instead of {expected}"
    );
}

fn edge_through(solid: &Solid, point: (f64, f64, f64)) -> EdgeId {
    let point = Point3::new(point.0, point.1, point.2);
    solid
        .edges()
        .find(|(_, edge)| {
            let parameter = edge.curve().closest_parameter(point, edge.interval());
            edge.curve().point(parameter).distance(point) < 1e-6
        })
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no edge passes through {point}"))
}

fn run(solid: &Solid, edges: &[EdgeId], shape: BlendShape) -> Solid {
    match blend(solid, edges, shape, 50) {
        Ok(result) => result,
        Err(error) => panic!("blend failed: {error}"),
    }
}

fn fillet(radius: f64) -> BlendShape {
    BlendShape::Fillet { radius }
}

fn blend_faces(solid: &Solid) -> Vec<FaceId> {
    solid
        .faces()
        .filter(|(_, face)| {
            matches!(
                face.origin(),
                Some(FaceOrigin::Fillet { .. } | FaceOrigin::Chamfer { .. })
            )
        })
        .map(|(id, _)| id)
        .collect()
}

#[test]
fn a_box_edge_is_rounded_and_the_face_is_named_after_the_edge() {
    let solid = cuboid(Vector3::splat(10.0));
    let edge = edge_through(&solid, (5.0, 0.0, 10.0));
    let name = solid.edge(edge).unwrap().name();
    let result = run(&solid, &[edge], fillet(2.0));
    check("fillet", &result, 1000.0 - 10.0 * spandrel(2.0));
    assert_eq!(result.faces().count(), 7);
    let faces = blend_faces(&result);
    assert_eq!(faces.len(), 1);
    let face = result.face(faces[0]).unwrap();
    assert_eq!(face.name(), FaceName::blend(50, name));
    assert_eq!(face.origin(), Some(FaceOrigin::Fillet { feature: 50 }));
    assert!(matches!(face.surface(), Surface::Cylinder(_)));
}

#[test]
fn a_box_edge_is_chamfered_by_a_flat_face() {
    let solid = cuboid(Vector3::splat(10.0));
    let edge = edge_through(&solid, (10.0, 5.0, 10.0));
    let result = run(&solid, &[edge], BlendShape::Chamfer { distance: 3.0 });
    check("chamfer", &result, 1000.0 - 10.0 * 4.5);
    let faces = blend_faces(&result);
    assert_eq!(faces.len(), 1);
    assert!(matches!(
        result.face(faces[0]).unwrap().surface(),
        Surface::Plane(_)
    ));
}

#[test]
fn edges_meeting_at_corners_are_mitred_or_rounded() {
    let solid = cuboid(Vector3::splat(10.0));
    let two = [
        edge_through(&solid, (5.0, 0.0, 10.0)),
        edge_through(&solid, (0.0, 5.0, 10.0)),
    ];
    let result = run(&solid, &two, fillet(2.0));
    assert_eq!(result.validate(), Ok(()));
    assert_eq!(result.faces().count(), 8);
    let every: Vec<EdgeId> = solid.edges().map(|(id, _)| id).collect();
    let result = run(&solid, &every, BlendShape::Chamfer { distance: 1.0 });
    assert_eq!(result.validate(), Ok(()));
    assert_watertight(
        "all",
        &result.tessellate(&result.default_tolerance()).unwrap(),
    );
    assert_eq!(blend_faces(&result).len(), 12);
    let result = run(&solid, &every, fillet(1.0));
    let inner: f64 = 8.0;
    let rounded = inner.powi(3) + 6.0 * inner * inner + 3.0 * PI * inner + 4.0 / 3.0 * PI;
    check("rounded box", &result, rounded);
    assert_eq!(blend_faces(&result).len(), 20);
    assert_eq!(result.faces().count(), 26);
}

#[test]
fn circular_edges_are_rounded_around_their_axis() {
    let solid = cylinder(5.0, 10.0);
    let rim = edge_through(&solid, (5.0, 0.0, 10.0));
    let result = run(&solid, &[rim], fillet(1.0));
    let ring = 2.0 * PI * (5.0 - SPANDREL_CENTROID) * spandrel(1.0);
    check("rim", &result, 250.0 * PI - ring);
    assert!(matches!(
        result.face(blend_faces(&result)[0]).unwrap().surface(),
        Surface::Torus(_)
    ));
    let chamfered = run(&solid, &[rim], BlendShape::Chamfer { distance: 1.0 });
    let cone = 2.0 * PI * (5.0 - 1.0 / 3.0) * 0.5;
    check("rim chamfer", &chamfered, 250.0 * PI - cone);
    assert!(matches!(
        chamfered
            .face(blend_faces(&chamfered)[0])
            .unwrap()
            .surface(),
        Surface::Cone(_)
    ));

    let plate = cuboid(Vector3::new(10.0, 10.0, 4.0));
    let drill = moved(cylinder(2.0, 6.0), (5.0, 5.0, -1.0));
    let holed = boolean(&plate, &drill, BooleanOperation::Difference).unwrap();
    let hole_rim = edge_through(&holed, (7.0, 5.0, 4.0));
    let result = run(&holed, &[hole_rim], fillet(0.5));
    let ring = 2.0 * PI * (2.0 + 0.5 * SPANDREL_CENTROID) * spandrel(0.5);
    check("hole rim", &result, 400.0 - 16.0 * PI - ring);
}

#[test]
fn inside_corners_are_filled() {
    let l_shape = swept(
        Plane::XY,
        &polygon(&[
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 4.0),
            (4.0, 4.0),
            (4.0, 10.0),
            (0.0, 10.0),
        ]),
        5.0,
    );
    let inner = edge_through(&l_shape, (4.0, 4.0, 2.5));
    let result = run(&l_shape, &[inner], fillet(1.0));
    check("inner corner", &result, 5.0 * 64.0 + 5.0 * spandrel(1.0));
    assert_eq!(result.faces().count(), 9);

    let plate = cuboid(Vector3::new(10.0, 10.0, 2.0));
    let boss = moved(cylinder(2.0, 3.0), (5.0, 5.0, 1.0));
    let joined = boolean(&plate, &boss, BooleanOperation::Union).unwrap();
    let foot = edge_through(&joined, (7.0, 5.0, 2.0));
    let result = run(&joined, &[foot], fillet(0.5));
    let ring = 2.0 * PI * (2.0 + 0.5 * SPANDREL_CENTROID) * spandrel(0.5);
    check("boss foot", &result, 200.0 + 8.0 * PI + ring);
}

#[test]
fn smooth_chains_are_followed_from_one_edge() {
    let stadium = [
        line(1, (0.0, 0.0), (10.0, 0.0)),
        arc(2, (10.0, 2.0), (10.0, 0.0), (10.0, 4.0)),
        line(3, (10.0, 4.0), (0.0, 4.0)),
        arc(4, (0.0, 2.0), (0.0, 4.0), (0.0, 0.0)),
    ];
    let slot = swept(Plane::XY, &stadium, 3.0);
    let edge = edge_through(&slot, (5.0, 0.0, 3.0));
    let result = run(&slot, &[edge], fillet(0.5));
    let area = spandrel(0.5);
    let removed = 20.0 * area + 2.0 * PI * (2.0 - 0.5 * SPANDREL_CENTROID) * area;
    check("slot", &result, 3.0 * (40.0 + 4.0 * PI) - removed);
    assert_eq!(blend_faces(&result).len(), 4);
}

#[test]
fn slanted_ends_follow_the_face_they_meet() {
    let wedge = swept(
        Plane::XY,
        &polygon(&[(0.0, 0.0), (10.0, 0.0), (8.0, 6.0), (0.0, 6.0)]),
        4.0,
    );
    let edge = edge_through(&wedge, (5.0, 0.0, 4.0));
    let result = run(&wedge, &[edge], fillet(1.0));
    let area = spandrel(1.0);
    check(
        "open end",
        &result,
        4.0 * 54.0 - area * (10.0 - SPANDREL_CENTROID / 3.0),
    );

    let step = swept(
        Plane::XZ,
        &polygon(&[
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 3.0),
            (4.0, 3.0),
            (3.0, 6.0),
            (0.0, 6.0),
        ]),
        5.0,
    );
    let edge = edge_through(&step, (7.0, 0.0, 3.0));
    let result = run(&step, &[edge], fillet(1.0));
    let full = 5.0 * 40.5;
    check(
        "wall end",
        &result,
        full - area * (6.0 - SPANDREL_CENTROID / 3.0),
    );
}

#[test]
fn impossible_blends_are_refused_with_the_edge() {
    let solid = cuboid(Vector3::new(10.0, 10.0, 2.0));
    let edge = edge_through(&solid, (5.0, 0.0, 2.0));
    assert_eq!(
        blend(&solid, &[edge], fillet(3.0), 1),
        Err(BlendError::TooLarge(edge))
    );
    assert_eq!(
        blend(&solid, &[edge], fillet(0.0), 1),
        Err(BlendError::InvalidSize)
    );
    assert_eq!(blend(&solid, &[], fillet(1.0), 1), Err(BlendError::NoEdges));
    let stadium = [
        line(1, (0.0, 0.0), (10.0, 0.0)),
        arc(2, (10.0, 2.0), (10.0, 0.0), (10.0, 4.0)),
        line(3, (10.0, 4.0), (0.0, 4.0)),
        arc(4, (0.0, 2.0), (0.0, 4.0), (0.0, 0.0)),
    ];
    let slot = swept(Plane::XY, &stadium, 3.0);
    let seam = edge_through(&slot, (10.0, 0.0, 1.5));
    assert_eq!(
        blend(&slot, &[seam], fillet(0.5), 1),
        Err(BlendError::Smooth(seam))
    );
}

#[test]
fn every_edge_of_assorted_prisms() {
    let hexagon: Vec<(f64, f64)> = (0..6)
        .map(|index| {
            let angle = PI / 3.0 * index as f64;
            (5.0 * angle.cos(), 5.0 * angle.sin())
        })
        .collect();
    let stadium = vec![
        line(1, (0.0, 0.0), (10.0, 0.0)),
        arc(2, (10.0, 2.0), (10.0, 0.0), (10.0, 4.0)),
        line(3, (10.0, 4.0), (0.0, 4.0)),
        arc(4, (0.0, 2.0), (0.0, 4.0), (0.0, 0.0)),
    ];
    let shapes = [
        (
            "wedge",
            polygon(&[(0.0, 0.0), (10.0, 0.0), (8.0, 6.0), (0.0, 6.0)]),
        ),
        ("hexagon", polygon(&hexagon)),
        (
            "l shape",
            polygon(&[
                (0.0, 0.0),
                (10.0, 0.0),
                (10.0, 4.0),
                (4.0, 4.0),
                (4.0, 10.0),
                (0.0, 10.0),
            ]),
        ),
        ("stadium", stadium),
    ];
    for (name, curves) in shapes {
        let solid = swept(Plane::XY, &curves, 3.0);
        let every: Vec<EdgeId> = solid.edges().map(|(id, _)| id).collect();
        let before = volume(&solid);
        for shape in [fillet(0.5), BlendShape::Chamfer { distance: 0.5 }] {
            let result = match blend(&solid, &every, shape, 50) {
                Ok(result) => result,
                Err(error) => panic!("{name} {shape:?}: {error}"),
            };
            assert_eq!(result.validate(), Ok(()), "{name} {shape:?}");
            assert_watertight(
                name,
                &result.tessellate(&result.default_tolerance()).unwrap(),
            );
            let after = volume(&result);
            assert!(
                after < before && after > 0.8 * before,
                "{name} {shape:?}: {after}"
            );
            for edge in blend_chain(&solid, &every) {
                let edge = solid.edge(edge).unwrap();
                let expected = crate::naming::FaceName::blend(50, edge.name());
                assert!(
                    result.faces().any(|(_, face)| face.name() == expected),
                    "{name} {shape:?}: an edge was left without its blend face"
                );
            }
        }
    }
}

#[test]
fn turned_parts_are_rounded_on_every_edge() {
    let axis =
        crate::build::Axis2::new(caditor_geometry::Point2::ZERO, caditor_geometry::Vector2::Y)
            .unwrap();
    let turned = |curves: &[ProfileCurve]| {
        let regions = Profile::new(curves)
            .unwrap()
            .select(&Selection::EvenDepth)
            .unwrap();
        crate::build::revolve(
            &Plane::XZ,
            &regions,
            axis,
            crate::build::AngularExtent::full(),
            1,
        )
        .unwrap()
    };
    let shaft = turned(&polygon(&[
        (0.0, 0.0),
        (5.0, 0.0),
        (5.0, 4.0),
        (3.0, 4.0),
        (3.0, 10.0),
        (0.0, 10.0),
    ]));
    let dome = turned(&[
        line(1, (0.0, 0.0), (5.0, 0.0)),
        arc(2, (0.0, 0.0), (5.0, 0.0), (0.0, 5.0)),
        line(3, (0.0, 5.0), (0.0, 0.0)),
    ]);
    for (name, solid) in [("shaft", shaft), ("dome", dome)] {
        let every: Vec<EdgeId> = solid.edges().map(|(id, _)| id).collect();
        let before = volume(&solid);
        for shape in [fillet(0.5), BlendShape::Chamfer { distance: 0.5 }] {
            let result = match blend(&solid, &every, shape, 50) {
                Ok(result) => result,
                Err(error) => panic!("{name} {shape:?}: {error}"),
            };
            assert_eq!(result.validate(), Ok(()), "{name} {shape:?}");
            assert_watertight(
                name,
                &result.tessellate(&result.default_tolerance()).unwrap(),
            );
            let after = volume(&result);
            assert!(
                (after - before).abs() < 0.1 * before,
                "{name} {shape:?}: {after}"
            );
            assert!(!blend_faces(&result).is_empty());
        }
    }
}

#[test]
fn a_face_that_narrows_away_from_the_middle_of_the_edge_is_too_small() {
    let notched = swept(
        Plane::XY,
        &polygon(&[
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 4.0),
            (3.0, 4.0),
            (3.0, 0.2),
            (2.0, 0.2),
            (2.0, 4.0),
            (0.0, 4.0),
        ]),
        3.0,
    );
    let (front, _) = notched
        .edges()
        .find(|(_, edge)| {
            let middle = edge.curve().point(edge.interval().middle());
            (middle - caditor_geometry::Point3::new(5.0, 0.0, 3.0)).length() < 1e-9
        })
        .unwrap();
    assert!(matches!(
        blend(&notched, &[front], fillet(0.5), 60),
        Err(BlendError::TooLarge(edge)) if edge == front
    ));
    assert!(blend(&notched, &[front], fillet(0.1), 60).is_ok());
}

#[test]
fn every_refusal_names_what_cannot_be_blended() {
    let block = cuboid(Vector3::new(10.0, 10.0, 2.0));
    let gone = EdgeId::from_index(999).unwrap();
    assert_eq!(
        blend(&block, &[gone], fillet(1.0), 1),
        Err(BlendError::MissingEdge(gone))
    );

    let bulged = crate::fixtures::spline_topped_block(10.0, 4.0, 3.0);
    let rim = edge_through(&bulged, (5.0, 0.0, 4.0));
    assert_eq!(
        blend(&bulged, &[rim], fillet(0.5), 1),
        Err(BlendError::Unsupported(rim))
    );

    let sheared = swept(
        Plane::XY,
        &polygon(&[(0.0, 0.0), (10.0, 0.0), (10.0, 5.0), (-60.0, 5.0)]),
        2.0,
    );
    let long = edge_through(&sheared, (5.0, 0.0, 2.0));
    assert!(
        matches!(
            blend(&sheared, &[long], fillet(0.2), 1),
            Err(BlendError::UnsupportedEnd { edge, vertex: Some(_) }) if edge == long
        ),
        "a blend along an edge that meets its end face at a glancing angle is refused"
    );
}

#[test]
fn an_edge_the_fill_swallowed_is_named_as_lost() {
    let block = swept(
        Plane::XY,
        &polygon(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]),
        10.0,
    );
    let edge = edge_through(&block, (5.0, 0.0, 10.0));
    let reference = EdgeReference::capture(&block, edge).unwrap();
    let filled = swept(
        Plane::XY,
        &polygon(&[(0.0, 0.0), (10.0, 0.0), (0.0, 10.0)]),
        10.0,
    );
    assert_eq!(
        find_again(&filled, &[(edge, reference)]),
        Err(BlendError::Lost(edge))
    );
    assert_eq!(
        find_again(
            &block,
            &[(edge, EdgeReference::capture(&block, edge).unwrap())]
        )
        .unwrap()
        .get(&edge),
        Some(&edge)
    );
}

#[test]
fn an_error_on_an_edge_the_fill_made_is_reported_after_the_fill() {
    let chosen = EdgeId::from_index(3).unwrap();
    let made = EdgeId::from_index(40).unwrap();
    let original = |edge: EdgeId| if edge == made { None } else { Some(chosen) };
    assert_eq!(
        BlendError::TooLarge(made).remapped(original),
        BlendError::AfterFill(Box::new(BlendError::TooLarge(made)))
    );
    assert_eq!(
        BlendError::Smooth(EdgeId::from_index(7).unwrap()).remapped(original),
        BlendError::Smooth(chosen)
    );
    assert_eq!(
        BlendError::InvalidSize.remapped(original),
        BlendError::InvalidSize
    );
}

#[test]
fn every_rim_of_a_perforated_plate_is_rounded_at_once() {
    let mut curves = polygon(&[(0.0, 0.0), (44.0, 0.0), (44.0, 32.0), (0.0, 32.0)]);
    for row in 0..3 {
        for column in 0..2 {
            curves.push(crate::test_support::circle(
                100 + row * 2 + column,
                (10.0 + 12.0 * row as f64, 10.0 + 12.0 * column as f64),
                3.0,
            ));
        }
    }
    let plate = swept(Plane::XY, &curves, 5.0);
    let rims: Vec<EdgeId> = plate
        .edges()
        .filter(|(_, edge)| {
            matches!(edge.curve(), Curve::Circle(_))
                && edge.curve().point(edge.interval().start()).z > 4.0
        })
        .map(|(id, _)| id)
        .collect();
    assert_eq!(rims.len(), 6);
    let result = run(&plate, &rims, fillet(1.0));
    let hole = PI * 9.0 * 5.0;
    let rounded = 2.0 * PI * (3.0 + SPANDREL_CENTROID) * spandrel(1.0);
    check(
        "perforated",
        &result,
        44.0 * 32.0 * 5.0 - 6.0 * (hole + rounded),
    );
    assert_eq!(blend_faces(&result).len(), 6);
}

#[test]
fn a_blend_around_an_almost_full_circle_with_slanted_ends_names_the_edge() {
    let slot = moved(cuboid(Vector3::new(10.0, 0.4, 7.0)), (5.0, -0.2, -1.0));
    let solid = boolean(&cylinder(10.0, 5.0), &slot, BooleanOperation::Difference).unwrap();
    let edge = edge_through(&solid, (-10.0, 0.0, 5.0));
    assert_eq!(
        blend(&solid, &[edge], fillet(1.0), 50),
        Err(BlendError::WrapsAround(edge))
    );
}

#[test]
fn a_hole_between_the_sampled_points_of_a_foot_refuses_the_blend() {
    let block = cuboid(Vector3::splat(10.0));
    let near = moved(cylinder(0.3, 3.0), (1.5, 2.0, 8.0));
    let pierced = boolean(&block, &near, BooleanOperation::Difference).unwrap();
    let edge = edge_through(&pierced, (5.0, 0.0, 10.0));
    assert_eq!(
        blend(&pierced, &[edge], fillet(2.0), 50),
        Err(BlendError::TooLarge(edge))
    );

    let far = moved(cylinder(0.3, 3.0), (1.5, 5.0, 8.0));
    let clear = boolean(&block, &far, BooleanOperation::Difference).unwrap();
    let edge = edge_through(&clear, (5.0, 0.0, 10.0));
    let hole = PI * 0.3 * 0.3 * 2.0;
    check(
        "clear of the hole",
        &run(&clear, &[edge], fillet(2.0)),
        1000.0 - hole - 10.0 * spandrel(2.0),
    );
}

#[test]
fn edges_of_one_lump_are_blended_without_disturbing_the_others() {
    let mut curves = polygon(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)]);
    curves.extend(
        polygon(&[(6.0, 0.0), (10.0, 0.0), (10.0, 4.0), (6.0, 4.0)])
            .into_iter()
            .map(|mut curve| {
                curve.entity += 10;
                curve
            }),
    );
    let blocks = swept(Plane::XY, &curves, 4.0);
    let right = edge_through(&blocks, (10.0, 4.0, 2.0));
    let left = edge_through(&blocks, (0.0, 0.0, 2.0));

    let one = run(&blocks, &[right], fillet(1.0));
    check("one lump rounded", &one, 128.0 - 4.0 * spandrel(1.0));
    assert_eq!(one.shells().count(), 2);

    let both = run(&blocks, &[right, left], fillet(1.0));
    check("both lumps rounded", &both, 128.0 - 8.0 * spandrel(1.0));
    assert_eq!(both.shells().count(), 2);
}

#[test]
fn edges_of_a_body_and_of_its_void_are_blended() {
    let hollow = hollow_cuboid(10.0, 2.0);
    let material = 1000.0 - 8.0;
    let outer = edge_through(&hollow, (10.0, 10.0, 5.0));
    let inner = edge_through(&hollow, (6.0, 6.0, 5.0));

    let rounded = run(&hollow, &[outer], fillet(1.0));
    check("outer edge", &rounded, material - 10.0 * spandrel(1.0));
    assert_eq!(rounded.shells().count(), 2);

    let filled = run(&hollow, &[inner], fillet(0.5));
    check("void edge", &filled, material + 2.0 * spandrel(0.5));
    assert_eq!(filled.shells().count(), 2);

    let bevelled = run(&hollow, &[inner], BlendShape::Chamfer { distance: 0.5 });
    check("void edge chamfer", &bevelled, material + 2.0 * 0.125);
}

#[test]
fn a_blend_cancelled_anywhere_stops_with_cancelled() {
    let block = cuboid(Vector3::new(10.0, 8.0, 6.0));
    let edges = [
        edge_through(&block, (10.0, 8.0, 3.0)),
        edge_through(&block, (5.0, 8.0, 6.0)),
    ];

    assert_cancelled_anywhere(
        "rounded block",
        || blend(&block, &edges, fillet(1.0), 50),
        |error| matches!(error, BlendError::Cancelled(_)),
    );
}
