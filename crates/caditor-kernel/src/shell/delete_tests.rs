use std::{collections::BTreeSet, f64::consts::PI};

use caditor_geometry::{Plane, Point3, Vector3};

use super::{
    DeleteError, delete_faces,
    tests::{check, face_facing, polygon, swept},
};
use crate::{
    blend::{BlendShape, blend},
    boolean::{BooleanOperation, boolean},
    build::{LinearExtent, extrude},
    fixtures::{cuboid, cylinder, holed_block},
    naming::FaceName,
    profile::{Profile, Selection},
    surface::Surface,
    test_support::{assert_cancelled_anywhere, circle},
    topology::{EdgeId, FaceId, Solid},
};

fn names(solid: &Solid) -> BTreeSet<FaceName> {
    solid.faces().map(|(_, face)| face.name()).collect()
}

fn without(solid: &Solid, faces: &[FaceId]) -> Solid {
    match delete_faces(solid, faces) {
        Ok(result) => result,
        Err(error) => panic!("delete failed: {error}"),
    }
}

fn edge_through(solid: &Solid, middle: Point3) -> EdgeId {
    solid
        .edges()
        .find(|(_, edge)| (edge.curve().point(edge.interval().middle()) - middle).length() < 1e-9)
        .map(|(id, _)| id)
        .unwrap()
}

fn faces_of_kind(solid: &Solid, kind: fn(&Surface) -> bool) -> Vec<FaceId> {
    solid
        .faces()
        .filter(|(_, face)| kind(face.surface()))
        .map(|(id, _)| id)
        .collect()
}

fn round(surface: &Surface) -> bool {
    !matches!(surface, Surface::Plane(_))
}

fn boss_on(base: &Solid, center: (f64, f64), radius: f64, from: f64, height: f64) -> Solid {
    let plane = Plane::from_frame(Point3::new(0.0, 0.0, from), Vector3::Z, Vector3::X).unwrap();
    let boss = extrude(
        &plane,
        &Profile::new(&[circle(1, center, radius)])
            .unwrap()
            .select(&Selection::EvenDepth)
            .unwrap(),
        LinearExtent::one_side(height).unwrap(),
        2,
    )
    .unwrap();
    boolean(base, &boss, BooleanOperation::Union).unwrap()
}

#[test]
fn deleting_a_fillet_restores_the_sharp_edge_of_the_box() {
    let block = cuboid(Vector3::splat(10.0));
    let rim = edge_through(&block, Point3::new(5.0, 0.0, 10.0));
    let rounded = blend(&block, &[rim], BlendShape::Fillet { radius: 2.0 }, 5).unwrap();
    let [fillet] = faces_of_kind(&rounded, round)[..] else {
        panic!("one fillet face");
    };

    let sharp = without(&rounded, &[fillet]);
    check("sharp", &sharp, 1000.0);
    assert_eq!(sharp.faces().count(), 6);
    assert_eq!(sharp.edges().count(), 12);
    assert!(names(&sharp).is_subset(&names(&rounded)));
}

#[test]
fn deleting_a_chamfer_restores_the_sharp_edge_of_the_box() {
    let block = cuboid(Vector3::splat(10.0));
    let rim = edge_through(&block, Point3::new(5.0, 0.0, 10.0));
    let chamfered = blend(&block, &[rim], BlendShape::Chamfer { distance: 2.0 }, 5).unwrap();
    let bevel = chamfered
        .faces()
        .find(|(_, face)| match face.surface() {
            Surface::Plane(plane) => {
                let normal = plane.frame().normal();
                normal.x.abs() < 1e-9 && normal.y.abs() > 0.1 && normal.z.abs() > 0.1
            }
            _ => false,
        })
        .map(|(id, _)| id)
        .unwrap();

    let sharp = without(&chamfered, &[bevel]);
    check("sharp", &sharp, 1000.0);
    assert_eq!(sharp.faces().count(), 6);
}

