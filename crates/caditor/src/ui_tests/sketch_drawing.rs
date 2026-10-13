use caditor_geometry::{Plane, Point2, Vector2};
use caditor_sketch::{Constraint, Entity, Sketch};
use egui::{Event, Key, Modifiers};

use super::{
    DRAWN, Harness, aligned_point_pairs, constraints_of_kind, drag_in_sketch, edit_free_sketch,
    entities_of_kind, line_ends, near, run_from_palette,
};

#[test]
fn a_press_dragged_or_held_with_a_drawing_tool_still_places_the_point_where_it_is_released() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);

    harness.click_at(Point2::new(10.0, 10.0));
    let from = harness.on_screen(Point2::new(20.0, 20.0));
    drag_in_sketch(&mut harness, from, Point2::new(40.0, 25.0));
    harness.frame();

    let sketch = harness.sketch(feature);
    let lines = entities_of_kind(sketch, "Line");
    assert_eq!(lines.len(), 1);
    let (start, end) = sketch.line_endpoints(lines[0]).unwrap();
    assert!(near(start, Point2::new(10.0, 10.0)), "{start}");
    assert!(near(end, Point2::new(40.0, 25.0)), "{end}");
}

#[test]
fn a_point_hovered_while_drawing_guides_later_points_into_line_with_it() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::P);
    harness.click_at(Point2::new(40.0, 30.0));
    let [guide] = entities_of_kind(harness.sketch(feature), "Point")[..] else {
        panic!("one point should be drawn");
    };
    harness.use_tool(Key::R);

    harness.point_at(Point2::new(40.0, 30.0));
    harness.click_at(Point2::new(10.0, 5.0));
    harness.point_at(Point2::new(25.0, 30.02));
    assert!(harness.shows(&format!("Horizontal from Point {guide}")));
    harness.click_at(Point2::new(25.0, 30.02));

    let sketch = harness.sketch(feature);
    let guided = sketch.point(guide).unwrap();
    let [Constraint::HorizontalPoints(corner, ref reference)] = aligned_point_pairs(sketch)[..]
    else {
        panic!("the far corner should line up beside the hovered point");
    };
    assert_eq!(*reference, guide.into());
    assert_eq!(sketch.point(corner).map(|at| at.y), Some(guided.y));

    harness.use_tool(Key::L);
    harness.click_at(Point2::new(70.0, 10.0));
    harness.point_at(Point2::new(40.0, 30.0));
    harness.point_at(Point2::new(40.02, 10.02));
    assert!(harness.shows(&format!("Horizontal, vertical from Point {guide}")));
    harness.click_at(Point2::new(40.02, 10.02));
    harness.key(Key::Escape, Modifiers::NONE);

    let sketch = harness.sketch(feature);
    let pairs = aligned_point_pairs(sketch);
    let [_, Constraint::VerticalPoints(end, ref reference)] = pairs[..] else {
        panic!("the line's end should line up under the hovered point: {pairs:?}");
    };
    let [.., line] = entities_of_kind(sketch, "Line")[..] else {
        panic!("a line should be drawn");
    };
    let (start, _) = line_ends(sketch, line);
    assert_eq!(*reference, guide.into());
    assert_eq!(
        sketch.point(end),
        Some(Point2::new(guided.x, sketch.point(start).unwrap().y))
    );
}

#[test]
fn a_line_drawn_to_the_side_of_one_circle_and_touching_another_keeps_both() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::C);
    harness.click_at(Point2::new(20.0, 20.0));
    harness.click_at(Point2::new(30.0, 20.0));
    let sketch = harness.sketch(feature);
    let [circle] = entities_of_kind(sketch, "Circle")[..] else {
        panic!("one circle should be drawn");
    };
    let (centre, radius) = sketch.circle(circle).unwrap();
    let top = centre + Vector2::new(0.0, radius);
    let from = Point2::new(70.0, top.y);
    let away = from - centre;
    let touch = centre + away * (radius * radius / away.length_squared())
        - away.perp()
            * (radius * (away.length_squared() - radius * radius).sqrt() / away.length_squared());
    harness.use_tool(Key::L);

    harness.point_at(top + Vector2::new(0.02, 0.02));
    assert!(harness.shows(&format!("Top of Circle {circle}")));
    harness.click_at(from);
    harness.point_at(touch + Vector2::new(0.02, 0.02));
    assert!(harness.shows(&format!("Tangent to Circle {circle}")));
    harness.click_at(touch + Vector2::new(0.02, 0.02));
    harness.key(Key::Escape, Modifiers::NONE);

    let sketch = harness.sketch(feature);
    let [line] = entities_of_kind(sketch, "Line")[..] else {
        panic!("one line should be drawn");
    };
    let (start, end) = line_ends(sketch, line);
    let (start_at, end_at) = sketch.line_endpoints(line).unwrap();
    assert!(constraints_of_kind(sketch, "Tangent").contains(&Constraint::Tangent(line, circle)));
    assert!(
        sketch
            .constraints()
            .any(|(_, constraint)| *constraint == Constraint::Coincident(end, circle))
    );
    assert!(((end_at - centre).length() - radius).abs() < 1e-9);
    assert!((end_at - centre).dot(end_at - start_at).abs() < 1e-6);
    assert_ne!(start, end);
}

