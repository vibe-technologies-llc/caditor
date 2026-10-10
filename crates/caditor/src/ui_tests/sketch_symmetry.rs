use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Constraint, EntityId, Sketch};
use egui::{Key, Modifiers};

use super::{
    Harness, constraints_of_kind, edit_free_sketch, entities_of_kind, entity_pickables, near,
    run_from_palette, type_point,
};
use crate::{
    editing::Tool,
    offsetting,
    shape_modes::{OffsetMode, ShapeMode},
    symmetric_drawing,
};

fn centreline() -> (Sketch, EntityId) {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(30.0, 0.0));
    (sketch, line)
}

#[test]
fn offset_run_again_offsets_a_line_to_both_sides_with_round_ends_in_one_undoable_step() {
    let mut harness = Harness::new();
    let (sketch, line) = centreline();
    let label = sketch.entity_label(line);
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = harness.sketch(feature).clone();

    harness.select(entity_pickables(feature, &[line]));
    harness.use_tool(Key::W);
    harness.use_tool(Key::W);
    assert_eq!(harness.tool(), Some(Tool::Offset));
    assert_eq!(
        harness.workspace.editing.modes().of(Tool::Offset),
        Some(ShapeMode::Offset(OffsetMode::BothRound))
    );
    assert!(harness.shows(offsetting::BOTH_PROMPT));
    assert!(harness.shows_containing("Offset to both sides, round ends"));

    harness.point_at(Point2::new(15.0, -4.0));
    assert!(harness.shows(&format!(
        "Offset {label} by 4 mm to both sides with round ends"
    )));
    harness.click_at(Point2::new(15.0, -4.0));

    let drawn = harness.sketch(feature);
    assert_eq!(entities_of_kind(drawn, "Line").len(), 3);
    assert_eq!(entities_of_kind(drawn, "Arc").len(), 2);
    assert!(drawn.open_ends().len() <= 2);
    assert_eq!(
        harness.model.undo_label(),
        Some(offsetting::BOTH_TRANSACTION)
    );

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert!(harness.sketch(feature).same_content(&before));
}

#[test]
fn a_typed_distance_offsets_both_ways_with_flat_ends_chosen_from_the_palette() {
    let mut harness = Harness::new();
    let (sketch, line) = centreline();
    let feature = edit_free_sketch(&mut harness, sketch);

    harness.select(entity_pickables(feature, &[line]));
    run_from_palette(&mut harness, "offset to both sides, flat ends");
    assert_eq!(harness.tool(), Some(Tool::Offset));
    type_point(&mut harness, "3");

    let drawn = harness.sketch(feature);
    assert_eq!(entities_of_kind(drawn, "Line").len(), 5);
    assert_eq!(constraints_of_kind(drawn, "Perpendicular").len(), 2);
    let ends: Vec<Point2> = entities_of_kind(drawn, "Line")
        .into_iter()
        .filter_map(|line| drawn.line_endpoints(line))
        .flat_map(|(start, end)| [start, end])
        .collect();
    for corner in [
        Point2::new(0.0, 3.0),
        Point2::new(30.0, 3.0),
        Point2::new(30.0, -3.0),
        Point2::new(0.0, -3.0),
    ] {
        assert!(ends.iter().any(|end| near(*end, corner)), "{corner}");
    }
}

#[test]
fn drawing_symmetrically_mirrors_each_shape_about_the_chosen_line_in_its_own_transaction() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let axis = sketch.add_line(Point2::new(0.0, -40.0), Point2::new(0.0, 40.0));
    sketch.set_construction(axis, true).unwrap();
    let axis_label = sketch.entity_label(axis);
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = harness.sketch(feature).clone();

    run_from_palette(&mut harness, "draw symmetrically");
    assert!(harness.shows_containing(symmetric_drawing::NO_LINE_SELECTED));
    assert!(!harness.workspace.viewport.draws_symmetrically());

    harness.select(entity_pickables(feature, &[axis]));
    run_from_palette(&mut harness, "draw symmetrically");
    assert!(harness.workspace.viewport.draws_symmetrically());
    assert!(harness.shows(&symmetric_drawing::turned_on(&axis_label)));

    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.point_at(Point2::new(20.0, 25.0));
    assert!(harness.shows_containing(&format!("mirrored about {axis_label}")));
    harness.click_at(Point2::new(20.0, 25.0));
    harness.click_at(Point2::new(20.5, -15.0));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    let drawn = harness.sketch(feature);
    assert_eq!(entities_of_kind(drawn, "Line").len(), 5);
    let symmetric = constraints_of_kind(drawn, "Symmetric");
    assert_eq!(symmetric.len(), 3);
    assert!(symmetric.iter().all(
        |constraint| matches!(constraint, Constraint::Symmetric { about, .. } if *about == axis)
    ));
    let ends: Vec<Point2> = entities_of_kind(drawn, "Line")
        .into_iter()
        .filter_map(|line| drawn.line_endpoints(line))
        .flat_map(|(start, end)| [start, end])
        .collect();
    assert!(ends.iter().any(|end| near(*end, Point2::new(-20.0, 25.0))));
    assert!(
        harness
            .model
            .undo_label()
            .is_some_and(|label| label.ends_with("symmetrically"))
    );

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert_eq!(entities_of_kind(harness.sketch(feature), "Line").len(), 3);
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert!(harness.sketch(feature).same_content(&before));

    run_from_palette(&mut harness, "draw symmetrically");
    assert!(!harness.workspace.viewport.draws_symmetrically());
}
