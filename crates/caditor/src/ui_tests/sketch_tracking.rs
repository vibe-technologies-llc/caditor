use caditor_geometry::{Plane, Point2, Vector2};
use caditor_sketch::{Constraint, EntityId, Sketch};
use egui::{Key, Modifiers};

use super::{Harness, constraints_of_kind, edit_free_sketch, entities_of_kind, line_ends, near};

fn placed_alone(sketch: &Sketch, before: &[EntityId]) -> EntityId {
    let [placed] = entities_of_kind(sketch, "Point")
        .into_iter()
        .filter(|point| !before.contains(point))
        .collect::<Vec<_>>()[..]
    else {
        panic!("one point should be placed");
    };
    placed
}

fn free_of_constraints(sketch: &Sketch, point: EntityId) -> bool {
    !sketch
        .constraints()
        .any(|(_, constraint)| constraint.entities().contains(&point))
}

#[test]
fn a_line_drawn_after_hovering_the_middle_of_another_ends_level_with_that_middle() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let base = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(60.0, 40.0));
    let label = sketch.entity_label(base);
    let feature = edit_free_sketch(&mut harness, sketch);

    harness.use_tool(Key::L);
    harness.point_at(Point2::new(30.05, 20.05));
    assert!(harness.shows(&format!("Midpoint of {label}")));
    harness.click_at(Point2::new(10.0, 35.0));
    harness.point_at(Point2::new(55.0, 20.1));
    assert!(harness.shows(&format!(
        "Horizontal from the midpoint of {label}, not kept there"
    )));
    harness.click_at(Point2::new(55.0, 20.1));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.settle();

    let sketch = harness.sketch(feature);
    let drawn = entities_of_kind(sketch, "Line")
        .into_iter()
        .find(|line| *line != base)
        .expect("a line is drawn");
    let (start, end) = line_ends(sketch, drawn);
    let landed = sketch.point(end).unwrap();
    assert!(near(sketch.point(start).unwrap(), Point2::new(10.0, 35.0)));
    assert!((landed.y - 20.0).abs() < 1e-9, "{landed}");
    assert!((landed.x - 55.0).abs() < 0.1, "{landed}");
    assert!(free_of_constraints(sketch, end));
}

#[test]
fn a_point_placed_square_to_a_line_through_its_middle_lands_on_its_perpendicular_bisector() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let base = sketch.add_line(Point2::new(10.0, 10.0), Point2::new(40.0, 50.0));
    let label = sketch.entity_label(base);
    let middle = Point2::new(25.0, 30.0);
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = entities_of_kind(harness.sketch(feature), "Point");

    harness.use_tool(Key::P);
    harness.point_at(middle + Vector2::new(0.05, 0.05));
    assert!(harness.shows(&format!("Midpoint of {label}")));
    harness.point_at(Point2::new(5.04, 45.03));
    assert!(harness.shows(&format!(
        "On the perpendicular bisector of {label}, not kept there"
    )));
    harness.click_at(Point2::new(5.04, 45.03));
    harness.settle();

    let sketch = harness.sketch(feature);
    let placed = placed_alone(sketch, &before);
    let at = sketch.point(placed).unwrap();
    assert!(
        (at - middle).dot(Vector2::new(3.0, 4.0)).abs() < 1e-9,
        "{at} is off the bisector"
    );
    assert!(at.distance(Point2::new(5.0, 45.0)) < 0.1, "{at}");
    assert!(free_of_constraints(sketch, placed));
}

#[test]
fn hovering_the_centre_of_a_rectangle_highlights_all_of_it_and_the_corners_holding_the_point() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::R);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(50.0, 30.0));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.settle();
    let sides = entities_of_kind(harness.sketch(feature), "Line");
    let corners = entities_of_kind(harness.sketch(feature), "Point");

    harness.use_tool(Key::P);
    harness.point_at(Point2::new(30.2, 20.1));
    assert!(harness.shows_containing("Centre of the outline of Line"));
    let highlighted = harness.workspace.viewport.snap_highlight();
    harness.click_at(Point2::new(30.2, 20.1));
    harness.settle();

    let symmetric = constraints_of_kind(harness.sketch(feature), "Symmetric");
    let [Constraint::Symmetric { first, second, .. }] = symmetric[..] else {
        panic!("one symmetry about the centre is expected");
    };
    assert_eq!(sides.len(), 4);
    assert!(sides.iter().all(|side| highlighted.contains(side)));
    assert!(highlighted.contains(&first) && highlighted.contains(&second));
    assert_eq!(
        highlighted
            .iter()
            .filter(|entity| corners.contains(entity))
            .count(),
        2
    );
}
