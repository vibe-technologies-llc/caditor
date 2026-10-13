use caditor_geometry::{Plane, Point2, Vector2};
use caditor_sketch::{Constraint, Entity, EntityId, PointTarget, Sketch};
use egui::{Event, Key, Modifiers, PointerButton};

use super::{
    Harness, constraints_of_kind, drag_in_sketch, edit_free_sketch, entities_of_kind,
    laptop_harness, line_ends, near, run_from_palette, scroll_in_view, viewport_centre,
};
use crate::{editing::Tool, model::Action, selection::Pickable};

#[test]
fn snapping_to_the_grid_places_free_points_on_its_crossings() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    assert!(harness.shows("Grid 10 mm"));
    run_from_palette(&mut harness, "snap to the grid");
    harness.use_tool(Key::L);

    harness.click_at(Point2::new(10.3, 10.2));
    harness.click_at(Point2::new(30.4, 20.3));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();

    let sketch = harness.sketch(feature);
    let [line] = entities_of_kind(sketch, "Line")[..] else {
        panic!("one line should be drawn");
    };
    let (start, end) = sketch.line_endpoints(line).unwrap();
    assert!(near(start, Point2::new(10.0, 10.0)), "{start}");
    assert!(near(end, Point2::new(30.0, 20.0)), "{end}");
}

#[test]
fn grid_snapping_ignores_the_sub_grid_until_it_becomes_the_current_level() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    assert!(harness.shows("Grid 10 mm"));
    run_from_palette(&mut harness, "snap to the grid");
    harness.use_tool(Key::L);

    let between = Point2::new(14.7, 15.4);
    harness.point_at(between);
    assert!(!harness.shows("On the grid"));
    harness.click_at(between);
    harness.point_at(Point2::new(30.4, 20.3));
    assert!(harness.shows("On the grid"));
    harness.click_at(Point2::new(30.4, 20.3));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();

    let sketch = harness.sketch(feature);
    let [line] = entities_of_kind(sketch, "Line")[..] else {
        panic!("one line should be drawn");
    };
    let (start, end) = sketch.line_endpoints(line).unwrap();
    assert!(near(start, between), "{start}");
    assert!(near(end, Point2::new(30.0, 20.0)), "{end}");

    harness.use_tool(Key::L);
    harness
        .events
        .push(Event::PointerMoved(viewport_centre(&harness)));
    harness.frame();
    while !harness.shows("Grid 1 mm") {
        scroll_in_view(&mut harness, egui::vec2(0.0, 60.0));
    }
    harness.click_at(Point2::new(2.3, 1.2));
    harness.click_at(Point2::new(5.2, 3.3));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();

    let sketch = harness.sketch(feature);
    let lines = entities_of_kind(sketch, "Line");
    let [_, finer] = lines[..] else {
        panic!("two lines should be drawn");
    };
    let (start, end) = sketch.line_endpoints(finer).unwrap();
    assert!(near(start, Point2::new(2.0, 1.0)), "{start}");
    assert!(near(end, Point2::new(5.0, 3.0)), "{end}");
}

#[test]
fn holding_ctrl_places_a_point_where_the_pointer_is_instead_of_snapping_to_a_point() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let target = sketch.add_point(Point2::new(40.0, 20.0));
    let feature = edit_free_sketch(&mut harness, sketch);
    harness.use_tool(Key::L);

    harness.click_at(Point2::new(10.0, 10.0));
    harness
        .events
        .push(Event::ModifiersChanged(Modifiers::COMMAND));
    harness.click_at(Point2::new(40.3, 20.2));
    harness
        .events
        .push(Event::ModifiersChanged(Modifiers::NONE));
    harness.frame();

    let sketch = harness.sketch(feature);
    let [line] = entities_of_kind(sketch, "Line")[..] else {
        panic!("one line should have been drawn");
    };
    let (_, end) = line_ends(sketch, line);
    assert_ne!(end, target);
    let at = sketch.point(end).unwrap();
    assert!(at.distance(Point2::new(40.3, 20.2)) < 0.1, "{at}");
    assert!(constraints_of_kind(sketch, "Coincident").is_empty());
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    harness.click_at(Point2::new(10.0, 40.0));
    harness.click_at(Point2::new(40.3, 20.2));
    let snapped = harness.sketch(feature);
    assert_eq!(constraints_of_kind(snapped, "Coincident").len(), 1);
}

