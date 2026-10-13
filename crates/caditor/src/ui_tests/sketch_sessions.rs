use caditor_document::FeatureId;
use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;
use egui::{Key, Modifiers};

use super::{CAMERA_SETTLE, Harness, extruded_plate, rectangle};
use crate::{model::Action, sketch_toolbar, status_bar};

fn plate_sketch_being_edited(harness: &mut Harness) -> FeatureId {
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::new(0.0, 0.0), Point2::new(40.0, 40.0));
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    feature
}

fn draw_a_line(harness: &mut Harness) {
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 15.0));
    harness.click_at(Point2::new(30.0, 25.0));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
}

#[test]
fn cancel_sketch_takes_back_everything_drawn_since_the_sketch_was_entered_and_undo_brings_it_back()
{
    let mut harness = Harness::new();
    let feature = plate_sketch_being_edited(&mut harness);
    let entities = harness.sketch(feature).entities().len();
    let features = harness.document().features().len();

    draw_a_line(&mut harness);
    let drawn = harness.sketch(feature).entities().len();
    harness.click_button(sketch_toolbar::CANCEL_LABEL);
    harness.settle();
    let cancelled = harness.sketch(feature).entities().len();
    let session_ended = harness.editing();
    let told = harness.shows("Took back the changes to Plate; Undo brings them back.");
    let label = harness.model.undo_label().map(str::to_owned);

    harness.perform(Action::Undo);
    harness.settle();

    assert!(drawn > entities);
    assert_eq!(cancelled, entities);
    assert_eq!(session_ended, None);
    assert!(told);
    assert_eq!(label.as_deref(), Some("Cancel the changes to Plate"));
    assert_eq!(harness.document().features().len(), features);
    assert_eq!(harness.editing(), None);
    assert_eq!(harness.sketch(feature).entities().len(), drawn);
}

#[test]
fn cancel_sketch_on_a_sketch_made_in_the_session_removes_it_and_undo_brings_it_back_with_what_was_drawn()
 {
    let mut harness = Harness::new();
    let features = harness.document().features().len();
    let feature = harness.draw_on_new_sketch();
    draw_a_line(&mut harness);

    harness.click_button(sketch_toolbar::CANCEL_LABEL);
    harness.settle();
    let removed = harness.document().feature(feature).is_none();
    let session_ended = harness.editing();
    let told = harness.shows("Took back the new Sketch 1; Undo brings it back.");
    let count = harness.document().features().len();

    harness.perform(Action::Undo);
    harness.settle();

    assert!(removed);
    assert_eq!(session_ended, None);
    assert!(told);
    assert_eq!(count, features);
    assert!(harness.document().feature(feature).is_some());
    assert_eq!(harness.sketch(feature).entities().len(), 3);
}

#[test]
fn the_context_switch_notice_s_undo_takes_back_what_the_switch_made_and_reopens_the_sketch() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    let sketch = plate_sketch_being_edited(&mut harness);
    let features = harness.document().features().len();
    harness.select([]);

    harness.click("Circular pattern");
    harness.settle();
    let opened = harness.workspace.editing.solid();
    let announced = harness.shows("Finished Plate 1 to edit Circular pattern 1.");
    let made = harness.document().features().len();

    harness.click_button(status_bar::UNDO_SWITCH);
    harness.settle();

    assert!(opened.is_some());
    assert!(announced);
    assert_eq!(made, features + 1);
    assert_eq!(harness.document().features().len(), features);
    assert_eq!(harness.workspace.editing.solid(), None);
    assert_eq!(harness.editing(), Some(sketch));
    assert!(harness.model.notice().is_none());
}
