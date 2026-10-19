use caditor_document::{Draft, FeatureId, PlaneReference};
use caditor_geometry::{Plane, Point2};
use caditor_kernel::Surface;
use egui::{Event, Id, Key, Modifiers};

use super::{
    CAMERA_SETTLE, Harness, Wheel, drag_value_arrow, extruded_plate, length_value,
    run_from_palette, volume_about,
};
use crate::{
    bodies, draft_panel, move_manipulator::Handle, reference_picking::Slot, selection::Pickable,
    value_gauges::Measured,
};

const SIDE: f64 = 40.0;
const HEIGHT: f64 = 10.0;

fn draft_of(harness: &Harness, feature: FeatureId) -> &Draft {
    harness
        .document()
        .feature(feature)
        .and_then(|feature| feature.kind.draft())
        .unwrap()
}

fn walls(harness: &mut Harness) -> Vec<Pickable> {
    let faces: Vec<Pickable> = harness
        .built()
        .picks
        .pickables()
        .filter(|pickable| matches!(pickable, Pickable::Face { .. }))
        .collect();
    faces
        .into_iter()
        .filter(|pickable| {
            let Pickable::Face { body, face } = *pickable else {
                return false;
            };
            let Some(result) = harness
                .model
                .evaluation()
                .body_result(body)
                .and_then(|result| result.solid())
            else {
                return false;
            };
            bodies::find_face(result, face)
                .and_then(|found| result.solid.face(found))
                .is_some_and(|face| match face.surface() {
                    Surface::Plane(plane) => plane.frame().normal().z.abs() < 0.5,
                    _ => false,
                })
        })
        .collect()
}

fn slope(degrees: f64) -> f64 {
    degrees.to_radians().tan()
}

fn drafted(sides: [f64; 4]) -> f64 {
    let [east, west, north, south] = sides.map(slope);
    let area = |z: f64| (SIDE - (east + west) * z) * (SIDE - (north + south) * z);
    HEIGHT / 6.0 * (area(0.0) + 4.0 * area(HEIGHT / 2.0) + area(HEIGHT))
}

fn close_feature(harness: &mut Harness) {
    for _ in 0..2 {
        harness.key(Key::Escape, Modifiers::NONE);
        harness.frame();
        harness.frame();
    }
}

#[test]
fn the_palette_drafts_selected_walls_from_their_floor_and_a_clicked_wall_takes_its_own_angle() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let sides = walls(&mut harness);
    let Some(Pickable::Face { face: first, .. }) = sides.first().copied() else {
        panic!("the plate has walls");
    };

    assert_eq!(sides.len(), 4);

    harness.select(sides);
    run_from_palette(&mut harness, "Draft");
    harness.settle();
    let draft = harness
        .workspace
        .editing
        .solid()
        .expect("the draft is open");

    assert_eq!(harness.model.undo_label(), Some("Create Draft 1"));
    assert_eq!(draft_of(&harness, draft).faces.len(), 4);
    assert!(matches!(
        draft_of(&harness, draft).neutral,
        PlaneReference::Face(_)
    ));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert!(volume_about(&harness, plate, drafted([3.0; 4])));
    assert!(harness.shows("Faces to draft"));
    assert!(harness.shows("Click faces to draft them, and again to leave one out"));
    assert!(harness.shows_containing("Toward +Z"));

    harness.type_into_field(Id::unique(("draft-angle", "Draft angle", draft)), "5 deg");
    harness.settle();

    assert!(volume_about(&harness, plate, drafted([5.0; 4])));

    harness.click("Choose faces");
    harness.frame();

    assert_eq!(
        harness
            .workspace
            .editing
            .picking()
            .map(|picking| picking.slot),
        Some(Slot::DraftOwnAngle)
    );

    harness.click_pickable(
        Plane::XY,
        Point2::new(20.0, 20.0),
        Pickable::ShellFace {
            feature: draft,
            face: first,
        },
    );
    harness.settle();

    assert_eq!(draft_of(&harness, draft).own.len(), 1);
    assert_eq!(draft_of(&harness, draft).faces.len(), 3);
    assert_eq!(
        harness.model.undo_label(),
        Some("Give a face of Draft 1 its own draft angle")
    );

    harness.type_into_field(Id::unique(("draft-own-angle", draft, 0_usize)), "10 deg");
    harness.settle();

    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert!(volume_about(
        &harness,
        plate,
        drafted([10.0, 5.0, 5.0, 5.0])
    ));

    close_feature(&mut harness);
    for _ in 0..4 {
        harness.key(Key::Z, Modifiers::COMMAND);
        harness.frame();
    }
    harness.settle();

    assert!(volume_about(&harness, plate, SIDE * SIDE * HEIGHT));
}

