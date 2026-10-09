use caditor_document::FeatureId;
use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Constraint, Sketch};
use egui::{Key, Modifiers};

use super::{Harness, constraints_of_kind, edit_free_sketch, entities_of_kind, run_from_palette};
use crate::{
    blend_curving,
    editing::Tool,
    shape_modes::{BlendMode, ShapeMode},
};

const BLEND_CURVE: &str = "draw blend curve";
const NEAR_FIRST_END: Point2 = Point2::new(38.0, 10.0);
const NEAR_SECOND_START: Point2 = Point2::new(60.0, 32.0);

fn two_lines(harness: &mut Harness) -> FeatureId {
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_line(Point2::new(10.0, 10.0), Point2::new(40.0, 10.0));
    sketch.add_line(Point2::new(60.0, 30.0), Point2::new(60.0, 60.0));
    edit_free_sketch(harness, sketch)
}

fn press(harness: &mut Harness, key: Key, modifiers: Modifiers) {
    harness.key(key, modifiers);
    harness.frame();
    harness.frame();
}

#[test]
fn two_clicked_ends_get_a_tangent_blend_previewed_first_and_undone_in_one_step() {
    let mut harness = Harness::new();
    let feature = two_lines(&mut harness);
    let before = harness.sketch(feature).clone();

    run_from_palette(&mut harness, BLEND_CURVE);
    assert_eq!(harness.tool(), Some(Tool::BlendCurve));
    assert!(harness.shows(blend_curving::PROMPT));
    assert!(harness.shows_containing("Blend curve tangent (G1)"));
    harness.click_at(NEAR_FIRST_END);
    assert!(harness.shows(blend_curving::SECOND_PROMPT));
    harness.point_at(NEAR_SECOND_START);
    assert!(harness.shows_containing("tangent to both (G1)"));
    harness.click_at(NEAR_SECOND_START);
    harness.settle();

    let sketch = harness.sketch(feature);
    let splines = entities_of_kind(sketch, "Spline");
    assert_eq!(splines.len(), 1);
    let curve = sketch.spline(splines[0]).unwrap();
    assert_eq!(curve.control_points().len(), 4);
    assert!(curve.point_at(0.0).distance(Point2::new(40.0, 10.0)) < 1e-6);
    assert!(curve.point_at(1.0).distance(Point2::new(60.0, 30.0)) < 1e-6);
    assert_eq!(constraints_of_kind(sketch, "Tangent").len(), 2);
    assert_eq!(constraints_of_kind(sketch, "Curvature").len(), 0);
    assert_eq!(harness.model.undo_label(), Some(blend_curving::TRANSACTION));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert!(harness.sketch(feature).same_content(&before));
}

#[test]
fn the_keyboard_alone_switches_to_curvature_and_blends_two_highlighted_ends() {
    let mut harness = Harness::new();
    let feature = two_lines(&mut harness);

    press(&mut harness, Key::B, Modifiers::ALT | Modifiers::SHIFT);
    assert_eq!(harness.tool(), Some(Tool::BlendCurve));
    press(&mut harness, Key::B, Modifiers::ALT | Modifiers::SHIFT);
    assert_eq!(
        harness.workspace.editing.modes().of(Tool::BlendCurve),
        Some(ShapeMode::Blend(BlendMode::Curvature))
    );
    for key in [Key::N, Key::N, Key::Space, Key::N, Key::N] {
        press(&mut harness, key, Modifiers::NONE);
    }
    assert!(harness.shows_containing("curvature (G2)"));
    press(&mut harness, Key::Space, Modifiers::NONE);
    harness.settle();

    let sketch = harness.sketch(feature);
    let splines = entities_of_kind(sketch, "Spline");
    assert_eq!(splines.len(), 1);
    assert_eq!(constraints_of_kind(sketch, "Tangent").len(), 2);
    assert_eq!(constraints_of_kind(sketch, "Curvature").len(), 2);
    assert_eq!(harness.tool(), Some(Tool::BlendCurve));
}

#[test]
fn ends_already_meeting_are_refused_in_words_and_nothing_is_drawn() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_line(Point2::new(10.0, 10.0), Point2::new(40.0, 10.0));
    let second = sketch.add_line(Point2::new(40.0, 10.0), Point2::new(40.0, 40.0));
    let [_, corner] = sketch.ends_of_curve(first).unwrap();
    let [joined, _] = sketch.ends_of_curve(second).unwrap();
    sketch
        .add_constraint(Constraint::Coincident(corner, joined))
        .unwrap();
    let feature = edit_free_sketch(&mut harness, sketch);

    run_from_palette(&mut harness, BLEND_CURVE);
    harness.click_at(Point2::new(38.0, 10.0));
    harness.point_at(Point2::new(40.0, 12.0));
    assert!(harness.shows_containing("no gap to blend"));
    harness.click_at(Point2::new(40.0, 12.0));
    harness.settle();

    assert!(entities_of_kind(harness.sketch(feature), "Spline").is_empty());
}
