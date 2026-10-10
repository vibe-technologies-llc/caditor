use std::collections::BTreeSet;

use caditor_geometry::{Plane, Point3};

use super::*;
use crate::{
    build::{LinearExtent, extrude},
    naming::{EdgeName, FaceName, FaceReference, SplitPiece},
    profile::{Profile, ProfileCurve, Selection},
    surface::Surface,
    test_support::{circle, rectangle},
    tolerance::SamplingTolerance,
};

const BODY: u64 = 1;
const TOOL: u64 = 2;
const SPLIT: u64 = 3;

fn swept(curves: &[ProfileCurve], extent: LinearExtent, feature: u64) -> Solid {
    let regions = Profile::new(curves)
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap();
    extrude(&Plane::XY, &regions, extent, feature).unwrap()
}

fn block(height: f64) -> Solid {
    swept(
        &rectangle(1, (0.0, 0.0), (10.0, 10.0)),
        LinearExtent::one_side(height).unwrap(),
        BODY,
    )
}

fn through(curves: &[ProfileCurve]) -> Solid {
    swept(curves, LinearExtent::new(-50.0, 50.0).unwrap(), TOOL)
}

fn half_space(at: f64) -> Solid {
    through(&rectangle(11, (at, -50.0), (50.0, 50.0)))
}

fn top_face(solid: &Solid, height: f64) -> FaceId {
    let middle = Point3::new(5.0, 5.0, height);
    solid
        .faces()
        .find(|(_, face)| {
            matches!(face.surface(), Surface::Plane(_)) && face.surface().distance(middle) < 1e-9
        })
        .map(|(id, _)| id)
        .unwrap()
}

fn volume(solid: &Solid) -> f64 {
    solid
        .tessellate(&SamplingTolerance::new(1e-3, 0.1).unwrap())
        .unwrap()
        .mass_properties()
        .volume
}

fn names(solid: &Solid) -> BTreeSet<FaceName> {
    solid.faces().map(|(_, face)| face.name()).collect()
}

fn split_top(height: f64, tool: &Solid) -> (Solid, FaceName) {
    let body = block(height);
    let top = top_face(&body, height);
    let original = body.face(top).unwrap().name();
    let split = split_faces(&body, &[top], tool, SPLIT).unwrap();
    (split, original)
}

#[test]
fn a_line_divides_the_top_of_a_box_and_nothing_else() {
    let (split, top) = split_top(5.0, &half_space(4.0));

    assert_eq!(split.validate(), Ok(()));
    assert_eq!(split.faces().count(), 7);
    assert_eq!(split.edges().count(), 15);
    assert_eq!(split.vertices().count(), 10);
    assert!((volume(&split) - 500.0).abs() < 1e-6);

    let inside = FaceName::split(SPLIT, top, SplitPiece::Inside);
    let outside = FaceName::split(SPLIT, top, SplitPiece::Outside);
    let found = names(&split);
    assert!(found.contains(&inside) && found.contains(&outside));
    assert!(!found.contains(&top));
    assert_eq!(
        split
            .edges()
            .filter(|(_, edge)| edge.name() == EdgeName::between(inside, outside))
            .count(),
        1
    );
}

#[test]
fn a_circle_cuts_an_island_out_of_the_top_of_a_box() {
    let (split, top) = split_top(5.0, &through(&[circle(21, (5.0, 5.0), 2.0)]));

    assert_eq!(split.validate(), Ok(()));
    assert_eq!(split.faces().count(), 7);
    assert!((volume(&split) - 500.0).abs() < 1e-6);

    let island = split
        .faces()
        .find(|(_, face)| face.name() == FaceName::split(SPLIT, top, SplitPiece::Inside))
        .map(|(_, face)| face)
        .unwrap();
    let ring = split
        .faces()
        .find(|(_, face)| face.name() == FaceName::split(SPLIT, top, SplitPiece::Outside))
        .map(|(_, face)| face)
        .unwrap();
    assert_eq!(island.loops().len(), 1);
    assert_eq!(ring.loops().len(), 2);
}

#[test]
fn pieces_keep_their_names_and_references_through_an_upstream_edit() {
    let tool = half_space(4.0);
    let (before, _) = split_top(5.0, &tool);
    let (after, _) = split_top(7.0, &tool);
    let moved_tool = half_space(6.0);
    let (moved, _) = split_top(5.0, &moved_tool);

    assert_eq!(names(&before), names(&after));
    assert_eq!(names(&before), names(&moved));

    for (id, _) in before.faces() {
        let reference = FaceReference::capture(&before, id).unwrap();
        assert!(reference.resolve(&after).is_ok());
        assert!(reference.resolve(&moved).is_ok());
    }
}

#[test]
fn a_tool_missing_every_chosen_face_divides_nothing() {
    let body = block(5.0);
    let top = top_face(&body, 5.0);

    let missed = split_faces(&body, &[top], &half_space(20.0), SPLIT);

    assert_eq!(missed, Err(FaceSplitError::Undivided));
    assert_eq!(
        split_faces(&body, &[], &half_space(4.0), SPLIT),
        Err(FaceSplitError::NoFaces)
    );
}

#[test]
fn a_plane_divides_the_wall_of_a_cylinder_into_two_bands() {
    let body = swept(
        &[circle(1, (0.0, 0.0), 3.0)],
        LinearExtent::one_side(5.0).unwrap(),
        BODY,
    );
    let wall = body
        .faces()
        .find(|(_, face)| matches!(face.surface(), Surface::Cylinder(_)))
        .map(|(id, _)| id)
        .unwrap();
    let above = swept(
        &rectangle(11, (-50.0, -50.0), (50.0, 50.0)),
        LinearExtent::new(2.0, 50.0).unwrap(),
        TOOL,
    );

    let split = split_faces(&body, &[wall], &above, SPLIT).unwrap();

    assert_eq!(split.validate(), Ok(()));
    assert_eq!(split.faces().count(), 4);
    assert_eq!(split.edges().count(), 5);
    assert!((volume(&split) - volume(&body)).abs() < 1e-6);
}
