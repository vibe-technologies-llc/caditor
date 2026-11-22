use std::f64::consts::PI;

use caditor_document::{BodyOperation, FeatureId, FeatureKind, MoveAxis};
use caditor_expression::{Expression, Unit};
use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;
use egui::{Id, Key, Modifiers};

use super::{
    Harness, add_peg, choose, click_with, extruded, extruded_plate, offer, pickable_described,
    plate_bounds, rectangle, run_from_palette, top_edge_along_x, vertex_at, volume_about,
};
use crate::{
    commands::Command,
    editing::EditingCommand,
    mate_panel, mirror_panel, mirror_tools,
    model::Action,
    pattern_panel,
    reference_picking::{Picking, Slot},
    selection::Pickable,
    solid_panel, split_panel, visibility,
};

fn any_edge(harness: &Harness, body: FeatureId) -> Pickable {
    let edge = harness
        .model
        .evaluation()
        .body(body)
        .unwrap()
        .edges()
        .next()
        .map(|(_, edge)| edge.name())
        .unwrap();
    Pickable::Edge { body, edge }
}

fn open_mirror(harness: &Harness) -> (FeatureId, caditor_document::Mirror) {
    let feature = harness
        .workspace
        .editing
        .solid()
        .expect("the mirror is open");
    match &harness.document().feature(feature).unwrap().kind {
        FeatureKind::Mirror(mirror) => (feature, mirror.clone()),
        other => panic!("expected a mirror, found {other:?}"),
    }
}

#[test]
fn several_selected_bodies_are_mirrored_by_one_mirror_making_a_body_of_each_image() {
    let mut harness = Harness::new();

    let (plate, _) = extruded_plate(&mut harness);
    let peg = add_peg(&mut harness);

    harness.select([
        Pickable::Edge {
            body: plate,
            edge: top_edge_along_x(&harness, plate, 0.0),
        },
        any_edge(&harness, peg),
    ]);
    harness.hover("Mirror body");

    assert!(harness.shows_containing("Mirror 2 bodies across"));

    harness.click("Mirror body");
    harness.settle();

    assert_eq!(harness.model.undo_label(), Some("Create Mirror body 1"));
    assert!(harness.shows("Mirror across"));
    assert!(harness.shows(mirror_panel::BODIES));
    assert!(harness.shows("Plate"));
    assert!(harness.shows("Peg"));

    let (mirror, definition) = open_mirror(&harness);

    assert_eq!(definition.body, plate);
    assert_eq!(definition.more_bodies, [peg]);
    assert_eq!(
        harness
            .document()
            .features()
            .filter(|feature| matches!(feature.kind, FeatureKind::Mirror(_)))
            .count(),
        1
    );

    let images = harness.model.evaluation().pieces_made_by(mirror);

    assert_eq!(images.len(), 2);

    let plate_image = mirror
        .piece(caditor_document::Mirror::image_key(plate))
        .unwrap();
    let peg_image = mirror
        .piece(caditor_document::Mirror::image_key(peg))
        .unwrap();
    let (_, high) = plate_bounds(&harness, peg);

    assert!((high.x - 60.0).abs() < 1e-6);

    let (low, _) = plate_bounds(&harness, peg_image);

    assert!((low.x + 60.0).abs() < 1e-6);

    let (low, high) = plate_bounds(&harness, plate_image);

    assert!((low.x + 40.0).abs() < 1e-6 && high.x.abs() < 1e-6);
    assert!(volume_about(&harness, plate_image, 16000.0));
    assert_eq!(
        harness.document().body_name(peg_image).as_deref(),
        Some("Mirror body 1 (image of Peg)")
    );
}

#[test]
fn a_mirror_takes_and_leaves_out_bodies_from_the_panel_and_clicks_in_the_view() {
    let mut harness = Harness::new();

    let (plate, _) = extruded_plate(&mut harness);
    let peg = add_peg(&mut harness);

    harness.select([Pickable::Edge {
        body: plate,
        edge: top_edge_along_x(&harness, plate, 0.0),
    }]);
    run_from_palette(&mut harness, "mirror body");
    harness.settle();

    let (mirror, _) = open_mirror(&harness);

    harness.click_lowest(crate::feature_fields::CHOOSE_IN_VIEW);
    harness.frame();

    assert!(harness.shows_containing("Click a body to mirror it too"));

    harness.click_pickable(Plane::XY, Point2::new(32.0, 20.0), any_edge(&harness, peg));
    harness.settle();

    let (_, definition) = open_mirror(&harness);

    assert_eq!(definition.more_bodies, [peg]);
    assert_eq!(harness.model.undo_label(), Some("Edit Mirror body 1"));
    assert_eq!(harness.model.evaluation().pieces_made_by(mirror).len(), 2);
    assert!(
        harness
            .workspace
            .editing
            .picking()
            .is_some_and(|picking| picking.slot == crate::reference_picking::Slot::MirrorBodies)
    );

    harness.click_pickable(Plane::XY, Point2::new(32.0, 20.0), any_edge(&harness, peg));
    harness.settle();

    let (_, definition) = open_mirror(&harness);

    assert!(definition.more_bodies.is_empty());
    assert!(volume_about(&harness, mirror, 16000.0));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    harness.select([any_edge(&harness, peg)]);
    harness.settle();
    harness.click_lowest(crate::feature_fields::USE_SELECTED);
    harness.settle();

    let (_, definition) = open_mirror(&harness);

    assert_eq!(definition.more_bodies, [peg]);

    harness.click_topmost_button("Stop mirroring this body");
    harness.settle();

    let (_, definition) = open_mirror(&harness);

    assert_eq!(definition.body, peg);
    assert!(definition.more_bodies.is_empty());
}

