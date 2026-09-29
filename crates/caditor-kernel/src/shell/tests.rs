use std::f64::consts::PI;

use caditor_geometry::{Plane, Point2, Point3, RigidTransform, Vector2, Vector3};

use super::*;
use crate::{
    blend::{BlendShape, blend},
    build::{AngularExtent, Axis2, LinearExtent, extrude, revolve},
    fixtures::{cuboid, cylinder, hollow_cuboid},
    profile::{Profile, ProfileCurve, Selection},
    test_support::{arc, assert_cancelled_anywhere, assert_watertight, line},
    tolerance::SamplingTolerance,
};

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

fn swept(curves: &[ProfileCurve], height: f64) -> Solid {
    let regions = Profile::new(curves)
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap();
    extrude(
        &Plane::XY,
        &regions,
        LinearExtent::one_side(height).unwrap(),
        1,
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

fn check(name: &str, solid: &Solid, expected: f64) {
    assert_eq!(solid.validate(), Ok(()), "{name}");
    assert_watertight(name, &solid.tessellate(&solid.default_tolerance()).unwrap());
    let found = volume(solid);
    assert!(
        (found - expected).abs() <= 2e-3 * expected.abs().max(1.0),
        "{name}: volume {found} instead of {expected}"
    );
}

fn face_facing(solid: &Solid, normal: Vector3, through: Point3) -> FaceId {
    solid
        .faces()
        .find(|(_, face)| match face.surface() {
            Surface::Plane(plane) => {
                let outward = plane.frame().normal() * face.sense().sign();
                outward.dot(normal) > 0.999 && plane.frame().signed_distance(through).abs() < 1e-9
            }
            _ => false,
        })
        .map(|(id, _)| id)
        .expect("the face exists")
}

fn run(solid: &Solid, open: &[FaceId], thickness: f64) -> Solid {
    match shell(solid, open, thickness, 70) {
        Ok(result) => result,
        Err(error) => panic!("shell failed: {error}"),
    }
}

#[test]
fn an_open_box_keeps_walls_of_the_thickness_and_names_its_inner_faces() {
    let solid = cuboid(Vector3::splat(10.0));
    let top = face_facing(&solid, Vector3::Z, Point3::new(5.0, 5.0, 10.0));
    let result = run(&solid, &[top], 1.0);
    check("open box", &result, 1000.0 - 8.0 * 8.0 * 9.0);
    assert_eq!(result.faces().count(), 11);
    assert_eq!(result.shells().count(), 1);
    let bottom = face_facing(&solid, Vector3::NEG_Z, Point3::ZERO);
    let bottom_name = solid.face(bottom).unwrap().name();
    let inner = result
        .faces()
        .find(|(_, face)| face.name() == FaceName::shell(70, bottom_name))
        .expect("the floor of the cavity is named after the bottom");
    assert_eq!(inner.1.origin(), Some(FaceOrigin::Shell { feature: 70 }));

    let closed = run(&solid, &[], 1.0);
    check("hollow box", &closed, 1000.0 - 512.0);
    assert_eq!(closed.shells().count(), 2);
}

#[test]
fn curved_walls_are_offset_around_their_axis() {
    let cup = cylinder(5.0, 10.0);
    let top = face_facing(&cup, Vector3::Z, Point3::new(0.0, 0.0, 10.0));
    check("cup", &run(&cup, &[top], 1.0), 250.0 * PI - 16.0 * PI * 9.0);

    let plate = cuboid(Vector3::new(10.0, 10.0, 4.0));
    let transform = RigidTransform::translation(Vector3::new(5.0, 5.0, -1.0)).unwrap();
    let drill = cylinder(2.0, 6.0).transformed(&transform).unwrap();
    let holed = boolean(&plate, &drill, BooleanOperation::Difference).unwrap();
    let top = face_facing(&holed, Vector3::Z, Point3::new(1.0, 1.0, 4.0));
    let outer = (100.0 - 4.0 * PI) * 4.0;
    let cavity = (81.0 - 6.25 * PI) * 3.5;
    check("holed plate", &run(&holed, &[top], 0.5), outer - cavity);
}

#[test]
fn prisms_with_inside_corners_and_smooth_sides_are_shelled() {
    let l_shape = swept(
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
    let top = face_facing(&l_shape, Vector3::Z, Point3::new(1.0, 1.0, 5.0));
    let cavity_area = 8.0 * 2.0 + 2.0 * 6.0;
    check(
        "l shape",
        &run(&l_shape, &[top], 1.0),
        64.0 * 5.0 - cavity_area * 4.0,
    );

    let slot = swept(
        &[
            line(1, (0.0, 0.0), (10.0, 0.0)),
            arc(2, (10.0, 2.0), (10.0, 0.0), (10.0, 4.0)),
            line(3, (10.0, 4.0), (0.0, 4.0)),
            arc(4, (0.0, 2.0), (0.0, 4.0), (0.0, 0.0)),
        ],
        3.0,
    );
    let top = face_facing(&slot, Vector3::Z, Point3::new(5.0, 2.0, 3.0));
    let outer = 40.0 + 4.0 * PI;
    let cavity = 10.0 * 2.0 + PI;
    check("slot", &run(&slot, &[top], 1.0), 3.0 * outer - 2.0 * cavity);
}

#[test]
fn rounded_and_turned_bodies_are_shelled() {
    let solid = cuboid(Vector3::splat(10.0));
    let every: Vec<EdgeId> = solid.edges().map(|(id, _)| id).collect();
    let rounded = blend(&solid, &every, BlendShape::Fillet { radius: 2.0 }, 5).unwrap();
    let top = face_facing(&rounded, Vector3::Z, Point3::new(5.0, 5.0, 10.0));
    let result = run(&rounded, &[top], 0.5);
    let steiner = |side: f64, radius: f64| {
        let flat = side - 2.0 * radius;
        flat.powi(3)
            + 6.0 * radius * flat * flat
            + 3.0 * PI * radius * radius * flat
            + 4.0 / 3.0 * PI * radius.powi(3)
    };
    let lip = 6.0 * 6.0 * 0.5;
    check(
        "rounded box",
        &result,
        steiner(10.0, 2.0) - steiner(9.0, 1.5) - lip,
    );

    let regions = Profile::new(&polygon(&[
        (0.0, 0.0),
        (5.0, 0.0),
        (5.0, 4.0),
        (3.0, 4.0),
        (3.0, 10.0),
        (0.0, 10.0),
    ]))
    .unwrap()
    .select(&Selection::EvenDepth)
    .unwrap();
    let shaft = revolve(
        &Plane::XZ,
        &regions,
        Axis2::new(Point2::ZERO, Vector2::Y).unwrap(),
        AngularExtent::full(),
        1,
    )
    .unwrap();
    let end = face_facing(&shaft, Vector3::Z, Point3::new(0.0, 0.0, 10.0));
    let outer = PI * (25.0 * 4.0 + 9.0 * 6.0);
    let cavity = PI * (16.0 * 2.0 + 4.0 * 7.0);
    check("shaft", &run(&shaft, &[end], 1.0), outer - cavity);
}

#[test]
fn opened_faces_move_outward_so_the_body_only_needs_room_across_its_walls() {
    let plate = cuboid(Vector3::new(10.0, 10.0, 2.0));
    let top = face_facing(&plate, Vector3::Z, Point3::new(5.0, 5.0, 2.0));
    let tray = run(&plate, &[top], 1.5);
    check("thin tray", &tray, 200.0 - 7.0 * 7.0 * 0.5);
    assert_eq!(tray.faces().count(), 11);
    assert_eq!(shell(&plate, &[top], 2.5, 1), Err(ShellError::TooThick));

    let solid = cuboid(Vector3::splat(10.0));
    let top = face_facing(&solid, Vector3::Z, Point3::new(5.0, 5.0, 10.0));
    let bottom = face_facing(&solid, Vector3::NEG_Z, Point3::ZERO);
    let front = face_facing(&solid, Vector3::NEG_Y, Point3::ZERO);
    check(
        "tube",
        &run(&solid, &[top, bottom], 1.0),
        1000.0 - 8.0 * 8.0 * 10.0,
    );
    check(
        "open corner",
        &run(&solid, &[top, front], 1.0),
        1000.0 - 8.0 * 9.0 * 9.0,
    );
}

fn convex_volume(planes: &[(Vector3, f64)]) -> f64 {
    let mut corners: Vec<Point3> = Vec::new();
    for (i, a) in planes.iter().enumerate() {
        for (j, b) in planes.iter().enumerate().skip(i + 1) {
            for c in planes.iter().skip(j + 1) {
                let determinant = a.0.dot(b.0.cross(c.0));
                if determinant.abs() < 1e-12 {
                    continue;
                }
                let point = (b.0.cross(c.0) * a.1 + c.0.cross(a.0) * b.1 + a.0.cross(b.0) * c.1)
                    / determinant;
                let inside = planes
                    .iter()
                    .all(|(normal, offset)| normal.dot(point) <= offset + 1e-9);
                if inside && corners.iter().all(|other| other.distance(point) > 1e-9) {
                    corners.push(point);
                }
            }
        }
    }
    let centre =
        corners.iter().fold(Point3::ZERO, |sum, point| sum + *point) / corners.len() as f64;
    planes
        .iter()
        .map(|(normal, offset)| {
            let mut on: Vec<Point3> = corners
                .iter()
                .copied()
                .filter(|point| (normal.dot(*point) - offset).abs() < 1e-9)
                .collect();
            if on.len() < 3 {
                return 0.0;
            }
            let middle = on.iter().fold(Point3::ZERO, |sum, point| sum + *point) / on.len() as f64;
            let u = (on[0] - middle).normalize();
            let v = normal.cross(u);
            on.sort_by(|a, b| {
                let angle = |p: &Point3| (*p - middle).dot(v).atan2((*p - middle).dot(u));
                angle(a).total_cmp(&angle(b))
            });
            let area: Vector3 = (0..on.len())
                .map(|index| (on[index] - middle).cross(on[(index + 1) % on.len()] - middle) / 2.0)
                .sum();
            area.length() * (offset - normal.dot(centre)) / 3.0
        })
        .sum()
}

#[test]
fn a_corner_of_four_faces_whose_walls_do_not_meet_becomes_an_edge() {
    let mut fixture = crate::fixtures::Fixture::new();
    let base = [(0.0, 0.0), (0.0, 10.0), (10.0, 10.0), (10.0, 0.0)]
        .map(|(x, y)| fixture.vertex(Point3::new(x, y, 0.0)));
    let apex = fixture.vertex(Point3::new(2.0, 3.0, 6.0));
    fixture.polygon(&base, &[]);
    for index in 0..4 {
        fixture.polygon(&[base[(index + 1) % 4], base[index], apex], &[]);
    }
    let pyramid = fixture.build();
    let floor = face_facing(&pyramid, Vector3::NEG_Z, Point3::ZERO);
    let sides: Vec<(Vector3, f64)> = pyramid
        .faces()
        .filter(|(id, _)| *id != floor)
        .map(|(_, face)| {
            let Surface::Plane(plane) = face.surface() else {
                panic!("the pyramid is flat")
            };
            let normal = plane.frame().normal() * face.sense().sign();
            (normal, normal.dot(plane.frame().origin()))
        })
        .collect();
    let thickness = 0.5;
    let mut cavity: Vec<(Vector3, f64)> = sides
        .iter()
        .map(|(normal, offset)| (*normal, offset - thickness))
        .collect();
    cavity.push((Vector3::NEG_Z, 0.0));
    let mut outer = sides.clone();
    outer.push((Vector3::NEG_Z, 0.0));
    let result = run(&pyramid, &[floor], thickness);
    check(
        "pyramid",
        &result,
        convex_volume(&outer) - convex_volume(&cavity),
    );
    let apex_point = Point3::new(2.0, 3.0, 6.0);
    let ridge = result
        .edges()
        .filter(|(_, edge)| {
            let middle = edge.curve().point(edge.interval().middle());
            middle.distance(apex_point) < 1.0 && middle.distance(apex_point) > 0.1
        })
        .count();
    assert_eq!(ridge, 1, "the cavity's apex is a short ridge");
}

#[test]
fn fillets_tighter_than_the_thickness_disappear_from_the_cavity() {
    let solid = cuboid(Vector3::splat(10.0));
    let every: Vec<EdgeId> = solid.edges().map(|(id, _)| id).collect();
    let rounded = blend(&solid, &every, BlendShape::Fillet { radius: 1.0 }, 5).unwrap();
    let top = face_facing(&rounded, Vector3::Z, Point3::new(5.0, 5.0, 10.0));
    let flat: f64 = 8.0;
    let rounded_volume = flat.powi(3) + 6.0 * flat * flat + 3.0 * PI * flat + 4.0 / 3.0 * PI;
    let result = run(&rounded, &[top], 2.0);
    check("rounded box", &result, rounded_volume - 6.0 * 6.0 * 8.0);
    let cavity_faces = result
        .faces()
        .filter(|(_, face)| face.origin() == Some(FaceOrigin::Shell { feature: 70 }))
        .count();
    assert_eq!(cavity_faces, 5, "the cavity is a plain box open at the top");

    let drum = cylinder(5.0, 10.0);
    let rim = drum
        .edges()
        .find(|(_, edge)| edge.curve().point(0.0).z > 5.0)
        .map(|(id, _)| id)
        .unwrap();
    let rounded_drum = blend(&drum, &[rim], BlendShape::Fillet { radius: 1.0 }, 5).unwrap();
    let bottom = face_facing(&rounded_drum, Vector3::NEG_Z, Point3::ZERO);
    let ring = 2.0 * PI * (25.0 / 6.0 - PI);
    check(
        "rounded drum",
        &run(&rounded_drum, &[bottom], 2.0),
        250.0 * PI - ring - 9.0 * PI * 8.0,
    );
}

#[test]
fn a_saddle_corner_whose_walls_do_not_meet_is_named() {
    let l_shape = swept(
        &polygon(&[
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 4.0),
            (4.0, 4.0),
            (4.0, 10.0),
            (0.0, 10.0),
        ]),
        10.0,
    );
    let gable_regions = Profile::new(&polygon(&[
        (-1.0, -1.0),
        (11.0, -1.0),
        (11.0, 5.0),
        (4.0, 8.0),
        (-1.0, 5.0),
    ]))
    .unwrap()
    .select(&Selection::EvenDepth)
    .unwrap();
    let gable = extrude(
        &Plane::YZ,
        &gable_regions,
        LinearExtent::one_side(12.0).unwrap(),
        2,
    )
    .unwrap()
    .transformed(&RigidTransform::translation(Vector3::new(-1.0, 0.0, 0.0)).unwrap())
    .unwrap();
    let roofed = boolean(&l_shape, &gable, BooleanOperation::Intersection).unwrap();
    let saddle = roofed
        .vertices()
        .find(|(_, vertex)| vertex.point().distance(Point3::new(4.0, 4.0, 8.0)) < 1e-9)
        .map(|(id, _)| id)
        .expect("the ridge meets the inside corner");
    let floor = face_facing(&roofed, Vector3::NEG_Z, Point3::ZERO);
    assert_eq!(
        shell(&roofed, &[floor], 0.5, 1),
        Err(ShellError::Corner(saddle))
    );
}

#[test]
fn an_edge_whose_wall_shrinks_past_nothing_is_named() {
    let ridge = swept(
        &polygon(&[(0.0, 0.0), (10.0, 0.0), (5.25, 4.75), (4.75, 4.75)]),
        10.0,
    );
    let narrow = |edge: EdgeId| {
        let definition = ridge.edge(edge).unwrap();
        let middle = definition.curve().point(definition.interval().middle());
        (middle.x - 5.0).abs() < 1e-9 && (middle.y - 4.75).abs() < 1e-9
    };
    match shell(&ridge, &[], 1.0, 1) {
        Err(ShellError::EdgeCollapses(edge)) => assert!(narrow(edge), "{edge:?}"),
        other => panic!("expected the narrow top to collapse, got {other:?}"),
    }
}

#[test]
fn an_opening_missing_from_the_offset_body_is_named() {
    let solid = cuboid(Vector3::splat(10.0));
    let top = face_facing(&solid, Vector3::Z, Point3::new(5.0, 5.0, 10.0));
    let offsets = Offsets {
        solid: &solid,
        thickness: 1.0,
        outward: BTreeSet::new(),
    };
    let unrelated = cylinder(2.0, 3.0);
    assert!(matches!(
        opening(&unrelated, &offsets, top, 1),
        Err(ShellError::Opening(face)) if face == top
    ));
}

#[test]
fn walls_thicker_than_the_body_are_refused() {
    let solid = cuboid(Vector3::splat(10.0));
    let top = face_facing(&solid, Vector3::Z, Point3::new(5.0, 5.0, 10.0));
    assert!(matches!(
        shell(&solid, &[top], 6.0, 1),
        Err(ShellError::EdgeCollapses(_))
    ));
    assert_eq!(
        shell(&solid, &[top], 0.0, 1),
        Err(ShellError::InvalidThickness)
    );
}

#[test]
fn a_round_face_tighter_than_the_thickness_is_named() {
    let solid = crate::fixtures::cylinder(1.0, 10.0);
    let (top, _) = solid
        .faces()
        .find(|(_, face)| {
            matches!(face.surface(), Surface::Plane(plane)
                if plane.frame().origin().z > 5.0)
        })
        .unwrap();
    let (round, _) = solid
        .faces()
        .find(|(_, face)| matches!(face.surface(), Surface::Cylinder(_)))
        .unwrap();
    match shell(&solid, &[top], 1.5, 70) {
        Err(ShellError::TooCurved(face)) => assert_eq!(face, round),
        other => panic!("expected the round face to be named, got {other:?}"),
    }
}

#[test]
fn every_refusal_names_what_cannot_be_shelled() {
    let block = cuboid(Vector3::new(10.0, 10.0, 4.0));
    let top = face_facing(&block, Vector3::Z, Point3::new(0.0, 0.0, 4.0));
    for thickness in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(
            shell(&block, &[top], thickness, 1),
            Err(ShellError::InvalidThickness)
        );
    }
    let gone = FaceId::from_index(999).unwrap();
    assert_eq!(
        shell(&block, &[gone], 1.0, 1),
        Err(ShellError::MissingFace(gone))
    );

    let drum = cylinder(3.0, 10.0);
    let side = drum
        .faces()
        .find(|(_, face)| matches!(face.surface(), Surface::Cylinder(_)))
        .map(|(id, _)| id)
        .unwrap();
    assert_eq!(
        shell(&drum, &[side], 1.0, 1),
        Err(ShellError::UnsupportedFace(side))
    );

    let bulged = crate::fixtures::spline_topped_block(10.0, 4.0, 3.0);
    let bottom = face_facing(&bulged, Vector3::NEG_Z, Point3::ZERO);
    let spline = bulged
        .faces()
        .find(|(_, face)| matches!(face.surface(), Surface::BSpline(_)))
        .map(|(id, _)| id)
        .unwrap();
    assert_eq!(
        shell(&bulged, &[bottom], 1.0, 1),
        Err(ShellError::UnsupportedFace(spline))
    );

    let regions = Profile::new(&polygon(&[
        (-10.0, -1.0),
        (10.0, -1.0),
        (10.0, 6.0),
        (-10.0, 2.0),
    ]))
    .unwrap()
    .select(&Selection::EvenDepth)
    .unwrap();
    let wedge = extrude(
        &Plane::XZ,
        &regions,
        LinearExtent::one_side(20.0).unwrap(),
        2,
    )
    .unwrap()
    .transformed(&RigidTransform::translation(Vector3::new(0.0, 10.0, 0.0)).unwrap())
    .unwrap();
    let slanted = crate::boolean::boolean(
        &cylinder(3.0, 10.0),
        &wedge,
        crate::boolean::BooleanOperation::Intersection,
    )
    .unwrap();
    let rim = slanted
        .edges()
        .find(|(_, edge)| !matches!(edge.curve(), Curve::Line(_) | Curve::Circle(_)))
        .map(|(id, _)| id)
        .expect("the slanted top meets the side along an ellipse");
    let lid = slanted
        .faces()
        .find(|(_, face)| match face.surface() {
            Surface::Plane(plane) => plane.frame().normal().z.abs() < 0.999,
            _ => false,
        })
        .map(|(id, _)| id)
        .unwrap();
    assert_eq!(
        shell(&slanted, &[lid], 0.5, 1),
        Err(ShellError::UnsupportedEdge(rim))
    );
}

fn two_blocks() -> Solid {
    let mut curves = polygon(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)]);
    curves.extend(
        polygon(&[(6.0, 0.0), (10.0, 0.0), (10.0, 4.0), (6.0, 4.0)])
            .into_iter()
            .map(|mut curve| {
                curve.entity += 10;
                curve
            }),
    );
    swept(&curves, 4.0)
}

