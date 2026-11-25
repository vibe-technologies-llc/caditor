use caditor_document::FeatureId;
use caditor_expression::Expression;
use caditor_geometry::Point2;
use caditor_sketch::{Constraint, EntityId, Sketch};
use egui::{Key, Modifiers};

use super::{
    DRAWN, Harness, constraints_of_kind, drag_in_sketch, entities_of_kind, length_value, line_ends,
    near, refused, run_from_palette, type_point,
};
use crate::{
    commands::Command,
    drawing::{LINE_CHAIN_PROMPT, Refusal},
    editing::{EditingCommand, Tool},
    model::Action,
    shape_modes::{LineMode, ShapeMode},
    sketch_toolbar,
};

const MIDDLE_PROMPT: &str = "Click the middle of the line";
const END_PROMPT: &str = "Click one end of the line";

fn drawing_from_the_middle() -> (Harness, FeatureId) {
    let mut harness = Harness::new();

    let feature = harness.draw_on_new_sketch();

    run_from_palette(&mut harness, "draw line from its middle");
    harness.frame();
    (harness, feature)
}

fn only_line(sketch: &Sketch) -> EntityId {
    let [line] = entities_of_kind(sketch, "Line")[..] else {
        panic!("one line should be drawn");
    };
    line
}

fn middle_of(sketch: &Sketch, line: EntityId) -> EntityId {
    let midpoints: Vec<EntityId> = constraints_of_kind(sketch, "Midpoint")
        .into_iter()
        .filter_map(|constraint| match constraint {
            Constraint::Midpoint { point, curve } if curve == line => Some(point),
            _ => None,
        })
        .collect();
    let [middle] = midpoints[..] else {
        panic!("one point should be held at the middle of the line, found {midpoints:?}");
    };
    middle
}

fn notice_text(harness: &Harness) -> Option<String> {
    harness.model.notice().map(|notice| notice.text.clone())
}

#[test]
fn a_line_from_its_middle_runs_as_far_again_past_it_and_keeps_a_point_there() {
    let (mut harness, feature) = drawing_from_the_middle();

    assert_eq!(harness.tool(), Some(Tool::Line));
    assert!(harness.shows(MIDDLE_PROMPT));

    harness.click_at(Point2::new(20.0, 20.0));

    assert!(harness.shows(END_PROMPT));

    harness.click_at(Point2::new(35.0, 28.0));
    harness.settle();

    let sketch = harness.sketch(feature);
    let line = only_line(sketch);

    let middle = middle_of(sketch, line);

    let (start, end) = sketch.line_endpoints(line).unwrap();
    let at_middle = sketch.point(middle).unwrap();

    assert!(near(at_middle, Point2::new(20.0, 20.0)), "{at_middle}");
    assert!(near(end, Point2::new(35.0, 28.0)), "{end}");
    assert!(near(start.midpoint(end), at_middle), "{start} {end}");
    assert_eq!(entities_of_kind(sketch, "Point").len(), 3);
    assert_eq!(constraints_of_kind(sketch, "Coincident"), Vec::new());
    assert!(harness.shows("4 degrees of freedom left"));
    assert_eq!(harness.model.undo_label(), Some("Draw line"));
    assert_eq!(harness.tool(), Some(Tool::Line));
    assert!(harness.shows(MIDDLE_PROMPT));
    assert_eq!(
        harness.workspace.editing.modes().of(Tool::Line),
        Some(ShapeMode::Line(LineMode::Middle))
    );

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.settle();

    let sketch = harness.sketch(feature);

    assert!(entities_of_kind(sketch, "Line").is_empty());
    assert!(entities_of_kind(sketch, "Point").is_empty());
    assert_eq!(sketch.constraints().count(), 0);
}

