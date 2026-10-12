use std::f64::consts::{FRAC_PI_2, PI};

use caditor_document::Feature;
use caditor_geometry::{Plane, Point2, Vector2};
use caditor_sketch::{Constraint, Entity, EntityId, FitSpacing, Sketch, SplineKind};
use egui::{Event, Key, Modifiers};

use super::{
    DRAWN, Harness, assert_readable, close, constraints_of_kind, edit_free_sketch,
    entities_of_kind, line_ends, near, offer, radii, refused, run_from_palette, type_point,
    with_type_hint,
};
use crate::{
    commands::Command,
    drawing::Refusal,
    editing::Tool,
    selection::Pickable,
    shape_modes::{CircleMode, RectangleMode, ShapeMode},
    sketch_toolbar,
};

#[test]
fn chained_lines_are_joined_and_clicking_the_last_point_again_stops() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(40.0, 25.0));
    harness.click_at(Point2::new(20.0, 40.0));

    let sketch = harness.sketch(feature);
    let lines = entities_of_kind(sketch, "Line");
    assert_eq!(lines.len(), 2);
    assert_eq!(entities_of_kind(sketch, "Point").len(), 4);
    let (_, first_end) = line_ends(sketch, lines[0]);
    let (second_start, _) = line_ends(sketch, lines[1]);
    assert_eq!(
        constraints_of_kind(sketch, "Coincident"),
        vec![Constraint::Coincident(second_start, first_end)]
    );
    assert_eq!(sketch.point(second_start), sketch.point(first_end));

    harness.point_at(Point2::new(20.3, 40.2));
    assert!(harness.shows("Stop here"));
    harness.click_at(Point2::new(20.3, 40.2));
    assert!(harness.shows("Click the start of the line"));
    assert_eq!(entities_of_kind(harness.sketch(feature), "Line").len(), 2);

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert_eq!(entities_of_kind(harness.sketch(feature), "Line").len(), 1);
}

#[test]
fn a_line_chain_goes_on_as_a_tangent_arc_and_back_to_lines_without_starting_over() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(40.0, 10.0));

    choose_tangent_arc_on_the_ribbon(&mut harness);
    assert_eq!(harness.tool(), Some(Tool::TangentArc));
    harness.click_at(Point2::new(60.0, 30.0));
    let sketch = harness.sketch(feature);
    let [line] = entities_of_kind(sketch, "Line")[..] else {
        panic!("the first line stays");
    };
    let [arc] = entities_of_kind(sketch, "Arc")[..] else {
        panic!("the arc continues the line");
    };
    assert_eq!(
        constraints_of_kind(sketch, "Tangent"),
        vec![Constraint::Tangent(line, arc)]
    );

    harness.use_tool(Key::L);
    assert_eq!(harness.tool(), Some(Tool::Line));
    harness.click_at(Point2::new(60.0, 60.0));
    let sketch = harness.sketch(feature);
    let lines = entities_of_kind(sketch, "Line");
    assert_eq!(lines.len(), 2);
    assert_eq!(constraints_of_kind(sketch, "Coincident").len(), 2);

    choose_tangent_arc_on_the_ribbon(&mut harness);
    harness.key(Key::Backspace, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert_eq!(entities_of_kind(harness.sketch(feature), "Line").len(), 1);
    assert_eq!(entities_of_kind(harness.sketch(feature), "Arc").len(), 1);
    harness.click_at(Point2::new(70.0, 50.0));
    assert_eq!(entities_of_kind(harness.sketch(feature), "Arc").len(), 2);
}

fn choose_tangent_arc_on_the_ribbon(harness: &mut Harness) {
    harness.click_button(sketch_toolbar::ARC_WAYS_LABEL);
    harness.click(Tool::TangentArc.label());
    harness.frame();
}

#[test]
fn backspace_and_undo_step_a_line_chain_back_one_line_at_a_time() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(40.0, 25.0));
    harness.click_at(Point2::new(20.0, 40.0));
    assert_eq!(entities_of_kind(harness.sketch(feature), "Line").len(), 2);

    harness.key(Key::Backspace, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert_eq!(entities_of_kind(harness.sketch(feature), "Line").len(), 1);
    assert!(harness.shows("Click to end the line, Enter or Escape to stop"));

    harness.click_at(Point2::new(60.0, 30.0));
    let sketch = harness.sketch(feature);
    let lines = entities_of_kind(sketch, "Line");
    assert_eq!(lines.len(), 2);
    let (_, first_end) = line_ends(sketch, lines[0]);
    let (second_start, _) = line_ends(sketch, lines[1]);
    assert_eq!(
        constraints_of_kind(sketch, "Coincident"),
        vec![Constraint::Coincident(second_start, first_end)]
    );

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.frame();
    assert_eq!(entities_of_kind(harness.sketch(feature), "Line").len(), 1);
    assert!(harness.shows("Click to end the line, Enter or Escape to stop"));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.frame();
    assert_eq!(entities_of_kind(harness.sketch(feature), "Line").len(), 0);
    assert!(harness.shows("Click to end the line, Enter or Escape to stop"));

    harness.click_at(Point2::new(30.0, 10.0));
    let sketch = harness.sketch(feature);
    let [line] = entities_of_kind(sketch, "Line")[..] else {
        panic!("one line should have been drawn from the first click");
    };
    let (start, _) = line_ends(sketch, line);
    let anchor = sketch.point(start).unwrap();
    assert!(anchor.distance(Point2::new(10.0, 10.0)) < 1e-3, "{anchor}");
}

#[test]
fn double_clicking_a_curve_selects_the_chain_it_belongs_to() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let a = Point2::new(10.0, 10.0);
    let b = Point2::new(40.0, 10.0);
    let c = Point2::new(40.0, 30.0);
    let first = sketch.add_line(a, b);
    let second = sketch.add_line(b, c);
    let (_, joint) = line_ends(&sketch, first);
    let (joined, _) = line_ends(&sketch, second);
    sketch
        .add_constraint(Constraint::Coincident(joint, joined))
        .unwrap();
    let apart = sketch.add_line(Point2::new(10.0, 50.0), Point2::new(40.0, 50.0));
    let feature = edit_free_sketch(&mut harness, sketch);
    let pick = |entity| Pickable::SketchEntity { feature, entity };

    let position = harness.hover_pickable(Plane::XY, Point2::new(25.0, 10.0), pick(first));
    harness.press(position);
    harness.press(position);
    harness.frame();

    let selected: Vec<Pickable> = harness.workspace.viewport.selection().iter().collect();
    assert_eq!(selected.len(), 2, "{selected:?}");
    assert!(selected.contains(&pick(first)) && selected.contains(&pick(second)));

    let position = harness.hover_pickable(Plane::XY, Point2::new(25.0, 50.0), pick(apart));
    harness.press(position);
    harness.press(position);
    harness.frame();

    let alone: Vec<Pickable> = harness.workspace.viewport.selection().iter().collect();
    assert_eq!(alone, [pick(apart)]);
}

