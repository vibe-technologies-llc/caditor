use std::collections::BTreeSet;

use caditor_geometry::{Plane, Point3, Vector3};

use super::*;
use crate::{
    boolean::{BooleanOperation, boolean},
    build::{LinearExtent, extrude},
    profile::{Profile, ProfileCurve, Selection},
    test_support::{circle, rectangle},
    topology::{EdgeId, FaceId, Solid},
};

fn swept(curves: &[ProfileCurve], plane: &Plane, height: f64, feature: u64) -> Solid {
    let regions = Profile::new(curves)
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap();
    extrude(
        plane,
        &regions,
        LinearExtent::one_side(height).unwrap(),
        feature,
    )
    .unwrap()
}

fn plate_with_hole(hole: (f64, f64)) -> Solid {
    let mut curves = rectangle(1, (0.0, 0.0), (10.0, 8.0));
    curves.push(circle(5, hole, 1.5));
    swept(&curves, &Plane::XY, 2.0, 11)
}

fn plate_with_slot(slot: f64) -> Solid {
    let plate = swept(&rectangle(1, (0.0, 0.0), (10.0, 8.0)), &Plane::XY, 4.0, 1);
    let floor = Plane::from_frame(Point3::new(0.0, 0.0, 2.0), Vector3::Z, Vector3::X).unwrap();
    let cutter = swept(
        &rectangle(11, (slot, -1.0), (slot + 2.0, 9.0)),
        &floor,
        5.0,
        2,
    );
    boolean(&plate, &cutter, BooleanOperation::Difference).unwrap()
}

fn face_from(solid: &Solid, origin: FaceOrigin) -> FaceId {
    let found: Vec<FaceId> = solid
        .faces()
        .filter(|(_, face)| face.origin() == Some(origin))
        .map(|(id, _)| id)
        .collect();
    assert_eq!(found.len(), 1, "{origin:?}");
    found[0]
}

fn name_of(solid: &Solid, origin: FaceOrigin) -> FaceName {
    solid.face(face_from(solid, origin)).unwrap().name()
}

fn top_fragment_beside(solid: &Solid, wall: FaceOrigin) -> FaceId {
    let wall = name_of(solid, wall);
    let top = FaceOrigin::EndCap { feature: 1 };
    let fragments: Vec<FaceId> = solid
        .faces()
        .filter(|(id, face)| {
            face.origin() == Some(top) && reference::neighbour_names(solid, *id).contains(&wall)
        })
        .map(|(id, _)| id)
        .collect();
    assert_eq!(fragments.len(), 1);
    fragments[0]
}

const LEFT_WALL: FaceOrigin = FaceOrigin::Side {
    feature: 1,
    entity: 4,
};
const RIGHT_WALL: FaceOrigin = FaceOrigin::Side {
    feature: 1,
    entity: 2,
};

const LEFT_WALL_OF_HOLE_PLATE: FaceOrigin = FaceOrigin::Side {
    feature: 11,
    entity: 4,
};

#[test]
fn a_face_reference_follows_its_name_when_the_geometry_moves() {
    let before = plate_with_hole((4.0, 4.0));
    let hole = face_from(
        &before,
        FaceOrigin::Side {
            feature: 11,
            entity: 5,
        },
    );
    let reference = FaceReference::capture(&before, hole).unwrap();
    assert_eq!(reference.neighbours().len(), 2);

    let after = plate_with_hole((7.0, 3.0));
    let resolved = reference.resolve(&after).unwrap();
    assert_eq!(
        after.face(resolved).unwrap().origin(),
        Some(FaceOrigin::Side {
            feature: 11,
            entity: 5
        })
    );
}

#[test]
fn a_split_face_is_chosen_by_its_neighbours() {
    let before = plate_with_slot(4.0);
    let top_fragments = before
        .faces()
        .filter(|(_, face)| face.origin() == Some(FaceOrigin::EndCap { feature: 1 }))
        .count();
    assert_eq!(top_fragments, 2);
    let left = top_fragment_beside(&before, LEFT_WALL);
    let right = top_fragment_beside(&before, RIGHT_WALL);
    let left_reference = FaceReference::capture(&before, left).unwrap();
    let right_reference = FaceReference::capture(&before, right).unwrap();
    assert_eq!(left_reference.name(), right_reference.name());

    for slot in [2.0, 4.0, 6.5] {
        let after = plate_with_slot(slot);
        assert_eq!(
            left_reference.resolve(&after),
            Ok(top_fragment_beside(&after, LEFT_WALL)),
            "slot at {slot}"
        );
        assert_eq!(
            right_reference.resolve(&after),
            Ok(top_fragment_beside(&after, RIGHT_WALL)),
            "slot at {slot}"
        );
    }
}

