use caditor_document::{FeatureKind, MoveAxis, SolidFeature};
use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Entity, Sketch};
use egui::{Event, Id, Key, Modifiers};

use super::{
    CAMERA_SETTLE, Harness, body_centre, choose, drag_screen, drag_value_arrow, extruded_plate,
    extrusion_above_plate, length_value, move_of, offset_of, open_combo, open_hole, open_solid,
    opened_move, primitive_of, rectangle, removed_about, start_of_solid, volume_about,
};
use crate::{model::Action, move_manipulator::Handle, selection::Pickable};

#[test]
fn a_move_being_typed_moves_the_drawn_body_before_it_is_entered() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    harness.select([top]);
    harness.click("Move body");
    harness.settle();
    let movement = harness.workspace.editing.solid().expect("the move is open");

    harness.draft_into_field(Id::new(("move-field", "offset", 0usize, movement)), "12 mm");

    let (body, placement) = harness
        .model
        .draft_placement()
        .expect("the move is previewed");
    assert_eq!(body, plate);
    assert!(
        placement
            .apply_point(caditor_geometry::Point3::ZERO)
            .distance(caditor_geometry::Point3::new(12.0, 0.0, 0.0))
            < 1e-9
    );
    assert_eq!(harness.model.undo_label(), Some("Create Move body 1"));
    let built = harness.built_with_meshes(2);
    let moved = built
        .scene
        .meshes
        .iter()
        .find_map(|mesh| mesh.placement)
        .expect("the drawn body carries the preview's placement");
    assert_eq!(moved, placement);

    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.settle();
    assert_eq!(harness.model.undo_label(), Some("Edit Move body 1"));
    assert!(harness.model.draft_placement().is_none());
    assert!(
        harness
            .built()
            .scene
            .meshes
            .iter()
            .all(|mesh| mesh.placement.is_none())
    );
}

#[test]
fn dragging_a_move_arrow_moves_the_body_along_it_in_one_change() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    harness.select([top]);
    harness.click("Move body");
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let movement = harness.workspace.editing.solid().expect("the move is open");
    let along_x = Handle::Along(MoveAxis::X);
    let from = harness
        .workspace
        .viewport
        .handle_position(along_x, 0.0)
        .expect("the X arrow is shown");
    let to = harness
        .workspace
        .viewport
        .handle_position(along_x, 15.0)
        .unwrap();
    let step = harness.workspace.viewport.manipulator_step().unwrap();
    harness.events.push(Event::PointerMoved(from));
    harness.frame();
    assert!(harness.shows("Drag to move the body along X"));

    drag_screen(&mut harness, from, to);
    harness.frame();
    harness.settle();

    assert_eq!(harness.model.undo_label(), Some("Edit Move body 1"));
    let Some(FeatureKind::Move(moved)) = harness
        .document()
        .feature(movement)
        .map(|feature| feature.kind.clone())
    else {
        panic!("the move is still a move");
    };
    let parameters = harness.model.parameters();
    let value = |expression: &Expression| {
        expression
            .evaluate_as(caditor_expression::Dimension::LENGTH, &|id| {
                parameters.value(id)
            })
            .unwrap()
    };
    let x = value(&moved.offset[0]);
    assert!((x - 15.0).abs() <= step, "{x} with steps of {step}");
    assert!(((x / step).round() * step - x).abs() < 1e-9);
    assert_eq!(value(&moved.offset[1]), 0.0);
    assert_eq!(value(&moved.offset[2]), 0.0);
    let bounds = harness
        .model
        .evaluation()
        .body(plate)
        .unwrap()
        .bounding_box()
        .unwrap();
    assert!((bounds.min().x - x).abs() < 1e-6);

    harness.perform(Action::Undo);
    harness.settle();
    assert_eq!(harness.model.undo_label(), Some("Create Move body 1"));
}

