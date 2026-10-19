use std::f64::consts::PI;

use caditor_document::{
    AxisReference, Edit, FeatureId, FeatureKind, MoveFace, PrincipalAxis, Transaction,
};
use caditor_geometry::{Plane, Point2};
use caditor_kernel::Surface;
use egui::{Event, Id, Key, Modifiers};

use super::{
    CAMERA_SETTLE, Harness, delete_face::holed_plate, drag_value_arrow, extruded_plate,
    length_value, volume_about,
};
use crate::{
    editing::EditingCommand,
    model::Action,
    move_face_panel,
    move_manipulator::Handle,
    reference_picking::{Picking, Slot},
    selection::{Axis, Pickable},
    value_gauges::Measured,
};

const PLATE: f64 = 40.0 * 40.0 * 10.0;
const PLATE_SIDE: f64 = 40.0 * 40.0;
const BORE: f64 = PI * 25.0 * 10.0;

fn move_face_of(harness: &Harness, feature: FeatureId) -> &MoveFace {
    harness
        .document()
        .feature(feature)
        .and_then(|feature| feature.kind.move_face())
        .unwrap()
}

fn bore_centre_x(harness: &Harness, plate: FeatureId) -> f64 {
    let solid = harness.model.evaluation().body(plate).unwrap();
    solid
        .faces()
        .find_map(|(_, face)| match face.surface() {
            Surface::Cylinder(cylinder) => Some(cylinder.frame().origin().x),
            _ => None,
        })
        .unwrap()
}

fn close_feature(harness: &mut Harness) {
    for _ in 0..2 {
        harness.key(Key::Escape, Modifiers::NONE);
        harness.frame();
        harness.frame();
    }
}