#[test]
fn a_line_chain_stops_when_it_closes_on_its_start() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(40.0, 10.0));
    harness.click_at(Point2::new(25.0, 35.0));
    harness.click_at(Point2::new(10.2, 10.1));
    assert!(harness.shows("Click the start of the line"));

    let sketch = harness.sketch(feature);
    let lines = entities_of_kind(sketch, "Line");
    assert_eq!(lines.len(), 3);
    let (first_start, _) = line_ends(sketch, lines[0]);
    let (_, last_end) = line_ends(sketch, lines[2]);
    assert!(
        constraints_of_kind(sketch, "Coincident")
            .contains(&Constraint::Coincident(last_end, first_start))
    );

    harness.click_at(Point2::new(60.0, 10.0));
    harness.click_at(Point2::new(60.0, 40.0));
    assert_eq!(entities_of_kind(harness.sketch(feature), "Line").len(), 4);
    assert!(!harness.shows("Click the start of the line"));
}

#[test]
fn a_rectangle_is_four_joined_lines_with_four_degrees_of_freedom() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::R);
    assert!(harness.shows("Click the rectangle's first corner"));
    harness.click_at(Point2::new(10.0, 10.0));
    assert!(harness.shows("Click the opposite corner"));
    harness.click_at(Point2::new(40.0, 30.0));

    let sketch = harness.sketch(feature);
    assert_eq!(entities_of_kind(sketch, "Line").len(), 4);
    assert_eq!(entities_of_kind(sketch, "Point").len(), 8);
    assert_eq!(constraints_of_kind(sketch, "Coincident").len(), 4);
    assert_eq!(constraints_of_kind(sketch, "Horizontal").len(), 2);
    assert_eq!(constraints_of_kind(sketch, "Vertical").len(), 2);
    assert_eq!(harness.model.undo_label(), Some("Draw rectangle"));
    harness.settle();
    assert!(harness.shows("4 degrees of freedom left"));
    assert!(harness.shows("Click the rectangle's first corner"));
}

#[test]
fn circles_and_arcs_get_the_drawn_geometry() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::C);
    assert!(harness.shows("Click the circle's centre"));
    harness.click_at(Point2::new(20.0, 20.0));
    assert!(harness.shows("Click a point on the circle"));
    harness.click_at(Point2::new(30.0, 20.0));
    let circle = entities_of_kind(harness.sketch(feature), "Circle")[0];
    let (center, radius) = harness.sketch(feature).circle(circle).unwrap();
    assert!(near(center, Point2::new(20.0, 20.0)), "{center}");
    assert!((radius - 10.0).abs() < DRAWN, "{radius}");
    assert_eq!(harness.model.undo_label(), Some("Draw circle"));

    harness.use_tool(Key::A);
    assert!(harness.shows("Click the arc's centre"));
    let center = Point2::new(-20.0, 20.0);
    harness.click_at(center);
    harness.click_at(center + Vector2::new(10.0, 0.0));
    for step in [Vector2::new(7.0, 7.0), Vector2::new(0.0, 10.0)] {
        harness.point_at(center + step);
    }
    harness.click_at(center + Vector2::new(-4.0, 3.0));
    let turned = entities_of_kind(harness.sketch(feature), "Arc")[0];
    let arc = harness.sketch(feature).arc(turned).unwrap();
    assert!((arc.radius - 10.0).abs() < DRAWN);
    assert!(arc.start_angle.abs() < DRAWN);
    let expected = 3f64.atan2(-4.0);
    assert!((arc.sweep - expected).abs() < DRAWN, "{}", arc.sweep);

    let center = Point2::new(-20.0, -20.0);
    harness.click_at(center);
    harness.click_at(center + Vector2::new(10.0, 0.0));
    for step in [Vector2::new(7.0, -7.0), Vector2::new(0.0, -10.0)] {
        harness.point_at(center + step);
    }
    harness.click_at(center + Vector2::new(-4.0, -3.0));
    let arcs = entities_of_kind(harness.sketch(feature), "Arc");
    let swept = harness.sketch(feature).arc(arcs[1]).unwrap();
    assert!((swept.sweep - expected).abs() < DRAWN, "{}", swept.sweep);
    let Some(Entity::Arc { end, .. }) = harness.sketch(feature).entity(arcs[1]) else {
        panic!("expected an arc");
    };
    assert!(near(
        harness.sketch(feature).point(*end).unwrap(),
        center + Vector2::new(10.0, 0.0)
    ));
    assert_eq!(harness.model.undo_label(), Some("Draw arc"));
}

fn arc_end(sketch: &Sketch, arc: EntityId) -> EntityId {
    let Some(Entity::Arc { end, .. }) = sketch.entity(arc) else {
        panic!("expected an arc");
    };
    *end
}

fn joins(sketch: &Sketch, point: EntityId, other: EntityId) -> bool {
    constraints_of_kind(sketch, "Coincident")
        .iter()
        .any(|constraint| {
            matches!(constraint, Constraint::Coincident(a, b)
            if (*a == point && *b == other) || (*a == other && *b == point))
        })
}

#[test]
fn an_arc_end_and_a_circle_rim_join_only_what_they_land_on() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    type_point(&mut harness, "46, 20");
    type_point(&mut harness, "46, 60");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let line = entities_of_kind(harness.sketch(feature), "Line")[0];
    let on_line = format!("On {}", harness.sketch(feature).entity_label(line));
    harness.use_tool(Key::P);
    type_point(&mut harness, "47.5, 47");
    let lone = harness
        .sketch(feature)
        .entities()
        .find(
            |(_, entity)| matches!(entity, Entity::Point(at) if near(*at, Point2::new(47.5, 47.0))),
        )
        .map(|(id, _)| id)
        .expect("the lone point was placed");
    let lone_label = format!("On {}", harness.sketch(feature).entity_label(lone));

    harness.use_tool(Key::A);
    type_point(&mut harness, "40, 40");
    type_point(&mut harness, "50, 40");
    harness.point_at(Point2::new(47.0, 46.5));
    let near_lone_on_arc = harness.shows(&lone_label);
    harness.point_at(Point2::new(46.1, 48.1));
    let near_crossing = harness.shows(&on_line);
    harness.click_at(Point2::new(46.1, 48.1));
    harness.settle();
    let arc = entities_of_kind(harness.sketch(feature), "Arc")[0];
    let crossing_end = arc_end(harness.sketch(feature), arc);

    type_point(&mut harness, "40, 40");
    type_point(&mut harness, "50, 40");
    harness.point_at(Point2::new(47.0, 46.0));
    type_point(&mut harness, "47.5, 47");
    let arcs = entities_of_kind(harness.sketch(feature), "Arc");
    let projected_end = arc_end(harness.sketch(feature), arcs[1]);
    let projected = harness.sketch(feature).point(projected_end).unwrap();

    harness.use_tool(Key::C);
    type_point(&mut harness, "60, 30");
    harness.point_at(Point2::new(46.2, 30.0));
    let rim_on_line = harness.shows(&on_line);
    harness.click_at(Point2::new(46.2, 30.0));
    harness.settle();
    let circle = entities_of_kind(harness.sketch(feature), "Circle")[0];

    assert!(!near_lone_on_arc);
    assert!(near_crossing);
    assert!(joins(harness.sketch(feature), crossing_end, line));
    assert!(near(
        harness.shown(feature).point(crossing_end).unwrap(),
        Point2::new(46.0, 48.0)
    ));
    assert!(!joins(harness.sketch(feature), projected_end, lone));
    assert!((projected.distance(Point2::new(40.0, 40.0)) - 10.0).abs() < DRAWN);
    assert!(!rim_on_line);
    assert!(!joins(harness.sketch(feature), line, circle));
    assert!(
        constraints_of_kind(harness.sketch(feature), "Coincident")
            .iter()
            .all(|constraint| !constraint.entities().contains(&circle))
    );
}