fn picking_bodies(harness: &Harness) -> bool {
    harness
        .workspace
        .editing
        .picking()
        .is_some_and(|picking| picking.slot == crate::reference_picking::Slot::MirrorBodies)
}

#[test]
fn mirror_body_with_nothing_selected_waits_for_the_bodies_clicked_in_the_view() {
    let mut harness = Harness::new();

    let (plate, _) = extruded_plate(&mut harness);
    let peg = add_peg(&mut harness);

    harness.select([]);

    let features = harness.document().features().count();

    harness.hover("Mirror body");

    assert!(harness.shows_containing(mirror_tools::CHOOSE_BODIES_HOVER));

    harness.click("Mirror body");
    harness.settle();

    assert_eq!(harness.document().features().count(), features);
    assert!(harness.workspace.editing.is_choosing_mirrored());
    assert!(harness.shows(mirror_tools::CHOOSE_BODIES_PROMPT));

    harness.click_pickable(
        Plane::XY,
        Point2::new(5.0, 5.0),
        Pickable::Edge {
            body: plate,
            edge: top_edge_along_x(&harness, plate, 0.0),
        },
    );
    harness.settle();

    let (_, definition) = open_mirror(&harness);

    assert_eq!(definition.body, plate);
    assert!(definition.more_bodies.is_empty());
    assert!(!harness.workspace.editing.is_choosing_mirrored());
    assert!(picking_bodies(&harness));

    harness.click_pickable(Plane::XY, Point2::new(32.0, 20.0), any_edge(&harness, peg));
    harness.settle();

    let (_, definition) = open_mirror(&harness);

    assert_eq!(definition.more_bodies, [peg]);
}

#[test]
fn escape_stops_choosing_bodies_to_mirror_without_making_a_mirror() {
    let mut harness = Harness::new();

    extruded_plate(&mut harness);
    harness.select([]);

    let features = harness.document().features().count();

    harness.click("Mirror body");
    harness.frame();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    assert!(!harness.workspace.editing.is_choosing_mirrored());
    assert_eq!(harness.document().features().count(), features);
}

#[test]
fn an_open_mirror_lights_the_bodies_it_mirrors() {
    let mut harness = Harness::new();

    let (plate, top) = extruded_plate(&mut harness);

    harness.select([]);
    harness.workspace.viewport.advance(super::CAMERA_SETTLE);
    harness.frame();

    let plain = drawn_colour(&mut harness, top);

    harness.select([Pickable::Edge {
        body: plate,
        edge: top_edge_along_x(&harness, plate, 0.0),
    }]);
    run_from_palette(&mut harness, "mirror body");
    harness.settle();
    harness.select([]);
    harness.frame();

    let open = drawn_colour(&mut harness, top);

    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();

    let closed = drawn_colour(&mut harness, top);

    assert_ne!(open, plain);
    assert_eq!(closed, plain);
}

fn drawn_colour(harness: &mut Harness, face: Pickable) -> caditor_render::Color {
    let built = harness.built();
    let pick = built.picks.id_of(face);
    built
        .scene
        .meshes
        .iter()
        .flat_map(|instance| instance.faces.iter())
        .find(|drawn| drawn.pick == pick)
        .map(|drawn| drawn.color)
        .expect("the face is drawn")
}

#[test]
fn a_body_is_mirrored_across_a_plane_as_a_new_body_joined_or_in_its_place_from_the_panel() {
    let mut harness = Harness::new();

    let (plate, top) = extruded_plate(&mut harness);

    harness.select([Pickable::Edge {
        body: plate,
        edge: top_edge_along_x(&harness, plate, 0.0),
    }]);
    run_from_palette(&mut harness, "mirror body");
    harness.settle();

    let mirror = harness
        .workspace
        .editing
        .solid()
        .expect("the mirror is open");

    assert_eq!(harness.model.undo_label(), Some("Create Mirror body 1"));
    assert!(harness.shows("Mirror across"));
    assert!(harness.shows(mirror_panel::RESULT));
    assert!(volume_about(&harness, plate, 16000.0));

    let (low, high) = plate_bounds(&harness, plate);

    assert!(low.x.abs() < 1e-6 && (high.x - 40.0).abs() < 1e-6);
    assert!(volume_about(&harness, mirror, 16000.0));

    let (low, high) = plate_bounds(&harness, mirror);

    assert!((low.x + 40.0).abs() < 1e-6 && high.x.abs() < 1e-6);

    harness.built_with_meshes(2);

    choose(
        &mut harness,
        mirror_panel::NEW_BODY,
        mirror_panel::ADD_TO_BODY,
    );

    assert_eq!(harness.model.undo_label(), Some("Edit Mirror body 1"));
    assert!(volume_about(&harness, plate, 2.0 * 16000.0));

    let (low, high) = plate_bounds(&harness, plate);

    assert!((low.x + 40.0).abs() < 1e-6 && (high.x - 40.0).abs() < 1e-6);

    choose(
        &mut harness,
        mirror_panel::ADD_TO_BODY,
        mirror_panel::REPLACE_BODY,
    );

    assert_eq!(harness.model.undo_label(), Some("Edit Mirror body 1"));
    assert!(volume_about(&harness, plate, 16000.0));

    let (low, high) = plate_bounds(&harness, plate);

    assert!((low.x + 40.0).abs() < 1e-6 && high.x.abs() < 1e-6);

    choose(&mut harness, "The YZ plane", "The XY plane");

    let (low, high) = plate_bounds(&harness, plate);

    assert!(
        (low.z + 10.0).abs() < 1e-6 && high.z.abs() < 1e-6,
        "{low:?} {high:?}"
    );

    harness.select([top]);
    run_from_palette(&mut harness, "mirror across selected");
    harness.settle();

    let (low, high) = plate_bounds(&harness, plate);

    assert!(
        (low.z - 10.0).abs() < 1e-6 && (high.z - 20.0).abs() < 1e-6,
        "{low:?} {high:?}"
    );
    assert_eq!(harness.workspace.editing.solid(), Some(mirror));

    for _ in 0..4 {
        harness.perform(Action::Undo);
    }
    harness.settle();

    assert!(volume_about(&harness, plate, 16000.0));
    assert!(volume_about(&harness, mirror, 16000.0));

    let (low, high) = plate_bounds(&harness, mirror);

    assert!((low.x + 40.0).abs() < 1e-6 && high.x.abs() < 1e-6);
}

