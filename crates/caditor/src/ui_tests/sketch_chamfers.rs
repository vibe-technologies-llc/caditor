use caditor_document::FeatureId;
use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;
use egui::{Key, Modifiers};

use super::{
    Harness, constraints_of_kind, edit_free_sketch, entities_of_kind, near, rectangle, type_point,
};
use crate::{editing::Tool, filleting};

const CORNER: Point2 = Point2::new(40.0, 0.0);

fn corner_fixture(harness: &mut Harness) -> FeatureId {
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::ZERO, Point2::new(40.0, 20.0));
    edit_free_sketch(harness, sketch)
}

fn chosen_corner(harness: &mut Harness) {
    harness.use_tool_with(Key::B, Modifiers::SHIFT);
    assert_eq!(harness.tool(), Some(Tool::Chamfer));
    harness.point_at(CORNER);
    harness.click_at(CORNER);
    assert!(harness.shows(filleting::DISTANCE_PROMPT));
}

fn cut_between(sketch: &Sketch, start: Point2, end: Point2) -> bool {
    entities_of_kind(sketch, "Line")
        .iter()
        .filter_map(|line| sketch.line_endpoints(*line))
        .any(|(from, to)| near(from, start) && near(to, end))
}

#[test]
fn a_chamfer_with_two_typed_distances_cuts_a_different_length_from_each_curve() {
    let mut harness = Harness::new();
    let feature = corner_fixture(&mut harness);
    let before = harness.sketch(feature).clone();

    chosen_corner(&mut harness);
    harness.type_text("8, 3");
    assert!(harness.shows_containing("from it on the first and"));
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();
    harness.settle();

    let sketch = harness.sketch(feature);
    assert!(cut_between(
        sketch,
        Point2::new(32.0, 0.0),
        Point2::new(40.0, 3.0)
    ));
    assert_eq!(constraints_of_kind(sketch, "Distance").len(), 2);
    assert_eq!(
        harness.model.undo_label(),
        Some(filleting::CHAMFER_TRANSACTION)
    );

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert!(harness.sketch(feature).same_content(&before));
}

#[test]
fn a_chamfer_with_a_typed_distance_and_angle_keeps_the_angle_as_a_dimension() {
    let mut harness = Harness::new();
    let feature = corner_fixture(&mut harness);

    chosen_corner(&mut harness);
    harness.type_text("6 < 45");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();
    harness.settle();

    let sketch = harness.sketch(feature);
    assert!(cut_between(
        sketch,
        Point2::new(34.0, 0.0),
        Point2::new(40.0, 6.0)
    ));
    assert_eq!(constraints_of_kind(sketch, "Angle").len(), 1);
    assert_eq!(
        harness.model.undo_label(),
        Some(filleting::CHAMFER_TRANSACTION)
    );
}

#[test]
fn a_chamfer_text_that_fits_no_form_or_cannot_be_cut_keeps_the_field_open_in_words() {
    let mut harness = Harness::new();
    let feature = corner_fixture(&mut harness);

    chosen_corner(&mut harness);
    type_point(&mut harness, "5, 3, 2");
    assert!(harness.shows_containing("Type a distance such as 5"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    type_point(&mut harness, "5 < 150");
    assert!(harness.shows_containing("does not meet"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    type_point(&mut harness, "5 < 190");
    assert!(harness.shows_containing("less than 180"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    assert_eq!(entities_of_kind(harness.sketch(feature), "Line").len(), 4);
}