#[test]
fn an_arc_starting_level_with_its_centre_and_a_spline_point_above_the_last_stay_so() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::A);
    harness.click_at(Point2::new(20.0, 20.0));
    harness.point_at(Point2::new(30.0, 20.02));
    assert!(harness.shows("Horizontal"));
    harness.click_at(Point2::new(30.0, 20.02));
    harness.point_at(Point2::new(20.0, 30.0));
    harness.click_at(Point2::new(20.0, 30.0));

    let sketch = harness.sketch(feature);
    let [arc] = entities_of_kind(sketch, "Arc")[..] else {
        panic!("one arc should be drawn");
    };
    let Some(Entity::Arc { center, start, .. }) = sketch.entity(arc).cloned() else {
        panic!("the arc is an arc");
    };
    let pairs = aligned_point_pairs(sketch);
    assert!(
        pairs.contains(&Constraint::HorizontalPoints(center, start.into())),
        "{pairs:?}"
    );

    harness.use_tool(Key::S);
    harness.click_at(Point2::new(50.0, 5.0));
    harness.click_at(Point2::new(50.02, 25.0));
    harness.click_at(Point2::new(70.0, 30.0));
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();

    let sketch = harness.sketch(feature);
    let [spline] = entities_of_kind(sketch, "Spline")[..] else {
        panic!("one spline should be drawn");
    };
    let Some(Entity::Spline {
        points: control_points,
        ..
    }) = sketch.entity(spline).cloned()
    else {
        panic!("the spline is a spline");
    };
    assert!(
        aligned_point_pairs(sketch).contains(&Constraint::VerticalPoints(
            control_points[0],
            control_points[1].into()
        ))
    );
}

#[test]
fn one_press_drag_release_draws_a_whole_line_rectangle_or_circle() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);

    let from = harness.on_screen(Point2::new(10.0, 10.0));
    drag_in_sketch(&mut harness, from, Point2::new(40.0, 25.0));
    harness.frame();

    let sketch = harness.sketch(feature);
    let [line] = entities_of_kind(sketch, "Line")[..] else {
        panic!("one drag should draw one line");
    };
    let (start, end) = sketch.line_endpoints(line).unwrap();
    assert!(near(start, Point2::new(10.0, 10.0)), "{start}");
    assert!(near(end, Point2::new(40.0, 25.0)), "{end}");
    assert!(harness.shows("Click to end the line, Enter or Escape to stop"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.key(Key::Escape, Modifiers::NONE);

    harness.use_tool(Key::R);
    let from = harness.on_screen(Point2::new(60.0, 10.0));
    drag_in_sketch(&mut harness, from, Point2::new(90.0, 30.0));
    harness.frame();
    assert_eq!(entities_of_kind(harness.sketch(feature), "Line").len(), 5);
}

#[test]
fn a_circle_is_drawn_by_dragging_from_its_centre_to_its_rim() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::C);
    let from = harness.on_screen(Point2::new(20.0, 35.0));
    drag_in_sketch(&mut harness, from, Point2::new(35.0, 35.0));
    harness.frame();
    let sketch = harness.sketch(feature);
    let [circle] = entities_of_kind(sketch, "Circle")[..] else {
        panic!("one drag should draw one circle");
    };
    let (centre, radius) = sketch.circle(circle).unwrap();
    assert!(near(centre, Point2::new(20.0, 35.0)), "{centre}");
    assert!((radius - 15.0).abs() < DRAWN, "{radius}");
}