#[test]
fn holding_alt_snaps_to_grid_crossings_or_to_geometry_nearer_than_them() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    let mut transaction = harness.document().transaction("Add point");
    let target = transaction.add_sketch_entity(feature, Entity::Point(Point2::new(40.5, 20.5)));
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    assert!(harness.shows("Grid 10 mm"));
    harness.use_tool(Key::L);
    assert!(harness.shows_hint("Alt: snap to the grid or nearby geometry"));

    harness.hold(Modifiers::ALT);
    harness.point_at(Point2::new(10.3, 10.2));
    assert!(harness.shows("On the grid"));
    harness.click_at(Point2::new(10.3, 10.2));
    harness.point_at(Point2::new(30.4, 10.3));
    assert!(harness.shows("On the grid, horizontal"));
    harness.click_at(Point2::new(30.4, 10.3));
    harness.hold(Modifiers::NONE);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    harness.hold(Modifiers::ALT);
    harness.click_at(Point2::new(10.2, 30.3));
    harness.point_at(Point2::new(40.3, 20.3));
    assert!(harness.shows(&format!("On Point {target}")));
    harness.click_at(Point2::new(40.3, 20.3));
    harness.hold(Modifiers::NONE);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();

    let sketch = harness.sketch(feature);
    let [level, slanted] = entities_of_kind(sketch, "Line")[..] else {
        panic!("two lines should be drawn");
    };
    let (start, end) = sketch.line_endpoints(level).unwrap();
    assert!(near(start, Point2::new(10.0, 10.0)), "{start}");
    assert!(near(end, Point2::new(30.0, 10.0)), "{end}");
    assert_eq!(
        constraints_of_kind(sketch, "Horizontal"),
        vec![Constraint::Horizontal(level)]
    );
    let (from, to) = line_ends(sketch, slanted);
    assert!(near(sketch.point(from).unwrap(), Point2::new(10.0, 30.0)));
    assert_eq!(
        constraints_of_kind(sketch, "Coincident"),
        vec![Constraint::Coincident(to, target)]
    );
}

#[test]
fn a_point_grabbed_with_alt_held_lands_on_the_grid_without_orbiting_in_laptop_mode() {
    let mut harness = laptop_harness();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(10.0, 10.0), Point2::new(30.0, 10.0));
    let (_, end) = line_ends(&sketch, line);
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = harness.sketch(feature).clone();
    let view = harness.workspace.viewport.viewpoint();

    let grabbed = harness.hover_pickable(
        Plane::XY,
        Point2::new(30.0, 10.0),
        Pickable::SketchEntity {
            feature,
            entity: end,
        },
    );
    harness.events.push(Event::PointerButton {
        pos: grabbed,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    harness
        .events
        .push(Event::PointerMoved(grabbed + egui::vec2(16.0, 0.0)));
    harness.frame();
    harness.hold(Modifiers::ALT);
    let target = harness.on_screen(Point2::new(26.33, 14.27));
    for step in 1..=4 {
        let position = grabbed + (target - grabbed) * (step as f32 / 4.0);
        harness.events.push(Event::PointerMoved(position));
        harness.frame();
    }
    assert!(harness.shows("On the grid"));
    harness.events.push(Event::PointerButton {
        pos: target,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::ALT,
    });
    harness.frame();
    harness.hold(Modifiers::NONE);
    harness.wait_until("the drag is committed", |harness| {
        harness.sketch(feature).point(end) != before.point(end)
    });
    harness.settle();

    assert!(harness.shows("Grid 1 mm"));
    let dragged = harness.sketch(feature).point(end).unwrap();
    let crossing = dragged.round();
    assert!(
        dragged.distance(Point2::new(26.33, 14.27)) < 0.5,
        "{dragged}"
    );
    assert!(dragged.distance(crossing) < 1e-9, "{dragged}");
    let after = harness.workspace.viewport.viewpoint();
    assert!(after.forward().dot(view.forward()) > 0.999_999);
    assert!(after.target.distance(view.target) < 1e-9);
}

#[test]
fn a_press_that_slips_a_few_pixels_is_still_one_click_where_it_is_released() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);

    let from = harness.on_screen(Point2::new(10.0, 10.0));
    drag_in_sketch(&mut harness, from, Point2::new(10.5, 10.4));
    harness.frame();

    assert_eq!(entities_of_kind(harness.sketch(feature), "Line").len(), 0);
    assert!(harness.shows("Click to end the line, Enter or Escape to stop"));
}

