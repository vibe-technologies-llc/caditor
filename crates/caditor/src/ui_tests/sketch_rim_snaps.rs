use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Constraint, Entity, Sketch};

use super::{Harness, constraints_of_kind, drag_in_sketch, edit_free_sketch, sketch_entity};
use crate::model::Action;

#[test]
fn a_circle_dragged_onto_a_corner_passes_through_it_and_keeps_it_there() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::ZERO, 10.0);
    let corner = sketch.add_point(Point2::new(30.0, 40.0));
    let feature = edit_free_sketch(&mut harness, sketch);

    let grabbed = harness.hover_pickable(
        Plane::XY,
        Point2::new(10.0, 0.0),
        sketch_entity(feature, circle),
    );
    drag_in_sketch(&mut harness, grabbed, Point2::new(30.2, 40.1));
    harness.wait_until("the drag is committed", |harness| {
        matches!(
            harness.sketch(feature).entity(circle),
            Some(Entity::Circle { radius, .. }) if (radius - 50.0).abs() < 1e-6
        )
    });
    harness.settle();

    assert_eq!(
        constraints_of_kind(harness.sketch(feature), "Coincident"),
        vec![Constraint::Coincident(corner, circle)]
    );

    harness.perform(Action::Undo);
    harness.settle();
    assert!(constraints_of_kind(harness.sketch(feature), "Coincident").is_empty());
}