#[test]
fn q_switches_the_selected_curves_or_new_drawing_to_construction_geometry() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    let construction = |harness: &Harness| {
        harness
            .workspace
            .editing
            .active()
            .is_some_and(|active| active.construction)
    };
    let dashed = |harness: &mut Harness| {
        harness
            .built()
            .scene
            .lines()
            .filter(|line| matches!(line.stroke, caditor_render::Stroke::Dashed { .. }))
            .count()
    };
    harness.use_tool(Key::L);
    type_point(&mut harness, "0, 0");
    type_point(&mut harness, "20, 0");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let ordinary = entities_of_kind(harness.sketch(feature), "Line")[0];

    harness.key(Key::Q, Modifiers::NONE);
    harness.frame();
    let drawing_construction = construction(&harness);
    harness.use_tool(Key::L);
    type_point(&mut harness, "0, 10");
    type_point(&mut harness, "20, 10");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let guide = entities_of_kind(harness.sketch(feature), "Line")[1];
    harness.key(Key::Q, Modifiers::NONE);
    harness.frame();

    assert!(drawing_construction);
    assert!(!construction(&harness));
    assert!(harness.sketch(feature).is_construction(guide));
    assert!(!harness.sketch(feature).is_construction(ordinary));
    assert_eq!(dashed(&mut harness), 1);

    let line = |entity| Pickable::SketchEntity { feature, entity };
    harness.select([line(ordinary)]);
    harness.key(Key::Q, Modifiers::NONE);
    harness.settle();
    assert!(harness.sketch(feature).is_construction(ordinary));
    assert_eq!(dashed(&mut harness), 2);

    harness.select([line(ordinary), line(guide)]);
    harness.key(Key::Q, Modifiers::NONE);
    harness.settle();
    assert_eq!(harness.sketch(feature).construction().len(), 0);
    assert!(!construction(&harness));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.settle();
    assert_eq!(harness.sketch(feature).construction().len(), 2);

    harness.select([]);
    harness.click_button("Construction");
    assert!(construction(&harness));
}

#[test]
fn a_typed_arc_goes_the_shorter_way_and_x_sends_it_the_long_way() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::A);
    assert!(offer(&harness, Command::ReverseArc).availability.is_err());
    type_point(&mut harness, "0, 0");
    type_point(&mut harness, "10 mm, 0");
    harness.expand_keys();
    assert!(harness.shows_hint("X: the other way round   The arc follows your sweep around the centre, a typed end the shorter way   Esc: cancel the arc   x,y or length<angle: type an exact point"));
    type_point(&mut harness, "0, -10 mm");
    let arcs = entities_of_kind(harness.sketch(feature), "Arc");
    let short = harness.sketch(feature).arc(arcs[0]).unwrap();
    assert!((short.sweep - FRAC_PI_2).abs() < DRAWN, "{}", short.sweep);

    type_point(&mut harness, "0, 0");
    type_point(&mut harness, "10 mm, 0");
    harness.key(Key::X, Modifiers::NONE);
    harness.frame();
    type_point(&mut harness, "0, -10 mm");
    let arcs = entities_of_kind(harness.sketch(feature), "Arc");
    let long = harness.sketch(feature).arc(arcs[1]).unwrap();
    assert!(
        (long.sweep - 3.0 * FRAC_PI_2).abs() < DRAWN,
        "{}",
        long.sweep
    );
    let Some(Entity::Arc { start, .. }) = harness.sketch(feature).entity(arcs[1]) else {
        panic!("expected an arc");
    };
    assert!(near(
        harness.sketch(feature).point(*start).unwrap(),
        Point2::new(10.0, 0.0)
    ));

    type_point(&mut harness, "0, 0");
    type_point(&mut harness, "10 mm, 0");
    run_from_palette(&mut harness, "reverse the arc");
    type_point(&mut harness, "0, -10 mm");
    let arcs = entities_of_kind(harness.sketch(feature), "Arc");
    let reversed = harness.sketch(feature).arc(arcs[2]).unwrap();
    assert!(
        (reversed.sweep - 3.0 * FRAC_PI_2).abs() < DRAWN,
        "{}",
        reversed.sweep
    );

    type_point(&mut harness, "0, 0");
    type_point(&mut harness, "10 mm, 0");
    type_point(&mut harness, "-10 mm, 0");
    let arcs = entities_of_kind(harness.sketch(feature), "Arc");
    let half = harness.sketch(feature).arc(arcs[3]).unwrap();
    assert!((half.sweep - PI).abs() < DRAWN, "{}", half.sweep);
    assert!(half.start_angle.abs() < DRAWN, "{}", half.start_angle);
}

fn line_lengths(sketch: &Sketch) -> Vec<f64> {
    entities_of_kind(sketch, "Line")
        .into_iter()
        .map(|line| {
            let (start, end) = sketch.line_endpoints(line).unwrap();
            start.distance(end)
        })
        .collect()
}

