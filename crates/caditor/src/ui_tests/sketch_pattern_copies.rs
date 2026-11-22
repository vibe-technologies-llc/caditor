use caditor_document::{FeatureId, FeatureState};
use caditor_expression::{BinaryOperator, Expression};
use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Constraint, EntityId, Sketch};
use egui::{Key, Modifiers};

use super::{
    Harness, Wheel, constraints_of_kind, edit_free_sketch, entities_of_kind, entity_pickables,
    near, run_from_palette,
};
use crate::{
    patterning::{self, Direction, PatternField},
    sketch_pattern_panel,
};

const CIRCLE_CENTRE: Point2 = Point2::new(5.0, 5.0);
const PIVOT: Point2 = Point2::new(20.0, 0.0);
const RECTANGULAR: &str = "repeat sketch geometry in a grid";
const CIRCULAR: &str = "repeat sketch geometry about a point";
const MOST_TABS: usize = 80;
const VIEW_CORNERS: [Point2; 2] = [Point2::new(-20.0, -30.0), Point2::new(80.0, 40.0)];

fn row_fixture(harness: &mut Harness) -> (FeatureId, EntityId, EntityId) {
    let mut sketch = Sketch::new(Plane::XY);

    let circle = sketch.add_circle(CIRCLE_CENTRE, 2.0);
    let line = sketch.add_line(Point2::new(0.0, 12.0), Point2::new(10.0, 12.0));

    widen_view(&mut sketch);
    (edit_free_sketch(harness, sketch), circle, line)
}

fn widen_view(sketch: &mut Sketch) {
    for corner in VIEW_CORNERS {
        sketch.add_point(corner);
    }
}

fn has_centre(sketch: &Sketch, expected: Point2) -> bool {
    entities_of_kind(sketch, "Circle")
        .into_iter()
        .filter_map(|circle| sketch.circle(circle))
        .any(|(centre, _)| near(centre, expected))
}

fn solves(harness: &Harness, feature: FeatureId) -> bool {
    harness
        .model
        .evaluation()
        .feature(feature)
        .is_some_and(|status| status.state == FeatureState::UpToDate)
}

fn set(harness: &mut Harness, field: PatternField, text: &str) {
    harness.type_into_field(sketch_pattern_panel::field_id(field), text);
}

fn show_copies(harness: &mut Harness) {
    let panel = harness.position_of(sketch_pattern_panel::SHAPE);

    harness.wheel_until_shown(panel, sketch_pattern_panel::REPEATS, Wheel::Down);
    harness.hold_still();
}

fn focused_label(harness: &Harness) -> Option<String> {
    let focused = harness.focused()?.accesskit_id();
    harness
        .accessible
        .iter()
        .find(|(id, _)| *id == focused)
        .and_then(|(_, node)| node.label().map(str::to_owned))
}

fn tab_to(harness: &mut Harness, label: &str) {
    for _ in 0..MOST_TABS {
        if focused_label(harness).as_deref() == Some(label) {
            return;
        }
        harness.key(Key::Tab, Modifiers::NONE);
        harness.frame();
    }
    panic!("Tab never reached {label}");
}

#[test]
fn a_previewed_copy_clicked_in_the_view_is_left_out_and_the_next_is_tied_two_steps_on() {
    let mut harness = Harness::new();

    let (feature, circle, line) = row_fixture(&mut harness);

    harness.select(entity_pickables(feature, &[circle, line]));
    run_from_palette(&mut harness, RECTANGULAR);
    set(&mut harness, PatternField::Count(Direction::FIRST), "4");
    set(&mut harness, PatternField::Spacing(Direction::FIRST), "20");

    let on_first_copy = CIRCLE_CENTRE + Point2::new(22.0, 0.0);

    harness.point_at(on_first_copy);

    assert!(harness.shows_containing("Click to leave copy 1 out"));

    harness.click_at(on_first_copy);

    assert!(harness.shows_containing("Click to make copy 1 again"));

    harness.point_at(Point2::new(10.0, -20.0));

    assert!(harness.shows("Repeat 2 items 2 more times"));

    show_copies(&mut harness);

    assert!(harness.shows("1 copy is left out."));

    harness.click(sketch_pattern_panel::MAKE);
    harness.settle();

    let sketch = harness.sketch(feature);

    assert_eq!(entities_of_kind(sketch, "Circle").len(), 3);
    assert!(!has_centre(sketch, Point2::new(25.0, 5.0)));

    for expected in [Point2::new(45.0, 5.0), Point2::new(65.0, 5.0)] {
        assert!(has_centre(sketch, expected), "{expected}");
    }

    let two_steps = constraints_of_kind(sketch, "Horizontal distance")
        .into_iter()
        .filter(|constraint| {
            matches!(
                constraint,
                Constraint::HorizontalDistance {
                    value: Expression::Binary(BinaryOperator::Multiply, ..),
                    ..
                }
            )
        })
        .count();

    assert_eq!(constraints_of_kind(sketch, "Horizontal distance").len(), 6);
    assert_eq!(two_steps, 3);
    assert_eq!(harness.model.undo_label(), Some(patterning::TRANSACTION));
    assert!(solves(&harness, feature));
}