#[test]
fn a_body_is_split_along_a_plane_into_two_bodies_from_the_panel() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);

    rectangle(
        &mut sketch,
        Point2::new(-20.0, -20.0),
        Point2::new(20.0, 20.0),
    );
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();

    let plate = harness
        .workspace
        .editing
        .solid()
        .expect("the extrusion is open");

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();

    let top = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| matches!(pickable, Pickable::Edge { body, .. } if *body == plate))
        .expect("an edge of the plate is pickable");

    harness.select([top]);
    harness.use_tool_with(Key::K, Modifiers::ALT);
    harness.settle();

    let split = harness
        .workspace
        .editing
        .solid()
        .expect("the split is open");

    assert_eq!(harness.model.undo_label(), Some("Create Split 1"));
    assert!(harness.shows("Split along"));
    assert!(harness.shows("Split-off body"));
    assert!(harness.shows("Waiting for a plane, curve or body"));
    assert!(harness.shows("None chosen"));
    assert_eq!(harness.model.evaluation().failed_count(), 0);

    choose(&mut harness, "None chosen", "The YZ plane");

    assert_eq!(harness.model.evaluation().failed_count(), 0);

    let (low, high) = plate_bounds(&harness, plate);

    assert!(low.x.abs() < 1e-6 && (high.x - 20.0).abs() < 1e-6);

    let (low, high) = plate_bounds(&harness, split);

    assert!((low.x + 20.0).abs() < 1e-6 && high.x.abs() < 1e-6);
    assert!(volume_about(&harness, split, 8000.0));

    harness.click(split_panel::KEEP_OTHER_SIDE);
    harness.settle();

    assert_eq!(harness.model.undo_label(), Some("Edit Split 1"));

    let (low, high) = plate_bounds(&harness, plate);

    assert!((low.x + 20.0).abs() < 1e-6 && high.x.abs() < 1e-6);

    choose(&mut harness, "The YZ plane", "The XZ plane");

    let (low, high) = plate_bounds(&harness, plate);

    assert!(
        low.y.abs() < 1e-6 && (high.y - 20.0).abs() < 1e-6,
        "{low:?} {high:?}"
    );

    let (low, high) = plate_bounds(&harness, split);

    assert!(
        (low.y + 20.0).abs() < 1e-6 && high.y.abs() < 1e-6,
        "{low:?} {high:?}"
    );

    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert_eq!(harness.workspace.editing.solid(), None);
    assert_eq!(harness.built_with_meshes(2).scene.meshes.len(), 2);
}

#[test]
fn a_body_is_split_along_a_sketch_curve_or_another_body_chosen_in_the_panel() {
    let mut harness = Harness::new();

    let (plate, _) = extruded_plate(&mut harness);
    let peg = add_peg(&mut harness);
    let mut line = Sketch::new(Plane::XY);

    line.add_line(Point2::new(20.0, 10.0), Point2::new(20.0, 30.0));

    let mut transaction = harness.document().transaction("Add a cut line");

    transaction.add_feature("Cut line", FeatureKind::from(line));
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();

    let edge = |harness: &mut Harness, of: FeatureId| {
        harness
            .built()
            .picks
            .pickables()
            .find(|pickable| matches!(pickable, Pickable::Edge { body, .. } if *body == of))
            .expect("an edge of the body is pickable")
    };
    let plate_edge = edge(&mut harness, plate);
    let peg_edge = edge(&mut harness, peg);

    harness.select([plate_edge, peg_edge]);
    harness.use_tool_with(Key::K, Modifiers::ALT);
    harness.settle();

    let split = harness
        .workspace
        .editing
        .solid()
        .expect("the split is open");

    assert_eq!(harness.model.undo_label(), Some("Create Split 1"));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert!(volume_about(&harness, plate, 16000.0 - 1000.0));
    assert!(volume_about(&harness, split, 1000.0));
    assert!(volume_about(&harness, peg, 3000.0));

    choose(&mut harness, "Peg", "The curve of Cut line");

    assert_eq!(harness.model.undo_label(), Some("Edit Split 1"));
    assert_eq!(harness.model.evaluation().failed_count(), 0);

    let (low, high) = plate_bounds(&harness, plate);

    assert!(low.x.abs() < 1e-6 && (high.x - 20.0).abs() < 1e-6);

    let (low, high) = plate_bounds(&harness, split);

    assert!((low.x - 20.0).abs() < 1e-6 && (high.x - 40.0).abs() < 1e-6);

    harness.click(split_panel::KEEP_OTHER_SIDE);
    harness.settle();

    let (low, high) = plate_bounds(&harness, plate);

    assert!((low.x - 20.0).abs() < 1e-6 && (high.x - 40.0).abs() < 1e-6);
}