#[test]
fn dragging_an_extrusion_arrow_changes_its_distance_in_one_change() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::new(0.0, 0.0), Point2::new(40.0, 40.0));
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let extrude = harness
        .workspace
        .editing
        .solid()
        .expect("the extrusion is open");
    let reach = Handle::Reach(crate::move_manipulator::Reach::Only);
    let from = harness
        .workspace
        .viewport
        .handle_position(reach, 0.0)
        .expect("the distance arrow is shown");
    let to = harness
        .workspace
        .viewport
        .handle_position(reach, 15.0)
        .unwrap();
    let step = harness.workspace.viewport.manipulator_step().unwrap();
    harness.events.push(Event::PointerMoved(from));
    harness.frame();
    assert!(harness.shows("Drag to change the extrusion's distance"));

    drag_screen(&mut harness, from, to);
    harness.frame();
    harness.settle();

    assert_eq!(harness.model.undo_label(), Some("Edit Extrude 1"));
    let top = harness
        .model
        .evaluation()
        .body(extrude)
        .unwrap()
        .bounding_box()
        .unwrap()
        .max()
        .z;
    assert!((top - 25.0).abs() <= step, "{top} with steps of {step}");
    assert!(((top / step).round() * step - top).abs() < 1e-9);

    harness.perform(Action::Undo);
    harness.settle();
    assert_eq!(harness.model.undo_label(), Some("Create Extrude 1"));
}

#[test]
fn dragging_a_hole_s_handles_moves_it_on_its_face_in_one_change() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    harness.select([top]);
    harness.click("Hole");
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let hole = harness.workspace.editing.solid().expect("the hole is open");
    let sketch = open_hole(&harness, hole).sketch;
    let point_of = |harness: &Harness| {
        harness
            .sketch(sketch)
            .entities()
            .find_map(|(_, entity)| match entity {
                Entity::Point(at) => Some(*at),
                _ => None,
            })
            .expect("the hole's sketch holds its point")
    };
    let on_face = Handle::Place(crate::move_manipulator::PlaceGrip::OnPlane);
    let along_y = Handle::Place(crate::move_manipulator::PlaceGrip::AlongY);

    let from = harness
        .workspace
        .viewport
        .handle_position(on_face, 0.0)
        .expect("the hole's square is shown");
    let to = harness
        .workspace
        .viewport
        .handle_position(on_face, 8.0)
        .unwrap();
    let step = harness.workspace.viewport.manipulator_step().unwrap();
    harness.events.push(Event::PointerMoved(from));
    harness.frame();

    assert!(harness.shows("Drag to move the hole on its face"));

    drag_screen(&mut harness, from, to);
    harness.frame();
    harness.settle();
    let moved = point_of(&harness);

    assert_eq!(harness.model.undo_label(), Some("Move Hole 1"));
    assert!(
        (moved.x - 28.0).abs() <= step,
        "{moved:?} with steps of {step}"
    );
    assert!((moved.y - 20.0).abs() < 1e-9, "{moved:?}");
    assert!(removed_about(
        &harness,
        plate,
        std::f64::consts::PI * 9.0 * 10.0
    ));

    let from = harness
        .workspace
        .viewport
        .handle_position(along_y, 0.0)
        .expect("the hole's Y arrow is shown");
    let to = harness
        .workspace
        .viewport
        .handle_position(along_y, -6.0)
        .unwrap();
    drag_screen(&mut harness, from, to);
    harness.frame();
    harness.settle();
    let again = point_of(&harness);

    assert!((again.x - moved.x).abs() < 1e-9, "{again:?}");
    assert!(
        (again.y - 14.0).abs() <= step,
        "{again:?} with steps of {step}"
    );

    harness.perform(Action::Undo);
    harness.settle();

    assert_eq!(point_of(&harness), moved);
}