#[test]
fn drawing_a_line_adds_its_points_and_the_line_in_one_undoable_step() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    assert_eq!(harness.tool(), Some(Tool::Line));
    assert!(harness.shows("Click the start of the line"));

    harness.click_at(Point2::new(10.0, 10.0));
    assert!(harness.shows("Click to end the line, Enter or Escape to stop"));
    harness.click_at(Point2::new(40.0, 25.0));

    let sketch = harness.sketch(feature);
    let lines = entities_of_kind(sketch, "Line");
    assert_eq!(lines.len(), 1);
    assert_eq!(entities_of_kind(sketch, "Point").len(), 2);
    assert_eq!(sketch.constraints().len(), 0);
    let (start, end) = sketch.line_endpoints(lines[0]).unwrap();
    assert!(near(start, Point2::new(10.0, 10.0)), "{start}");
    assert!(near(end, Point2::new(40.0, 25.0)), "{end}");
    assert_eq!(harness.model.undo_label(), Some("Draw line"));
    assert!(harness.workspace.viewport.selection().is_empty());

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::Line));
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert_eq!(harness.sketch(feature).entities().len(), 0);
    assert_eq!(harness.model.undo_label(), Some("Create Sketch 1"));
}

#[test]
fn a_nearly_level_line_is_made_horizontal() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.point_at(Point2::new(40.0, 11.0));
    assert!(harness.shows("Horizontal"));
    harness.click_at(Point2::new(40.0, 11.0));

    let line = entities_of_kind(harness.sketch(feature), "Line")[0];
    assert_eq!(
        constraints_of_kind(harness.sketch(feature), "Horizontal"),
        vec![Constraint::Horizontal(line)]
    );
    harness.settle();
    let (start, end) = harness.shown(feature).line_endpoints(line).unwrap();
    assert!((start.y - end.y).abs() < 1e-12, "{start} {end}");
    assert!(near(end, Point2::new(40.0, 10.0)), "{end}");
    assert!(harness.shows("3 degrees of freedom left"));
}

fn unit_cross(sketch: &Sketch, a: EntityId, b: EntityId) -> (f64, f64) {
    let (a_start, a_end) = sketch.line_endpoints(a).unwrap();
    let (b_start, b_end) = sketch.line_endpoints(b).unwrap();
    let a = (a_end - a_start).normalize();
    let b = (b_end - b_start).normalize();
    (a.perp_dot(b), a.dot(b))
}

#[test]
fn a_line_drawn_nearly_parallel_to_a_slanted_line_is_kept_parallel_in_one_undoable_step() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(5.0, 40.0));
    harness.click_at(Point2::new(35.0, 60.0));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let slanted = entities_of_kind(harness.sketch(feature), "Line")[0];

    harness.click_at(Point2::new(10.0, 10.0));
    harness.point_at(Point2::new(40.0, 30.4));
    assert!(harness.shows(&format!("Parallel to Line {slanted}")));
    harness.click_at(Point2::new(40.0, 30.4));

    let sketch = harness.sketch(feature);
    let lines = entities_of_kind(sketch, "Line");
    assert_eq!(lines.len(), 2);
    let drawn = lines[1];
    assert_eq!(
        constraints_of_kind(sketch, "Parallel"),
        vec![Constraint::Parallel(drawn, slanted)]
    );
    assert_eq!(harness.model.undo_label(), Some("Draw line"));
    harness.settle();
    let (cross, _) = unit_cross(&harness.shown(feature), slanted, drawn);
    assert!(cross.abs() < 1e-9, "{cross}");
    assert!(harness.shows("7 degrees of freedom left"));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    let sketch = harness.sketch(feature);
    assert_eq!(entities_of_kind(sketch, "Line"), vec![slanted]);
    assert!(constraints_of_kind(sketch, "Parallel").is_empty());
    assert_eq!(entities_of_kind(sketch, "Point").len(), 2);
}