#[test]
fn a_body_is_mated_face_to_face_from_the_palette_and_its_panel_sets_the_distance() {
    let mut harness = Harness::new();

    let (plate, top) = extruded_plate(&mut harness);
    let peg = add_peg(&mut harness);
    let bottom = pickable_described(&mut harness, "Peg › Peg start face");

    harness.select([bottom, top]);
    run_from_palette(&mut harness, "mate body");
    harness.settle();

    let mate = harness.workspace.editing.solid().expect("the mate is open");

    assert_eq!(harness.model.undo_label(), Some("Create Mate 1"));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert!(harness.shows("Face to mate"));
    assert!(harness.shows("Onto"));

    let (low, high) = plate_bounds(&harness, peg);

    assert!((low.z - 10.0).abs() < 1e-6 && (high.z - 15.0).abs() < 1e-6);
    assert!((low.x - 30.0).abs() < 1e-6);

    harness.type_into_field(Id::unique(("mate-field", "distance", mate)), "2 mm");
    harness.settle();

    assert_eq!(harness.model.undo_label(), Some("Edit Mate 1"));

    let (low, _) = plate_bounds(&harness, peg);

    assert!((low.z - 12.0).abs() < 1e-6, "{low:?}");

    harness.click(mate_panel::FACE_SAME_WAY);
    harness.settle();

    let (low, high) = plate_bounds(&harness, peg);

    assert!(
        (high.z - 12.0).abs() < 1e-6 && (low.z - 7.0).abs() < 1e-6,
        "{low:?} {high:?}"
    );

    for _ in 0..3 {
        harness.perform(Action::Undo);
    }
    harness.settle();

    let (low, _) = plate_bounds(&harness, peg);

    assert!(low.z.abs() < 1e-6);
    assert!(volume_about(&harness, plate, 16000.0));
}

#[test]
fn a_body_is_mated_at_an_angle_and_by_a_corner_from_the_palette() {
    let mut harness = Harness::new();

    let (_, top) = extruded_plate(&mut harness);
    let peg = add_peg(&mut harness);
    let bottom = pickable_described(&mut harness, "Peg › Peg start face");

    harness.select([bottom, top]);
    run_from_palette(&mut harness, "mate body at an angle");
    harness.settle();

    let mate = harness.workspace.editing.solid().expect("the mate is open");

    assert_eq!(harness.model.undo_label(), Some("Create Mate 1"));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert!(harness.shows(mate_panel::ANGLE));

    let (low, high) = plate_bounds(&harness, peg);

    assert!((high.z - low.z - 5.0).abs() > 1.0, "{low:?} {high:?}");

    harness.type_into_field(Id::unique(("mate-field", "angle", mate)), "0 deg");
    harness.settle();

    assert_eq!(harness.model.undo_label(), Some("Edit Mate 1"));

    let (low, high) = plate_bounds(&harness, peg);

    assert!((high.z - low.z - 5.0).abs() < 1e-6, "{low:?} {high:?}");
    assert!((low.x - 30.0).abs() < 1e-6, "{low:?}");

    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();

    let corner = vertex_at(
        &harness,
        peg,
        caditor_geometry::Point3::new(30.0, 10.0, 0.0),
    );

    harness.select([corner, top]);
    run_from_palette(&mut harness, "mate body");
    harness.settle();

    assert_eq!(harness.model.undo_label(), Some("Create Mate 2"));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert!(harness.shows("Point to mate"));

    let (low, _) = plate_bounds(&harness, peg);

    assert!((low.z - 10.0).abs() < 1e-6, "{low:?}");
    assert!((low.x - 30.0).abs() < 1e-6, "{low:?}");
}

fn upright_edge_at(harness: &Harness, body: FeatureId, x: f64, y: f64) -> Pickable {
    let solid = harness.model.evaluation().body(body).unwrap();
    let edge = solid
        .edges()
        .find(|(_, edge)| {
            let middle = edge.curve().point(edge.interval().middle());
            (middle.x - x).abs() < 1e-6 && (middle.y - y).abs() < 1e-6
        })
        .map(|(_, edge)| edge.name())
        .expect("the body has an upright edge there");
    Pickable::Edge { body, edge }
}

#[test]
fn four_picks_mate_a_body_flush_and_concentric_in_one_feature() {
    let mut harness = Harness::new();

    let (plate, top) = extruded_plate(&mut harness);
    let peg = add_peg(&mut harness);
    let bottom = pickable_described(&mut harness, "Peg › Peg start face");
    let peg_edge = upright_edge_at(&harness, peg, 30.0, 10.0);
    let plate_edge = upright_edge_at(&harness, plate, 40.0, 40.0);

    harness.select([bottom, peg_edge, top, plate_edge]);
    run_from_palette(&mut harness, "mate body");
    harness.settle();

    assert_eq!(harness.model.undo_label(), Some("Create Mate 1"));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert!(harness.shows(mate_panel::AXIS_TO_MATE));
    assert!(harness.shows(mate_panel::ONTO_AXIS));

    let (low, high) = plate_bounds(&harness, peg);

    assert!(
        (low - caditor_geometry::Point3::new(40.0, 40.0, 10.0)).length() < 1e-6,
        "{low:?}"
    );
    assert!(
        (high - caditor_geometry::Point3::new(70.0, 60.0, 15.0)).length() < 1e-6,
        "{high:?}"
    );
}