#[test]
fn fragments_that_look_alike_are_ambiguous() {
    let solid = plate_with_slot(4.0);
    let left = top_fragment_beside(&solid, LEFT_WALL);
    let right = top_fragment_beside(&solid, RIGHT_WALL);
    let name = solid.face(left).unwrap().name();
    let reference = FaceReference::new(name, Some(FaceOrigin::EndCap { feature: 1 }), []);
    let Err(ReferenceError::Ambiguous(candidates)) = reference.resolve(&solid) else {
        panic!("expected an ambiguous reference");
    };
    let candidates: BTreeSet<FaceId> = candidates.into_iter().collect();
    assert_eq!(candidates, BTreeSet::from([left, right]));
}

#[test]
fn a_renamed_face_is_found_by_its_origin_and_neighbours() {
    let before = plate_with_hole((4.0, 4.0));
    let top = face_from(&before, FaceOrigin::EndCap { feature: 11 });
    let captured = FaceReference::capture(&before, top).unwrap();
    let renamed = FaceReference::new(
        FaceName::from_digest(42),
        captured.origin(),
        captured.neighbours().iter().copied(),
    );
    assert_eq!(renamed.resolve(&before), Ok(top));

    let strangers = FaceReference::new(
        FaceName::from_digest(42),
        captured.origin(),
        [FaceName::from_digest(7)],
    );
    assert_eq!(strangers.resolve(&before), Err(ReferenceError::Missing));
    let unknown = FaceReference::new(FaceName::from_digest(42), None, []);
    assert_eq!(unknown.resolve(&before), Err(ReferenceError::Missing));
}

fn edge_between(solid: &Solid, first: FaceOrigin, second: FaceOrigin) -> Vec<EdgeId> {
    let first = name_of(solid, first);
    let second = name_of(solid, second);
    solid
        .edges()
        .map(|(id, _)| id)
        .filter(|id| {
            let reference = EdgeReference::capture(solid, *id).unwrap();
            let mut expected = [first, second];
            expected.sort();
            reference.faces() == expected
        })
        .collect()
}

#[test]
fn an_edge_reference_survives_a_resize() {
    let before = plate_with_hole((4.0, 4.0));
    let top = FaceOrigin::EndCap { feature: 11 };
    let [edge] = edge_between(&before, top, LEFT_WALL_OF_HOLE_PLATE)[..] else {
        panic!("expected one edge");
    };
    let reference = EdgeReference::capture(&before, edge).unwrap();

    let after = plate_with_hole((6.0, 2.5));
    let [expected] = edge_between(&after, top, LEFT_WALL_OF_HOLE_PLATE)[..] else {
        panic!("expected one edge");
    };
    assert_eq!(reference.resolve(&after), Ok(expected));
}

#[test]
fn pieces_of_a_split_edge_are_told_apart_by_their_ends() {
    let front = FaceOrigin::Side {
        feature: 1,
        entity: 1,
    };
    let pieces_at = |solid: &Solid| -> Vec<(EdgeId, f64)> {
        let top = name_of_fragments(solid);
        let front_names: BTreeSet<FaceName> = solid
            .faces()
            .filter(|(_, face)| face.origin() == Some(front))
            .map(|(_, face)| face.name())
            .collect();
        solid
            .edges()
            .filter(|(id, _)| {
                let faces = EdgeReference::capture(solid, *id).unwrap().faces();
                faces.contains(&top) && faces.iter().any(|name| front_names.contains(name))
            })
            .map(|(id, edge)| {
                let middle = edge.curve().point(edge.interval().middle());
                (id, middle.x)
            })
            .collect()
    };
    let before = plate_with_slot(4.0);
    let pieces = pieces_at(&before);
    assert_eq!(pieces.len(), 2);
    let references: Vec<(EdgeReference, bool)> = pieces
        .iter()
        .map(|(id, x)| (EdgeReference::capture(&before, *id).unwrap(), *x < 4.0))
        .collect();

    let after = plate_with_slot(6.0);
    let moved = pieces_at(&after);
    for (reference, on_left) in references {
        let resolved = reference.resolve(&after).unwrap();
        let x = moved
            .iter()
            .find_map(|(id, x)| (*id == resolved).then_some(*x))
            .unwrap();
        assert_eq!(x < 6.0, on_left);
    }
}

