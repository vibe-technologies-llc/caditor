use caditor_document::FeatureId;
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use egui::{Event, Modifiers, PointerButton, Pos2, vec2};

use super::{CAMERA_SETTLE, Harness, add_block, run_from_palette};
use crate::{
    selection::Pickable,
    viewport::{HOLD_ALT_HINT, LET_GO_OF_ALT_HINT, TAKING_HIDDEN_PROMPT, TAKING_SEEN_PROMPT},
};

const BOX_HALF_SIZE: f32 = 8.0;

struct Blocks {
    front: FeatureId,
    back: FeatureId,
    centre: Pos2,
}

fn one_block_behind_another(harness: &mut Harness) -> Blocks {
    let front = add_block(
        harness,
        "Front",
        [Point2::new(0.0, 0.0), Point2::new(40.0, 10.0)],
        "20 mm",
    );
    let back = add_block(
        harness,
        "Back",
        [Point2::new(0.0, 30.0), Point2::new(40.0, 40.0)],
        "20 mm",
    );
    harness.select([]);
    run_from_palette(harness, "view from the front");
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    run_from_palette(harness, "fit view");
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.frame();

    let middle_height =
        Plane::from_frame(Point3::new(0.0, 0.0, 10.0), Vector3::Z, Vector3::X).unwrap();
    let centre = harness
        .workspace
        .viewport
        .screen_position(middle_height, Point2::new(20.0, 0.0))
        .expect("the front face is in view");
    Blocks {
        front,
        back,
        centre,
    }
}

fn press(harness: &mut Harness, at: Pos2, modifiers: Modifiers) {
    harness.events.push(Event::ModifiersChanged(modifiers));
    harness.events.push(Event::PointerMoved(at));
    harness.frame();
    harness.events.push(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: true,
        modifiers,
    });
    harness.frame();
}

fn move_to(harness: &mut Harness, at: Pos2) {
    harness.events.push(Event::PointerMoved(at));
    harness.frame();
}

fn release(harness: &mut Harness, at: Pos2, modifiers: Modifiers) {
    harness.events.push(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: false,
        modifiers,
    });
    harness.frame();
    harness
        .events
        .push(Event::ModifiersChanged(Modifiers::NONE));
    harness.frame();
}

fn faces_of(harness: &Harness, body: FeatureId) -> usize {
    harness
        .workspace
        .viewport
        .selection()
        .iter()
        .filter(|pickable| matches!(pickable, Pickable::Face { body: owner, .. } if *owner == body))
        .count()
}

#[test]
fn holding_alt_during_a_box_also_takes_the_faces_hidden_behind_the_front_body() {
    let mut harness = Harness::new();
    let blocks = one_block_behind_another(&mut harness);
    let from = blocks.centre + vec2(BOX_HALF_SIZE, BOX_HALF_SIZE);
    let to = blocks.centre - vec2(BOX_HALF_SIZE, BOX_HALF_SIZE);
    let midway = from.lerp(to, 0.5);

    press(&mut harness, from, Modifiers::NONE);
    move_to(&mut harness, midway);
    move_to(&mut harness, to);
    let offered = harness.shows(TAKING_SEEN_PROMPT) && harness.shows_hint(HOLD_ALT_HINT);
    release(&mut harness, to, Modifiers::NONE);
    let seen = (
        faces_of(&harness, blocks.front),
        faces_of(&harness, blocks.back),
    );

    press(&mut harness, from, Modifiers::NONE);
    move_to(&mut harness, midway);
    harness.events.push(Event::ModifiersChanged(Modifiers::ALT));
    move_to(&mut harness, to);
    let reaching = harness.shows(TAKING_HIDDEN_PROMPT) && harness.shows_hint(LET_GO_OF_ALT_HINT);
    release(&mut harness, to, Modifiers::ALT);
    let through = (
        faces_of(&harness, blocks.front),
        faces_of(&harness, blocks.back),
    );

    press(&mut harness, from, Modifiers::ALT);
    move_to(&mut harness, midway);
    move_to(&mut harness, to);
    release(&mut harness, to, Modifiers::ALT);
    let held_from_the_start = (
        faces_of(&harness, blocks.front),
        faces_of(&harness, blocks.back),
    );

    assert!(offered);
    assert_eq!(seen, (1, 0));
    assert!(reaching);
    assert_eq!(through, (2, 2));
    assert_eq!(held_from_the_start, (2, 2));
}

#[test]
fn letting_go_of_alt_before_the_release_takes_only_what_can_be_seen() {
    let mut harness = Harness::new();
    let blocks = one_block_behind_another(&mut harness);
    let from = blocks.centre + vec2(BOX_HALF_SIZE, BOX_HALF_SIZE);
    let to = blocks.centre - vec2(BOX_HALF_SIZE, BOX_HALF_SIZE);

    press(&mut harness, from, Modifiers::NONE);
    harness.events.push(Event::ModifiersChanged(Modifiers::ALT));
    move_to(&mut harness, from.lerp(to, 0.5));
    harness
        .events
        .push(Event::ModifiersChanged(Modifiers::NONE));
    move_to(&mut harness, to);
    let back_to_seen = harness.shows(TAKING_SEEN_PROMPT);
    release(&mut harness, to, Modifiers::NONE);

    assert!(back_to_seen);
    assert_eq!(faces_of(&harness, blocks.front), 1);
    assert_eq!(faces_of(&harness, blocks.back), 0);
}
