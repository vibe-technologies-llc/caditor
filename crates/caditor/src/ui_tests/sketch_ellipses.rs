use std::f64::consts::FRAC_PI_2;

use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Constraint, Entity, Sketch};
use egui::Key;

use super::{
    DRAWN, Harness, constraints_of_kind, edit_free_sketch, entities_of_kind, entity_pickables,
    fillet_arc, run_from_palette, type_point,
};
use crate::{editing::Tool, filleting, sketch_toolbar, sketch_tools};

#[test]
fn an_ellipse_is_drawn_from_its_centre_the_end_of_its_major_axis_and_its_minor_radius() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();

    harness.click_button(sketch_toolbar::CURVE_WAYS_LABEL);
    harness.click(Tool::Ellipse.label());
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::Ellipse));
    assert!(harness.shows("Click the ellipse's centre"));
    harness.click_at(Point2::new(20.0, 20.0));
    assert!(harness.shows("Click the end of its major axis"));
    harness.click_at(Point2::new(40.0, 20.0));
    assert!(harness.shows("Click to set its minor radius"));
    harness.point_at(Point2::new(24.0, 26.0));
    assert!(harness.shows("R 20.00 mm   r 6.00 mm"));
    harness.click_at(Point2::new(26.0, 28.0));

    let sketch = harness.sketch(feature);
    let [ellipse] = entities_of_kind(sketch, "Ellipse")[..] else {
        panic!("expected one ellipse");
    };
    let shape = sketch.ellipse(ellipse).unwrap();
    assert!(shape.center.distance(Point2::new(20.0, 20.0)) < DRAWN);
    assert!((shape.major_radius() - 20.0).abs() < DRAWN);
    assert!((shape.minor_radius - 8.0).abs() < DRAWN);
    assert_eq!(constraints_of_kind(sketch, "Horizontal").len(), 1);
    assert_eq!(harness.model.undo_label(), Some("Draw ellipse"));
    harness.settle();
    assert!(harness.shows("4 degrees of freedom left"));
}

#[test]
fn an_elliptical_arc_is_chosen_from_the_curve_button_and_follows_the_sweep() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();

    harness.click_button(sketch_toolbar::CURVE_WAYS_LABEL);
    for tool in sketch_toolbar::CURVE_TOOLS {
        assert!(harness.shows(tool.label()), "{tool:?}");
    }
    harness.click("Elliptical arc");
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::EllipticalArc));
    let center = Point2::new(-20.0, 20.0);
    harness.click_at(center);
    harness.click_at(Point2::new(-10.0, 20.0));
    harness.click_at(Point2::new(-20.0, 26.0));
    assert!(harness.shows("Click where the arc ends"));
    for step in [Point2::new(-24.0, 25.5), Point2::new(-29.0, 22.0)] {
        harness.point_at(step);
    }
    harness.click_at(Point2::new(-30.0, 20.0));

    let sketch = harness.sketch(feature);
    let [arc] = entities_of_kind(sketch, "Elliptical arc")[..] else {
        panic!("expected one elliptical arc");
    };
    let shape = sketch.ellipse(arc).unwrap();
    assert!((shape.minor_radius - 6.0).abs() < DRAWN);
    assert!((shape.sweep - FRAC_PI_2).abs() < DRAWN, "{}", shape.sweep);
    let Some(Entity::EllipticalArc { start, .. }) = sketch.entity(arc) else {
        panic!("expected an elliptical arc");
    };
    assert!(
        sketch
            .point(*start)
            .unwrap()
            .distance(Point2::new(-20.0, 26.0))
            < DRAWN
    );
    assert_eq!(harness.model.undo_label(), Some("Draw elliptical arc"));
}

#[test]
fn the_radius_button_holds_both_radii_of_a_selected_ellipse() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = sketch.add_ellipse(Point2::new(10.0, 10.0), Point2::new(25.0, 10.0), 5.0);
    let feature = edit_free_sketch(&mut harness, sketch);

    harness.select(entity_pickables(feature, &[ellipse]));
    harness.click_button("Radius");
    harness.settle();

    let sketch = harness.sketch(feature);
    assert_eq!(constraints_of_kind(sketch, "Major radius").len(), 1);
    assert_eq!(constraints_of_kind(sketch, "Minor radius").len(), 1);
}

#[test]
fn trim_opens_an_ellipse_where_a_line_crosses_it() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = sketch.add_ellipse(Point2::new(30.0, 30.0), Point2::new(45.0, 30.0), 6.0);
    sketch.add_line(Point2::new(40.0, 20.0), Point2::new(40.0, 40.0));
    let label = sketch.entity_label(ellipse);
    let feature = edit_free_sketch(&mut harness, sketch);

    run_from_palette(&mut harness, "trim sketch curves");
    assert_eq!(harness.tool(), Some(Tool::Trim));
    harness.click_at(Point2::new(45.0, 30.0));
    harness.settle();

    let sketch = harness.sketch(feature);
    assert!(entities_of_kind(sketch, "Ellipse").is_empty());
    assert_eq!(entities_of_kind(sketch, "Elliptical arc"), vec![ellipse]);
    assert_eq!(
        harness.model.undo_label(),
        Some(format!("Trim {label}").as_str())
    );
}