#[test]
fn flipping_the_pull_widens_the_walls_and_the_split_adds_an_angle_beyond() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let sides = walls(&mut harness);

    harness.select(sides);
    run_from_palette(&mut harness, "Draft");
    harness.settle();
    let draft = harness
        .workspace
        .editing
        .solid()
        .expect("the draft is open");
    let flipped = draft_of(&harness, draft).flipped;
    harness.hold_still();
    harness.click(draft_panel::FLIP);
    harness.settle();

    assert_eq!(harness.model.undo_label(), Some("Edit Draft 1"));
    assert_ne!(draft_of(&harness, draft).flipped, flipped);
    assert!(harness.shows_containing("Toward -Z"));
    assert!(volume_about(&harness, plate, drafted([-3.0; 4])));

    let panel = harness.position_of("Pull direction");
    harness.wheel_until_shown(panel, draft_panel::SPLIT, Wheel::Down);
    harness.hold_still();
    harness.click(draft_panel::SPLIT);
    harness.settle();

    assert!(draft_of(&harness, draft).beyond.is_some());
    let panel = harness.position_of(draft_panel::SPLIT);
    harness.wheel_until_shown(panel, "Angle beyond", Wheel::Down);
    assert!(harness.shows("Angle beyond"));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert_eq!(harness.model.undo_label(), Some("Edit Draft 1"));
}

#[test]
fn dragging_the_draft_angle_arrow_changes_the_angle_in_one_undoable_change() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let sides = walls(&mut harness);

    harness.select(sides);
    run_from_palette(&mut harness, "Draft");
    harness.settle();
    harness.select([]);
    let draft = harness
        .workspace
        .editing
        .solid()
        .expect("the draft is open");

    let step = drag_value_arrow(&mut harness, Measured::DraftAngle, 15.0);
    let angle = length_value(&harness, &draft_of(&harness, draft).angle);

    assert_eq!(harness.model.undo_label(), Some("Edit Draft 1"));
    assert!((angle - 18.0).abs() <= step, "{angle} with steps of {step}");
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert!(volume_about(&harness, plate, drafted([angle; 4])));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.settle();

    assert!((length_value(&harness, &draft_of(&harness, draft).angle) - 3.0).abs() < 1e-9);
}

#[test]
fn an_angle_typed_over_the_draft_arrow_sets_the_draft_angle() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let sides = walls(&mut harness);

    harness.select(sides);
    run_from_palette(&mut harness, "Draft");
    harness.settle();
    harness.select([]);
    let draft = harness
        .workspace
        .editing
        .solid()
        .expect("the draft is open");

    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let at = harness
        .workspace
        .viewport
        .handle_position(Handle::Value(Measured::DraftAngle), 0.0)
        .expect("the draft angle arrow is shown");
    harness.events.push(Event::PointerMoved(at));
    harness.frame();
    harness.frame();
    harness.type_text("6");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.settle();

    assert_eq!(harness.model.undo_label(), Some("Edit Draft 1"));
    assert!((length_value(&harness, &draft_of(&harness, draft).angle) - 6.0).abs() < 1e-9);
    assert!(volume_about(&harness, plate, drafted([6.0; 4])));
}