#[test]
fn dragging_a_box_s_square_moves_it_on_its_plane_and_keeps_a_named_position() {
    let mut harness = Harness::new();
    harness.select([]);
    harness.use_tool_with(Key::B, Modifiers::ALT);
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let feature = harness.workspace.editing.solid().expect("the box is open");
    let on_plane = Handle::Place(crate::move_manipulator::PlaceGrip::OnPlane);
    let along_x = Handle::Place(crate::move_manipulator::PlaceGrip::AlongX);

    harness.type_into_field(Id::new(("primitive-field", feature, ("at", 1))), "y = 4 mm");
    harness.settle();
    let from = harness
        .workspace
        .viewport
        .handle_position(on_plane, 0.0)
        .expect("the box's square is shown");
    let to = harness
        .workspace
        .viewport
        .handle_position(on_plane, 12.0)
        .unwrap();
    let step = harness.workspace.viewport.manipulator_step().unwrap();
    harness.events.push(Event::PointerMoved(from));
    harness.frame();

    assert!(harness.shows("Drag to move the shape where it stands"));

    drag_screen(&mut harness, from, to);
    harness.frame();
    harness.settle();
    let centre = body_centre(&harness, feature);

    assert_eq!(harness.model.undo_label(), Some("Edit Box 1"));
    assert!(
        (centre.x - 12.0).abs() <= step,
        "{centre:?} with steps of {step}"
    );
    assert!((centre.y - 4.0).abs() < 1e-6, "{centre:?}");
    assert!(matches!(
        primitive_of(&harness, feature).at[1],
        caditor_expression::Expression::Parameter(_)
    ));

    let from = harness
        .workspace
        .viewport
        .handle_position(along_x, 0.0)
        .expect("the box's X arrow is shown");
    let to = harness
        .workspace
        .viewport
        .handle_position(along_x, -5.0)
        .unwrap();
    drag_screen(&mut harness, from, to);
    harness.frame();
    harness.settle();
    let again = body_centre(&harness, feature);

    assert!(
        (again.x - centre.x + 5.0).abs() <= step,
        "{again:?} with steps of {step}"
    );
    assert!((again.y - 4.0).abs() < 1e-6, "{again:?}");
}

#[test]
fn dragging_a_hole_s_depth_arrow_changes_its_blind_depth() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    harness.select([top]);
    harness.click("Hole");
    harness.settle();
    harness.select([]);
    let hole = harness.workspace.editing.solid().expect("the hole is open");

    let step = drag_value_arrow(&mut harness, crate::value_gauges::Measured::HoleDepth, -4.0);
    let caditor_document::HoleDepth::Blind(depth) = open_hole(&harness, hole).depth else {
        panic!("the hole stays blind");
    };

    assert_eq!(harness.model.undo_label(), Some("Edit Hole 1"));
    assert!(
        (length_value(&harness, &depth) - 6.0).abs() <= step,
        "{depth:?} with steps of {step}"
    );
}

#[test]
fn dragging_an_offset_face_arrow_changes_its_distance() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    harness.select([top]);
    harness.use_tool_with(Key::Q, Modifiers::ALT);
    harness.settle();
    harness.select([]);
    let offset = harness
        .workspace
        .editing
        .solid()
        .expect("the offset face is open");

    let step = drag_value_arrow(
        &mut harness,
        crate::value_gauges::Measured::OffsetDistance,
        3.0,
    );
    let distance = length_value(&harness, &offset_of(&harness, offset).distance);

    assert_eq!(harness.model.undo_label(), Some("Edit Offset face 1"));
    assert!(
        (distance - 4.0).abs() <= step,
        "{distance} with steps of {step}"
    );
    assert!(volume_about(
        &harness,
        plate,
        40.0 * 40.0 * (10.0 + distance)
    ));
}

#[test]
fn dragging_a_datum_plane_s_arrow_changes_its_offset() {
    let mut harness = Harness::new();
    harness.select([]);
    harness.click("Plane");
    harness.settle();
    let plane = harness
        .workspace
        .editing
        .solid()
        .expect("the new plane is open");
    let offset_of = |harness: &Harness| {
        let Some(caditor_document::Datum::Plane(definition)) = harness
            .document()
            .feature(plane)
            .and_then(|feature| feature.kind.datum())
        else {
            panic!("the plane stays a plane");
        };
        length_value(harness, &definition.offset)
    };
    let before = offset_of(&harness);

    let step = drag_value_arrow(
        &mut harness,
        crate::value_gauges::Measured::PlaneOffset,
        5.0,
    );
    let offset = offset_of(&harness);

    assert_eq!(harness.model.undo_label(), Some("Edit Plane 1"));
    assert!(
        (offset - before - 5.0).abs() <= step,
        "{before} to {offset} with steps of {step}"
    );
}