#[test]
fn a_three_point_arc_runs_from_its_start_through_the_third_point_to_its_end() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool_with(Key::A, Modifiers::ALT);
    assert_eq!(harness.tool(), Some(Tool::ThreePointArc));
    assert!(harness.shows("Click where the arc starts"));
    type_point(&mut harness, "10, 0");
    assert!(harness.shows("Click where the arc ends"));
    type_point(&mut harness, "-10, 0");
    assert!(harness.shows("Click a point the arc passes through"));
    type_point(&mut harness, "30, 0");
    assert!(harness.shows(Refusal::ArcInLine.reason()));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.point_at(Point2::new(1.0, -5.0));
    type_point(&mut harness, "6, -8");

    let sketch = harness.sketch(feature);
    let arcs = entities_of_kind(sketch, "Arc");
    assert_eq!(arcs.len(), 1);
    let arc = sketch.arc(arcs[0]).unwrap();
    assert!(near(arc.center, Point2::ZERO), "{}", arc.center);
    assert!((arc.radius - 10.0).abs() < DRAWN);
    assert!((arc.sweep - PI).abs() < DRAWN, "{}", arc.sweep);
    assert!(
        (arc.start_angle.abs() - PI).abs() < DRAWN,
        "{}",
        arc.start_angle
    );
    assert_eq!(harness.model.undo_label(), Some("Draw 3-point arc"));
    assert!(harness.shows("Click where the arc starts"));
}

#[test]
fn tangent_arcs_continue_smoothly_from_a_line_and_from_each_other() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    keep_typed_values_free(&mut harness);
    harness.use_tool(Key::L);
    type_point(&mut harness, "0, 0");
    type_point(&mut harness, "20, 0");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let line = entities_of_kind(harness.sketch(feature), "Line")[0];
    let continue_line = format!("Continue {}", harness.sketch(feature).entity_label(line));

    harness.use_tool(Key::T);
    assert!(harness.shows("Click the end of a line, arc or spline to continue from"));
    type_point(&mut harness, "40, 40");
    let refused = harness.shows(Refusal::TangentStart.reason());
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.point_at(Point2::new(20.0, 0.0));
    let offered = harness.shows(&continue_line);
    type_point(&mut harness, "20, 0");
    type_point(&mut harness, "30, 10");
    type_point(&mut harness, "40, 20");
    assert!(harness.shows("Click where the arc ends, Enter or Escape to stop"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.settle();

    let sketch = harness.sketch(feature);
    let arcs = entities_of_kind(sketch, "Arc");
    assert!(refused);
    assert!(offered);
    assert_eq!(arcs.len(), 2);
    assert_eq!(constraints_of_kind(sketch, "Tangent").len(), 2);
    let first = harness.shown(feature).arc(arcs[0]).unwrap();
    let second = harness.shown(feature).arc(arcs[1]).unwrap();
    assert!(
        near(first.center, Point2::new(20.0, 10.0)),
        "{}",
        first.center
    );
    assert!((first.sweep - FRAC_PI_2).abs() < DRAWN, "{}", first.sweep);
    assert!(
        near(second.center, Point2::new(40.0, 10.0)),
        "{}",
        second.center
    );
    assert!((second.sweep - FRAC_PI_2).abs() < DRAWN, "{}", second.sweep);
    assert!(harness.shows("6 degrees of freedom left"));
    assert_eq!(harness.model.undo_label(), Some("Draw tangent arc"));
    assert!(harness.shows("Click the end of a line, arc or spline to continue from"));
}

#[test]
fn a_slot_has_round_ends_of_one_radius_tangent_to_its_sides() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::U);
    assert!(harness.shows("Click the centre of the slot's first end"));
    type_point(&mut harness, "0, 0");
    type_point(&mut harness, "30, 10");
    type_point(&mut harness, "@0, 0");
    assert!(harness.shows(Refusal::SlotWidth.reason()));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.point_at(Point2::new(20.0, 20.0));
    type_point(&mut harness, "@-5, 15");
    harness.settle();

    let sketch = harness.sketch(feature);
    let arcs = entities_of_kind(sketch, "Arc");
    assert_eq!(arcs.len(), 2);
    assert_eq!(entities_of_kind(sketch, "Line").len(), 2);
    assert_eq!(constraints_of_kind(sketch, "Tangent").len(), 4);
    assert_eq!(constraints_of_kind(sketch, "Equal").len(), 1);
    let radius = 10f64.hypot(30.0) / 2.0;
    for arc in &arcs {
        let arc = harness.shown(feature).arc(*arc).unwrap();
        assert!((arc.radius - radius).abs() < DRAWN, "{}", arc.radius);
        assert!((arc.sweep - PI).abs() < DRAWN, "{}", arc.sweep);
    }
    for length in line_lengths(&harness.shown(feature)) {
        assert!((length - 10f64.hypot(30.0)).abs() < DRAWN, "{length}");
    }
    assert!(harness.shows("3 degrees of freedom left"));
    assert_eq!(harness.model.undo_label(), Some("Draw slot"));

    harness.click_at(Point2::new(5.0, -40.0));
    harness.click_at(Point2::new(35.0, -39.9));
    harness.click_at(Point2::new(15.0, -35.0));
    harness.settle();
    assert_eq!(
        constraints_of_kind(harness.sketch(feature), "Horizontal").len(),
        1
    );
    assert!(harness.shows("7 degrees of freedom left"));

    harness.click("Extrude");
    harness.settle();
    let extrude = harness
        .document()
        .features()
        .find(|feature| feature.name == "Extrude 1")
        .map(Feature::id)
        .expect("the slots were extruded");
    let slot_area = |length: f64, radius: f64| length * 2.0 * radius + PI * radius * radius;
    let area = slot_area(10f64.hypot(30.0), radius) + slot_area(30.0, 5.0);
    let volume = harness.body_volume(extrude);
    assert!((volume / (10.0 * area) - 1.0).abs() < 1e-2, "{volume}");
}

#[test]
fn a_polygon_is_regular_with_as_many_sides_as_chosen() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    assert!(offer(&harness, Command::MoreSides).availability.is_err());
    harness.use_tool(Key::G);
    assert!(harness.shows("Click the hexagon's centre"));
    harness.key(Key::CloseBracket, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert!(harness.shows("Click the heptagon's centre"));
    for _ in 0..4 {
        harness.key(Key::OpenBracket, Modifiers::NONE);
        harness.frame();
    }
    harness.frame();
    assert!(harness.shows("Click the triangle's centre"));
    assert!(offer(&harness, Command::FewerSides).availability.is_err());
    run_from_palette(&mut harness, "another side");
    harness.frame();
    assert!(harness.shows("Click the square's centre"));

    harness.use_tool(Key::L);
    harness.use_tool(Key::G);
    assert!(harness.shows("Click the square's centre"));
    type_point(&mut harness, "5, 5");
    assert!(harness.shows("Click a corner of the square"));
    type_point(&mut harness, "15, 5");
    harness.settle();

    let sketch = harness.sketch(feature);
    let lines = entities_of_kind(sketch, "Line");
    let circles = entities_of_kind(sketch, "Circle");
    assert_eq!(lines.len(), 4);
    assert_eq!(circles.len(), 1);
    assert!(sketch.is_construction(circles[0]));
    assert!(lines.iter().all(|line| !sketch.is_construction(*line)));
    assert_eq!(constraints_of_kind(sketch, "Equal").len(), 3);
    assert_eq!(constraints_of_kind(sketch, "Coincident").len(), 8);
    for length in line_lengths(&harness.shown(feature)) {
        assert!((length - 10.0 * 2f64.sqrt()).abs() < DRAWN, "{length}");
    }
    assert!(harness.shows("4 degrees of freedom left"));
    assert_eq!(harness.model.undo_label(), Some("Draw square"));

    harness.click("Extrude");
    harness.settle();
    let extrude = harness
        .document()
        .features()
        .find(|feature| feature.name == "Extrude 1")
        .map(Feature::id)
        .expect("the square was extruded");
    assert!((harness.body_volume(extrude) - 2000.0).abs() < 1.0);
}