#[test]
fn the_size_of_what_is_being_drawn_shows_beside_the_pointer() {
    let cases = [
        (
            Key::L,
            Point2::new(10.0, 10.0),
            Point2::new(40.0, 10.0),
            "30.00 mm   0.0°",
        ),
        (
            Key::R,
            Point2::new(10.0, 10.0),
            Point2::new(40.0, 30.0),
            "30.00 mm × 20.00 mm",
        ),
        (
            Key::C,
            Point2::new(20.0, 20.0),
            Point2::new(35.0, 20.0),
            "R 15.00 mm",
        ),
        (
            Key::G,
            Point2::new(20.0, 20.0),
            Point2::new(35.0, 20.0),
            "R 15.00 mm   6 sides",
        ),
        (
            Key::A,
            Point2::new(20.0, 20.0),
            Point2::new(35.0, 20.0),
            "R 15.00 mm",
        ),
        (
            Key::U,
            Point2::new(10.0, 10.0),
            Point2::new(40.0, 10.0),
            "30.00 mm",
        ),
    ];
    for (tool, start, to, expected) in cases {
        let mut harness = Harness::new();
        harness.draw_on_new_sketch();
        harness.use_tool(tool);
        harness.point_at(to);
        assert!(
            !harness.shows(expected),
            "{expected} before anything is placed"
        );

        harness.click_at(start);
        harness.point_at(to);

        assert!(harness.shows(expected), "{expected}");
    }
}

#[test]
fn arcs_and_slots_show_their_radius_sweep_and_width_while_the_second_point_is_chosen() {
    let cases = [
        (
            Key::A,
            Modifiers::ALT,
            [Point2::new(10.0, 20.0), Point2::new(20.0, 30.0)],
            Point2::new(30.0, 20.0),
            "R 10.00 mm",
        ),
        (
            Key::A,
            Modifiers::NONE,
            [Point2::new(20.0, 20.0), Point2::new(30.0, 20.0)],
            Point2::new(20.0, 30.0),
            "R 10.00 mm   90.0°",
        ),
        (
            Key::U,
            Modifiers::NONE,
            [Point2::new(10.0, 10.0), Point2::new(40.0, 10.0)],
            Point2::new(25.0, 14.0),
            "30.00 mm × Ø 8.00 mm",
        ),
    ];
    for (tool, modifiers, placed, to, expected) in cases {
        let mut harness = Harness::new();
        harness.draw_on_new_sketch();
        harness.use_tool_with(tool, modifiers);

        for point in placed {
            harness.click_at(point);
        }
        harness.point_at(to);
        harness.point_at(to);

        assert!(harness.shows(expected), "{expected}");
    }
}

#[test]
fn tangent_arcs_arc_slots_and_spline_legs_show_their_size_too() {
    let mut harness = Harness::new();
    harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(30.0, 10.0));
    harness.use_tool(Key::T);
    harness.point_at(Point2::new(42.0, 18.0));
    harness.point_at(Point2::new(42.0, 18.0));
    assert!(harness.shows("R 13.00 mm   67.4°"));

    let mut harness = Harness::new();
    harness.draw_on_new_sketch();
    for _ in 0..3 {
        harness.use_tool(Key::U);
    }
    harness.click_at(Point2::new(20.0, 20.0));
    harness.point_at(Point2::new(30.0, 20.0));
    assert!(harness.shows("R 10.00 mm"));
    harness.click_at(Point2::new(30.0, 20.0));
    harness.point_at(Point2::new(20.0, 30.0));
    harness.point_at(Point2::new(20.0, 30.0));
    assert!(harness.shows("R 10.00 mm   90.0°"));
    harness.click_at(Point2::new(20.0, 30.0));
    harness.point_at(Point2::new(20.0, 33.0));
    harness.point_at(Point2::new(20.0, 33.0));
    assert!(harness.shows("R 10.00 mm   90.0°   × 6.00 mm"));

    let mut harness = Harness::new();
    harness.draw_on_new_sketch();
    harness.use_tool(Key::S);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.point_at(Point2::new(20.0, 10.0));
    assert!(harness.shows("10.00 mm   0.0°"));
    harness.click_at(Point2::new(20.0, 10.0));
    harness.point_at(Point2::new(20.0, 30.0));
    assert!(harness.shows("20.00 mm   90.0°"));
}