#[test]
fn one_cut_removes_material_from_every_body_chosen_in_its_panel() {
    let mut harness = Harness::new();

    let first = extruded(&mut harness, Point2::new(0.0, 0.0), Point2::new(40.0, 40.0));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    let second = extruded(
        &mut harness,
        Point2::new(50.0, 0.0),
        Point2::new(90.0, 40.0),
    );

    assert_eq!(harness.solid(second).operation(), BodyOperation::NewBody);

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    assert!(volume_about(&harness, second, 16000.0));

    let cut = extruded(
        &mut harness,
        Point2::new(30.0, 10.0),
        Point2::new(60.0, 20.0),
    );

    choose(&mut harness, "Add to body", "Remove from body");

    assert_eq!(
        harness.solid(cut).operation(),
        BodyOperation::Remove(second)
    );
    assert!(harness.shows(solid_panel::ALSO_CUTS));
    assert!(volume_about(&harness, second, 15000.0));
    assert!(volume_about(&harness, first, 16000.0));

    choose(&mut harness, solid_panel::ADD_CUT_BODY, "Extrude 1");

    assert_eq!(harness.solid(cut).other_bodies(), [first]);
    assert!(volume_about(&harness, first, 15000.0));
    assert!(volume_about(&harness, second, 15000.0));
    assert_eq!(harness.model.evaluation().failed_count(), 0);

    harness.click_button("Stop cutting Extrude 1");
    harness.settle();

    assert!(harness.solid(cut).other_bodies().is_empty());
    assert!(volume_about(&harness, first, 16000.0));

    choose(&mut harness, solid_panel::ADD_CUT_BODY, "Extrude 1");
    choose(&mut harness, "Remove from body", "Add to body");

    assert!(harness.solid(cut).other_bodies().is_empty());
}

fn start_picking(harness: &mut Harness, feature: FeatureId, slot: Slot) {
    harness.perform(Action::Editing(EditingCommand::Pick(Picking::new(
        feature, slot,
    ))));
    harness.frame();
}

#[test]
fn the_bodies_a_cut_changes_are_clicked_in_the_view() {
    let mut harness = Harness::new();

    let first = extruded(&mut harness, Point2::new(0.0, 0.0), Point2::new(40.0, 40.0));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    let second = extruded(
        &mut harness,
        Point2::new(50.0, 0.0),
        Point2::new(90.0, 40.0),
    );

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    let cut = extruded(
        &mut harness,
        Point2::new(30.0, 10.0),
        Point2::new(60.0, 20.0),
    );

    choose(&mut harness, "Add to body", "Remove from body");

    assert_eq!(
        harness.solid(cut).operation(),
        BodyOperation::Remove(second)
    );

    start_picking(&mut harness, cut, Slot::AlsoCutBody);

    assert!(harness.shows_containing("Click a body to cut it too"));

    harness.click_pickable(
        Plane::XY,
        Point2::new(10.0, 10.0),
        any_edge(&harness, first),
    );
    harness.settle();

    assert_eq!(harness.solid(cut).other_bodies(), [first]);
    assert!(volume_about(&harness, first, 15000.0));
    assert!(
        harness
            .workspace
            .editing
            .picking()
            .is_some_and(|picking| picking.slot == Slot::AlsoCutBody)
    );

    harness.click_pickable(
        Plane::XY,
        Point2::new(10.0, 10.0),
        any_edge(&harness, first),
    );
    harness.settle();

    assert!(harness.solid(cut).other_bodies().is_empty());

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    start_picking(&mut harness, cut, Slot::OperationBody);

    assert!(harness.shows_containing("Click the body this feature changes"));

    harness.click_pickable(
        Plane::XY,
        Point2::new(10.0, 10.0),
        any_edge(&harness, first),
    );
    harness.settle();

    assert_eq!(harness.solid(cut).operation(), BodyOperation::Remove(first));
    assert!(harness.workspace.editing.picking().is_none());
    assert!(volume_about(&harness, second, 16000.0));
}

fn open_kind(harness: &Harness) -> Option<&FeatureKind> {
    let open = harness.workspace.editing.solid()?;
    Some(&harness.document().feature(open)?.kind)
}

#[test]
fn move_scale_and_split_take_the_only_body_shown_when_nothing_is_selected() {
    let mut harness = Harness::new();

    let (plate, _) = extruded_plate(&mut harness);

    harness.select([]);
    harness.frame();

    harness.click("Move body");
    harness.settle();

    let moved = match open_kind(&harness) {
        Some(FeatureKind::Move(moving)) => Some(moving.body),
        _ => None,
    };

    harness.perform(Action::Editing(EditingCommand::CancelSolid));
    harness.settle();
    harness.select([]);
    run_from_palette(&mut harness, "Scale body");
    harness.settle();

    let scaled = match open_kind(&harness) {
        Some(FeatureKind::Scale(scale)) => Some(scale.body),
        _ => None,
    };

    harness.perform(Action::Editing(EditingCommand::CancelSolid));
    harness.settle();
    harness.select([]);
    run_from_palette(&mut harness, "Split body");
    harness.settle();

    let split = match open_kind(&harness) {
        Some(FeatureKind::Split(split)) => Some(split.body),
        _ => None,
    };

    assert_eq!(moved, Some(plate));
    assert_eq!(scaled, Some(plate));
    assert_eq!(split, Some(plate));
}