#[test]
fn a_polygons_sides_are_typed_as_a_count_or_scrubbed_with_shift_and_the_pointer() {
    let mut harness = Harness::new();
    harness.draw_on_new_sketch();
    harness.use_tool(Key::G);
    harness.click_at(Point2::new(20.0, 20.0));
    harness.point_at(Point2::new(35.0, 20.0));
    assert!(harness.shows("R 15.00 mm   6 sides"));

    type_point(&mut harness, "9 sides");
    harness.point_at(Point2::new(35.0, 20.0));
    assert!(harness.shows("R 15.00 mm   9 sides"));

    type_point(&mut harness, "2 sides");
    assert!(harness.shows("A polygon needs at least three sides"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();
    type_point(&mut harness, "65 sides");
    assert!(harness.shows("A polygon has at most 64 sides"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();

    harness.point_at(Point2::new(35.0, 20.0));
    let start = harness.on_screen(Point2::new(35.0, 20.0));
    harness.hold(Modifiers::SHIFT);
    harness.point_at(Point2::new(35.0, 20.0));
    for (offset, sides) in [(48.0, 11), (-72.0, 6), (-96.0, 5), (150.0, 15)] {
        harness
            .events
            .push(Event::PointerMoved(start + egui::vec2(offset, 0.0)));
        harness.frame();
        assert!(
            harness.shows(&format!("R 15.00 mm   {sides} sides")),
            "{offset}"
        );
    }
    harness.hold(Modifiers::NONE);
    harness.point_at(Point2::new(30.0, 20.0));
    assert!(harness.shows("R 10.00 mm   15 sides"));
}

#[test]
fn pressing_a_shape_key_again_cycles_its_ways_of_drawing_and_each_is_remembered() {
    let mut harness = Harness::new();
    harness.draw_on_new_sketch();

    harness.use_tool(Key::R);
    assert!(harness.shows("Click the rectangle's first corner"));
    assert!(harness.shows_hint(&with_type_hint(
        "Rectangle from two corners   R: from its centre",
        "Esc: back to Select"
    )));
    harness.use_tool(Key::R);
    assert_eq!(harness.tool(), Some(Tool::Rectangle));
    assert!(harness.shows("Click the rectangle's centre"));
    assert!(harness.shows_hint(&with_type_hint(
        "Rectangle from its centre   R: from three points",
        "Esc: back to Select"
    )));
    harness.use_tool(Key::R);
    assert!(harness.shows("Click where the rectangle's first side starts"));
    harness.use_tool(Key::R);
    assert!(harness.shows("Click the rectangle's first corner"));
    harness.use_tool(Key::R);
    harness.use_tool(Key::L);
    harness.use_tool(Key::C);
    assert!(harness.shows("Click the circle's centre"));
    harness.use_tool(Key::R);
    assert!(harness.shows("Click the rectangle's centre"));
    assert_eq!(
        harness.workspace.editing.modes().of(Tool::Rectangle),
        Some(ShapeMode::Rectangle(RectangleMode::Center))
    );

    harness.use_tool(Key::U);
    harness.click_at(Point2::new(10.0, 10.0));
    assert!(harness.workspace.viewport.is_drawing());
    harness.use_tool(Key::U);
    assert!(harness.workspace.viewport.is_drawing());
    assert!(harness.shows("Click the centre of the slot's other end"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.use_tool(Key::U);
    assert!(!harness.workspace.viewport.is_drawing());
    assert!(harness.shows("Click the slot's centre"));
    harness.use_tool(Key::G);
    harness.use_tool(Key::G);
    harness.use_tool(Key::G);
    assert!(harness.shows("Click where a side of the hexagon starts"));
    harness.use_tool(Key::G);
    assert!(harness.shows("Click the hexagon's centre"));
    harness.expand_keys();
    assert!(harness.shows_hint(&with_type_hint(
        "Polygon from its centre and a corner   G: from its centre and a side's middle   ] or \
         [: more or fewer sides   Shift: move sideways to set the sides",
        "Esc: back to Select"
    )));
    assert!(harness.shows("6 sides: set the sides"));
}

#[test]
fn every_way_of_drawing_is_a_command_in_the_palette_and_in_a_menu_on_its_button() {
    let mut harness = Harness::new();
    harness.draw_on_new_sketch();
    for mode in ShapeMode::ALL {
        assert!(
            offer(&harness, Command::ShapeMode(mode))
                .availability
                .is_ok()
        );
    }

    run_from_palette(&mut harness, "draw circle through three points");
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::Circle));
    assert!(harness.shows("Click a first point on the circle"));
    harness.hover("Circle");
    assert!(
        harness.shows("Draw a circle through three points on it (C)\nC again: from its centre")
    );

    harness.hover_button(&sketch_toolbar::modes_label(Tool::Slot));
    assert!(harness.shows(&sketch_toolbar::modes_label(Tool::Slot)));
    assert!(!harness.shows(
        "Draw a slot from the centres of its round ends and its width (U)\nU again: from its centre"
    ));
    harness.hover("Slot");
    assert!(harness.shows(
        "Draw a slot from the centres of its round ends and its width (U)\nU again: from its centre"
    ));
    harness.click_button(&sketch_toolbar::modes_label(Tool::Slot));
    assert!(harness.shows("From the centres of its ends"));
    assert!(harness.shows("From its centre"));
    harness.click("Along an arc");
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::Slot));
    assert!(harness.shows("Click the centre of the arc the slot follows"));
    assert!(!harness.shows("From the centres of its ends"));
    assert_eq!(
        harness.workspace.editing.modes().of(Tool::Circle),
        Some(ShapeMode::Circle(CircleMode::ThreePoints))
    );

    harness.select([]);
    harness.frame();
    assert_readable(&harness, "The sketch bar with its mode menus");

    harness.click("Sketch");
    harness.hover("Drawing tools");
    harness.hover("Ways to draw shapes");
    harness.click("Draw polygon from one side");
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::Polygon));
    assert!(harness.shows("Click where a side of the hexagon starts"));
}

