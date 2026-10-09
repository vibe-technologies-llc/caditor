use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Constraint, Entity, Sketch};
use egui::{Key, Modifiers};

use super::{Harness, edit_free_sketch, entity_pickables, run_from_palette};
use crate::sketch_drag;

const SELECT_FREE: &str = "select what is still free";

#[test]
fn selecting_what_is_still_free_takes_only_the_geometry_that_can_still_move() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let held = sketch.add_line(Point2::ZERO, Point2::new(20.0, 0.0));
    let loose = sketch.add_line(Point2::new(0.0, 10.0), Point2::new(20.0, 10.0));
    let lone = sketch.add_point(Point2::new(30.0, 30.0));
    let Some(Entity::Line { start, end }) = sketch.entity(held).cloned() else {
        panic!("the held line is a line");
    };
    for point in [start, end] {
        let at = sketch.point(point).unwrap();
        sketch
            .add_constraint(Constraint::Fix { point, at })
            .unwrap();
    }
    let feature = edit_free_sketch(&mut harness, sketch);
    harness.settle();

    run_from_palette(&mut harness, SELECT_FREE);
    harness.frame();

    let mut chosen: Vec<_> = harness.workspace.viewport.selection().iter().collect();
    chosen.sort();
    let mut expected = entity_pickables(feature, &[loose, lone]);
    expected.sort();
    assert_eq!(chosen, expected);

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let mut fixed = Sketch::new(Plane::XY);
    let point = fixed.add_point(Point2::new(5.0, 5.0));
    fixed
        .add_constraint(Constraint::Fix {
            point,
            at: Point2::new(5.0, 5.0),
        })
        .unwrap();
    edit_free_sketch(&mut harness, fixed);
    harness.settle();

    run_from_palette(&mut harness, SELECT_FREE);
    harness.frame();

    assert!(harness.shows_containing(sketch_drag::NOTHING_FREE));
    assert!(harness.workspace.viewport.selection().is_empty());
}