#[test]
fn dragging_a_ring_turns_the_body_about_its_centre_and_the_panel_switches_to_the_origin() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    harness.select([top]);
    harness.click("Move body");
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let movement = harness.workspace.editing.solid().expect("the move is open");
    let about_z = Handle::Turn(MoveAxis::Z);
    let from = harness
        .workspace
        .viewport
        .handle_position(about_z, 3.0)
        .expect("the Z ring is shown");
    let to = harness
        .workspace
        .viewport
        .handle_position(about_z, 12.0)
        .unwrap();
    harness.events.push(Event::PointerMoved(from));
    harness.frame();
    let described = harness.shows("Drag to turn the body about Z through its centre");

    drag_screen(&mut harness, from, to);
    harness.frame();
    harness.settle();

    assert!(described);
    assert_eq!(harness.model.undo_label(), Some("Edit Move body 1"));
    let moved = |harness: &Harness| match harness
        .document()
        .feature(movement)
        .map(|feature| feature.kind.clone())
    {
        Some(FeatureKind::Move(moved)) => moved,
        other => panic!("the move is still a move: {other:?}"),
    };
    let turned = moved(&harness);
    assert_eq!(turned.about, caditor_document::TurnCentre::Body);
    let parameters = harness.model.parameters();
    let z = turned.turn[2]
        .evaluate_as(caditor_expression::Dimension::ANGLE, &|id| {
            parameters.value(id)
        })
        .unwrap();
    assert!((z - 45.0).abs() < 1e-9, "{z}");
    let bounds = harness
        .model
        .evaluation()
        .body(plate)
        .unwrap()
        .bounding_box()
        .unwrap();
    assert!((bounds.center().x - 20.0).abs() < 1e-6);
    assert!((bounds.max().x - bounds.min().x - 40.0 * 2.0_f64.sqrt()).abs() < 1e-6);

    assert!(harness.shows(crate::move_panel::TURN_ABOUT));
    harness.click(crate::move_panel::ABOUT_ORIGIN);
    harness.settle();
    assert_eq!(moved(&harness).about, caditor_document::TurnCentre::Origin);
    let bounds = harness
        .model
        .evaluation()
        .body(plate)
        .unwrap()
        .bounding_box()
        .unwrap();
    assert!(
        bounds.center().distance(caditor_geometry::Point3::new(
            0.0,
            20.0 * 2.0_f64.sqrt(),
            5.0
        )) < 1e-6
    );
}

#[test]
fn a_move_switched_to_an_axis_turns_about_it_by_dragging_its_ring() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let movement = opened_move(&mut harness, top);

    harness.click_beside(crate::move_panel::ABOUT_AXIS, crate::move_panel::TURN_ABOUT);
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let about = Handle::TurnAbout;
    let from = harness
        .workspace
        .viewport
        .handle_position(about, 3.0)
        .expect("the ring about the axis is shown");
    let to = harness
        .workspace
        .viewport
        .handle_position(about, 12.0)
        .unwrap();
    let ring_about_x = harness
        .workspace
        .viewport
        .handle_position(Handle::Turn(MoveAxis::X), 3.0);
    harness.events.push(Event::PointerMoved(from));
    harness.frame();
    let described = harness.shows("Drag to turn the body about its axis");
    let captioned = harness.shows(crate::move_panel::ANGLE);

    drag_screen(&mut harness, from, to);
    harness.frame();
    harness.settle();

    assert!(described);
    assert!(captioned);
    assert_eq!(ring_about_x, None);
    assert_eq!(harness.model.undo_label(), Some("Edit Move body 1"));
    let turned = move_of(&harness, movement);
    let turn = turned.about.axis_turn().expect("it turns about an axis");
    assert_eq!(
        turn.axis,
        caditor_document::AxisReference::Principal(caditor_document::PrincipalAxis::Z)
    );
    let parameters = harness.model.parameters();
    let angle = turn
        .angle
        .evaluate_as(caditor_expression::Dimension::ANGLE, &|id| {
            parameters.value(id)
        })
        .unwrap();
    assert!((angle - 45.0).abs() < 1e-9, "{angle}");
    let bounds = harness
        .model
        .evaluation()
        .body(plate)
        .unwrap()
        .bounding_box()
        .unwrap();
    assert!(
        bounds.center().distance(caditor_geometry::Point3::new(
            0.0,
            20.0 * 2.0_f64.sqrt(),
            5.0
        )) < 1e-6,
        "{:?}",
        bounds.center()
    );
}