#[test]
fn the_placed_end_of_a_line_from_its_middle_turns_level_with_it() {
    let (mut harness, feature) = drawing_from_the_middle();

    harness.click_at(Point2::new(20.0, 20.0));
    harness.point_at(Point2::new(45.0, 20.4));

    assert!(harness.shows("Horizontal"));

    harness.click_at(Point2::new(45.0, 20.4));
    harness.settle();

    let sketch = harness.sketch(feature);
    let line = only_line(sketch);

    let (start, end) = sketch.line_endpoints(line).unwrap();

    assert_eq!(
        constraints_of_kind(sketch, "Horizontal"),
        vec![Constraint::Horizontal(line)]
    );
    assert!((start.y - end.y).abs() < DRAWN, "{start} {end}");
    assert!(near(start, Point2::new(-5.0, 20.0)), "{start}");
    assert!(harness.shows("3 degrees of freedom left"));
}

#[test]
fn a_length_typed_from_the_middle_is_the_whole_lines() {
    let (mut harness, feature) = drawing_from_the_middle();

    run_from_palette(&mut harness, "Keep typed values as dimensions");
    type_point(&mut harness, "10, 10");
    harness.point_at(Point2::new(30.0, 10.0));
    type_point(&mut harness, "40");

    type_point(&mut harness, "50, 50");
    type_point(&mut harness, "@30 < 90");

    type_point(&mut harness, "-50, 50");
    harness.point_at(Point2::new(-50.0, 80.0));
    type_point(&mut harness, "16 <");
    harness.point_at(Point2::new(-20.0, 50.0));
    harness.click_at(Point2::new(-20.0, 50.0));
    harness.settle();

    let sketch = harness.sketch(feature);
    let ends: Vec<(Point2, Point2)> = entities_of_kind(sketch, "Line")
        .into_iter()
        .map(|line| sketch.line_endpoints(line).unwrap())
        .collect();
    let expected = [
        (Point2::new(-10.0, 10.0), Point2::new(30.0, 10.0)),
        (Point2::new(50.0, 35.0), Point2::new(50.0, 65.0)),
        (Point2::new(-58.0, 50.0), Point2::new(-42.0, 50.0)),
    ];

    assert_eq!(ends.len(), expected.len());

    for ((start, end), (expected_start, expected_end)) in ends.iter().zip(expected) {
        assert!(
            near(*start, expected_start) && near(*end, expected_end),
            "{start} {end}"
        );
    }

    assert!(constraints_of_kind(sketch, "Distance").is_empty());
    assert_eq!(constraints_of_kind(sketch, "Midpoint").len(), 3);
}

#[test]
fn a_length_and_angle_typed_from_the_middle_dimension_the_whole_line() {
    let (mut harness, feature) = drawing_from_the_middle();

    type_point(&mut harness, "0, 0");
    type_point(&mut harness, "@30 < 60");
    harness.settle();

    let sketch = harness.sketch(feature);
    let line = only_line(sketch);

    let (start, end) = line_ends(sketch, line);

    let [
        Constraint::Distance {
            from,
            to,
            value: length,
        },
    ] = &constraints_of_kind(sketch, "Distance")[..]
    else {
        panic!("the typed length should be one distance");
    };
    let [
        Constraint::Angle {
            to: turned,
            value: angle,
            ..
        },
    ] = &constraints_of_kind(sketch, "Angle")[..]
    else {
        panic!("the typed angle should be one angle");
    };

    assert_eq!((*from, *to), (start, end));
    assert!((length_value(&harness, length) - 30.0).abs() < DRAWN);
    assert_eq!(*turned, line);
    assert_eq!(*angle, Expression::Number(60.0));
    assert!(harness.shows("Fully constrained"));
}

#[test]
fn a_line_from_its_middle_needs_its_end_away_from_it() {
    let (mut harness, feature) = drawing_from_the_middle();

    type_point(&mut harness, "10, 10");
    refused(&mut harness, "10, 10", Refusal::LineMiddle);

    assert!(harness.shows(END_PROMPT));
    assert!(entities_of_kind(harness.sketch(feature), "Line").is_empty());
}

