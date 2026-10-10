use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;
use egui::{Key, Modifiers};

use super::{
    Harness, constraints_of_kind, edit_free_sketch, entities_of_kind, entity_pickables,
    run_from_palette,
};
use crate::sketch_tools;

const BREAK: &str = "break the selected curves at every crossing";

#[test]
fn breaking_the_selected_line_cuts_it_at_every_crossing_in_one_undoable_step() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let across = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
    sketch.add_line(Point2::new(10.0, -5.0), Point2::new(10.0, 5.0));
    sketch.add_line(Point2::new(30.0, -5.0), Point2::new(30.0, 5.0));
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = harness.sketch(feature).clone();

    run_from_palette(&mut harness, BREAK);
    harness.settle();
    assert!(harness.shows_containing(sketch_tools::NOTHING_TO_BREAK));
    assert!(harness.sketch(feature).same_content(&before));
    assert!(!harness.workspace.palette.is_open());

    harness.select(entity_pickables(feature, &[across]));
    run_from_palette(&mut harness, BREAK);
    harness.settle();

    let sketch = harness.sketch(feature);
    assert_eq!(entities_of_kind(sketch, "Line").len(), 5);
    assert_eq!(constraints_of_kind(sketch, "Collinear").len(), 2);
    assert_eq!(constraints_of_kind(sketch, "Coincident").len(), 2);
    assert_eq!(harness.model.undo_label(), Some(sketch_tools::BREAK_TITLE));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert!(harness.sketch(feature).same_content(&before));
}

#[test]
fn breaking_a_curve_that_crosses_nothing_says_so_and_changes_nothing() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let alone = sketch.add_line(Point2::new(5.0, 5.0), Point2::new(15.0, 5.0));
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = harness.sketch(feature).clone();

    harness.select(entity_pickables(feature, &[alone]));
    run_from_palette(&mut harness, BREAK);
    harness.settle();

    assert!(harness.shows_containing("crosses no other curve"));
    assert!(harness.sketch(feature).same_content(&before));
}