#[test]
fn a_move_turns_about_a_selected_edge_from_its_panel() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let movement = opened_move(&mut harness, top);
    harness.click_beside(crate::move_panel::ABOUT_AXIS, crate::move_panel::TURN_ABOUT);
    harness.settle();
    let edge = harness
        .model
        .evaluation()
        .body(plate)
        .unwrap()
        .edges()
        .find(|(_, edge)| {
            let middle = edge.curve().point(edge.interval().middle());
            (middle - caditor_geometry::Point3::new(40.0, 0.0, 5.0)).length() < 1e-6
        })
        .map(|(_, edge)| edge.name())
        .expect("the plate has that upright edge");

    harness.select([Pickable::Edge { body: plate, edge }]);
    harness.frame();
    harness.click_button("Use selected");
    harness.settle();

    let turn = move_of(&harness, movement)
        .about
        .axis_turn()
        .cloned()
        .expect("it turns about an axis");
    assert!(
        matches!(turn.axis, caditor_document::AxisReference::Edge { body, .. } if body == plate),
        "{:?}",
        turn.axis
    );
    assert!(harness.shows("An edge of Extrude 1"));
}

#[test]
fn a_body_is_moved_by_distances_and_turns_typed_in_the_panel() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);

    harness.select([top]);
    harness.click("Move body");
    harness.settle();
    let movement = harness.workspace.editing.solid().expect("the move is open");
    assert_eq!(harness.model.undo_label(), Some("Create Move body 1"));
    assert!(harness.shows("Move along X"));
    assert!(harness.shows("Turn about Z"));

    harness.type_into_field(Id::new(("move-field", "offset", 0usize, movement)), "5 mm");
    harness.settle();
    let bounds = harness
        .model
        .evaluation()
        .body(plate)
        .unwrap()
        .bounding_box()
        .unwrap();
    assert!((bounds.min().x - 5.0).abs() < 1e-6, "{:?}", bounds.min());
    assert_eq!(harness.model.undo_label(), Some("Edit Move body 1"));
    let built = harness.built_with_meshes(2);
    let [moved] = &built.scene.meshes[..] else {
        panic!("the moved body is drawn solid");
    };
    let [ghost] = &built.scene.translucent_meshes[..] else {
        panic!("the body before the move is drawn see-through");
    };
    assert!(moved.faces.iter().all(|face| face.pick.is_some()));
    assert!(ghost.faces.iter().all(|face| face.pick.is_none()));

    harness.type_into_field(Id::new(("move-field", "turn", 2usize, movement)), "90 deg");
    harness.settle();
    let bounds = harness
        .model
        .evaluation()
        .body(plate)
        .unwrap()
        .bounding_box()
        .unwrap();
    assert!((bounds.min().x - 5.0).abs() < 1e-6, "{:?}", bounds.min());

    harness.type_into_field(Id::new(("move-field", "offset", 1usize, movement)), "5 deg");
    assert!(harness.shows_containing("length"));
    assert_eq!(harness.workspace.editing.solid(), Some(movement));

    harness.perform(Action::Undo);
    harness.perform(Action::Undo);
    harness.perform(Action::Undo);
    harness.settle();
    let bounds = harness
        .model
        .evaluation()
        .body(plate)
        .unwrap()
        .bounding_box()
        .unwrap();
    assert!(bounds.min().x.abs() < 1e-6);
}

#[test]
fn dragging_a_revolve_s_angle_arrows_turns_it_further() {
    let mut harness = Harness::new();
    let mut section = Sketch::new(Plane::XZ);
    rectangle(
        &mut section,
        Point2::new(10.0, 0.0),
        Point2::new(20.0, 10.0),
    );
    harness.add_sketch(section);
    harness.select([]);
    harness.click("Revolve");
    harness.settle();
    let revolve = open_solid(&harness);
    choose(&mut harness, "Full turn", "Two angles");
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let angles = |harness: &Harness| match harness.solid(revolve) {
        SolidFeature::Revolve(revolve) => match &revolve.extent {
            caditor_document::RevolveExtent::TwoSides { forward, backward } => (
                length_or_angle(harness, forward),
                length_or_angle(harness, backward),
            ),
            other => panic!("expected two angles, not {other:?}"),
        },
        SolidFeature::Extrude(_) | SolidFeature::Sweep(_) => panic!("expected a revolve"),
    };
    let (forward, backward) = angles(&harness);
    let handle = Handle::Revolve(crate::turn_handles::TurnEnd::Forward);
    let from = harness
        .workspace
        .viewport
        .handle_position(handle, 0.0)
        .expect("the forward arrow is shown");
    let to = harness
        .workspace
        .viewport
        .handle_position(handle, 30.0)
        .unwrap();
    harness.events.push(Event::PointerMoved(from));
    harness.frame();

    assert!(harness.shows("Drag to change the revolve's forward"));

    drag_screen(&mut harness, from, to);
    harness.frame();
    harness.settle();
    let (turned, kept) = angles(&harness);

    assert_eq!(harness.model.undo_label(), Some("Edit Revolve 1"));
    assert!(
        (turned - forward - 30.0).abs() <= 5.0,
        "{forward} to {turned}"
    );
    assert_eq!(kept, backward);
}

