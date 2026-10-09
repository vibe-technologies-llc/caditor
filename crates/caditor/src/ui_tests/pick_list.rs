use caditor_geometry::{Plane, Point2, Point3, Vector3};
use egui::{Event, Key, Modifiers, PointerButton, Pos2};

use super::{CAMERA_SETTLE, FRAME_SECONDS, Harness, extruded_plate, run_from_palette};
use crate::{
    pick_list::{HOLD_TO_LIST, LIST_TITLE},
    selection::Pickable,
};

const TOP: &str = "Extrude 1 › Extrude 1 end face";
const BOTTOM: &str = "Extrude 1 › Extrude 1 start face";

fn fitted_plate(harness: &mut Harness) -> Pos2 {
    extruded_plate(harness);
    harness.select([]);
    run_from_palette(harness, "fit view");
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.frame();
    let top = Plane::from_frame(Point3::new(0.0, 0.0, 10.0), Vector3::Z, Vector3::X).unwrap();
    let middle = harness
        .workspace
        .viewport
        .screen_position(top, Point2::new(20.0, 20.0))
        .expect("the top face is in view");
    harness.events.push(Event::PointerMoved(middle));
    harness.frame();
    middle
}

fn face(harness: &mut Harness, words: &str) -> Pickable {
    let built = harness.built();
    built
        .picks
        .pickables()
        .find(|pickable| pickable.describe(harness.document(), harness.model.evaluation()) == words)
        .unwrap_or_else(|| panic!("{words} is pickable"))
}

fn button(harness: &mut Harness, position: Pos2, pressed: bool) {
    harness.events.push(Event::PointerButton {
        pos: position,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    });
}

fn selected(harness: &Harness) -> Vec<Pickable> {
    harness.workspace.viewport.selection().iter().collect()
}

#[test]
fn a_press_held_on_the_model_lists_the_faces_behind_one_another_and_a_row_selects_one() {
    let mut harness = Harness::new();
    let middle = fitted_plate(&mut harness);
    let bottom = face(&mut harness, BOTTOM);
    let hold_frames = (HOLD_TO_LIST.as_secs_f64() / FRAME_SECONDS).ceil() as usize;

    button(&mut harness, middle, true);
    for _ in 0..hold_frames - 2 {
        harness.frame();
    }
    let early = harness.workspace.viewport.listed();
    for _ in 0..4 {
        harness.frame();
    }
    let listed = harness.workspace.viewport.listed().unwrap_or_default();
    button(&mut harness, middle, false);
    harness.frame();
    harness.frame();
    let after_release = selected(&harness);
    let titled = harness.shows(LIST_TITLE);

    let row = harness.position_of(BOTTOM);
    harness.events.push(Event::PointerMoved(row));
    harness.frame();
    harness.frame();
    let pointed = harness.workspace.viewport.listed_highlight();
    let described = harness.count_shown(BOTTOM);
    harness.click_screen(row);
    harness.frame();

    assert_eq!(early, None);
    assert_eq!(
        listed.get(..2),
        Some([TOP, BOTTOM].map(str::to_owned).as_slice())
    );
    assert!(after_release.is_empty());
    assert!(titled);
    assert_eq!(pointed, Some(bottom));
    assert_eq!(described, 2);
    assert_eq!(selected(&harness), vec![bottom]);
    assert_eq!(harness.workspace.viewport.listed(), None);
    assert!(!harness.shows(LIST_TITLE));
}

#[test]
fn the_list_key_opens_the_same_list_and_the_keyboard_adds_from_it_or_closes_it() {
    let mut harness = Harness::new();
    fitted_plate(&mut harness);
    let top = face(&mut harness, TOP);
    let bottom = face(&mut harness, BOTTOM);
    harness.select([top]);

    harness.key(Key::W, Modifiers::ALT);
    harness.frame();
    harness.frame();
    let first = harness.workspace.viewport.listed_highlight();
    harness.key(Key::ArrowDown, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let second = harness.workspace.viewport.listed_highlight();
    harness.hold(Modifiers::SHIFT);
    harness.key(Key::Enter, Modifiers::SHIFT);
    harness.frame();
    harness.hold(Modifiers::NONE);
    harness.frame();
    let mut added = selected(&harness);
    added.sort();

    harness.key(Key::W, Modifiers::ALT);
    harness.frame();
    harness.frame();
    let reopened = harness.workspace.viewport.listed().is_some();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let mut kept = selected(&harness);
    kept.sort();
    let mut both = vec![top, bottom];
    both.sort();

    assert_eq!(first, Some(top));
    assert_eq!(second, Some(bottom));
    assert_eq!(added, both);
    assert!(reopened);
    assert_eq!(harness.workspace.viewport.listed(), None);
    assert_eq!(kept, both);
}
