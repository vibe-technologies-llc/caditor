use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Constraint, EntityId, Sketch};
use egui::{Key, Modifiers};

use super::{
    Harness, constraints_of_kind, drag_in_sketch, edit_free_sketch, line_ends, sketch_entity,
};

const NOT_APPLIED: &str = "was not applied, because the model changed while it was being";

fn undo_by_key(harness: &mut Harness) {
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.frame();
}

fn notice_says_not_applied(harness: &Harness) -> bool {
    harness
        .model
        .notice()
        .is_some_and(|notice| notice.text.contains(NOT_APPLIED))
}

#[test]
fn a_drag_still_solving_at_release_is_dropped_when_an_undo_comes_first() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);

    let line = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
    let (start, end) = line_ends(&sketch, line);

    sketch
        .add_constraint(Constraint::Coincident(start, EntityId::ORIGIN))
        .unwrap();

    let feature = edit_free_sketch(&mut harness, sketch);
    let created = harness.sketch(feature).point(end);

    let grabbed = harness.hover_pickable(
        Plane::XY,
        Point2::new(40.0, 0.0),
        sketch_entity(feature, end),
    );

    drag_in_sketch(&mut harness, grabbed, Point2::new(30.0, 12.0));
    harness.wait_until("the first drag is committed", |harness| {
        harness.sketch(feature).point(end) != created
    });
    harness.settle();

    let first_drag = harness.model.undo_label().map(str::to_owned);
    let moved = harness.sketch(feature).point(end).unwrap();

    harness.model.hold_sketch_results(true);

    let grabbed = harness.hover_pickable(Plane::XY, moved, sketch_entity(feature, end));

    drag_in_sketch(&mut harness, grabbed, Point2::new(20.0, -10.0));
    undo_by_key(&mut harness);

    assert_eq!(harness.model.redo_label(), first_drag.as_deref());

    harness.model.hold_sketch_results(false);
    harness.wait_until("the held drag is answered", notice_says_not_applied);
    harness.settle();

    assert_eq!(harness.sketch(feature).point(end), created);
    assert_eq!(harness.model.redo_label(), first_drag.as_deref());
    assert!(notice_says_not_applied(&harness));
}

#[test]
fn a_constraint_still_being_checked_is_dropped_when_an_undo_comes_first() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);

    let levelled = sketch.add_line(Point2::ZERO, Point2::new(30.0, 10.0));
    let upright = sketch.add_line(Point2::new(0.0, 20.0), Point2::new(10.0, 50.0));
    let feature = harness.add_sketch(sketch);

    harness.edit(feature);

    harness.select([sketch_entity(feature, levelled)]);
    harness.click_button("Horizontal");
    harness.settle();

    let horizontal = harness.model.undo_label().map(str::to_owned);

    assert_eq!(
        constraints_of_kind(harness.sketch(feature), "Horizontal").len(),
        1
    );

    harness.model.hold_sketch_results(true);
    harness.select([sketch_entity(feature, upright)]);
    harness.click_button("Vertical");

    assert!(harness.model.is_checking_constraints());

    undo_by_key(&mut harness);

    assert_eq!(harness.model.redo_label(), horizontal.as_deref());

    harness.model.hold_sketch_results(false);
    harness.settle();

    let sketch = harness.sketch(feature);

    assert!(!harness.model.is_checking_constraints());
    assert!(constraints_of_kind(sketch, "Horizontal").is_empty());
    assert!(constraints_of_kind(sketch, "Vertical").is_empty());
    assert_eq!(harness.model.redo_label(), horizontal.as_deref());
    assert!(notice_says_not_applied(&harness));
}

#[test]
fn a_grab_begun_before_the_last_release_is_solved_commits_after_it() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);

    let first = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
    let second = sketch.add_line(Point2::new(0.0, 30.0), Point2::new(40.0, 30.0));
    let (first_start, first_end) = line_ends(&sketch, first);
    let (second_start, second_end) = line_ends(&sketch, second);

    for start in [first_start, second_start] {
        sketch
            .add_constraint(Constraint::Fix {
                point: start,
                at: sketch.point(start).unwrap(),
            })
            .unwrap();
    }

    let feature = edit_free_sketch(&mut harness, sketch);
    let first_created = harness.sketch(feature).point(first_end);
    let second_created = harness.sketch(feature).point(second_end);
    let before = harness.model.undo_label().map(str::to_owned);

    harness.model.hold_sketch_results(true);

    let grabbed = harness.hover_pickable(
        Plane::XY,
        Point2::new(40.0, 0.0),
        sketch_entity(feature, first_end),
    );

    drag_in_sketch(&mut harness, grabbed, Point2::new(30.0, 12.0));

    let grabbed = harness.hover_pickable(
        Plane::XY,
        Point2::new(40.0, 30.0),
        sketch_entity(feature, second_end),
    );

    drag_in_sketch(&mut harness, grabbed, Point2::new(20.0, 45.0));

    assert_eq!(harness.sketch(feature).point(first_end), first_created);
    assert_eq!(harness.model.undo_label(), before.as_deref());

    harness.model.hold_sketch_results(false);
    harness.wait_until("both drags are committed", |harness| {
        harness.sketch(feature).point(second_end) != second_created
    });
    harness.settle();

    let first_moved = harness.sketch(feature).point(first_end);

    assert_ne!(first_moved, first_created);
    assert!(
        harness
            .model
            .notice()
            .is_none_or(|notice| !notice.text.contains(NOT_APPLIED))
    );

    undo_by_key(&mut harness);
    harness.settle();

    assert_eq!(harness.sketch(feature).point(second_end), second_created);
    assert_eq!(harness.sketch(feature).point(first_end), first_moved);

    undo_by_key(&mut harness);
    harness.settle();

    assert_eq!(harness.sketch(feature).point(first_end), first_created);
    assert_eq!(harness.model.undo_label(), before.as_deref());
}
