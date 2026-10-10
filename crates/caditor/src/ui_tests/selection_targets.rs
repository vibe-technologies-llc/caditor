use caditor_document::{AxisReference, BodyOperation, FeatureId, FeatureKind, PlaneReference};
use caditor_geometry::{Plane, Point2};
use caditor_sketch::{EntityId, Sketch};
use egui::{Key, Modifiers};

use super::{
    Harness, add_block, extruded_plate, open_solid, pattern_of, plate_bounds, rectangle,
    run_from_palette, top_edge_along_x,
};
use crate::{
    datum_tools,
    editing::EditingCommand,
    mirror_tools,
    model::Action,
    selection::{Axis, Pickable, PrincipalPlane, Selection},
};

fn plate_and_block(harness: &mut Harness) -> (FeatureId, Pickable, FeatureId) {
    let (plate, top) = extruded_plate(harness);
    let block = add_block(
        harness,
        "Block",
        [Point2::new(60.0, 0.0), Point2::new(80.0, 20.0)],
        "10 mm",
    );
    harness.frame();
    (plate, top, block)
}

fn sketch_line(harness: &Harness, sketch: FeatureId) -> Pickable {
    let entity = harness
        .document()
        .feature(sketch)
        .and_then(|feature| feature.kind.sketch())
        .and_then(|sketch| sketch.entities().next())
        .map(|(entity, _)| entity)
        .expect("the sketch has a curve");
    Pickable::SketchEntity {
        feature: sketch,
        entity,
    }
}

fn notice_text(harness: &Harness) -> String {
    harness
        .model
        .notice()
        .map(|notice| notice.text.clone())
        .unwrap_or_default()
}

#[test]
fn a_pattern_refuses_a_lone_axis_and_takes_the_body_of_a_vertex_or_of_the_tree() {
    let mut harness = Harness::new();
    let (plate, _, block) = plate_and_block(&mut harness);
    let features = harness.document().features().count();

    harness.select([Pickable::Axis(Axis::Y)]);
    harness.click("Linear pattern");
    harness.settle();
    let refused = harness.document().features().count() == features;

    harness.select([Pickable::Axis(Axis::Y)]);
    harness.workspace.panels.choose_only(plate);
    harness.frame();
    harness.click("Linear pattern");
    harness.settle();
    let from_tree = open_solid(&harness);
    let tree_body = pattern_of(&harness, from_tree).body;
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.perform(Action::Undo);
    harness.settle();

    harness.workspace.panels.selected = None;
    let corner = super::vertex_at(&harness, plate, caditor_geometry::Point3::ZERO);
    harness.select([corner, Pickable::Axis(Axis::Y)]);
    harness.click("Linear pattern");
    harness.settle();
    let from_vertex = open_solid(&harness);

    assert!(refused);
    assert_eq!(tree_body, plate);
    assert_eq!(pattern_of(&harness, from_vertex).body, plate);
    assert_ne!(pattern_of(&harness, from_vertex).body, block);
    let caditor_document::PatternKind::Linear { first, .. } =
        &pattern_of(&harness, from_vertex).kind
    else {
        panic!("a linear pattern was made");
    };
    assert_eq!(
        first.axis,
        AxisReference::Principal(caditor_document::PrincipalAxis::Y)
    );
}

#[test]
fn an_edge_naming_the_body_does_not_become_the_pattern_direction() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let edge = Pickable::Edge {
        body: plate,
        edge: top_edge_along_x(&harness, plate, 0.0),
    };

    harness.select([edge, Pickable::Axis(Axis::Z)]);
    harness.click("Circular pattern");
    harness.settle();
    let pattern = open_solid(&harness);

    let caditor_document::PatternKind::Circular(circular) = &pattern_of(&harness, pattern).kind
    else {
        panic!("a circular pattern was made");
    };
    assert_eq!(
        circular.axis,
        AxisReference::Principal(caditor_document::PrincipalAxis::Z)
    );
}

#[test]
fn a_body_mirrors_across_its_selected_flat_face_and_two_planes_are_refused() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let bottom = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| {
            pickable.describe(harness.document(), harness.model.evaluation())
                == "Extrude 1 › Extrude 1 start face"
        })
        .expect("the bottom face is pickable");

    let two_faces = mirror_tools::source(&harness.model, &selection_of([top, bottom]), (&[], &[]));
    let datum_and_face = mirror_tools::source(
        &harness.model,
        &selection_of([top, Pickable::Plane(PrincipalPlane::Xz)]),
        (&[], &[]),
    );
    harness.select([top]);
    harness.click("Mirror body");
    harness.settle();
    let (low, high) = plate_bounds(&harness, plate);

    assert_eq!(two_faces, Err(datum_tools::SEVERAL_PLANES));
    assert_eq!(
        datum_and_face.map(|source| source.plane),
        Ok(Some(PlaneReference::Principal(PrincipalPlane::Xz)))
    );
    assert!(
        low.z.abs() < 1e-6 && (high.z - 20.0).abs() < 1e-6,
        "{low:?} {high:?}"
    );
}

fn selection_of(items: impl IntoIterator<Item = Pickable>) -> Selection {
    let mut selection = Selection::default();
    selection.extend(items);
    selection
}

#[test]
fn remove_body_takes_the_body_chosen_in_the_tree_after_faces_of_another() {
    let mut harness = Harness::new();
    let (plate, top, block) = plate_and_block(&mut harness);

    harness.select([top]);
    harness.workspace.panels.choose_only(block);
    harness.frame();
    run_from_palette(&mut harness, "remove body");
    harness.settle();

    assert!(harness.model.evaluation().body(block).is_none());
    assert!(harness.model.evaluation().body(plate).is_some());
}

