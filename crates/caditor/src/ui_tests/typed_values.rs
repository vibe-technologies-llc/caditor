use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Constraint, Sketch};
use egui::{Key, Modifiers};

use super::{
    Action, Harness, close, constraints_of_kind, edit_free_sketch, entities_of_kind,
    entity_pickables, near, radii_of_circles, run_from_palette, type_point,
};
use crate::{canvas, drawing, editing::Tool, typed_point, viewport};

const TYPE_VALUE: &str = "type an exact value";

fn add_parameter(harness: &mut Harness, name: &str, value: f64) {
    let mut transaction = harness.document().transaction("Add parameter");
    transaction.add_parameter(name, Expression::Number(value));
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
}

fn line_ends(sketch: &Sketch) -> Vec<(Point2, Point2)> {
    entities_of_kind(sketch, "Line")
        .into_iter()
        .filter_map(|line| sketch.line_endpoints(line))
        .collect()
}

#[test]
fn an_equals_sign_opens_the_point_field_so_a_value_can_start_with_a_parameter() {
    let mut harness = Harness::new();
    add_parameter(&mut harness, "span", 30.0);
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);

    type_point(&mut harness, "0, 0");
    type_point(&mut harness, "=span, 20");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    let sketch = harness.sketch(feature);
    let ends = line_ends(sketch);
    assert_eq!(ends.len(), 1);
    assert!(near(ends[0].1, Point2::new(30.0, 20.0)), "{:?}", ends[0]);
    assert!(
        constraints_of_kind(sketch, "Horizontal distance")
            .iter()
            .any(
                |constraint| matches!(constraint, Constraint::HorizontalDistance { value, .. }
            if !value.parameters().is_empty())
            )
    );
}

#[test]
fn an_equals_sign_opens_a_modify_tools_value_field_too() {
    let mut harness = Harness::new();
    add_parameter(&mut harness, "t", 3.0);
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::ZERO, 10.0);
    let feature = edit_free_sketch(&mut harness, sketch);

    harness.select(entity_pickables(feature, &[circle]));
    run_from_palette(&mut harness, "offset sketch curves");
    assert_eq!(harness.tool(), Some(Tool::Offset));
    type_point(&mut harness, "=t");

    assert!(close(
        &radii_of_circles(harness.sketch(feature)),
        &[10.0, 13.0]
    ));
}

#[test]
fn type_an_exact_value_opens_the_field_empty_and_says_why_without_a_tool() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();

    harness.key(Key::P, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.show_new_windows();
    harness.type_text(TYPE_VALUE);
    let refused = harness.shows_containing(viewport::TYPE_VALUE_UNAVAILABLE);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();
    let closed = !harness.shows(typed_point::FIELD_LABEL);
    harness.use_tool(Key::L);
    run_from_palette(&mut harness, TYPE_VALUE);
    harness.frame();
    let opened = harness.shows(typed_point::FIELD_LABEL);
    harness.type_text("width");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();

    assert!(refused);
    assert!(closed);
    assert!(opened);
    assert!(harness.shows(typed_point::FIELD_LABEL));
    assert!(entities_of_kind(harness.sketch(feature), "Line").is_empty());
    assert_eq!(harness.tool(), Some(Tool::Line));
}

#[test]
fn a_typed_point_previews_the_shape_until_enter_places_it() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    type_point(&mut harness, "0, 0");
    harness.point_at(Point2::new(10.0, 10.0));

    harness.type_text("30, 0");
    let previewed = harness.shows("30.00 mm   0.0°");
    let nothing_placed = entities_of_kind(harness.sketch(feature), "Line").is_empty();
    harness.type_text(", nowhere");
    let follows_pointer = harness.shows("14.14 mm   45.0°");
    harness.key(Key::A, Modifiers::COMMAND);
    harness.type_text("30, 0");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();
    harness.settle();

    assert!(previewed);
    assert!(nothing_placed);
    assert!(follows_pointer);
    let ends = line_ends(harness.sketch(feature));
    assert_eq!(ends.len(), 1);
    assert!(near(ends[0].1, Point2::new(30.0, 0.0)));
}

#[test]
fn an_angle_alone_locks_the_direction_and_the_pointer_sets_the_length() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);

    type_point(&mut harness, "<30");
    let needs_a_point = harness.shows_containing("An angle alone sets the direction");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    type_point(&mut harness, "0, 0");
    type_point(&mut harness, "@<90");
    let locked = harness.shows(drawing::HEADING_PROMPT);
    harness.click_at(Point2::new(5.0, 20.0));
    type_point(&mut harness, "=<0");
    harness.point_at(Point2::new(12.0, 4.0));
    type_point(&mut harness, "8");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    assert!(needs_a_point);
    assert!(locked);
    let sketch = harness.sketch(feature);
    let ends = line_ends(sketch);
    assert_eq!(ends.len(), 2, "{ends:?}");
    assert!(near(ends[0].1, Point2::new(0.0, 20.0)), "{:?}", ends[0]);
    assert!(near(ends[1].1, Point2::new(8.0, 20.0)), "{:?}", ends[1]);
    assert_eq!(constraints_of_kind(sketch, "Angle").len(), 1);
}

#[test]
fn escape_lets_go_of_a_locked_direction_before_the_shape() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    type_point(&mut harness, "0, 0");
    type_point(&mut harness, "<45");

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let released = !harness.shows(drawing::HEADING_PROMPT);
    harness.click_at(Point2::new(20.0, 0.0));

    assert!(released);
    let ends = line_ends(harness.sketch(feature));
    assert_eq!(ends.len(), 1);
    assert!(near(ends[0].1, Point2::new(20.0, 0.0)), "{:?}", ends[0]);
}

#[test]
fn a_field_that_loses_focus_keeps_its_text_muted_until_typing_resumes_it() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);

    harness.type_text("10, 2");
    harness.click_at(Point2::new(-10.0, -10.0));
    let kept = harness.shows(typed_point::FIELD_LABEL);
    let muted = harness.color_of(typed_point::FIELD_LABEL) == canvas::MUTED;
    let waiting = harness.shows_hint(typed_point::WAITING_HINT);
    harness.type_text("0");
    let resumed = harness.color_of(typed_point::FIELD_LABEL) == canvas::TEXT;
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();
    harness.settle();

    assert!(kept);
    assert!(muted);
    assert!(waiting);
    assert!(resumed);
    let ends = line_ends(harness.sketch(feature));
    assert_eq!(ends.len(), 1);
    assert!(near(ends[0].0, Point2::new(-10.0, -10.0)), "{:?}", ends[0]);
    assert!(near(ends[0].1, Point2::new(10.0, 20.0)), "{:?}", ends[0]);
}

#[test]
fn escape_or_another_tool_clears_a_waiting_field() {
    let mut harness = Harness::new();
    harness.draw_on_new_sketch();
    harness.use_tool(Key::L);

    harness.type_text("10, 2");
    harness.click_at(Point2::new(-10.0, -10.0));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();
    let escaped = !harness.shows(typed_point::FIELD_LABEL);
    let drawing = harness.workspace.viewport.is_drawing();
    harness.type_text("5, 5");
    harness.click_at(Point2::new(-20.0, -10.0));
    harness.use_tool(Key::R);
    harness.frame();

    assert!(escaped);
    assert!(drawing);
    assert!(!harness.shows(typed_point::FIELD_LABEL));
}