#[test]
fn deleting_an_inside_fillet_fills_the_corner_back_out() {
    let step = swept(
        &polygon(&[
            (0.0, 0.0),
            (20.0, 0.0),
            (20.0, 5.0),
            (10.0, 5.0),
            (10.0, 15.0),
            (0.0, 15.0),
        ]),
        8.0,
    );
    let corner = edge_through(&step, Point3::new(10.0, 5.0, 4.0));
    let rounded = blend(&step, &[corner], BlendShape::Fillet { radius: 2.0 }, 5).unwrap();
    let [fillet] = faces_of_kind(&rounded, round)[..] else {
        panic!("one fillet face");
    };

    check("sharp", &without(&rounded, &[fillet]), 200.0 * 8.0);
}

#[test]
fn deleting_the_rim_fillet_of_a_drum_leaves_a_plain_cylinder() {
    let drum = cylinder(5.0, 10.0);
    let rim = drum
        .edges()
        .find(|(_, edge)| edge.curve().point(0.0).z > 5.0)
        .map(|(id, _)| id)
        .unwrap();
    let rounded = blend(&drum, &[rim], BlendShape::Fillet { radius: 1.0 }, 5).unwrap();
    let torus = rounded
        .faces()
        .find(|(_, face)| matches!(face.surface(), Surface::Torus(_)))
        .map(|(id, _)| id)
        .unwrap();

    let plain = without(&rounded, &[torus]);
    check("plain", &plain, 250.0 * PI);
    assert_eq!(plain.faces().count(), 3);
}

#[test]
fn deleting_the_wall_of_a_through_hole_fills_it() {
    let plate = holed_block(20.0, 6.0, 3.0);
    let bore = faces_of_kind(&plate, round);

    let filled = without(&plate, &bore);
    check("filled", &filled, 20.0 * 20.0 * 6.0);
    assert_eq!(filled.faces().count(), 6);
    assert!(filled.faces().all(|(_, face)| face.loops().len() == 1));
}

#[test]
fn deleting_a_boss_closes_the_face_it_stood_on() {
    let base = cuboid(Vector3::new(20.0, 20.0, 5.0));
    let body = boss_on(&base, (10.0, 10.0), 3.0, 5.0, 4.0);
    let side = faces_of_kind(&body, round);
    let top = face_facing(&body, Vector3::Z, Point3::new(10.0, 10.0, 9.0));
    let mut boss = side.clone();
    boss.push(top);

    let flat = without(&body, &boss);
    check("flat", &flat, 2000.0);
    assert_eq!(flat.faces().count(), 6);
    assert!(matches!(
        delete_faces(&body, &side),
        Err(DeleteError::CannotClose(_))
    ));
}

#[test]
fn deleting_the_chamfer_of_a_hole_leaves_a_plain_hole() {
    let plate = holed_block(20.0, 6.0, 3.0);
    let rim = plate
        .edges()
        .find(|(_, edge)| {
            matches!(edge.curve(), crate::curve::Curve::Circle(_))
                && edge.curve().point(0.0).z > 5.0
        })
        .map(|(id, _)| id)
        .unwrap();
    let chamfered = blend(&plate, &[rim], BlendShape::Chamfer { distance: 1.0 }, 5).unwrap();
    let cone = chamfered
        .faces()
        .find(|(_, face)| matches!(face.surface(), Surface::Cone(_)))
        .map(|(id, _)| id)
        .unwrap();

    let plain = without(&chamfered, &[cone]);
    check("plain", &plain, 20.0 * 20.0 * 6.0 - 9.0 * PI * 6.0);
    assert!(
        plain
            .faces()
            .all(|(_, face)| !matches!(face.surface(), Surface::Cone(_)))
    );
}

