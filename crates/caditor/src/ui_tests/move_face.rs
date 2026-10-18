use std::f64::consts::PI;

use caditor_document::{AxisReference, FeatureId, MoveFace, PrincipalAxis};
use caditor_geometry::{Plane, Point2};
use caditor_kernel::Surface;
use egui::{Id, Key, Modifiers};

use super::{Harness, delete_face::holed_plate, volume_about};
use crate::{
    editing::EditingCommand,
    model::Action,
    move_face_panel,
    reference_picking::{Picking, Slot},
    selection::{Axis, Pickable},
};

const PLATE: f64 = 40.0 * 40.0 * 10.0;
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
