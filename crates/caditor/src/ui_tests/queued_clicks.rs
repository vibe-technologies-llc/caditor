use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;
use egui::{Key, Modifiers};

use super::{Harness, extruded_plate, offer, pickable_described, rectangle, shell_of};
use crate::{commands::Command, reference_picking::Slot};

const WAITING: &str = "waiting for the model to recompute";

fn plate_with_a_body_still_computing(harness: &mut Harness) -> usize {
    extruded_plate(harness);
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(
        &mut sketch,
        Point2::new(60.0, 0.0),
        Point2::new(100.0, 40.0),
    );
    harness.add_sketch(sketch);
    harness.select([]);
    let before = harness.document().features().count();

    harness.model.hold_evaluations(true);
    harness.click("Extrude");
    harness.frame();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    let second = harness
        .document()
        .features()
        .find(|feature| feature.name == "Extrude 2")
        .map(|feature| feature.id())
        .expect("the second extrusion exists");
    harness.workspace.panels.choose_only(second);
    harness.frame();
    harness.frame();
    before
}

#[test]
fn a_ribbon_button_blocked_only_by_the_recompute_queues_its_command_and_runs_it_when_the_model_is_ready()
 {
    let mut harness = Harness::new();
    let before = plate_with_a_body_still_computing(&mut harness);

    let unavailable = offer(&harness, Command::Move).availability;
    harness.click("Move body");
    harness.frame();
    let queued = harness.model.queued().is_some();
    let waiting = harness.shows_containing(WAITING);
    let opened = harness.workspace.editing.solid().is_some();
    let features = harness.document().features().count();

    harness.model.hold_evaluations(false);
    harness.settle();

    assert!(unavailable.is_err());
    assert!(queued);
    assert!(waiting);
    assert!(!opened);
    assert_eq!(features, before + 1);
    assert!(harness.model.queued().is_none());
    assert!(harness.workspace.editing.solid().is_some());
    assert_eq!(harness.document().features().count(), before + 2);
    assert_eq!(harness.model.undo_label(), Some("Create Move body 1"));
}

#[test]
fn a_menu_item_blocked_only_by_the_recompute_queues_its_command() {
    let mut harness = Harness::new();
    plate_with_a_body_still_computing(&mut harness);

    harness.click("Model");
    harness.click("Bodies");
    harness.click_lowest("Move body");
    harness.frame();
    let queued = harness.model.queued().is_some();
    let waiting = harness.shows_containing(WAITING);

    harness.model.hold_evaluations(false);
    harness.settle();

    assert!(queued);
    assert!(waiting);
    assert!(harness.model.queued().is_none());
    assert!(harness.workspace.editing.solid().is_some());
}

#[test]
fn a_ribbon_button_blocked_for_another_reason_stays_disabled() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    harness.select([]);
    harness.frame();

    let unavailable = offer(&harness, Command::Move).availability;
    harness.click("Move body");
    harness.frame();

    assert!(unavailable.is_err());
    assert!(harness.model.queued().is_none());
    assert!(!harness.shows_containing(WAITING));
    assert!(harness.workspace.editing.solid().is_none());
}

#[test]
fn a_click_giving_a_shell_face_its_own_thickness_waits_for_the_recompute_and_then_applies() {
    let mut harness = Harness::new();
    let (_plate, top) = extruded_plate(&mut harness);
    let bottom = pickable_described(&mut harness, "Extrude 1 › Extrude 1 start face");
    harness.select([top]);

    harness.model.hold_evaluations(true);
    harness.click("Shell");
    harness.frame();
    let shell = harness
        .workspace
        .editing
        .solid()
        .expect("the shell is open");
    harness.click("Choose faces");
    harness.frame();
    assert_eq!(
        harness
            .workspace
            .editing
            .picking()
            .map(|picking| picking.slot),
        Some(Slot::ShellOwnThickness)
    );

    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), bottom);
    let queued = harness.model.queued().is_some();
    let waiting = harness.shows_containing(WAITING);
    let faces_meanwhile = shell_of(&harness, shell).faces.len();

    harness.model.hold_evaluations(false);
    harness.settle();

    assert!(queued);
    assert!(waiting);
    assert_eq!(faces_meanwhile, 0);
    assert!(harness.model.queued().is_none());
    assert_eq!(shell_of(&harness, shell).faces.len(), 1);
    assert_eq!(
        harness.model.undo_label(),
        Some("Give a face of Shell 1 its own thickness")
    );
    assert!(harness.workspace.editing.picking().is_some());
}

#[test]
fn escape_drops_a_queued_own_thickness_click_and_the_shell_keeps_its_walls() {
    let mut harness = Harness::new();
    let (_plate, top) = extruded_plate(&mut harness);
    let bottom = pickable_described(&mut harness, "Extrude 1 › Extrude 1 start face");
    harness.select([top]);

    harness.model.hold_evaluations(true);
    harness.click("Shell");
    harness.frame();
    let shell = harness
        .workspace
        .editing
        .solid()
        .expect("the shell is open");
    harness.click("Choose faces");
    harness.frame();
    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), bottom);
    let queued = harness.model.queued().is_some();

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let dropped = harness.model.queued().is_none();
    harness.model.hold_evaluations(false);
    harness.settle();

    assert!(queued);
    assert!(dropped);
    assert_eq!(shell_of(&harness, shell).faces.len(), 0);
}
