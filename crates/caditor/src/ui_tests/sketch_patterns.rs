use caditor_document::{FeatureId, FeatureState};
use caditor_geometry::{Plane, Point2};
use caditor_sketch::{EntityId, Sketch};
use egui::{Key, Modifiers};

use super::{
    Harness, constraints_of_kind, edit_free_sketch, entities_of_kind, entity_pickables, near,
    run_from_palette, type_point,
};
use crate::{editing::Tool, patterning};

const CIRCLE_CENTRE: Point2 = Point2::new(5.0, 5.0);
const RECTANGULAR: &str = "repeat sketch geometry in a grid";
const CIRCULAR: &str = "repeat sketch geometry about a point";

fn row_fixture(harness: &mut Harness) -> (FeatureId, EntityId, EntityId) {
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(CIRCLE_CENTRE, 2.0);
    let line = sketch.add_line(Point2::new(0.0, 12.0), Point2::new(10.0, 12.0));
    (edit_free_sketch(harness, sketch), circle, line)
}

fn centres(sketch: &Sketch) -> Vec<Point2> {
    entities_of_kind(sketch, "Circle")
        .into_iter()
        .filter_map(|circle| sketch.circle(circle))
        .map(|(centre, _)| centre)
        .collect()
}

fn has_centre(sketch: &Sketch, expected: Point2) -> bool {
    centres(sketch).iter().any(|centre| near(*centre, expected))
}

fn solves(harness: &Harness, feature: FeatureId) -> bool {
    harness
        .model
        .evaluation()
        .feature(feature)
        .is_some_and(|status| status.state == FeatureState::UpToDate)
}

#[test]
fn a_rectangular_pattern_repeats_the_selection_at_a_typed_count_and_spacing_in_one_undoable_step() {
    let mut harness = Harness::new();
    let (feature, circle, line) = row_fixture(&mut harness);
    let before = harness.sketch(feature).clone();

    run_from_palette(&mut harness, RECTANGULAR);
    assert_eq!(harness.tool(), Some(Tool::RectangularPattern));
    assert!(harness.shows(patterning::RECTANGULAR_SELECT_FIRST));
    type_point(&mut harness, "3 x 20");
    assert!(harness.shows("Rectangular pattern: select the geometry to pattern."));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::RectangularPattern));

    harness.select(entity_pickables(feature, &[circle, line]));
    assert!(harness.shows(patterning::RECTANGULAR_PROMPT));
    harness.type_text("3 x 20");
    assert!(harness.shows("Repeat 2 items 2 more times"));
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();
    harness.settle();

    let sketch = harness.sketch(feature);
    assert_eq!(entities_of_kind(sketch, "Circle").len(), 3);
    assert_eq!(entities_of_kind(sketch, "Line").len(), 3);
    for expected in [Point2::new(25.0, 5.0), Point2::new(45.0, 5.0)] {
        assert!(has_centre(sketch, expected), "{expected}");
    }
    assert_eq!(constraints_of_kind(sketch, "Horizontal distance").len(), 6);
    assert_eq!(constraints_of_kind(sketch, "Equal").len(), 2);
    assert_eq!(harness.model.undo_label(), Some(patterning::TRANSACTION));
    assert_eq!(harness.tool(), Some(Tool::RectangularPattern));
    assert!(solves(&harness, feature));
    assert!(has_centre(&harness.shown(feature), Point2::new(45.0, 5.0)));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert!(harness.sketch(feature).same_content(&before));
}