#[test]
fn copies_are_left_out_from_the_panels_grid_by_pointer_or_keyboard_and_brought_back_together() {
    let mut harness = Harness::new();

    let (feature, circle, _) = row_fixture(&mut harness);

    harness.context.enable_accesskit();
    harness.select(entity_pickables(feature, &[circle]));
    run_from_palette(&mut harness, RECTANGULAR);

    let panel = harness.position_of(sketch_pattern_panel::SHAPE);

    harness.wheel_until_shown(panel, patterning::ADD_DIRECTION, Wheel::Down);
    harness.hold_still();
    harness.click(patterning::ADD_DIRECTION);
    harness.frame();
    show_copies(&mut harness);

    assert!(harness.shows(sketch_pattern_panel::COPIES_HINT));
    assert!(harness.has_button("The original"));
    assert!(harness.shows("Repeat 1 item 5 more times"));

    harness.click_button("Copy (1, 1)");
    harness.frame();

    assert!(harness.shows("1 copy is left out."));
    assert!(harness.shows("Repeat 1 item 4 more times"));

    harness.focus_field(sketch_pattern_panel::field_id(PatternField::Count(
        Direction(1),
    )));
    tab_to(&mut harness, "Copy (2, 0)");
    harness.key(Key::Space, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert!(harness.shows("2 copies are left out."));

    harness.click_button(sketch_pattern_panel::BRING_BACK);
    harness.frame();

    assert!(harness.shows(sketch_pattern_panel::COPIES_HINT));

    harness.click_button("Copy (1, 1)");
    harness.frame();
    harness.click(sketch_pattern_panel::MAKE);
    harness.settle();

    let sketch = harness.sketch(feature);

    assert_eq!(entities_of_kind(sketch, "Circle").len(), 5);
    assert!(!has_centre(sketch, Point2::new(15.0, 15.0)));
    assert!(has_centre(sketch, Point2::new(25.0, 15.0)));
    assert!(solves(&harness, feature));
}

#[test]
fn a_circular_pattern_leaves_out_a_turn_clicked_once_the_centre_is_chosen() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);

    let line = sketch.add_line(Point2::new(30.0, 0.0), Point2::new(34.0, 0.0));
    let circle = sketch.add_circle(Point2::new(32.0, 3.0), 1.0);

    sketch.add_point(PIVOT);
    widen_view(&mut sketch);

    let feature = edit_free_sketch(&mut harness, sketch);

    harness.select(entity_pickables(feature, &[line, circle]));
    run_from_palette(&mut harness, CIRCULAR);
    harness.click_at(PIVOT);
    set(&mut harness, PatternField::Copies, "4");

    let on_first_turn = Point2::new(20.0, 12.0);

    harness.point_at(on_first_turn);

    assert!(harness.shows_containing("Click to leave copy 1 out"));

    harness.click_at(on_first_turn);

    assert!(harness.shows_containing("Click to make copy 1 again"));

    harness.point_at(Point2::new(60.0, 30.0));

    assert!(harness.shows_containing("Repeat 2 items 2 more times about"));

    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();
    harness.settle();

    let sketch = harness.sketch(feature);

    assert_eq!(entities_of_kind(sketch, "Circle").len(), 3);
    assert_eq!(constraints_of_kind(sketch, "Angle").len(), 3 * 2);
    assert!(solves(&harness, feature));

    let shown = harness.shown(feature);

    assert!(!has_centre(&shown, Point2::new(17.0, 12.0)));

    for expected in [Point2::new(8.0, -3.0), Point2::new(23.0, -12.0)] {
        assert!(has_centre(&shown, expected), "{expected}");
    }
}