#[test]
fn copied_sketch_geometry_pastes_under_the_pointer_and_cut_takes_it_away() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(10.0, 10.0), Point2::new(30.0, 10.0));
    sketch.add_constraint(Constraint::Horizontal(line)).unwrap();
    let feature = edit_free_sketch(&mut harness, sketch);
    harness.point_at(Point2::new(25.0, 14.0));

    harness.key(Key::V, Modifiers::COMMAND);
    harness.settle();
    let before_copy = entities_of_kind(harness.sketch(feature), "Line").len();
    harness.frame();
    assert!(harness.shows("Nothing was pasted: copy or cut sketch geometry first."));

    harness.key(Key::A, Modifiers::COMMAND);
    harness.frame();
    harness.events.push(Event::Copy);
    harness.settle();
    assert!(harness.shows("Copied 1 curve and 1 constraint."));
    let copied = harness.clipboard.clone().unwrap();
    assert!(copied.starts_with("caditor clipboard: sketch geometry, version 1\n"));
    harness.point_at(Point2::new(25.0, 14.0));
    harness.events.push(Event::Paste(copied.clone()));
    harness.settle();

    let sketch = harness.sketch(feature);
    let lines = entities_of_kind(sketch, "Line");
    assert_eq!(before_copy, 1);
    assert_eq!(lines.len(), 2);
    let pasted = *lines.iter().find(|candidate| **candidate != line).unwrap();
    let (start, end) = sketch.line_endpoints(pasted).unwrap();
    assert!(near(start, Point2::new(15.0, 14.0)), "{start}");
    assert!(near(end, Point2::new(35.0, 14.0)), "{end}");
    assert_eq!(sketch.constraints().len(), 2);
    assert_eq!(
        harness.model.undo_label(),
        Some("Paste 1 curve and 1 constraint")
    );

    harness.events.push(Event::Cut);
    harness.settle();
    let sketch = harness.sketch(feature);
    assert_eq!(entities_of_kind(sketch, "Line"), vec![line]);
    harness.point_at(Point2::new(20.0, 6.0));
    run_from_palette(&mut harness, "paste sketch geometry");
    harness.settle();
    assert_eq!(entities_of_kind(harness.sketch(feature), "Line").len(), 2);
}

#[test]
fn geometry_copied_in_another_caditor_pastes_and_foreign_text_is_refused_in_words() {
    let mut source = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(12.0, 0.0));
    sketch.add_constraint(Constraint::Horizontal(line)).unwrap();
    edit_free_sketch(&mut source, sketch);
    source.key(Key::A, Modifiers::COMMAND);
    source.frame();
    source.events.push(Event::Copy);
    source.settle();
    let copied =
        source
            .clipboard
            .clone()
            .unwrap()
            .replacen("\"source\":\"", "\"source\":\"elsewhere ", 1);

    let mut harness = Harness::new();
    let feature = edit_free_sketch(&mut harness, Sketch::new(Plane::XY));
    harness
        .events
        .push(Event::Paste("a shopping list".to_owned()));
    harness.settle();
    assert!(
        harness
            .shows("Nothing was pasted: the clipboard holds text that did not come from caditor.")
    );
    assert!(entities_of_kind(harness.sketch(feature), "Line").is_empty());

    harness.events.push(Event::Paste(copied));
    harness.settle();
    let sketch = harness.sketch(feature);
    assert_eq!(entities_of_kind(sketch, "Line").len(), 1);
    assert_eq!(sketch.constraints().len(), 1);
}

#[test]
fn the_sketch_bar_counts_curve_ends_joined_to_nothing() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(20.0, 0.0));
    let second = sketch.add_line(Point2::new(20.0, 0.0), Point2::new(20.0, 10.0));
    let Some(&Entity::Line { end, .. }) = sketch.entity(first) else {
        panic!("a line");
    };
    let Some(&Entity::Line { start, end: loose }) = sketch.entity(second) else {
        panic!("a line");
    };
    sketch
        .add_constraint(Constraint::Coincident(end, start))
        .unwrap();
    let feature = edit_free_sketch(&mut harness, sketch);
    harness.context.enable_accesskit();
    harness.settle();

    assert!(harness.shows("2 open ends"));
    let description = harness.view_description();
    assert!(
        description
            .as_deref()
            .is_some_and(|description| description.contains("; 2 open ends.")),
        "{description:?}"
    );

    harness.add_stored_constraint(feature, Constraint::Coincident(loose, first));
    harness.settle();
    assert!(harness.shows("1 open end"));
}