#[test]
fn a_rectangle_from_its_centre_stays_centred_on_it() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::R);
    harness.use_tool(Key::R);

    type_point(&mut harness, "0, 0");
    assert!(harness.shows("Click a corner of the rectangle"));
    refused(&mut harness, "15, 0", Refusal::Rectangle);
    type_point(&mut harness, "15, 10");
    harness.settle();

    let sketch = harness.sketch(feature);
    let mut lengths = line_lengths(&harness.shown(feature));
    lengths.sort_by(f64::total_cmp);
    assert_eq!(entities_of_kind(sketch, "Line").len(), 4);
    assert_eq!(constraints_of_kind(sketch, "Symmetric").len(), 1);
    assert_eq!(constraints_of_kind(sketch, "Horizontal").len(), 2);
    assert_eq!(constraints_of_kind(sketch, "Vertical").len(), 2);
    assert_eq!(constraints_of_kind(sketch, "Coincident").len(), 5);
    assert!(close(&lengths, &[20.0, 20.0, 30.0, 30.0]), "{lengths:?}");
    assert!(harness.shows("2 degrees of freedom left"));
    assert_eq!(harness.model.undo_label(), Some("Draw rectangle"));
    assert!(harness.shows("Click the rectangle's centre"));
}

#[test]
fn a_rectangle_from_three_points_turns_with_its_first_side_and_stays_square() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::R);
    harness.use_tool(Key::R);
    harness.use_tool(Key::R);

    type_point(&mut harness, "10, 0");
    assert!(harness.shows("Click where its first side ends"));
    refused(&mut harness, "10, 0", Refusal::RectangleSide);
    type_point(&mut harness, "40, 40");
    assert!(harness.shows("Click to set the rectangle's width"));
    refused(&mut harness, "@3, 4", Refusal::RectangleWidth);
    type_point(&mut harness, "@-8, 6");
    harness.settle();

    let sketch = harness.sketch(feature);
    let shown = harness.shown(feature);
    let corners: Vec<Point2> = entities_of_kind(&shown, "Line")
        .into_iter()
        .map(|line| shown.line_endpoints(line).unwrap().0)
        .collect();
    let expected = [
        Point2::new(10.0, 0.0),
        Point2::new(40.0, 40.0),
        Point2::new(32.0, 46.0),
        Point2::new(2.0, 6.0),
    ];
    assert_eq!(constraints_of_kind(sketch, "Perpendicular").len(), 1);
    assert_eq!(constraints_of_kind(sketch, "Parallel").len(), 2);
    assert_eq!(constraints_of_kind(sketch, "Coincident").len(), 4);
    assert_eq!(corners.len(), expected.len());
    for (corner, expected) in corners.iter().zip(expected) {
        assert!(near(*corner, expected), "{corner} is not {expected}");
    }
    assert!(harness.shows("5 degrees of freedom left"));
    assert_eq!(harness.model.undo_label(), Some("Draw rectangle"));

    harness.click_at(Point2::new(10.0, -30.0));
    harness.click_at(Point2::new(40.0, -29.9));
    harness.click_at(Point2::new(30.0, -20.0));
    harness.settle();
    assert_eq!(
        constraints_of_kind(harness.sketch(feature), "Horizontal").len(),
        1
    );
    assert!(harness.shows("9 degrees of freedom left"));
}

#[test]
fn a_circle_through_the_ends_of_a_diameter_is_centred_between_them() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    keep_typed_values_free(&mut harness);
    harness.use_tool(Key::P);
    type_point(&mut harness, "10, 10");
    type_point(&mut harness, "30, 10");
    harness.use_tool(Key::C);
    harness.use_tool(Key::C);

    assert!(harness.shows("Click one end of the circle's diameter"));
    type_point(&mut harness, "10, 10");
    assert!(harness.shows("Click the other end of the diameter"));
    refused(&mut harness, "10, 10", Refusal::CircleDiameter);
    type_point(&mut harness, "30, 10");
    harness.settle();
    let joined = harness.shows("4 degrees of freedom left");
    type_point(&mut harness, "50, 0");
    type_point(&mut harness, "50, 30");
    harness.settle();

    let sketch = harness.sketch(feature);
    let circles = entities_of_kind(sketch, "Circle");
    let (center, radius) = sketch.circle(circles[0]).unwrap();
    let (free_center, free_radius) = sketch.circle(circles[1]).unwrap();
    assert!(joined);
    assert!(near(center, Point2::new(20.0, 10.0)), "{center}");
    assert!((radius - 10.0).abs() < DRAWN);
    assert!(near(free_center, Point2::new(50.0, 15.0)), "{free_center}");
    assert!((free_radius - 15.0).abs() < DRAWN);
    assert_eq!(constraints_of_kind(sketch, "Symmetric").len(), 1);
    assert_eq!(constraints_of_kind(sketch, "Coincident").len(), 1);
    assert!(harness.shows("7 degrees of freedom left"));
    assert_eq!(harness.model.undo_label(), Some("Draw circle"));
}

#[test]
fn a_circle_through_three_points_passes_through_each() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    keep_typed_values_free(&mut harness);
    harness.use_tool(Key::P);
    type_point(&mut harness, "0, 10");
    harness.use_tool(Key::C);
    harness.use_tool(Key::C);
    harness.use_tool(Key::C);

    type_point(&mut harness, "10, 0");
    assert!(harness.shows("Click a second point on the circle"));
    refused(&mut harness, "10, 0", Refusal::CircleInLine);
    type_point(&mut harness, "0, 10");
    assert!(harness.shows("Click a third point on the circle"));
    refused(&mut harness, "-10, 20", Refusal::CircleInLine);
    type_point(&mut harness, "-6, -8");
    harness.settle();

    let sketch = harness.sketch(feature);
    let circles = entities_of_kind(sketch, "Circle");
    let (center, radius) = harness.shown(feature).circle(circles[0]).unwrap();
    assert_eq!(circles.len(), 1);
    assert!(near(center, Point2::ZERO), "{center}");
    assert!((radius - 10.0).abs() < DRAWN, "{radius}");
    assert_eq!(constraints_of_kind(sketch, "Coincident").len(), 1);
    assert!(harness.shows("4 degrees of freedom left"));
    assert_eq!(harness.model.undo_label(), Some("Draw circle"));
    assert!(harness.shows("Click a first point on the circle"));
}

