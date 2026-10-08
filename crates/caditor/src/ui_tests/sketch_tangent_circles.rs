use caditor_document::FeatureId;
use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;
use egui::{Key, Modifiers};

use super::{
    Harness, constraints_of_kind, edit_free_sketch, entities_of_kind, near, run_from_palette,
    type_point,
};
use crate::{editing::Tool, tangent_circling};

const TANGENT_CIRCLE: &str = "draw a circle tangent to sketch curves";
const BOTTOM: Point2 = Point2::new(30.0, 10.0);
const LEFT: Point2 = Point2::new(10.0, 25.0);
const SLOPE: Point2 = Point2::new(30.0, 25.0);

fn triangle_fixture(harness: &mut Harness) -> FeatureId {
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_line(Point2::new(10.0, 10.0), Point2::new(50.0, 10.0));
    sketch.add_line(Point2::new(10.0, 10.0), Point2::new(10.0, 40.0));
    sketch.add_line(Point2::new(50.0, 10.0), Point2::new(10.0, 40.0));
    edit_free_sketch(harness, sketch)
}

fn circles(sketch: &Sketch) -> Vec<(Point2, f64)> {
    entities_of_kind(sketch, "Circle")
        .into_iter()
        .filter_map(|circle| sketch.circle(circle))
        .collect()
}

#[test]
fn three_clicked_lines_get_a_tangent_circle_nearest_the_pointer_in_one_undoable_step() {
    let mut harness = Harness::new();
    let feature = triangle_fixture(&mut harness);
    let before = harness.sketch(feature).clone();

    run_from_palette(&mut harness, TANGENT_CIRCLE);
    assert_eq!(harness.tool(), Some(Tool::TangentCircle));
    assert!(harness.shows(tangent_circling::PROMPT));
    harness.point_at(BOTTOM);
    harness.click_at(BOTTOM);
    harness.point_at(LEFT);
    harness.click_at(LEFT);
    assert!(harness.shows(tangent_circling::RADIUS_PROMPT));
    harness.point_at(SLOPE);
    assert!(harness.shows_containing("Circle touching"));
    harness.click_at(SLOPE);
    harness.settle();

    let sketch = harness.sketch(feature);
    let made = circles(sketch);
    assert_eq!(made.len(), 1);
    assert!(near(made[0].0, Point2::new(20.0, 20.0)));
    assert!((made[0].1 - 10.0).abs() < 1e-3);
    assert_eq!(constraints_of_kind(sketch, "Tangent").len(), 3);
    assert_eq!(
        harness.model.undo_label(),
        Some(tangent_circling::TRANSACTION)
    );

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert!(harness.sketch(feature).same_content(&before));
}

#[test]
fn two_clicked_lines_and_a_typed_radius_give_the_tangent_circle_nearest_the_pointer() {
    let mut harness = Harness::new();
    let feature = triangle_fixture(&mut harness);

    run_from_palette(&mut harness, TANGENT_CIRCLE);
    harness.point_at(BOTTOM);
    harness.click_at(BOTTOM);
    type_point(&mut harness, "5");
    assert!(harness.shows_containing("choose the two curves"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    harness.point_at(LEFT);
    harness.click_at(LEFT);
    harness.point_at(Point2::new(25.0, 22.0));
    type_point(&mut harness, "5");

    let sketch = harness.sketch(feature);
    let made = circles(sketch);
    assert_eq!(made.len(), 1);
    assert!(near(made[0].0, Point2::new(15.0, 15.0)));
    assert!((made[0].1 - 5.0).abs() < 1e-3);
    assert_eq!(constraints_of_kind(sketch, "Tangent").len(), 2);
    assert_eq!(constraints_of_kind(sketch, "Radius").len(), 1);
}