#[test]
fn move_body_with_nothing_selected_asks_for_a_body_when_two_are_shown_and_takes_the_one_left() {
    let mut harness = Harness::new();

    let (plate, _) = extruded_plate(&mut harness);
    let peg = add_peg(&mut harness);

    harness.settle();
    harness.select([]);
    harness.frame();

    let refused = offer(&harness, Command::Move).availability.is_err();

    let hide = visibility::toggle_body(harness.document(), plate).unwrap();

    harness.perform(Action::Apply(hide));
    harness.settle();
    harness.click("Move body");
    harness.settle();

    assert!(refused);
    assert!(matches!(
        open_kind(&harness),
        Some(FeatureKind::Move(moving)) if moving.body == peg
    ));
}

fn open_move(harness: &Harness) -> (FeatureId, caditor_document::Move) {
    let feature = harness.workspace.editing.solid().expect("the move is open");
    match &harness.document().feature(feature).unwrap().kind {
        FeatureKind::Move(movement) => (feature, movement.clone()),
        other => panic!("expected a move, found {other:?}"),
    }
}

#[test]
fn several_selected_bodies_are_moved_together_with_alt_m_and_previewed_together() {
    let mut harness = Harness::new();

    let (plate, _) = extruded_plate(&mut harness);
    let peg = add_peg(&mut harness);

    let (plate_before, _) = plate_bounds(&harness, plate);
    let (peg_before, _) = plate_bounds(&harness, peg);

    harness.select([
        Pickable::Edge {
            body: plate,
            edge: top_edge_along_x(&harness, plate, 0.0),
        },
        any_edge(&harness, peg),
    ]);
    harness.hover("Move body");

    assert!(harness.shows_containing("Shift and turn 2 bodies"));

    harness.use_tool_with(Key::M, Modifiers::ALT);
    harness.settle();

    assert_eq!(harness.model.undo_label(), Some("Create Move body 1"));
    assert!(harness.shows(crate::move_panel::BODIES));
    assert!(harness.shows("Plate"));
    assert!(harness.shows("Peg"));

    let (movement, definition) = open_move(&harness);

    assert_eq!(definition.body, plate);
    assert_eq!(definition.more_bodies, [peg]);
    assert_eq!(
        harness
            .document()
            .features()
            .filter(|feature| matches!(feature.kind, FeatureKind::Move(_)))
            .count(),
        1
    );

    harness.draft_into_field(
        Id::unique(("move-field", "offset", 0usize, movement)),
        "25 mm",
    );

    let moved = harness
        .model
        .draft_placement()
        .expect("both bodies are previewed");
    let placement = moved.placement;

    assert_eq!(
        moved.from.iter().map(|(body, _)| *body).collect::<Vec<_>>(),
        [plate, peg]
    );
    assert!(
        placement
            .apply_point(caditor_geometry::Point3::ZERO)
            .distance(caditor_geometry::Point3::new(25.0, 0.0, 0.0))
            < 1e-9
    );

    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.settle();

    let (plate_after, _) = plate_bounds(&harness, plate);
    let (peg_after, _) = plate_bounds(&harness, peg);

    assert!((plate_after.x - (plate_before.x + 25.0)).abs() < 1e-6);
    assert!((peg_after.x - (peg_before.x + 25.0)).abs() < 1e-6);
}

#[test]
fn bodies_moved_together_are_moved_once_while_their_new_meshes_arrive() {
    use crate::{
        bodies::{Hold, Holds},
        model::RecomputeStatus,
        move_manipulator::Handle,
    };

    let mut harness = Harness::new();

    let (plate, _) = extruded_plate(&mut harness);
    let peg = add_peg(&mut harness);

    harness.select([
        Pickable::Edge {
            body: plate,
            edge: top_edge_along_x(&harness, plate, 0.0),
        },
        any_edge(&harness, peg),
    ]);
    harness.use_tool_with(Key::M, Modifiers::ALT);
    harness.settle();
    harness.workspace.viewport.advance(super::CAMERA_SETTLE);
    harness.frame();

    let (movement, _) = open_move(&harness);
    let holds = Holds {
        converts: Hold::holding(),
        ..Holds::default()
    };

    harness.draft_into_field(
        Id::unique(("move-field", "offset", 0usize, movement)),
        "25 mm",
    );
    harness.model.hold_body_work(&holds);
    harness.key(Key::Enter, Modifiers::NONE);
    harness.wait_until("the move is recomputed with its meshes held", |harness| {
        harness.model.status() == RecomputeStatus::UpToDate && harness.model.bodies_pending()
    });

    let moved = harness
        .model
        .draft_placement()
        .expect("the preview is held until the new meshes are shown");
    let held = harness.built();
    let held_handle = harness
        .workspace
        .viewport
        .handle_position(Handle::Along(MoveAxis::X), 0.0)
        .expect("the X arrow is shown");

    for body in [plate, peg] {
        let result = harness.model.evaluation().body_result(body).unwrap();

        assert!(!moved.moves_from(body, result));
    }

    assert_eq!(
        held.scene
            .meshes
            .iter()
            .chain(&held.scene.translucent_meshes)
            .filter(|mesh| mesh.placement == Some(moved.placement))
            .count(),
        2
    );

    holds.converts.release();
    harness.settle();

    let settled = harness.built();
    let settled_handle = harness
        .workspace
        .viewport
        .handle_position(Handle::Along(MoveAxis::X), 0.0)
        .expect("the X arrow is shown");

    assert!(harness.model.draft_placement().is_none());
    assert!(
        settled
            .scene
            .meshes
            .iter()
            .chain(&settled.scene.translucent_meshes)
            .all(|mesh| mesh.placement.is_none())
    );
    assert!(held_handle.distance(settled_handle) < 0.5);
}

