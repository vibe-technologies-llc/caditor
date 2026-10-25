use caditor_document::FeatureId;
use caditor_geometry::Point2;
use caditor_sketch::Constraint;
use egui::{Key, Modifiers};

use super::{Harness, constraints_of_kind, entities_of_kind};
use crate::{
    drawing::LINE_CHAIN_PROMPT,
    editing::{EditingCommand, Tool},
    model::Action,
    selection::Pickable,
};

const FIRST_POINT_PROMPT: &str = "Click the start of the line";

fn lines_in(harness: &Harness, feature: FeatureId) -> usize {
    entities_of_kind(harness.sketch(feature), "Line").len()
}

fn arcs_in(harness: &Harness, feature: FeatureId) -> usize {
    entities_of_kind(harness.sketch(feature), "Arc").len()
}

fn switch_to(harness: &mut Harness, tool: Tool) {
    harness.perform(Action::Editing(EditingCommand::SetTool(tool)));
    harness.settle();
    assert_eq!(harness.tool(), Some(tool));
}

fn back(harness: &mut Harness) {
    harness.key(Key::Backspace, Modifiers::NONE);
    harness.settle();
}

fn two_line_chain(harness: &mut Harness) -> FeatureId {
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(40.0, 25.0));
    harness.click_at(Point2::new(20.0, 40.0));
    harness.settle();
    assert_eq!(lines_in(harness, feature), 2);
    feature
}

fn round_trip_through_select(harness: &mut Harness) {
    switch_to(harness, Tool::Select);
    harness.use_tool(Key::L);
    assert_eq!(harness.tool(), Some(Tool::Line));
}

#[test]
fn backspace_steps_back_through_a_line_chain_after_a_round_trip_through_select() {
    let mut harness = Harness::new();
    let feature = two_line_chain(&mut harness);

    round_trip_through_select(&mut harness);
    assert!(harness.shows(FIRST_POINT_PROMPT));

    back(&mut harness);
    assert_eq!(lines_in(&harness, feature), 1);
    assert!(harness.shows(LINE_CHAIN_PROMPT));

    back(&mut harness);
    assert_eq!(lines_in(&harness, feature), 0);
    assert!(harness.shows(LINE_CHAIN_PROMPT));

    back(&mut harness);
    assert!(harness.shows(FIRST_POINT_PROMPT));
}

#[test]
fn a_chain_taken_back_after_a_round_trip_goes_on_from_the_point_it_stepped_back_to() {
    let mut harness = Harness::new();
    let feature = two_line_chain(&mut harness);

    round_trip_through_select(&mut harness);
    back(&mut harness);
    harness.click_at(Point2::new(60.0, 30.0));
    harness.settle();

    assert_eq!(lines_in(&harness, feature), 2);
    assert_eq!(
        constraints_of_kind(harness.sketch(feature), "Coincident").len(),
        1
    );
}

#[test]
fn the_shape_in_progress_is_still_dropped_by_a_tool_change() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 10.0));
    assert!(harness.shows(LINE_CHAIN_PROMPT));

    round_trip_through_select(&mut harness);

    assert!(harness.shows(FIRST_POINT_PROMPT));
    assert!(!harness.workspace.viewport.is_drawing());
    back(&mut harness);
    assert_eq!(lines_in(&harness, feature), 0);
}

#[test]
fn a_tangent_arc_chain_steps_back_after_a_round_trip_through_select() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(40.0, 10.0));
    switch_to(&mut harness, Tool::TangentArc);
    harness.click_at(Point2::new(60.0, 30.0));
    harness.settle();
    assert_eq!(arcs_in(&harness, feature), 1);

    switch_to(&mut harness, Tool::Select);
    switch_to(&mut harness, Tool::TangentArc);
    back(&mut harness);

    assert_eq!(arcs_in(&harness, feature), 0);
    assert_eq!(lines_in(&harness, feature), 1);
    harness.click_at(Point2::new(60.0, 30.0));
    harness.settle();
    assert_eq!(arcs_in(&harness, feature), 1);
}

#[test]
fn another_edit_made_in_between_ends_the_chain_for_backspace() {
    let mut harness = Harness::new();
    let feature = two_line_chain(&mut harness);
    let line = entities_of_kind(harness.sketch(feature), "Line")[1];

    switch_to(&mut harness, Tool::Select);
    harness.add_stored_constraint(feature, Constraint::Horizontal(line));
    harness.use_tool(Key::L);
    back(&mut harness);

    assert_eq!(lines_in(&harness, feature), 2);
}

#[test]
fn undo_and_redo_in_between_end_the_chain_for_backspace() {
    let mut harness = Harness::new();
    let feature = two_line_chain(&mut harness);

    switch_to(&mut harness, Tool::Select);
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.settle();
    assert_eq!(lines_in(&harness, feature), 1);
    harness.use_tool(Key::L);
    back(&mut harness);
    assert_eq!(lines_in(&harness, feature), 1);

    switch_to(&mut harness, Tool::Select);
    harness.key(Key::Y, Modifiers::COMMAND);
    harness.settle();
    assert_eq!(lines_in(&harness, feature), 2);
    harness.use_tool(Key::L);
    back(&mut harness);
    assert_eq!(lines_in(&harness, feature), 2);
}

#[test]
fn a_shape_drawn_with_another_tool_in_between_ends_the_chain_for_backspace() {
    let mut harness = Harness::new();
    let feature = two_line_chain(&mut harness);

    harness.use_tool(Key::R);
    harness.click_at(Point2::new(-30.0, -30.0));
    harness.click_at(Point2::new(-10.0, -10.0));
    harness.settle();
    let lines = lines_in(&harness, feature);

    harness.use_tool(Key::L);
    back(&mut harness);

    assert_eq!(lines_in(&harness, feature), lines);
}

#[test]
fn starting_a_new_line_ends_the_old_chain_for_backspace() {
    let mut harness = Harness::new();
    let feature = two_line_chain(&mut harness);

    round_trip_through_select(&mut harness);
    harness.click_at(Point2::new(-30.0, -30.0));
    back(&mut harness);
    back(&mut harness);

    assert_eq!(lines_in(&harness, feature), 2);
    assert!(harness.shows(FIRST_POINT_PROMPT));
}

#[test]
fn leaving_the_sketch_ends_the_chain_for_backspace() {
    let mut harness = Harness::new();
    let feature = two_line_chain(&mut harness);

    switch_to(&mut harness, Tool::Select);
    harness.perform(Action::Editing(EditingCommand::Finish));
    harness.settle();
    harness.perform(Action::Editing(EditingCommand::Enter(feature)));
    harness.settle();
    harness.use_tool(Key::L);
    back(&mut harness);

    assert_eq!(lines_in(&harness, feature), 2);
}

#[test]
fn backspace_resuming_a_chain_does_not_also_delete_the_selection() {
    let mut harness = Harness::new();
    let feature = two_line_chain(&mut harness);
    let first = entities_of_kind(harness.sketch(feature), "Line")[0];

    switch_to(&mut harness, Tool::Select);
    harness.select([Pickable::SketchEntity {
        feature,
        entity: first,
    }]);
    harness.use_tool(Key::L);
    back(&mut harness);

    assert_eq!(lines_in(&harness, feature), 1);
    assert!(harness.sketch(feature).contains(first));
}