#[test]
fn move_face_raises_a_top_and_takes_or_leaves_faces_clicked_in_the_view() {
    let mut harness = Harness::new();
    let (plate, bore, top) = holed_plate(&mut harness);
    let Pickable::Face { face: bore_key, .. } = bore else {
        panic!("the bore is a face");
    };

    harness.select([top]);
    harness.use_tool_with(Key::J, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();
    let moving = harness
        .workspace
        .editing
        .solid()
        .expect("the move face is open");
    assert_eq!(harness.model.undo_label(), Some("Create Move face 1"));
    assert_eq!(move_face_of(&harness, moving).faces.len(), 1);
    assert!(volume_about(&harness, plate, (PLATE - BORE) * 1.1));
    assert!(harness.shows("Click faces to move them, and again to leave one out"));
    assert!(harness.shows("Faces to move"));
    assert!(harness.shows(move_face_panel::SQUARE_TO_FACE));
    assert!(harness.shows(move_face_panel::NO_TILT));

    harness.click_pickable(
        Plane::XY,
        Point2::new(20.0, 20.0),
        Pickable::ShellFace {
            feature: moving,
            face: bore_key,
        },
    );
    harness.settle();
    assert_eq!(move_face_of(&harness, moving).faces.len(), 2);
    assert_eq!(
        harness.model.undo_label(),
        Some("Move a face with Move face 1")
    );
    assert_eq!(harness.model.evaluation().failed_count(), 0);

    harness.click_pickable(
        Plane::XY,
        Point2::new(20.0, 20.0),
        Pickable::ShellFace {
            feature: moving,
            face: bore_key,
        },
    );
    harness.settle();
    assert_eq!(move_face_of(&harness, moving).faces.len(), 1);
    assert_eq!(
        harness.model.undo_label(),
        Some("Leave a face out of Move face 1")
    );

    close_feature(&mut harness);
    assert_eq!(harness.workspace.editing.solid(), None);

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.settle();
    assert!(volume_about(&harness, plate, PLATE - BORE));
}

#[test]
fn a_bore_moves_along_an_axis_chosen_in_the_view_by_a_typed_distance() {
    let mut harness = Harness::new();
    let (plate, bore, _) = holed_plate(&mut harness);

    harness.select([bore]);
    harness.use_tool_with(Key::J, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();
    let moving = harness
        .workspace
        .editing
        .solid()
        .expect("the move face is open");
    assert_eq!(harness.model.evaluation().failed_count(), 1);
    assert!(harness.shows_containing("no flat face is chosen"));

    harness.perform(Action::Editing(EditingCommand::Pick(Picking::new(
        moving,
        Slot::MoveFaceDirection,
    ))));
    harness.frame();
    harness.click_pickable(Plane::XY, Point2::new(0.0, 0.0), Pickable::Axis(Axis::X));
    harness.settle();
    assert_eq!(
        move_face_of(&harness, moving).direction,
        Some(AxisReference::Principal(PrincipalAxis::X))
    );
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert!((bore_centre_x(&harness, plate) - 21.0).abs() < 1e-9);

    harness.type_into_field(Id::unique(("move-face-value", "Distance", moving)), "-8 mm");
    harness.settle();
    assert!((bore_centre_x(&harness, plate) - 12.0).abs() < 1e-9);
    assert!(volume_about(&harness, plate, PLATE - BORE));
}

fn open_on_plate_top(harness: &mut Harness) -> (FeatureId, FeatureId) {
    let (plate, top) = extruded_plate(harness);
    harness.select([top]);
    harness.use_tool_with(Key::J, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();
    harness.select([]);
    let moving = harness
        .workspace
        .editing
        .solid()
        .expect("the move face is open");
    (plate, moving)
}

fn tilt_about_x(harness: &mut Harness, moving: FeatureId) {
    let mut kind = move_face_of(harness, moving).clone();
    kind.axis = Some(AxisReference::Principal(PrincipalAxis::X));
    let transaction = Transaction::new(
        "Tilt about X",
        vec![Edit::SetFeatureKind {
            id: moving,
            kind: FeatureKind::MoveFace(kind),
        }],
    );
    harness.perform(Action::Apply(transaction));
    harness.settle();
}

fn hover_arrow(harness: &mut Harness, measured: Measured) {
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let at = harness
        .workspace
        .viewport
        .handle_position(Handle::Value(measured), 0.0)
        .expect("the arrow is shown");
    harness.events.push(Event::PointerMoved(at));
    harness.frame();
    harness.frame();
}

#[test]
fn dragging_the_move_face_distance_arrow_shifts_the_faces_in_one_undoable_change() {
    let mut harness = Harness::new();
    let (plate, moving) = open_on_plate_top(&mut harness);
    let undo_label = harness.model.undo_label().map(str::to_owned);

    let step = drag_value_arrow(&mut harness, Measured::MoveDistance, 3.0);
    let distance = length_value(&harness, &move_face_of(&harness, moving).distance);

    assert_eq!(harness.model.undo_label(), Some("Edit Move face 1"));
    assert!((distance - 4.0).abs() <= step, "{distance} with {step}");
    assert!(volume_about(
        &harness,
        plate,
        PLATE_SIDE * (10.0 + distance)
    ));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.settle();

    assert_eq!(harness.model.undo_label().map(str::to_owned), undo_label);
    assert!((length_value(&harness, &move_face_of(&harness, moving).distance) - 1.0).abs() < 1e-9);
}

#[test]
fn a_distance_typed_over_the_move_face_arrow_may_be_an_expression() {
    let mut harness = Harness::new();
    let (plate, moving) = open_on_plate_top(&mut harness);

    hover_arrow(&mut harness, Measured::MoveDistance);
    harness.type_text("2 * 3");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.settle();

    assert_eq!(harness.model.undo_label(), Some("Edit Move face 1"));
    assert!((length_value(&harness, &move_face_of(&harness, moving).distance) - 6.0).abs() < 1e-9);
    assert!(volume_about(&harness, plate, PLATE_SIDE * 16.0));
}

#[test]
fn the_move_face_angle_arrow_stands_only_with_a_tilt_axis_and_drags_the_angle() {
    let mut harness = Harness::new();
    let (_, moving) = open_on_plate_top(&mut harness);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();

    assert!(
        harness
            .workspace
            .viewport
            .handle_position(Handle::Value(Measured::MoveAngle), 0.0)
            .is_none()
    );

    tilt_about_x(&mut harness, moving);
    drag_value_arrow(&mut harness, Measured::MoveAngle, 10.0);
    let angle = length_value(&harness, &move_face_of(&harness, moving).angle);

    assert_eq!(harness.model.undo_label(), Some("Edit Move face 1"));
    assert!((angle - 10.0).abs() <= 5.0, "{angle}");
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}