#[test]
fn a_polygon_sized_by_the_middle_of_a_side_keeps_a_circle_inside_touching_it() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::G);
    harness.use_tool(Key::G);

    type_point(&mut harness, "0, 0");
    assert!(harness.shows("Click the middle of a side of the hexagon"));
    refused(&mut harness, "0, 0", Refusal::PolygonSideMiddle);
    type_point(&mut harness, "0, -5");
    harness.settle();

    let sketch = harness.sketch(feature);
    let shown = harness.shown(feature);
    let circles = entities_of_kind(sketch, "Circle");
    let across_corners = 5.0 / (PI / 6.0).cos();
    assert_eq!(entities_of_kind(sketch, "Line").len(), 6);
    assert!(circles.iter().all(|circle| sketch.is_construction(*circle)));
    assert!(close(&radii(&shown, "Circle"), &[5.0, across_corners]));
    assert_eq!(constraints_of_kind(sketch, "Tangent").len(), 1);
    assert_eq!(constraints_of_kind(sketch, "Equal").len(), 5);
    for line in entities_of_kind(&shown, "Line") {
        let (start, end) = shown.line_endpoints(line).unwrap();
        assert!((start.midpoint(end).length() - 5.0).abs() < DRAWN);
    }
    assert!(harness.shows("2 degrees of freedom left"));
    assert_eq!(harness.model.undo_label(), Some("Draw hexagon"));
}

#[test]
fn a_polygon_drawn_from_one_side_lies_to_its_left() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    for _ in 0..3 {
        harness.use_tool(Key::G);
    }
    harness.key(Key::OpenBracket, Modifiers::NONE);
    harness.frame();

    type_point(&mut harness, "10, 0");
    assert!(harness.shows("Click where that side of the pentagon ends"));
    refused(&mut harness, "10, 0", Refusal::PolygonSide);
    type_point(&mut harness, "20, 0");
    harness.settle();

    let shown = harness.shown(feature);
    let (center, _) = shown.circle(entities_of_kind(&shown, "Circle")[0]).unwrap();
    assert_eq!(entities_of_kind(&shown, "Line").len(), 5);
    assert!(
        center.y > 0.0 && (center.x - 15.0).abs() < DRAWN,
        "{center}"
    );
    for length in line_lengths(&shown) {
        assert!((length - 10.0).abs() < DRAWN, "{length}");
    }
    assert!(harness.shows("4 degrees of freedom left"));
    assert_eq!(harness.model.undo_label(), Some("Draw pentagon"));

    harness.click_at(Point2::new(40.0, -40.0));
    harness.click_at(Point2::new(55.0, -39.95));
    harness.settle();
    assert_eq!(
        constraints_of_kind(harness.sketch(feature), "Horizontal").len(),
        1
    );
    assert!(harness.shows("7 degrees of freedom left"));
}

#[test]
fn a_slot_from_its_centre_stays_symmetric_about_it() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::U);
    harness.use_tool(Key::U);

    type_point(&mut harness, "0, 0");
    assert!(harness.shows("Click the centre of one of the slot's ends"));
    refused(&mut harness, "0, 0", Refusal::SlotLength);
    type_point(&mut harness, "20, 0");
    assert!(harness.shows("Click to set the slot's width"));
    refused(&mut harness, "@0, 0", Refusal::SlotWidth);
    type_point(&mut harness, "@0, 5");
    harness.settle();

    let sketch = harness.sketch(feature);
    let shown = harness.shown(feature);
    let centers: Vec<Point2> = entities_of_kind(&shown, "Arc")
        .into_iter()
        .map(|arc| shown.arc(arc).unwrap().center)
        .collect();
    assert!(close(&radii(&shown, "Arc"), &[5.0, 5.0]));
    assert!(
        centers
            .iter()
            .any(|center| near(*center, Point2::new(-20.0, 0.0)))
    );
    assert!(
        centers
            .iter()
            .any(|center| near(*center, Point2::new(20.0, 0.0)))
    );
    for length in line_lengths(&shown) {
        assert!((length - 40.0).abs() < DRAWN, "{length}");
    }
    assert_eq!(constraints_of_kind(sketch, "Symmetric").len(), 1);
    assert_eq!(constraints_of_kind(sketch, "Tangent").len(), 4);
    assert_eq!(constraints_of_kind(sketch, "Equal").len(), 1);
    assert!(harness.shows("3 degrees of freedom left"));
    assert_eq!(harness.model.undo_label(), Some("Draw slot"));
}

#[test]
fn an_arc_slot_follows_its_arc_with_round_ends_tangent_to_both_sides() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    for _ in 0..3 {
        harness.use_tool(Key::U);
    }

    type_point(&mut harness, "0, 0");
    refused(&mut harness, "0, 0", Refusal::ArcSlotRadius);
    type_point(&mut harness, "20, 0");
    assert!(harness.shows("Click the centre of the slot's other end"));
    refused(&mut harness, "20, 0", Refusal::ArcSlotSweep);
    type_point(&mut harness, "0, 20");
    assert!(harness.shows("Click to set the slot's width"));
    refused(&mut harness, "@0, 0", Refusal::ArcSlotWidth);
    refused(&mut harness, "@0, 21", Refusal::ArcSlotWidth);
    type_point(&mut harness, "@0, 3");
    harness.settle();

    let sketch = harness.sketch(feature);
    let shown = harness.shown(feature);
    let outer = entities_of_kind(&shown, "Arc")
        .into_iter()
        .filter_map(|arc| shown.arc(arc))
        .find(|arc| (arc.radius - 23.0).abs() < DRAWN)
        .expect("the slot has an outer arc");
    assert!(close(&radii(&shown, "Arc"), &[3.0, 3.0, 17.0, 23.0]));
    assert!((outer.sweep - FRAC_PI_2).abs() < DRAWN, "{}", outer.sweep);
    assert!(outer.start_angle.abs() < DRAWN, "{}", outer.start_angle);
    assert_eq!(constraints_of_kind(sketch, "Tangent").len(), 4);
    assert_eq!(constraints_of_kind(sketch, "Coincident").len(), 5);
    assert!(harness.shows("4 degrees of freedom left"));
    assert_eq!(harness.model.undo_label(), Some("Draw arc slot"));

    type_point(&mut harness, "100, 0");
    type_point(&mut harness, "120, 0");
    harness.key(Key::X, Modifiers::NONE);
    harness.frame();
    type_point(&mut harness, "100, 20");
    type_point(&mut harness, "@0, 3");
    harness.settle();
    let shown = harness.shown(feature);
    let long_way = entities_of_kind(&shown, "Arc")
        .into_iter()
        .filter_map(|arc| shown.arc(arc))
        .find(|arc| (arc.radius - 23.0).abs() < DRAWN && arc.center.x > 50.0)
        .expect("the reversed slot has an outer arc");
    assert!(
        (long_way.sweep - 3.0 * FRAC_PI_2).abs() < DRAWN,
        "{}",
        long_way.sweep
    );

    harness.click("Extrude");
    harness.settle();
    let extrude = harness
        .document()
        .features()
        .find(|feature| feature.name == "Extrude 1")
        .map(Feature::id)
        .expect("the arc slots were extruded");
    let slot_area = |sweep: f64| sweep * (23.0 * 23.0 - 17.0 * 17.0) / 2.0 + PI * 9.0;
    let area = slot_area(FRAC_PI_2) + slot_area(3.0 * FRAC_PI_2);
    let volume = harness.body_volume(extrude);
    assert!((volume / (10.0 * area) - 1.0).abs() < 1e-2, "{volume}");
}

