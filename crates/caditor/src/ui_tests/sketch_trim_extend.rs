use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Constraint, EntityId, Sketch};
use egui::{Event, Key, Modifiers};

use super::{
    Harness, constraints_of_kind, drag_in_sketch, edit_free_sketch, entities_of_kind, line_ends,
    near, run_from_palette,
};
use crate::{editing::Tool, trimming};

fn crossed_line() -> (Sketch, EntityId, EntityId, EntityId) {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(60.0, 0.0));
    sketch.add_constraint(Constraint::Horizontal(line)).unwrap();
    let left = sketch.add_line(Point2::new(20.0, -20.0), Point2::new(20.0, 20.0));
    let right = sketch.add_line(Point2::new(40.0, -20.0), Point2::new(40.0, 20.0));
    (sketch, line, left, right)
}

#[test]
fn trim_cuts_away_the_hovered_piece_between_its_crossings_in_one_undoable_step() {
    let mut harness = Harness::new();
    let (sketch, line, left, right) = crossed_line();
    let [line_label, left_label, right_label] =
        [line, left, right].map(|id| sketch.entity_label(id));
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = harness.sketch(feature).clone();

    harness.use_tool(Key::K);
    assert_eq!(harness.tool(), Some(Tool::Trim));
    assert!(harness.shows(trimming::TRIM_PROMPT));
    harness.point_at(Point2::new(30.0, 0.0));
    assert!(harness.shows(&format!(
        "Trim {line_label} back to {left_label} and {right_label}"
    )));

    harness.click_at(Point2::new(30.0, 0.0));
    let sketch = harness.sketch(feature);
    let lines = entities_of_kind(sketch, "Line");
    assert_eq!(lines.len(), 4);
    let (start, end) = sketch.line_endpoints(line).unwrap();
    assert!(near(start, Point2::ZERO) && near(end, Point2::new(20.0, 0.0)));
    let piece = lines
        .into_iter()
        .find(|id| ![line, left, right].contains(id))
        .unwrap();
    let (piece_start, piece_end) = sketch.line_endpoints(piece).unwrap();
    assert!(near(piece_start, Point2::new(40.0, 0.0)));
    assert!(near(piece_end, Point2::new(60.0, 0.0)));
    assert_eq!(
        constraints_of_kind(sketch, "Horizontal"),
        vec![Constraint::Horizontal(line), Constraint::Horizontal(piece)]
    );
    assert_eq!(
        harness.model.undo_label(),
        Some(format!("Trim {line_label}").as_str())
    );
    assert_eq!(harness.tool(), Some(Tool::Trim));

    harness.settle();
    assert!(
        harness
            .model
            .evaluation()
            .feature(feature)
            .is_some_and(|status| { status.state == caditor_document::FeatureState::UpToDate })
    );
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert!(harness.sketch(feature).same_content(&before));
}

#[test]
fn dragging_across_pieces_trims_each_one_crossed_in_one_step() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let post = sketch.add_line(Point2::new(30.0, -30.0), Point2::new(30.0, 30.0));
    let upper = sketch.add_line(Point2::new(0.0, 10.0), Point2::new(60.0, 10.0));
    let lower = sketch.add_line(Point2::new(0.0, -10.0), Point2::new(60.0, -10.0));
    let feature = edit_free_sketch(&mut harness, sketch);
    harness.click_button("Trim");
    assert_eq!(harness.tool(), Some(Tool::Trim));

    let from = harness.on_screen(Point2::new(15.0, 20.0));
    harness.events.push(Event::PointerMoved(from));
    harness.frame();
    drag_in_sketch(&mut harness, from, Point2::new(15.0, -20.0));

    let sketch = harness.sketch(feature);
    for line in [upper, lower] {
        let (start, end) = sketch.line_endpoints(line).unwrap();
        assert!(near(start, Point2::new(30.0, start.y)), "{start}");
        assert!(near(end, Point2::new(60.0, start.y)), "{end}");
    }
    let (bottom, top) = sketch.line_endpoints(post).unwrap();
    assert!(near(bottom, Point2::new(30.0, -30.0)) && near(top, Point2::new(30.0, 30.0)));
    assert_eq!(harness.model.undo_label(), Some("Trim 2 pieces"));
    assert!(harness.workspace.viewport.selection().is_empty());
}