#[test]
fn escape_after_the_middle_cancels_the_line_then_returns_to_select() {
    let (mut harness, feature) = drawing_from_the_middle();

    harness.click_at(Point2::new(20.0, 20.0));

    assert!(harness.workspace.viewport.is_drawing());

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    assert!(!harness.workspace.viewport.is_drawing());
    assert!(harness.shows(MIDDLE_PROMPT));
    assert_eq!(harness.sketch(feature).entities().count(), 0);

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    assert_eq!(harness.tool(), Some(Tool::Select));
}

#[test]
fn a_press_dragged_to_its_release_draws_one_line_from_its_middle() {
    let (mut harness, feature) = drawing_from_the_middle();

    let from = harness.on_screen(Point2::new(20.0, 20.0));

    drag_in_sketch(&mut harness, from, Point2::new(32.0, 35.0));
    harness.settle();

    let sketch = harness.sketch(feature);
    let line = only_line(sketch);

    let at_middle = sketch.point(middle_of(sketch, line)).unwrap();
    let (start, end) = sketch.line_endpoints(line).unwrap();

    assert!(near(at_middle, Point2::new(20.0, 20.0)), "{at_middle}");
    assert!(near(end, Point2::new(32.0, 35.0)), "{end}");
    assert!(near(start, Point2::new(8.0, 5.0)), "{start}");
    assert!(harness.shows(MIDDLE_PROMPT));
}

#[test]
fn a_line_changes_way_only_with_nothing_placed_and_its_key_keeps_the_way() {
    let mut harness = Harness::new();

    harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.perform(Action::Editing(EditingCommand::SetMode(ShapeMode::Line(
        LineMode::Middle,
    ))));
    harness.frame();

    assert_eq!(
        notice_text(&harness).as_deref(),
        Some("Finish or cancel the line first, then change how it is drawn")
    );
    assert!(harness.shows(LINE_CHAIN_PROMPT));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.use_tool(Key::L);

    assert!(harness.shows("Click the start of the line"));

    harness.perform(Action::Editing(EditingCommand::SetMode(ShapeMode::Line(
        LineMode::Middle,
    ))));
    harness.frame();

    assert!(harness.shows(MIDDLE_PROMPT));

    harness.click_at(Point2::new(10.0, 10.0));
    harness.use_tool(Key::L);

    assert!(harness.shows(END_PROMPT));

    harness.perform(Action::Editing(EditingCommand::SetMode(ShapeMode::Line(
        LineMode::End,
    ))));
    harness.frame();

    assert!(harness.shows(END_PROMPT));
    assert_eq!(
        harness.workspace.editing.modes().of(Tool::Line),
        Some(ShapeMode::Line(LineMode::Middle))
    );
}

#[test]
fn both_ways_of_drawing_a_line_are_in_the_palette_and_the_menu_on_its_button() {
    let mut harness = Harness::new();

    harness.draw_on_new_sketch();
    for way in [LineMode::End, LineMode::Middle] {
        assert!(
            super::offer(&harness, Command::ShapeMode(ShapeMode::Line(way)))
                .availability
                .is_ok()
        );
    }

    harness.use_tool(Key::L);
    harness.hover("Line");

    assert!(harness.shows("Draw connected lines, one click per corner (L)"));

    assert!(!harness.shows(&sketch_toolbar::modes_label(Tool::Line)));

    harness.click_button(&sketch_toolbar::group_menu_name("Draw"));

    assert!(harness.shows("Draw line from one end"));
    assert!(harness.shows("Draw line from its middle"));

    harness.click("Draw line from its middle");
    harness.frame();

    assert_eq!(harness.tool(), Some(Tool::Line));
    assert!(harness.shows(MIDDLE_PROMPT));
    assert!(!harness.shows_hint("Line from its middle"));

    run_from_palette(&mut harness, "draw line from one end");
    harness.frame();

    assert!(harness.shows("Click the start of the line"));
}
