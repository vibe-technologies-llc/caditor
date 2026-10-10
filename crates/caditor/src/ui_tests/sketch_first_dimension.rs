use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Entity, Sketch};
use egui::{Event, Key, Modifiers};

use super::{Harness, edit_free_sketch, run_from_palette};
use crate::selection::Pickable;

const SCALE_FIRST: &str = "scale the whole sketch on its first dimension";

fn dimension_line(harness: &mut Harness, scales: bool, typed: &str) -> (f64, f64) {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(10.0, 0.0), Point2::new(40.0, 40.0));
    let circle = sketch.add_circle(Point2::new(0.0, 20.0), 5.0);
    let feature = edit_free_sketch(harness, sketch);
    harness.settle();
    if scales {
        run_from_palette(harness, SCALE_FIRST);
        harness.frame();
    }
    harness.select([Pickable::SketchEntity {
        feature,
        entity: line,
    }]);
    harness.key(Key::D, Modifiers::SHIFT);
    harness.settle();

    harness.events.push(Event::Text(typed.to_owned()));
    harness.frame();
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.settle();

    let shown = harness.shown(feature);
    let (start, end) = shown.line_endpoints(line).unwrap();
    let Some(Entity::Circle { center, radius }) = shown.entity(circle).cloned() else {
        panic!("the circle is still a circle");
    };
    let centre = shown.point(center).unwrap();
    assert!((start.distance(end) - 100.0).abs() < 1e-6);
    (centre.y, radius)
}

#[test]
fn the_first_dimension_scales_the_whole_sketch_about_its_origin_when_asked() {
    let mut harness = Harness::new();

    let (height, radius) = dimension_line(&mut harness, true, "100");

    assert!((height - 40.0).abs() < 1e-6, "{height}");
    assert!((radius - 10.0).abs() < 1e-6, "{radius}");
    assert_eq!(
        harness.model.undo_label(),
        Some("Scale Plate to its first dimension")
    );
}

#[test]
fn without_the_option_the_first_dimension_moves_only_what_it_measures() {
    let mut harness = Harness::new();

    let (height, radius) = dimension_line(&mut harness, false, "100");

    assert!((height - 20.0).abs() < 1e-6, "{height}");
    assert!((radius - 5.0).abs() < 1e-6, "{radius}");
    assert_eq!(harness.model.undo_label(), Some("Edit dimension in Plate"));
}