#[test]
fn a_chained_line_turned_nearly_square_is_kept_perpendicular_to_the_last() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(40.0, 30.0));
    let first = entities_of_kind(harness.sketch(feature), "Line")[0];

    harness.point_at(Point2::new(20.0, 60.4));
    assert!(harness.shows(&format!("Perpendicular to Line {first}")));
    harness.click_at(Point2::new(20.0, 60.4));

    let sketch = harness.sketch(feature);
    let lines = entities_of_kind(sketch, "Line");
    assert_eq!(lines.len(), 2);
    let second = lines[1];
    let (_, first_end) = line_ends(sketch, first);
    let (second_start, _) = line_ends(sketch, second);
    assert_eq!(
        constraints_of_kind(sketch, "Coincident"),
        vec![Constraint::Coincident(second_start, first_end)]
    );
    assert_eq!(
        constraints_of_kind(sketch, "Perpendicular"),
        vec![Constraint::Perpendicular(second, first)]
    );
    assert!(constraints_of_kind(sketch, "Parallel").is_empty());
    assert_eq!(harness.model.undo_label(), Some("Draw line"));
    harness.settle();
    let (_, dot) = unit_cross(&harness.shown(feature), first, second);
    assert!(dot.abs() < 1e-9, "{dot}");

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    let sketch = harness.sketch(feature);
    assert_eq!(entities_of_kind(sketch, "Line"), vec![first]);
    assert!(constraints_of_kind(sketch, "Perpendicular").is_empty());
}

#[test]
fn a_line_started_on_the_origin_is_joined_to_it() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.point_at(Point2::new(0.4, 0.3));
    assert!(harness.shows("Origin"));
    harness.click_at(Point2::new(0.4, 0.3));
    harness.click_at(Point2::new(20.0, 30.0));

    let sketch = harness.sketch(feature);
    let line = entities_of_kind(sketch, "Line")[0];
    let (start, _) = line_ends(sketch, line);
    assert_eq!(sketch.point(start), Some(Point2::ZERO));
    assert_eq!(
        constraints_of_kind(sketch, "Coincident"),
        vec![Constraint::Coincident(start, EntityId::ORIGIN)]
    );
}

#[test]
fn a_point_placed_where_two_lines_cross_is_held_on_both() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(70.0, 50.0));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.click_at(Point2::new(10.0, 40.0));
    harness.click_at(Point2::new(50.0, 20.0));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    let lines = entities_of_kind(harness.sketch(feature), "Line");
    let [first, second] = lines[..] else {
        panic!("two lines are drawn");
    };

    let along = 30.0 / (2.0 / 3.0 + 0.5);
    let crossing = Point2::new(10.0 + along, 40.0 - 0.5 * along);
    harness.use_tool(Key::P);
    harness.point_at(Point2::new(crossing.x + 0.3, crossing.y + 0.4));
    assert!(harness.shows(&format!("Crossing of Line {first} and Line {second}")));
    harness.click_at(Point2::new(crossing.x + 0.3, crossing.y + 0.4));
    harness.settle();

    let sketch = harness.sketch(feature);
    let point = *entities_of_kind(sketch, "Point").last().unwrap();
    let landed = sketch.point(point).unwrap();
    assert!(near(landed, crossing), "{landed} is not at {crossing}");
    let held: Vec<EntityId> = constraints_of_kind(sketch, "Coincident")
        .into_iter()
        .filter_map(|constraint| match constraint {
            Constraint::Coincident(a, b) if a == point => Some(b),
            _ => None,
        })
        .collect();
    assert_eq!(held, vec![first, second]);
}