#[test]
fn several_selected_bodies_are_copied_together_into_separate_bodies() {
    let mut harness = Harness::new();

    let (plate, _) = extruded_plate(&mut harness);
    let peg = add_peg(&mut harness);

    harness.select([
        Pickable::Edge {
            body: plate,
            edge: top_edge_along_x(&harness, plate, 0.0),
        },
        any_edge(&harness, peg),
    ]);
    harness.frame();

    run_from_palette(&mut harness, "copy body");
    harness.settle();

    let (copy, definition) = open_move(&harness);

    assert!(definition.copy);
    assert_eq!(definition.body, plate);
    assert_eq!(definition.more_bodies, [peg]);

    harness.type_into_field(Id::unique(("move-field", "offset", 1usize, copy)), "50 mm");
    harness.settle();

    let copies = harness.model.evaluation().pieces_made_by(copy);

    assert_eq!(copies.len(), 2);

    let peg_copy = copy.piece(caditor_document::Move::copy_key(peg)).unwrap();

    assert_eq!(
        harness.document().body_name(peg_copy).as_deref(),
        Some("Copy 1 (copy of Peg)")
    );
}

#[test]
fn a_move_takes_and_leaves_out_bodies_from_the_panel_and_clicks_in_the_view() {
    let mut harness = Harness::new();

    let (plate, _) = extruded_plate(&mut harness);
    let peg = add_peg(&mut harness);

    harness.select([Pickable::Edge {
        body: plate,
        edge: top_edge_along_x(&harness, plate, 0.0),
    }]);
    harness.click("Move body");
    harness.settle();

    harness.click_lowest(crate::feature_fields::CHOOSE_IN_VIEW);
    harness.frame();

    assert!(harness.shows_containing("Click a body to move it too"));

    harness.click_pickable(Plane::XY, Point2::new(32.0, 20.0), any_edge(&harness, peg));
    harness.settle();

    let (_, definition) = open_move(&harness);

    assert_eq!(definition.more_bodies, [peg]);
    assert_eq!(harness.model.undo_label(), Some("Edit Move body 1"));
    assert!(
        harness
            .workspace
            .editing
            .picking()
            .is_some_and(|picking| picking.slot == Slot::MoveBodies)
    );

    harness.click_pickable(Plane::XY, Point2::new(32.0, 20.0), any_edge(&harness, peg));
    harness.settle();

    let (_, definition) = open_move(&harness);

    assert!(definition.more_bodies.is_empty());

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    harness.select([any_edge(&harness, peg)]);
    harness.settle();
    harness.click_lowest(crate::feature_fields::USE_SELECTED);
    harness.settle();

    let (_, definition) = open_move(&harness);

    assert_eq!(definition.more_bodies, [peg]);

    harness.click_topmost_button("Stop moving this body");
    harness.settle();

    let (_, definition) = open_move(&harness);

    assert_eq!(definition.body, peg);
    assert!(definition.more_bodies.is_empty());
}

#[test]
fn several_bodies_chosen_in_the_tree_are_moved_together_with_alt_m() {
    let mut harness = Harness::new();

    let (plate, _) = extruded_plate(&mut harness);
    let peg = add_peg(&mut harness);

    harness.settle();
    harness.select([]);
    harness.click("Bodies (2)");
    harness.frame();

    harness.click("Extrude 1");
    click_with(&mut harness, "Peg", Modifiers::COMMAND);
    harness.frame();

    harness.use_tool_with(Key::M, Modifiers::ALT);
    harness.settle();

    let (_, definition) = open_move(&harness);

    assert_eq!(definition.body, plate);
    assert_eq!(definition.more_bodies, [peg]);
}