#[test]
fn edges_between_the_same_faces_sharing_no_end_are_not_the_referenced_edge() {
    let solid = plate_with_slot(4.0);

    let references: Vec<EdgeReference> = solid
        .edges()
        .map(|(id, _)| EdgeReference::capture(&solid, id).unwrap())
        .collect();
    let piece = references
        .iter()
        .find(|reference| {
            references
                .iter()
                .filter(|other| other.faces() == reference.faces())
                .count()
                == 2
        })
        .unwrap();
    let unknown = EdgeName::from_digest(3);
    let strangers = [VertexName::from_digest(1), VertexName::from_digest(2)];
    let lost = EdgeReference::new(unknown, piece.faces(), strangers);
    let renamed = EdgeReference::new(unknown, piece.faces(), piece.ends());

    assert_eq!(lost.resolve(&solid), Err(ReferenceError::Missing));
    assert_eq!(renamed.resolve(&solid), piece.resolve(&solid));
    assert!(renamed.resolve(&solid).is_ok());
}

fn name_of_fragments(solid: &Solid) -> FaceName {
    let names: BTreeSet<FaceName> = solid
        .faces()
        .filter(|(_, face)| face.origin() == Some(FaceOrigin::EndCap { feature: 1 }))
        .map(|(_, face)| face.name())
        .collect();
    assert_eq!(names.len(), 1);
    names.into_iter().next().unwrap()
}

#[test]
fn a_unique_name_is_not_trusted_when_none_of_its_neighbours_remain() {
    let solid = plate_with_hole((4.0, 4.0));
    let hole = face_from(
        &solid,
        FaceOrigin::Side {
            feature: 11,
            entity: 5,
        },
    );
    let captured = FaceReference::capture(&solid, hole).unwrap();
    let strangers = FaceReference::new(
        captured.name(),
        captured.origin(),
        [FaceName::side(
            99,
            &crate::profile::PieceId::new(
                1,
                crate::profile::PieceBound::Start,
                crate::profile::PieceBound::End,
            ),
        )],
    );
    assert_eq!(strangers.resolve(&solid), Err(ReferenceError::Missing));
    assert_eq!(captured.resolve(&solid), Ok(hole));
    let unrecorded = FaceReference::new(captured.name(), captured.origin(), []);
    assert_eq!(unrecorded.resolve(&solid), Ok(hole));
}

fn plain_plate() -> Solid {
    swept(&rectangle(1, (0.0, 0.0), (10.0, 8.0)), &Plane::XY, 2.0, 11)
}

#[test]
fn an_edge_on_a_cap_renamed_by_a_new_hole_is_found_by_its_faces_origins() {
    let before = plain_plate();
    let top = FaceOrigin::EndCap { feature: 11 };
    let [edge] = edge_between(&before, top, LEFT_WALL_OF_HOLE_PLATE)[..] else {
        panic!("expected one edge");
    };
    let reference = EdgeReference::capture(&before, edge).unwrap();

    let after = plate_with_hole((4.0, 4.0));
    let [expected] = edge_between(&after, top, LEFT_WALL_OF_HOLE_PLATE)[..] else {
        panic!("expected one edge");
    };

    assert_ne!(name_of(&before, top), name_of(&after, top));
    assert_eq!(reference.resolve(&after), Ok(expected));
    assert_eq!(
        EdgeReference::new(reference.name(), reference.faces(), reference.ends()).resolve(&after),
        Err(ReferenceError::Missing)
    );
}

#[test]
fn an_edge_is_found_by_its_faces_origins_alone_only_where_both_faces_meet() {
    let before = plain_plate();
    let top = FaceOrigin::EndCap { feature: 11 };
    let [edge] = edge_between(&before, top, LEFT_WALL_OF_HOLE_PLATE)[..] else {
        panic!("expected one edge");
    };
    let captured = EdgeReference::capture(&before, edge).unwrap();
    let unknown = [FaceName::from_digest(5), FaceName::from_digest(6)];
    let strangers = [VertexName::from_digest(1), VertexName::from_digest(2)];
    let top_and_bottom = EdgeReference::new(EdgeName::from_digest(3), unknown, strangers)
        .with_origins([Some(top), Some(FaceOrigin::StartCap { feature: 11 })]);
    let top_and_wall = EdgeReference::new(EdgeName::from_digest(3), unknown, strangers)
        .with_origins(captured.origins());

    assert_eq!(
        top_and_bottom.resolve(&before),
        Err(ReferenceError::Missing)
    );
    assert_eq!(top_and_wall.resolve(&before), Ok(edge));
}