#[test]
fn snapping_can_be_turned_off_for_good_and_back_on() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(50.0, 10.0));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.settle();
    assert!(harness.workspace.viewport.snapping());

    run_from_palette(&mut harness, "snapping");
    harness.frame();
    assert!(!harness.workspace.viewport.snapping());
    harness.use_tool(Key::L);
    harness.point_at(Point2::new(50.3, 10.4));
    assert!(!harness.shows("On Point"));
    harness.click_at(Point2::new(50.3, 10.4));
    harness.click_at(Point2::new(50.3, 40.0));
    harness.settle();

    let sketch = harness.sketch(feature);
    let lines = entities_of_kind(sketch, "Line");
    let (start, _) = line_ends(sketch, lines[1]);
    assert!(near(sketch.point(start).unwrap(), Point2::new(50.3, 10.4)));
    assert!(constraints_of_kind(sketch, "Coincident").is_empty());

    run_from_palette(&mut harness, "snapping");
    harness.frame();
    assert!(harness.workspace.viewport.snapping());
}

#[test]
fn a_point_placed_at_the_centre_of_a_rectangle_is_held_there_by_symmetry() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::R);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(50.0, 30.0));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.settle();

    harness.use_tool(Key::P);
    harness.point_at(Point2::new(30.4, 20.3));
    assert!(harness.shows_containing("Centre of the outline of Line"));
    harness.click_at(Point2::new(30.4, 20.3));
    harness.settle();

    let sketch = harness.sketch(feature);
    let symmetric = constraints_of_kind(sketch, "Symmetric");
    let [Constraint::Symmetric { about, .. }] = symmetric[..] else {
        panic!("one symmetry about the centre is expected");
    };
    assert!(near(sketch.point(about).unwrap(), Point2::new(30.0, 20.0)));
}

#[test]
fn a_point_placed_at_the_middle_of_an_arc_is_held_at_its_midpoint() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::A);
    let center = Point2::new(20.0, 20.0);
    harness.click_at(center);
    harness.click_at(center + Vector2::new(10.0, 0.0));
    harness.point_at(center + Vector2::new(7.0, 7.0));
    harness.click_at(center + Vector2::new(0.0, 10.0));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.settle();
    let arc = entities_of_kind(harness.sketch(feature), "Arc")[0];
    let middle = center + Vector2::from_angle(std::f64::consts::FRAC_PI_4) * 10.0;

    harness.use_tool(Key::P);
    harness.point_at(middle + Vector2::new(0.3, -0.2));
    assert!(harness.shows(&format!("Midpoint of Arc {arc}")));
    harness.click_at(middle + Vector2::new(0.3, -0.2));
    harness.settle();

    let sketch = harness.sketch(feature);
    let midpoints = constraints_of_kind(sketch, "Midpoint");
    let [Constraint::Midpoint { point, curve }] = midpoints[..] else {
        panic!("one midpoint constraint is expected");
    };
    assert_eq!(curve, arc);
    assert!(near(sketch.point(point).unwrap(), middle));
}

#[test]
fn a_line_started_at_the_middle_of_another_is_held_at_its_midpoint() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(50.0, 10.0));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.settle();

    harness.use_tool(Key::L);
    harness.point_at(Point2::new(30.4, 10.5));
    let base = entities_of_kind(harness.sketch(feature), "Line")[0];
    assert!(harness.shows(&format!("Midpoint of Line {base}")));
    harness.click_at(Point2::new(30.4, 10.5));
    harness.click_at(Point2::new(30.0, 40.0));
    harness.settle();

    let sketch = harness.sketch(feature);
    let midpoints = constraints_of_kind(sketch, "Midpoint");
    let [Constraint::Midpoint { point, curve }] = midpoints[..] else {
        panic!("one midpoint constraint is expected");
    };
    assert_eq!(curve, base);
    assert!(near(sketch.point(point).unwrap(), Point2::new(30.0, 10.0)));
}