#[test]
fn a_face_its_neighbours_cannot_close_over_is_refused() {
    let block = cuboid(Vector3::splat(10.0));
    let top = face_facing(&block, Vector3::Z, Point3::new(5.0, 5.0, 10.0));
    let every: Vec<FaceId> = block.faces().map(|(id, _)| id).collect();

    assert_eq!(
        delete_faces(&block, &[top]),
        Err(DeleteError::CannotClose(top))
    );
    assert_eq!(delete_faces(&block, &[]), Err(DeleteError::NothingToDelete));
    assert_eq!(delete_faces(&block, &every), Err(DeleteError::Everything));
}

#[test]
fn a_missing_face_is_named() {
    let block = cuboid(Vector3::splat(10.0));
    let other = holed_block(20.0, 6.0, 3.0);
    let stray = other.faces().map(|(id, _)| id).max().unwrap();

    assert_eq!(
        delete_faces(&block, &[stray]),
        Err(DeleteError::MissingFace(stray))
    );
}

#[test]
fn fillets_on_separate_edges_go_together() {
    let block = cuboid(Vector3::splat(10.0));
    let front = edge_through(&block, Point3::new(5.0, 0.0, 10.0));
    let back = edge_through(&block, Point3::new(5.0, 10.0, 0.0));
    let rounded = blend(
        &block,
        &[front, back],
        BlendShape::Fillet { radius: 2.0 },
        5,
    )
    .unwrap();
    let fillets = faces_of_kind(&rounded, round);

    assert_eq!(fillets.len(), 2);
    check("sharp", &without(&rounded, &fillets), 1000.0);
}

fn top_rim(block: &Solid) -> Vec<EdgeId> {
    block
        .edges()
        .filter(|(_, edge)| {
            let start = edge.curve().point(edge.interval().start());
            let end = edge.curve().point(edge.interval().end());
            (start.z - 10.0).abs() < 1e-9 && (end.z - 10.0).abs() < 1e-9
        })
        .map(|(id, _)| id)
        .collect()
}

fn slanted_or_round(solid: &Solid, top: FaceId) -> Vec<FaceId> {
    solid
        .faces()
        .filter(|(id, face)| {
            *id != top
                && match face.surface() {
                    Surface::Plane(plane) => {
                        let normal = plane.frame().normal();
                        normal.z.abs() > 0.1 && normal.z.abs() < 0.99
                    }
                    _ => true,
                }
        })
        .map(|(id, _)| id)
        .collect()
}

#[test]
fn a_whole_rim_of_fillets_or_chamfers_goes_at_once() {
    let block = cuboid(Vector3::splat(10.0));
    let rim = top_rim(&block);
    for shape in [
        BlendShape::Fillet { radius: 2.0 },
        BlendShape::Chamfer { distance: 2.0 },
    ] {
        let blended = blend(&block, &rim, shape, 5).unwrap();
        let top = face_facing(&blended, Vector3::Z, Point3::new(5.0, 5.0, 10.0));
        let around = slanted_or_round(&blended, top);

        assert_eq!(around.len(), 4, "{shape:?}");
        let sharp = without(&blended, &around);
        check(&format!("{shape:?}"), &sharp, 1000.0);
        assert_eq!(sharp.faces().count(), 6, "{shape:?}");
    }
}

#[test]
fn one_fillet_of_a_tangent_chain_is_refused() {
    let block = cuboid(Vector3::splat(10.0));
    let rounded = blend(
        &block,
        &top_rim(&block),
        BlendShape::Fillet { radius: 2.0 },
        5,
    )
    .unwrap();
    let top = face_facing(&rounded, Vector3::Z, Point3::new(5.0, 5.0, 10.0));
    let first = slanted_or_round(&rounded, top)[0];

    assert_eq!(
        delete_faces(&rounded, &[first]),
        Err(DeleteError::CannotClose(first))
    );
}