#[test]
fn two_bodies_chosen_in_the_tree_are_combined() {
    let mut harness = Harness::new();
    let (plate, _, block) = plate_and_block(&mut harness);

    harness.select([]);
    harness.workspace.panels.choose_all(&[plate, block]);
    harness.frame();
    harness.click("Combine");
    harness.settle();
    let combine = open_solid(&harness);

    let definition = harness
        .document()
        .feature(combine)
        .and_then(|feature| feature.kind.combine())
        .cloned()
        .expect("a combine was made");
    assert_eq!((definition.body, definition.tool), (plate, block));
}

#[test]
fn a_hole_and_an_extrusion_of_a_principal_plane_sketch_go_into_the_selected_body() {
    let mut harness = Harness::new();
    let (plate, top, block) = plate_and_block(&mut harness);
    let mut points = Sketch::new(Plane::XY);
    let point = points.add_point(Point2::new(20.0, 20.0));
    let drilled = harness.add_sketch(points);
    let mut boss = Sketch::new(Plane::XY);
    rectangle(&mut boss, Point2::new(10.0, 10.0), Point2::new(20.0, 20.0));
    let boss = harness.add_sketch(boss);

    harness.select([
        top,
        Pickable::SketchEntity {
            feature: drilled,
            entity: point,
        },
    ]);
    harness.click("Hole");
    harness.settle();
    let hole = open_solid(&harness);
    let hole_body = harness
        .document()
        .feature(hole)
        .and_then(|feature| match &feature.kind {
            FeatureKind::Hole(hole) => Some(hole.body),
            _ => None,
        });
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let line = sketch_line(&harness, boss);
    harness.select([top, line]);
    harness.click("Extrude");
    harness.settle();
    let extrusion = open_solid(&harness);

    assert_eq!(hole_body, Some(plate));
    assert_eq!(
        harness.solid(extrusion).operation(),
        BodyOperation::Add(plate)
    );
    assert_ne!(
        harness.solid(extrusion).operation(),
        BodyOperation::Add(block)
    );
}

#[test]
fn extrude_again_with_nothing_selected_does_not_sweep_the_same_profile_twice() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::new(0.0, 0.0), Point2::new(40.0, 40.0));
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    let first = open_solid(&harness);
    let features = harness.document().features().count();

    harness.click("Extrude");
    harness.settle();

    assert_eq!(harness.document().features().count(), features);
    assert_eq!(open_solid(&harness), first);
}

#[test]
fn a_fillet_and_a_shell_say_what_they_left_out_of_the_selection() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let edge = Pickable::Edge {
        body: plate,
        edge: top_edge_along_x(&harness, plate, 0.0),
    };
    let corner = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| matches!(pickable, Pickable::Vertex { body, .. } if *body == plate))
        .expect("a corner of the plate is pickable");

    harness.select([edge, corner]);
    harness.click("Fillet");
    harness.settle();
    let fillet_told = notice_text(&harness);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.perform(Action::Undo);
    harness.settle();
    harness.select([edge, top]);
    harness.click("Shell");
    harness.settle();
    let shell_told = notice_text(&harness);

    assert_eq!(
        fillet_told,
        "Fillet takes edges, faces and bodies only, so 1 vertex selected was left out."
    );
    assert_eq!(
        shell_told,
        "Shell takes faces to leave open only, so 1 edge selected was left out."
    );
}

#[test]
fn a_selection_command_changes_the_open_feature_not_the_row_looked_at() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    harness.select([Pickable::Edge {
        body: plate,
        edge: top_edge_along_x(&harness, plate, 0.0),
    }]);
    harness.click("Mirror body");
    harness.settle();
    let mirror = open_solid(&harness);

    harness.workspace.panels.choose_only(plate);
    harness.select([Pickable::Plane(PrincipalPlane::Xy)]);
    run_from_palette(&mut harness, "mirror across selected");
    harness.settle();

    let plane = harness
        .document()
        .feature(mirror)
        .and_then(|feature| feature.kind.mirror())
        .map(|mirror| mirror.plane.clone());
    assert_eq!(plane, Some(PlaneReference::Principal(PrincipalPlane::Xy)));
}

#[test]
fn new_sketch_refuses_several_planes_instead_of_preferring_one() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);

    harness.select([top, Pickable::Plane(PrincipalPlane::Xz)]);
    harness.click("New sketch");
    harness.settle();
    harness.hover("New sketch");

    assert_eq!(harness.editing(), None);
    assert!(!harness.workspace.editing.is_choosing_plane());
    assert!(harness.shows_containing(crate::sketch_placement::SEVERAL_TO_SKETCH_ON));
}

#[test]
fn a_revolve_turns_about_the_axis_picked_after_its_profile() {
    let mut harness = Harness::new();
    let mut section = Sketch::new(Plane::XZ);
    rectangle(
        &mut section,
        Point2::new(10.0, 0.0),
        Point2::new(20.0, 10.0),
    );
    let section = harness.add_sketch(section);
    let lines: Vec<EntityId> = harness
        .sketch(section)
        .entities()
        .map(|(entity, _)| entity)
        .collect();

    let selection = harness.workspace.viewport.selection_mut();
    selection.clear();
    for entity in &lines {
        selection.toggle(Pickable::SketchEntity {
            feature: section,
            entity: *entity,
        });
    }
    selection.toggle(Pickable::Axis(Axis::Z));
    harness.frame();
    harness.click("Revolve");
    harness.settle();
    let revolve = open_solid(&harness);
    harness.perform(Action::Editing(EditingCommand::CloseSolid));
    harness.settle();

    assert_eq!(
        harness.solid(revolve).axis().cloned(),
        Some(caditor_document::RevolveAxis::Model(
            AxisReference::Principal(caditor_document::PrincipalAxis::Z)
        ))
    );
}