#[test]
fn a_point_dropped_on_a_line_stays_on_it() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(40.0, 40.0));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.settle();
    harness.use_tool(Key::P);
    assert!(harness.shows("Click to place a point"));
    harness.point_at(Point2::new(30.5, 30.0));
    let line = entities_of_kind(harness.sketch(feature), "Line")[0];
    assert!(harness.shows(&format!("On Line {line}")));
    harness.click_at(Point2::new(30.5, 30.0));

    let sketch = harness.sketch(feature);
    let point = *entities_of_kind(sketch, "Point").last().unwrap();
    let position = sketch.point(point).unwrap();
    let (start, end) = sketch.line_endpoints(line).unwrap();
    assert!(
        (end - start).perp_dot(position - start).abs() < 1e-9,
        "{position}"
    );
    assert_eq!(
        constraints_of_kind(sketch, "Coincident"),
        vec![Constraint::Coincident(point, line)]
    );
    assert_eq!(harness.model.undo_label(), Some("Draw point"));
}

#[test]
fn a_point_placed_at_the_centre_of_a_drawn_triangle_is_held_at_the_centre_of_its_corners() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    for corner in [
        Point2::new(10.0, 10.0),
        Point2::new(40.0, 10.0),
        Point2::new(10.0, 40.0),
        Point2::new(10.0, 10.0),
    ] {
        harness.click_at(corner);
    }
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.settle();
    assert_eq!(entities_of_kind(harness.sketch(feature), "Line").len(), 3);

    harness.use_tool(Key::P);
    harness.point_at(Point2::new(20.3, 19.8));
    assert!(harness.shows_containing("Centre of the outline of Line"));
    assert!(!harness.shows_containing("not kept there"));
    harness.click_at(Point2::new(20.3, 19.8));
    harness.settle();

    let sketch = harness.sketch(feature);
    let lines = entities_of_kind(sketch, "Line");
    let owned: Vec<EntityId> = lines
        .iter()
        .flat_map(|line| sketch.entity(*line).unwrap().points())
        .collect();
    let [placed] = entities_of_kind(sketch, "Point")
        .into_iter()
        .filter(|point| !owned.contains(point))
        .collect::<Vec<_>>()[..]
    else {
        panic!("one lone point should be placed");
    };
    let landed = sketch.point(placed).unwrap();
    assert!(
        near(landed, Point2::new(20.0, 20.0)),
        "{landed} is not at the centre"
    );
    let held: Vec<&Constraint> = sketch
        .constraints()
        .map(|(_, constraint)| constraint)
        .filter(|constraint| constraint.entities().contains(&placed))
        .collect();
    assert!(
        matches!(
            held[..],
            [Constraint::Centroid { point, corners }]
                if *point == placed
                    && corners.len() == 3
                    && corners.iter().all(|corner| owned.contains(corner))
        ),
        "{held:?}"
    );
}

#[test]
fn a_point_drawn_square_to_a_line_from_its_end_lands_on_the_perpendicular_and_is_held_there() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(15.0, 20.0));
    let (_, end) = line_ends(&sketch, line);
    sketch.add_point(Point2::new(30.0, 0.0));
    let feature = edit_free_sketch(&mut harness, sketch);

    harness.use_tool(Key::P);
    harness.point_at(Point2::new(15.0, 20.0));
    harness.point_at(Point2::new(23.05, 14.03));
    let label = harness.sketch(feature).entity_label(line);
    let end_label = harness.sketch(feature).entity_label(end);
    assert!(harness.shows(&format!("Perpendicular to {label} from {end_label}")));
    harness.click_at(Point2::new(23.05, 14.03));
    harness.settle();

    let sketch = harness.sketch(feature);
    let placed = entities_of_kind(sketch, "Point")
        .into_iter()
        .rfind(|point| !sketch.entity(line).unwrap().points().contains(point))
        .expect("a point is placed");
    let at = sketch.point(placed).unwrap();
    assert!(
        (at - Point2::new(15.0, 20.0))
            .dot(Vector2::new(3.0, 4.0))
            .abs()
            < 1e-6,
        "{at}"
    );
    assert!(sketch.constraints().any(|(_, constraint)| *constraint
        == Constraint::OnDirection {
            point: placed,
            through: PointTarget::Point(end),
            line,
            square: true,
        }));
}