#[test]
fn extend_reaches_the_curve_in_the_way_and_says_when_there_is_none() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
    let circle = sketch.add_circle(Point2::new(40.0, 0.0), 10.0);
    let [line_label, circle_label] = [line, circle].map(|id| sketch.entity_label(id));
    let feature = edit_free_sketch(&mut harness, sketch);

    harness.use_tool(Key::J);
    assert_eq!(harness.tool(), Some(Tool::Extend));
    assert!(harness.shows(trimming::EXTEND_PROMPT));
    harness.point_at(Point2::new(9.0, 0.0));
    assert!(harness.shows(&format!("Extend {line_label} to {circle_label}")));
    harness.click_at(Point2::new(9.0, 0.0));

    let sketch = harness.sketch(feature);
    let (_, end) = line_ends(sketch, line);
    assert!(near(sketch.point(end).unwrap(), Point2::new(30.0, 0.0)));
    assert_eq!(
        constraints_of_kind(sketch, "Coincident"),
        vec![Constraint::Coincident(end, circle)]
    );
    assert_eq!(
        harness.model.undo_label(),
        Some(format!("Extend {line_label}").as_str())
    );

    harness.settle();
    let before = harness.sketch(feature).clone();
    harness.click_at(Point2::new(1.0, 0.0));
    assert_eq!(
        harness.model.notice().unwrap().text,
        format!("Extend: nothing lies beyond this end of {line_label} to extend it to.")
    );
    assert!(harness.sketch(feature).same_content(&before));
}

#[test]
fn trim_and_extend_act_on_the_keyboard_highlight() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(30.0, 0.0));
    let cutter = sketch.add_line(Point2::new(10.0, -10.0), Point2::new(10.0, 10.0));
    let circle = sketch.add_circle(Point2::new(60.0, 0.0), 5.0);
    let spline = sketch.add_spline(&[
        Point2::new(0.0, 40.0),
        Point2::new(10.0, 50.0),
        Point2::new(20.0, 40.0),
    ]);
    let [line_label, cutter_label, circle_label, spline_label] =
        [line, cutter, circle, spline].map(|id| sketch.entity_label(id));
    let feature = edit_free_sketch(&mut harness, sketch);

    run_from_palette(&mut harness, "trim sketch curves");
    assert_eq!(harness.tool(), Some(Tool::Trim));
    harness.key(Key::N, Modifiers::NONE);
    harness.frame();
    assert!(harness.shows(&format!("Trim {line_label} back to {cutter_label}")));
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    let (start, end) = harness.sketch(feature).line_endpoints(line).unwrap();
    assert!(near(start, Point2::new(10.0, 0.0)) && near(end, Point2::new(30.0, 0.0)));
    harness.settle();

    harness.click_at(Point2::new(10.0, 45.0));
    assert_eq!(
        harness.model.notice().unwrap().text,
        format!(
            "Trim: {spline_label} cannot be trimmed; only lines, circles, arcs and ellipses can."
        )
    );

    run_from_palette(&mut harness, "extend a line");
    assert_eq!(harness.tool(), Some(Tool::Extend));
    harness.key(Key::N, Modifiers::NONE);
    harness.frame();
    assert!(harness.shows(&format!("Extend {line_label} to Vertical axis")));
    harness.key(Key::N, Modifiers::NONE);
    harness.frame();
    assert!(harness.shows(&format!("Extend {line_label} to {circle_label}")));
    harness.key(Key::Space, Modifiers::NONE);
    harness.frame();
    let (_, end) = line_ends(harness.sketch(feature), line);
    assert!(near(
        harness.sketch(feature).point(end).unwrap(),
        Point2::new(55.0, 0.0)
    ));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::Select));
}

