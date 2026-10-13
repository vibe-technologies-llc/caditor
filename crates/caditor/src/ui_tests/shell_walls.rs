use caditor_document::{FeatureId, ShellSide};
use caditor_geometry::{Plane, Point2, Point3};
use caditor_sketch::Sketch;
use egui::{Id, Key, Modifiers};

use super::{
    CAMERA_SETTLE, Harness, choose, drag_value_arrow, extruded_plate, length_value,
    pickable_described, shell_of,
};
use crate::{
    model::Action, move_manipulator::Handle, reference_picking::Slot, selection::Pickable,
    value_gauges::Measured,
};

const PLATE: f64 = 40.0 * 40.0 * 10.0;

fn about(found: f64, expected: f64) -> bool {
    (found - expected).abs() < 0.01 * expected
}

#[test]
fn a_shell_grows_its_walls_outside_and_gives_a_clicked_face_its_own_thickness() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    harness.select([top]);
    harness.click("Shell");
    harness.settle();
    let shell = harness
        .workspace
        .editing
        .solid()
        .expect("the shell is open");
    let bottom = pickable_described(
        &mut harness,
        "Extrude 1 start face: click to open it in Shell 1 or close it again",
    );
    let Pickable::ShellFace { face: top_key, .. } = pickable_described(
        &mut harness,
        "Extrude 1 end face: click to open it in Shell 1 or close it again",
    ) else {
        panic!("the top is a face of the shell");
    };

    choose(&mut harness, "Inside", "Outside");

    assert_eq!(shell_of(&harness, shell).side, ShellSide::Outside);
    assert_eq!(
        harness.model.undo_label(),
        Some("Set the walls of Shell 1 to outside")
    );
    assert!(about(
        harness.body_volume(plate),
        42.0 * 42.0 * 11.0 - PLATE
    ));

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
    harness.settle();

    assert_eq!(shell_of(&harness, shell).faces.len(), 1);
    assert_eq!(
        harness.model.undo_label(),
        Some("Give a face of Shell 1 its own thickness")
    );
    assert!(harness.workspace.editing.picking().is_some());

    harness.type_into_field(Id::new(("shell-own-thickness", shell, 0_usize)), "3 mm");
    harness.settle();
    assert!(about(
        harness.body_volume(plate),
        42.0 * 42.0 * 13.0 - PLATE
    ));

    harness.click_pickable(
        Plane::XY,
        Point2::new(20.0, 20.0),
        Pickable::ShellFace {
            feature: shell,
            face: top_key,
        },
    );
    harness.settle();
    assert!(harness.shows_containing("That face is open, so it has no wall"));
    assert_eq!(shell_of(&harness, shell).faces.len(), 1);

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert!(harness.workspace.editing.picking().is_none());

    choose(&mut harness, "Outside", "Both sides");
    assert!(about(
        harness.body_volume(plate),
        41.0 * 41.0 * 11.5 - 39.0 * 39.0 * 8.5,
    ));

    harness.perform(Action::Undo);
    harness.settle();
    assert_eq!(shell_of(&harness, shell).side, ShellSide::Outside);
    assert!(about(
        harness.body_volume(plate),
        42.0 * 42.0 * 13.0 - PLATE
    ));
}

fn shell_with_an_own_thickness_bottom(harness: &mut Harness) -> FeatureId {
    let (_, top) = extruded_plate(harness);
    harness.select([top]);
    harness.click("Shell");
    harness.settle();
    let shell = harness
        .workspace
        .editing
        .solid()
        .expect("the shell is open");
    let bottom = pickable_described(
        harness,
        "Extrude 1 start face: click to open it in Shell 1 or close it again",
    );
    harness.click("Choose faces");
    harness.frame();
    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), bottom);
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    shell
}

fn own_thickness(harness: &Harness, shell: FeatureId) -> f64 {
    let own = &shell_of(harness, shell).faces[0];
    length_value(harness, &own.thickness)
}

#[test]
fn dragging_a_face_s_own_thickness_arrow_changes_that_face_s_wall_in_one_undoable_change() {
    let mut harness = Harness::new();
    let shell = shell_with_an_own_thickness_bottom(&mut harness);
    let before = own_thickness(&harness, shell);
    let shells = shell_of(&harness, shell).thickness.clone();

    let step = drag_value_arrow(&mut harness, Measured::ShellFaceThickness(0), 2.0);
    let after = own_thickness(&harness, shell);

    assert_eq!(harness.model.undo_label(), Some("Edit Shell 1"));
    assert!(
        (after - before - 2.0).abs() <= step,
        "{before} to {after} with steps of {step}"
    );
    assert_eq!(shell_of(&harness, shell).thickness, shells);

    harness.perform(Action::Undo);
    harness.settle();

    assert_eq!(own_thickness(&harness, shell), before);
}

#[test]
fn a_face_s_own_thickness_arrow_stands_on_that_face_and_follows_the_walls_side() {
    let mut harness = Harness::new();
    let shell = shell_with_an_own_thickness_bottom(&mut harness);
    let own = own_thickness(&harness, shell);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let handle = Handle::Value(Measured::ShellFaceThickness(0));

    let inside = harness.workspace.viewport.handle_foot(handle).unwrap();

    assert!(
        inside.distance(Point3::new(20.0, 20.0, own)) < 1e-6,
        "{inside:?}"
    );

    choose(&mut harness, "Inside", "Outside");
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let outside = harness.workspace.viewport.handle_foot(handle).unwrap();

    assert!(
        outside.distance(Point3::new(20.0, 20.0, -own)) < 1e-6,
        "{outside:?}"
    );

    choose(&mut harness, "Outside", "Both sides");
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let both = harness.workspace.viewport.handle_foot(handle).unwrap();

    assert!(
        both.distance(Point3::new(20.0, 20.0, -own / 2.0)) < 1e-6,
        "{both:?}"
    );
}

#[test]
fn a_curved_face_s_own_thickness_arrow_stands_on_its_surface_and_drags() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_circle(Point2::ZERO, 20.0);
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click_tool("Extrude");
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let top = pickable_described(&mut harness, "Extrude 1 › Extrude 1 end face");
    harness.select([top]);
    harness.click("Shell");
    harness.settle();
    let shell = harness
        .workspace
        .editing
        .solid()
        .expect("the shell is open");
    let side = pickable_described(
        &mut harness,
        "Extrude 1 side from Circle 1: click to open it in Shell 1 or close it again",
    );
    harness.click("Choose faces");
    harness.frame();
    harness.click_pickable(Plane::XY, Point2::new(20.0, 0.0), side);
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let own = own_thickness(&harness, shell);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let handle = Handle::Value(Measured::ShellFaceThickness(0));

    let foot = harness.workspace.viewport.handle_foot(handle).unwrap();

    assert!(
        (foot.x.hypot(foot.y) - (20.0 - own)).abs() < 1e-6,
        "{foot:?} for a wall of {own}"
    );

    let step = drag_value_arrow(&mut harness, Measured::ShellFaceThickness(0), 1.0);
    let after = own_thickness(&harness, shell);

    assert!(
        (after - own - 1.0).abs() <= step,
        "{own} to {after} with steps of {step}"
    );
}