#[test]
fn a_pattern_that_cannot_be_made_says_why_in_the_field_and_changes_nothing() {
    let mut harness = Harness::new();
    let (feature, circle, _) = row_fixture(&mut harness);
    let before = harness.sketch(feature).clone();
    harness.select(entity_pickables(feature, &[circle]));
    run_from_palette(&mut harness, RECTANGULAR);

    type_point(&mut harness, "1 x 20");
    assert!(harness.shows_containing("a pattern needs a count of at least 2"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    type_point(&mut harness, "3 x 0");
    assert!(harness.shows_containing("the spacing cannot be zero"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    type_point(&mut harness, "3");
    assert!(harness.shows_containing("Type a count and a spacing"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    type_point(&mut harness, "2.5 x 10");
    assert!(harness.shows_containing("Use a whole number"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    type_point(&mut harness, "3 x 10, 2 x 10 < 180");
    assert!(harness.shows_containing("the two directions run along one line"));

    assert!(harness.sketch(feature).same_content(&before));
}

#[test]
fn a_second_term_adds_rows_square_to_the_first_and_an_angle_slants_a_direction() {
    let mut harness = Harness::new();
    let (feature, circle, _) = row_fixture(&mut harness);
    harness.select(entity_pickables(feature, &[circle]));
    run_from_palette(&mut harness, RECTANGULAR);

    type_point(&mut harness, "2 x 10, 3 x 12");

    let sketch = harness.sketch(feature);
    assert_eq!(entities_of_kind(sketch, "Circle").len(), 6);
    for (x, y) in [
        (15.0, 5.0),
        (5.0, 17.0),
        (15.0, 17.0),
        (5.0, 29.0),
        (15.0, 29.0),
    ] {
        assert!(has_centre(sketch, Point2::new(x, y)), "{x}, {y}");
    }
    assert!(solves(&harness, feature));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    type_point(&mut harness, "2 x 10 < 90");
    assert!(has_centre(harness.sketch(feature), Point2::new(5.0, 15.0)));
}

#[test]
fn a_circular_pattern_turns_the_selection_about_the_clicked_point_and_keeps_it_tied() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(10.0, 0.0), Point2::new(14.0, 0.0));
    let circle = sketch.add_circle(Point2::new(12.0, 3.0), 1.0);
    let pivot = sketch.add_point(Point2::ZERO);
    let pivot_label = sketch.entity_label(pivot);
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = harness.sketch(feature).clone();

    harness.select(entity_pickables(feature, &[line, circle]));
    run_from_palette(&mut harness, CIRCULAR);
    assert_eq!(harness.tool(), Some(Tool::CircularPattern));
    assert!(harness.shows(patterning::CENTRE_PROMPT));
    type_point(&mut harness, "4");
    assert!(harness.shows_containing("click the point to repeat the selection about first"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    harness.click_at(Point2::ZERO);
    assert!(harness.shows(patterning::CIRCULAR_PROMPT));
    harness.type_text("4");
    assert!(harness.shows(&format!("Repeat 2 items 3 more times about {pivot_label}")));
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();
    harness.settle();

    let sketch = harness.sketch(feature);
    assert_eq!(entities_of_kind(sketch, "Line").len(), 4 + 4 * 3);
    assert_eq!(entities_of_kind(sketch, "Circle").len(), 4);
    assert_eq!(constraints_of_kind(sketch, "Angle").len(), 3 * 3);
    assert_eq!(harness.model.undo_label(), Some(patterning::TRANSACTION));
    assert!(solves(&harness, feature));
    let shown = harness.shown(feature);
    for expected in [
        Point2::new(-3.0, 12.0),
        Point2::new(-12.0, -3.0),
        Point2::new(3.0, -12.0),
    ] {
        assert!(has_centre(&shown, expected), "{expected}");
    }

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert!(harness.sketch(feature).same_content(&before));
}

#[test]
fn a_circular_pattern_works_from_the_keyboard_alone_over_a_total_angle() {
    let mut harness = Harness::new();
    let (feature, circle, _) = row_fixture(&mut harness);
    harness.select(entity_pickables(feature, &[circle]));

    run_from_palette(&mut harness, CIRCULAR);
    assert_eq!(harness.tool(), Some(Tool::CircularPattern));
    harness.key(Key::N, Modifiers::NONE);
    harness.frame();
    harness.key(Key::Space, Modifiers::NONE);
    harness.frame();
    assert!(harness.shows(patterning::CIRCULAR_PROMPT));
    type_point(&mut harness, "3 over 90");

    let sketch = harness.sketch(feature);
    assert_eq!(entities_of_kind(sketch, "Circle").len(), 3);
    assert!(solves(&harness, feature));
    let shown = harness.shown(feature);
    let side = 50.0_f64.sqrt();
    assert!(has_centre(&shown, Point2::new(0.0, side)));
    assert!(has_centre(&shown, Point2::new(-5.0, 5.0)));
}

#[test]
fn both_patterns_are_commands_with_keys_and_a_selected_point_is_the_centre() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let pivot = sketch.add_point(Point2::new(20.0, 0.0));
    let circle = sketch.add_circle(Point2::new(30.0, 0.0), 1.0);
    let feature = edit_free_sketch(&mut harness, sketch);

    harness.select(entity_pickables(feature, &[pivot, circle]));
    run_from_palette(&mut harness, RECTANGULAR);
    assert_eq!(harness.tool(), Some(Tool::RectangularPattern));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    run_from_palette(&mut harness, CIRCULAR);
    assert_eq!(harness.tool(), Some(Tool::CircularPattern));
    assert!(harness.shows(patterning::CIRCULAR_PROMPT));
    type_point(&mut harness, "2");

    let sketch = harness.sketch(feature);
    assert_eq!(entities_of_kind(sketch, "Circle").len(), 2);
    assert!(has_centre(sketch, Point2::new(10.0, 0.0)));
    assert_eq!(entities_of_kind(sketch, "Point").len(), 3);
    assert!(solves(&harness, feature));
}