#[test]
fn an_elliptical_arc_is_split_at_the_selected_point_on_it() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let arc = sketch.add_elliptical_arc(
        Point2::new(30.0, 30.0),
        Point2::new(45.0, 30.0),
        6.0,
        Point2::new(30.0 + 15.0 * 0.3_f64.cos(), 30.0 + 6.0 * 0.3_f64.sin()),
        Point2::new(30.0 + 15.0 * 2.8_f64.cos(), 30.0 + 6.0 * 2.8_f64.sin()),
    );
    let point = sketch.add_point(Point2::new(30.0, 36.0));
    sketch
        .add_constraint(Constraint::Coincident(point, arc))
        .unwrap();
    let feature = edit_free_sketch(&mut harness, sketch);

    harness.select(entity_pickables(feature, &[point]));
    run_from_palette(&mut harness, "split the selected curve");
    harness.settle();

    let sketch = harness.sketch(feature);
    assert_eq!(entities_of_kind(sketch, "Elliptical arc").len(), 2);
    assert_eq!(harness.model.undo_label(), Some(sketch_tools::SPLIT_TITLE));
}

#[test]
fn a_circle_apart_from_an_ellipse_is_made_tangent_to_it() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = sketch.add_ellipse(Point2::new(10.0, 10.0), Point2::new(25.0, 10.0), 5.0);
    let circle = sketch.add_circle(Point2::new(35.0, 14.0), 3.0);
    let feature = edit_free_sketch(&mut harness, sketch);

    harness.select(entity_pickables(feature, &[circle, ellipse]));
    harness.click_button("Tangent");
    harness.settle();

    assert_eq!(
        constraints_of_kind(harness.sketch(feature), "Tangent").len(),
        1
    );
    let shown = harness.shown(feature);
    let (center, radius) = shown.circle(circle).unwrap();
    let touching = shown.closest_on_ellipse(ellipse, center).unwrap();
    assert!((touching.distance(center) - radius).abs() < DRAWN);
}

#[test]
fn the_distance_button_dimensions_a_point_from_an_ellipse() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = sketch.add_ellipse(Point2::new(10.0, 10.0), Point2::new(25.0, 10.0), 5.0);
    let point = sketch.add_point(Point2::new(10.0, 19.0));
    let feature = edit_free_sketch(&mut harness, sketch);

    harness.select(entity_pickables(feature, &[point, ellipse]));
    harness.click_button("Distance");
    harness.settle();

    let sketch = harness.sketch(feature);
    let [distance] = &constraints_of_kind(sketch, "Distance")[..] else {
        panic!("expected one distance");
    };
    let measured = sketch.measured(distance).unwrap();
    assert!((measured - 4.0).abs() < DRAWN, "{measured}");
}

#[test]
fn extend_carries_an_elliptical_arc_round_its_ellipse_to_a_line() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let at = |angle: f64| Point2::new(30.0 + 15.0 * angle.cos(), 30.0 + 6.0 * angle.sin());
    let arc = sketch.add_elliptical_arc(
        Point2::new(30.0, 30.0),
        Point2::new(45.0, 30.0),
        6.0,
        at(0.3),
        at(2.0),
    );
    let wall = sketch.add_line(Point2::new(20.0, 20.0), Point2::new(20.0, 45.0));
    let [arc_label, wall_label] = [arc, wall].map(|id| sketch.entity_label(id));
    let feature = edit_free_sketch(&mut harness, sketch);

    harness.use_tool(Key::J);
    assert_eq!(harness.tool(), Some(Tool::Extend));
    harness.point_at(at(1.9));
    assert!(harness.shows(&format!("Extend {arc_label} to {wall_label}")));
    harness.click_at(at(1.9));

    let sketch = harness.sketch(feature);
    let Some(&Entity::EllipticalArc { end, .. }) = sketch.entity(arc) else {
        panic!("expected an elliptical arc");
    };
    assert!((sketch.point(end).unwrap().x - 20.0).abs() < DRAWN);
    assert_eq!(
        constraints_of_kind(sketch, "Coincident"),
        vec![Constraint::Coincident(end, wall)]
    );
}

#[test]
fn a_corner_of_a_line_and_an_elliptical_arc_is_rounded_by_a_sketch_fillet() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let arc = sketch.add_elliptical_arc(
        Point2::new(30.0, 30.0),
        Point2::new(45.0, 30.0),
        6.0,
        Point2::new(45.0, 30.0),
        Point2::new(30.0, 36.0),
    );
    let line = sketch.add_line(Point2::new(45.0, 30.0), Point2::new(60.0, 30.0));
    let Some(&Entity::EllipticalArc { start, .. }) = sketch.entity(arc) else {
        panic!("expected an elliptical arc");
    };
    let Some(&Entity::Line {
        start: line_start, ..
    }) = sketch.entity(line)
    else {
        panic!("expected a line");
    };
    sketch
        .add_constraint(Constraint::Coincident(line_start, start))
        .unwrap();
    let feature = edit_free_sketch(&mut harness, sketch);

    harness.select(entity_pickables(feature, &[arc, line]));
    run_from_palette(&mut harness, "fillet a sketch corner");
    assert_eq!(harness.tool(), Some(Tool::Fillet));
    type_point(&mut harness, "2");

    let sketch = harness.sketch(feature);
    let rounded = fillet_arc(sketch).unwrap();
    assert!((rounded.radius - 2.0).abs() < DRAWN);
    assert!((rounded.center.y - 32.0).abs() < DRAWN);
    assert_eq!(constraints_of_kind(sketch, "Tangent").len(), 2);
    assert_eq!(harness.model.undo_label(), Some(filleting::TRANSACTION));
}
