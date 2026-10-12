use caditor_document::ShellSide;
use caditor_geometry::{Plane, Point2};
use egui::{Id, Key, Modifiers};

use super::{Harness, choose, extruded_plate, pickable_described, shell_of};
use crate::{model::Action, reference_picking::Slot, selection::Pickable};

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