#[test]
fn a_typed_point_can_be_a_length_and_angle_or_a_length_toward_the_pointer() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    type_point(&mut harness, "12");
    let alone_first = harness.shows(
        "Type x, y such as 10, 20, or a length and an angle such as 25 < 30; a length alone \
         needs a placed point to measure from",
    );
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    type_point(&mut harness, "0, 0");
    type_point(&mut harness, "@10 < 90");
    type_point(&mut harness, "20 < 0");
    harness.point_at(Point2::new(20.0, 30.0));
    type_point(&mut harness, "5");
    type_point(&mut harness, "@4 < (pi / 2) rad");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    let sketch = harness.sketch(feature);
    let ends: Vec<Point2> = entities_of_kind(sketch, "Line")
        .into_iter()
        .map(|line| sketch.line_endpoints(line).unwrap().1)
        .collect();
    let expected = [
        Point2::new(0.0, 10.0),
        Point2::new(20.0, 0.0),
        Point2::new(20.0, 5.0),
        Point2::new(20.0, 9.0),
    ];
    assert!(alone_first);
    assert_eq!(ends.len(), expected.len());
    for (end, expected) in ends.iter().zip(expected) {
        assert!(near(*end, expected), "{end} is not {expected}");
    }
}

#[test]
fn a_spline_takes_clicked_control_points_until_enter() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::S);
    assert!(harness.shows("Click the spline's first control point"));
    let clicked = [
        Point2::new(10.0, 10.0),
        Point2::new(20.0, 30.0),
        Point2::new(30.0, 10.0),
        Point2::new(40.0, 30.0),
    ];
    for point in clicked {
        harness.click_at(point);
    }
    harness.click_at(Point2::new(45.0, -30.0));
    harness.key(Key::Backspace, Modifiers::NONE);
    harness.frame();
    assert!(harness.shows("Click the next control point"));
    assert_eq!(harness.sketch(feature).entities().len(), 0);
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();

    let sketch = harness.sketch(feature);
    let spline = entities_of_kind(sketch, "Spline")[0];
    let Some(Entity::Spline {
        points: control_points,
        ..
    }) = sketch.entity(spline)
    else {
        panic!("expected a spline");
    };
    assert_eq!(control_points.len(), 4);
    for (id, expected) in control_points.iter().zip(clicked) {
        assert!(near(sketch.point(*id).unwrap(), expected));
    }
    assert_eq!(harness.model.undo_label(), Some("Draw spline"));
    assert!(harness.shows("Click the spline's first control point"));
}

#[test]
fn a_fit_point_spline_closes_on_its_first_point_and_passes_every_point() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::S);
    harness.use_tool(Key::S);
    assert!(harness.shows("Click the first point the spline passes"));
    let clicked = [
        Point2::new(10.0, 10.0),
        Point2::new(40.0, 12.0),
        Point2::new(35.0, 40.0),
        Point2::new(12.0, 35.0),
    ];
    for point in clicked {
        harness.click_at(point);
    }
    harness.click_at(clicked[0]);

    let sketch = harness.sketch(feature);
    let spline = entities_of_kind(sketch, "Closed fit-point spline")[0];
    let Some(Entity::Spline { points, kind }) = sketch.entity(spline) else {
        panic!("expected a spline");
    };
    assert_eq!(*kind, SplineKind::fit(true));
    assert_eq!(points.len(), 4);
    for expected in clicked {
        let at = sketch.closest_on_curve(spline, expected).unwrap();
        assert!(near(at, expected), "{at} is not {expected}");
    }
}

#[test]
fn respacing_an_evenly_spaced_fit_spline_keeps_its_points_and_stops_the_overshoot() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let through = [
        Point2::new(0.0, 0.0),
        Point2::new(1.0, 0.0),
        Point2::new(2.0, 0.0),
        Point2::new(30.0, 0.0),
    ];
    let spline = sketch.add_spline_of(
        &through,
        SplineKind::Fit {
            closed: false,
            spacing: FitSpacing::Even,
        },
    );
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);

    harness.select([Pickable::SketchEntity {
        feature,
        entity: spline,
    }]);
    run_from_palette(&mut harness, "Respace the selected fit-point splines");
    harness.settle();

    assert_eq!(
        harness.model.undo_label(),
        Some("Respace fit-point splines")
    );
    let sketch = harness.sketch(feature);
    assert_eq!(
        sketch.entity(spline).and_then(Entity::spline_kind),
        Some(SplineKind::fit(false))
    );
    let backtracking = |curve: &caditor_sketch::BSpline| {
        (0..1000)
            .map(|step| {
                let at = |index: i32| curve.point_at(f64::from(index) / 1000.0).x;
                (at(step) - at(step + 1)).max(0.0)
            })
            .sum::<f64>()
    };
    assert!(backtracking(&sketch.spline(spline).unwrap()) < 1e-9);
    assert!(backtracking(&caditor_sketch::BSpline::interpolate(&through).unwrap()) > 0.1);
}

#[test]
fn escape_cancels_the_shape_in_progress_then_returns_to_select() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    let before = harness.document().clone();
    harness.use_tool(Key::R);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.point_at(Point2::new(30.0, 30.0));
    assert!(harness.shows("Click the opposite corner"));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert_eq!(*harness.document(), before);
    assert_eq!(harness.tool(), Some(Tool::Rectangle));
    assert!(harness.shows("Click the rectangle's first corner"));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::Select));
    assert_eq!(harness.editing(), Some(feature));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert_eq!(harness.editing(), None);
    assert_eq!(*harness.document(), before);
}

#[test]
fn tool_buttons_name_their_shortcuts() {
    let mut harness = Harness::new();
    harness.draw_on_new_sketch();
    harness.hover("Line");
    assert!(harness.shows("Draw connected lines, one click per corner (L)"));
    harness.click("Circle");
    assert_eq!(harness.tool(), Some(Tool::Circle));
    harness.hover_button("Parallel");
    assert!(harness.shows("Make lines parallel. Select two or more lines (Shift+P)"));
}

fn keep_typed_values_free(harness: &mut Harness) {
    run_from_palette(harness, "Keep typed values as dimensions");
    harness.frame();
    assert!(!harness.workspace.viewport.typed_dimensions());
}