#[test]
fn a_body_is_scaled_by_a_factor_about_a_centre_typed_in_the_panel() {
    let mut harness = Harness::new();

    let (plate, top) = extruded_plate(&mut harness);

    harness.select([top]);
    harness.use_tool_with(Key::S, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();

    let scale = harness
        .workspace
        .editing
        .solid()
        .expect("the scale is open");

    assert_eq!(harness.model.undo_label(), Some("Create Scale body 1"));
    assert!(harness.shows("Factor"));
    assert!(harness.shows("Centre X"));
    assert!(volume_about(&harness, plate, 16000.0));

    harness.type_into_field(Id::unique(("scale-field", ("factor", 0usize), scale)), "2");
    harness.settle();

    assert!(volume_about(&harness, plate, 8.0 * 16000.0));

    harness.type_into_field(
        Id::unique(("scale-field", ("factor", 0usize), scale)),
        "0.5",
    );
    harness.settle();

    assert_eq!(harness.model.undo_label(), Some("Edit Scale body 1"));
    assert!(volume_about(&harness, plate, 16000.0 / 8.0));

    harness.type_into_field(
        Id::unique(("scale-field", ("center", 0usize), scale)),
        "40 mm",
    );
    harness.settle();

    let (low, high) = plate_bounds(&harness, plate);

    assert!(
        (low.x - 20.0).abs() < 1e-6 && (high.x - 40.0).abs() < 1e-6,
        "{low:?} {high:?}"
    );

    harness.type_into_field(Id::unique(("scale-field", ("factor", 0usize), scale)), "-1");

    assert!(harness.shows_containing("above zero"));
    assert_eq!(harness.workspace.editing.solid(), Some(scale));

    for _ in 0..4 {
        harness.perform(Action::Undo);
    }
    harness.settle();

    assert!(volume_about(&harness, plate, 16000.0));
}

fn add_round(
    harness: &mut Harness,
    name: &str,
    shape: caditor_document::PrimitiveShape,
    at: [f64; 2],
    reversed: bool,
) -> FeatureId {
    use caditor_document::{PlaneReference, Primitive, PrimitiveAnchor, PrincipalPlane};

    let mut transaction = harness.document().transaction("Add a round body");

    let body = transaction.add_feature(
        name,
        FeatureKind::Primitive(Primitive {
            shape,
            plane: PlaneReference::Principal(PrincipalPlane::Xy),
            at: at.map(|value| Expression::measure(value, Unit::Millimetre)),
            anchor: PrimitiveAnchor::BaseCentre,
            reversed,
            operation: BodyOperation::NewBody,
        }),
    );

    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    body
}

#[test]
fn a_body_is_split_along_the_surface_of_a_curved_face_of_another_body() {
    let mut harness = Harness::new();

    let (plate, _) = extruded_plate(&mut harness);
    let whole = harness.body_volume(plate);
    let height = whole / 1600.0;

    add_round(
        &mut harness,
        "Rod",
        caditor_document::PrimitiveShape::Cylinder {
            diameter: Expression::measure(10.0, Unit::Millimetre),
            height: Expression::measure(5.0, Unit::Millimetre),
        },
        [20.0, 20.0],
        true,
    );

    let plate_edge = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| matches!(pickable, Pickable::Edge { body, .. } if *body == plate))
        .expect("an edge of the plate is pickable");
    let wall = pickable_described(&mut harness, "Rod › Rod wall");

    harness.select([plate_edge, wall]);
    harness.use_tool_with(Key::K, Modifiers::ALT);
    harness.settle();

    let split = harness
        .workspace
        .editing
        .solid()
        .expect("the split is open");

    assert_eq!(harness.model.undo_label(), Some("Create Split 1"));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert!(harness.shows("The surface of Rod wall"));

    let inside = PI * 25.0 * height;
    let roughly = |harness: &Harness, body: FeatureId, expected: f64| {
        let found = harness.body_volume(body);

        assert!(
            (found - expected).abs() < 2e-2 * expected,
            "{found} {expected}"
        );
    };

    roughly(&harness, split, inside);
    roughly(&harness, plate, whole - inside);

    harness.click(split_panel::KEEP_OTHER_SIDE);
    harness.settle();
    roughly(&harness, plate, inside);
}

#[test]
fn a_flat_face_is_mated_resting_on_a_sphere_of_another_body_from_the_palette() {
    let mut harness = Harness::new();

    let peg = add_peg(&mut harness);

    add_round(
        &mut harness,
        "Ball",
        caditor_document::PrimitiveShape::Sphere {
            diameter: Expression::measure(10.0, Unit::Millimetre),
        },
        [100.0, 0.0],
        false,
    );

    let bottom = pickable_described(&mut harness, "Peg › Peg start face");
    let ball = pickable_described(&mut harness, "Ball › Ball surface");

    harness.select([bottom, ball]);
    run_from_palette(&mut harness, "mate body");
    harness.settle();

    assert_eq!(harness.model.undo_label(), Some("Create Mate 1"));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert!(harness.shows("Flat face"));
    assert!(harness.shows("Rests on"));

    let (low, high) = plate_bounds(&harness, peg);

    assert!(
        (low.z - 10.0).abs() < 1e-6 && (high.z - 15.0).abs() < 1e-6,
        "{low:?} {high:?}"
    );

    harness.click(mate_panel::OTHER_SIDE);
    harness.settle();

    let (low, _) = plate_bounds(&harness, peg);

    assert!(low.z.abs() < 1e-6, "{low:?}");
}

#[test]
fn a_pattern_makes_a_body_of_each_copy_from_its_result_row_and_undo_joins_them_again() {
    let mut harness = Harness::new();

    let (plate, _) = extruded_plate(&mut harness);

    harness.select([]);
    harness.click("Linear pattern");
    harness.settle();

    let pattern = harness
        .workspace
        .editing
        .solid()
        .expect("the pattern is open");
    let copies = harness.model.evaluation().pieces_made_by(pattern).len();
    let joined = harness.model.evaluation().body(plate).map(|_| ());

    assert!(harness.shows(pattern_panel::RESULT));
    assert!(copies == 0 && joined.is_some());

    choose(
        &mut harness,
        pattern_panel::ADD_TO_BODY,
        pattern_panel::NEW_BODIES,
    );

    let copies = harness.model.evaluation().pieces_made_by(pattern);

    assert_eq!(harness.model.undo_label(), Some("Edit Linear pattern 1"));
    assert!(!copies.is_empty());
    assert!(volume_about(&harness, plate, 16000.0));

    for copy in &copies {
        assert!(volume_about(&harness, *copy, 16000.0));
    }

    harness.perform(Action::Undo);
    harness.settle();

    assert!(
        harness
            .model
            .evaluation()
            .pieces_made_by(pattern)
            .is_empty()
    );
    assert!(volume_about(
        &harness,
        plate,
        16000.0 * (copies.len() + 1) as f64
    ));
}