#[test]
fn a_pocket_and_the_fillets_along_its_floor_are_deleted() {
    let base = cuboid(Vector3::new(20.0, 20.0, 10.0));
    let floor = Plane::from_frame(Point3::new(0.0, 0.0, 5.0), Vector3::Z, Vector3::X).unwrap();
    let outline = polygon(&[(5.0, 5.0), (15.0, 5.0), (15.0, 15.0), (5.0, 15.0)]);
    let tool = extrude(
        &floor,
        &Profile::new(&outline)
            .unwrap()
            .select(&Selection::EvenDepth)
            .unwrap(),
        LinearExtent::one_side(10.0).unwrap(),
        3,
    )
    .unwrap();
    let pocketed = boolean(&base, &tool, BooleanOperation::Difference).unwrap();
    let lid = face_facing(&pocketed, Vector3::Z, Point3::new(1.0, 1.0, 10.0));
    let inside: Vec<FaceId> = pocketed
        .faces()
        .filter(|(id, face)| {
            *id != lid
                && face
                    .origin()
                    .is_some_and(|origin| origin.features().contains(&3))
        })
        .map(|(id, _)| id)
        .collect();
    let floor_edges: Vec<EdgeId> = pocketed
        .edges()
        .filter(|(_, edge)| {
            let start = edge.curve().point(edge.interval().start());
            let end = edge.curve().point(edge.interval().end());
            (start.z - 5.0).abs() < 1e-9 && (end.z - 5.0).abs() < 1e-9
        })
        .map(|(id, _)| id)
        .collect();
    let filleted = blend(
        &pocketed,
        &floor_edges,
        BlendShape::Fillet { radius: 1.0 },
        6,
    )
    .unwrap();
    let fillets = faces_of_kind(&filleted, round);

    assert_eq!(inside.len(), 5);
    let closed = without(&pocketed, &inside);
    check("closed", &closed, 4000.0);
    assert_eq!(closed.faces().count(), 6);
    check("sharp", &without(&filleted, &fillets), 3500.0);
}

#[test]
fn deleting_stops_wherever_it_is_cancelled() {
    let block = cuboid(Vector3::splat(10.0));
    let rim = edge_through(&block, Point3::new(5.0, 0.0, 10.0));
    let rounded = blend(&block, &[rim], BlendShape::Fillet { radius: 2.0 }, 5).unwrap();
    let fillets = faces_of_kind(&rounded, round);
    let plate = holed_block(20.0, 6.0, 3.0);
    let bore = faces_of_kind(&plate, round);

    assert_cancelled_anywhere(
        "fillet",
        || delete_faces(&rounded, &fillets),
        |error| matches!(error, DeleteError::Cancelled(_)),
    );
    assert_cancelled_anywhere(
        "bore",
        || delete_faces(&plate, &bore),
        |error| matches!(error, DeleteError::Cancelled(_)),
    );
}

#[test]
fn every_blend_of_a_rounded_or_bevelled_box_goes_with_its_corners() {
    let block = cuboid(Vector3::splat(10.0));
    let every: Vec<EdgeId> = block.edges().map(|(id, _)| id).collect();
    for shape in [
        BlendShape::Fillet { radius: 1.0 },
        BlendShape::Chamfer { distance: 1.0 },
    ] {
        let blended = blend(&block, &every, shape, 5).unwrap();
        let blends: Vec<FaceId> = blended
            .faces()
            .filter(|(_, face)| match face.surface() {
                Surface::Plane(plane) => {
                    let normal = plane.frame().normal();
                    [normal.x, normal.y, normal.z]
                        .iter()
                        .filter(|component| component.abs() > 1e-6)
                        .count()
                        > 1
                }
                _ => true,
            })
            .map(|(id, _)| id)
            .collect();

        assert_eq!(blends.len(), 20, "{shape:?}");
        let sharp = without(&blended, &blends);
        check(&format!("{shape:?}"), &sharp, 1000.0);
        assert_eq!(sharp.faces().count(), 6, "{shape:?}");
    }
}