#[test]
fn trim_cuts_a_line_back_to_a_sketch_axis_and_joins_it() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(-30.0, 20.0), Point2::new(30.0, 20.0));
    let line_label = sketch.entity_label(line);
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = harness.sketch(feature).clone();

    harness.use_tool(Key::K);
    assert_eq!(harness.tool(), Some(Tool::Trim));
    harness.point_at(Point2::new(-15.0, 20.0));
    assert!(harness.shows(&format!("Trim {line_label} back to Vertical axis")));
    harness.click_at(Point2::new(-15.0, 20.0));

    let sketch = harness.sketch(feature);
    let (start, end) = line_ends(sketch, line);
    assert!(near(sketch.point(start).unwrap(), Point2::new(0.0, 20.0)));
    assert!(near(sketch.point(end).unwrap(), Point2::new(30.0, 20.0)));
    assert_eq!(
        constraints_of_kind(sketch, "Coincident"),
        vec![Constraint::Coincident(start, EntityId::VERTICAL_AXIS)]
    );
    assert_eq!(
        harness.model.undo_label(),
        Some(format!("Trim {line_label}").as_str())
    );

    harness.settle();
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert!(harness.sketch(feature).same_content(&before));
}

#[test]
fn extend_reaches_a_sketch_axis_and_the_axis_itself_is_not_offered() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(10.0, 20.0), Point2::new(30.0, 20.0));
    let line_label = sketch.entity_label(line);
    let feature = edit_free_sketch(&mut harness, sketch);

    harness.use_tool(Key::J);
    harness.point_at(Point2::new(12.0, 20.0));
    assert!(harness.shows(&format!("Extend {line_label} to Vertical axis")));
    harness.click_at(Point2::new(12.0, 20.0));

    let sketch = harness.sketch(feature);
    let (start, _) = line_ends(sketch, line);
    assert!(near(sketch.point(start).unwrap(), Point2::new(0.0, 20.0)));
    assert_eq!(
        constraints_of_kind(sketch, "Coincident"),
        vec![Constraint::Coincident(start, EntityId::VERTICAL_AXIS)]
    );
    harness.settle();

    harness.point_at(Point2::new(40.0, 0.0));
    assert!(!harness.shows("Extend Horizontal axis"));
    assert!(!harness.shows("Trim Horizontal axis"));
}

#[test]
fn trim_cuts_a_line_where_a_collinear_line_overlaps_it() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(10.0, 20.0), Point2::new(70.0, 20.0));
    let other = sketch.add_line(Point2::new(30.0, 20.0), Point2::new(50.0, 20.0));
    let [line_label, other_label] = [line, other].map(|id| sketch.entity_label(id));
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = harness.sketch(feature).clone();
    let (other_start, other_end) = line_ends(&before, other);

    harness.use_tool(Key::K);
    harness.point_at(Point2::new(40.0, 20.0));
    assert!(harness.shows(&format!("Trim {line_label} back to {other_label}")));
    harness.click_at(Point2::new(40.0, 20.0));

    let sketch = harness.sketch(feature);
    let lines = entities_of_kind(sketch, "Line");
    assert_eq!(lines.len(), 3);
    let piece = lines
        .into_iter()
        .find(|id| ![line, other].contains(id))
        .unwrap();
    let (start, near_end) = line_ends(sketch, line);
    let (far_start, end) = line_ends(sketch, piece);
    assert!(near(sketch.point(start).unwrap(), Point2::new(10.0, 20.0)));
    assert!(near(
        sketch.point(near_end).unwrap(),
        Point2::new(30.0, 20.0)
    ));
    assert!(near(
        sketch.point(far_start).unwrap(),
        Point2::new(50.0, 20.0)
    ));
    assert!(near(sketch.point(end).unwrap(), Point2::new(70.0, 20.0)));
    let joints = constraints_of_kind(sketch, "Coincident");
    assert!(joints.contains(&Constraint::Coincident(near_end, other_start)));
    assert!(joints.contains(&Constraint::Coincident(far_start, other_end)));

    harness.settle();
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert!(harness.sketch(feature).same_content(&before));
}