#[test]
fn an_extrusion_starting_at_a_face_shows_and_drags_its_arrow_where_it_ends() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let tower = extrusion_above_plate(&mut harness, 30.0);
    choose(&mut harness, "New body", "Add to body");
    open_combo(&mut harness, "Start");
    harness.click_lowest("Face or plane");
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), top);
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let reach = Handle::Reach(crate::move_manipulator::Reach::Only);
    let highest = |harness: &Harness| {
        harness
            .model
            .evaluation()
            .body(plate)
            .and_then(|solid| solid.bounding_box())
            .map(|bounds| bounds.max().z)
            .unwrap()
    };

    let started = start_of_solid(&harness, tower);
    let foot = harness
        .workspace
        .viewport
        .handle_foot(reach)
        .expect("the distance arrow is shown");
    let from = harness
        .workspace
        .viewport
        .handle_position(reach, 0.0)
        .unwrap();
    let to = harness
        .workspace
        .viewport
        .handle_position(reach, 3.0)
        .unwrap();
    let step = harness.workspace.viewport.manipulator_step().unwrap();

    assert!(matches!(
        started,
        Some(caditor_document::SolidStart::Plane(_))
    ));
    assert!(
        (foot - caditor_geometry::Point3::new(15.0, 15.0, 20.0)).length() < 1e-9,
        "{foot:?}"
    );

    drag_screen(&mut harness, from, to);
    harness.frame();
    harness.settle();

    assert_eq!(harness.model.undo_label(), Some("Edit Extrude 2"));
    assert!(
        (highest(&harness) - 23.0).abs() <= step,
        "{} with steps of {step}",
        highest(&harness)
    );
    assert_eq!(start_of_solid(&harness, tower), started);
}

#[test]
fn a_revolve_starting_off_its_sketch_plane_shows_and_drags_its_angle_arrows_there() {
    let mut harness = Harness::new();
    let mut section = Sketch::new(Plane::XZ);
    rectangle(
        &mut section,
        Point2::new(10.0, 0.0),
        Point2::new(20.0, 10.0),
    );
    harness.add_sketch(section);
    harness.select([]);
    harness.click("Revolve");
    harness.settle();
    let revolve = open_solid(&harness);
    choose(&mut harness, "Full turn", "Two angles");
    harness.type_into_field(Id::new(("solid-field", "start", revolve)), "5 mm");
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let forward_angle = |harness: &Harness| match harness.solid(revolve) {
        SolidFeature::Revolve(revolve) => match &revolve.extent {
            caditor_document::RevolveExtent::TwoSides { forward, .. } => {
                length_or_angle(harness, forward)
            }
            other => panic!("expected two angles, not {other:?}"),
        },
        SolidFeature::Extrude(_) | SolidFeature::Sweep(_) => panic!("expected a revolve"),
    };
    let handle = Handle::Revolve(crate::turn_handles::TurnEnd::Forward);

    let forward = forward_angle(&harness);
    let foot = harness
        .workspace
        .viewport
        .handle_foot(handle)
        .expect("the forward arrow is shown");
    let from = harness
        .workspace
        .viewport
        .handle_position(handle, 0.0)
        .unwrap();
    let to = harness
        .workspace
        .viewport
        .handle_position(handle, 30.0)
        .unwrap();

    assert!((foot.z - 5.0).abs() < 1e-9, "{foot:?}");
    assert!((foot.x.hypot(foot.y + 5.0) - 15.0).abs() < 1e-9, "{foot:?}");

    drag_screen(&mut harness, from, to);
    harness.frame();
    harness.settle();
    let turned = forward_angle(&harness);

    assert_eq!(harness.model.undo_label(), Some("Edit Revolve 1"));
    assert!(
        (turned - forward - 30.0).abs() <= 5.0,
        "{forward} to {turned}"
    );
}

fn length_or_angle(harness: &Harness, expression: &Expression) -> f64 {
    harness
        .model
        .parameters()
        .evaluate_expression(expression)
        .unwrap()
        .value
}