fn faces_facing(solid: &Solid, normal: Vector3) -> Vec<FaceId> {
    solid
        .faces()
        .filter(|(_, face)| match face.surface() {
            Surface::Plane(plane) => {
                (plane.frame().normal() * face.sense().sign()).dot(normal) > 0.999
            }
            _ => false,
        })
        .map(|(id, _)| id)
        .collect()
}

#[test]
fn every_lump_of_a_body_is_shelled() {
    let blocks = two_blocks();
    let tops = faces_facing(&blocks, Vector3::Z);
    assert_eq!(tops.len(), 2);

    let closed = run(&blocks, &[], 1.0);
    check("closed blocks", &closed, 2.0 * (64.0 - 8.0));
    assert_eq!(closed.shells().count(), 4);

    let open = run(&blocks, &tops, 1.0);
    check("open blocks", &open, 2.0 * (64.0 - 12.0));
    assert_eq!(open.shells().count(), 2);

    let one_open = run(&blocks, &tops[..1], 1.0);
    check("one open block", &one_open, 128.0 - 12.0 - 8.0);
    assert_eq!(one_open.shells().count(), 3);
}

#[test]
fn a_body_with_a_void_keeps_a_wall_around_it() {
    let hollow = hollow_cuboid(10.0, 2.0);
    let material = 1000.0 - 8.0;
    let around_void = 4.0 * 4.0 * 4.0;

    let closed = run(&hollow, &[], 1.0);
    check("closed hollow", &closed, material - (512.0 - around_void));
    assert_eq!(closed.shells().count(), 4);

    let top = face_facing(&hollow, Vector3::Z, Point3::new(5.0, 5.0, 10.0));
    let open = run(&hollow, &[top], 1.0);
    check(
        "open hollow",
        &open,
        material - (8.0 * 8.0 * 9.0 - around_void),
    );
    assert_eq!(open.shells().count(), 3);
}

#[test]
fn a_shell_cancelled_anywhere_stops_with_cancelled() {
    let cup = cylinder(5.0, 10.0);
    let top = face_facing(&cup, Vector3::Z, Point3::new(0.0, 0.0, 10.0));

    assert_cancelled_anywhere(
        "cup",
        || shell(&cup, &[top], 1.0, 70),
        |error| matches!(error, ShellError::Cancelled(_)),
    );
}
