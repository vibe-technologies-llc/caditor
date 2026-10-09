use caditor_expression::Quantity;
use caditor_geometry::{Plane, Point2, Vector2};
use caditor_sketch::{Constraint, Entity, EntityId, RelationKind, Sketch, Tolerance};
use egui::{Key, Modifiers};

use super::{
    Harness, constraints_of_kind, edit_free_sketch, entities_of_kind, entity_pickables,
    run_from_palette,
};
use crate::{tidy_panel, tidying::Task};

const FIND_RELATIONS: &str = "add the relations the drawing shows";
const CHECK: &str = "check the sketch for flaws";
const DIMENSION: &str = "dimension fully from a datum point";

fn loose_rectangle(sketch: &mut Sketch) -> Vec<EntityId> {
    let corners = [
        Point2::new(10.0, 10.0),
        Point2::new(50.0, 10.05),
        Point2::new(50.03, 35.0),
        Point2::new(10.0, 35.0),
    ];
    let hair = Vector2::new(0.002, -0.001);
    (0..4)
        .map(|index| sketch.add_line(corners[index], corners[(index + 1) % 4] + hair))
        .collect()
}

fn found(harness: &mut Harness) {
    harness.frame();
    harness.workspace.tidying.wait();
    harness.frame();
    harness.frame();
}

#[test]
fn relations_the_drawing_shows_are_listed_then_added_as_one_undoable_change() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    loose_rectangle(&mut sketch);
    let feature = edit_free_sketch(&mut harness, sketch);
    harness.settle();
    let before = harness.sketch(feature).clone();

    run_from_palette(&mut harness, FIND_RELATIONS);
    found(&mut harness);

    for (text, _) in &harness.texts {
        eprintln!("TEXT {text}");
    }
    assert_eq!(harness.workspace.tidying.task, Task::Relations);
    assert!(harness.shows_containing(tidy_panel::TITLE));
    assert!(harness.shows("Coincident ends (4)"));
    assert!(harness.shows("Horizontal lines (2)"));
    assert!(harness.shows("Adding them leaves 4 degrees of freedom."));
    assert!(harness.sketch(feature).same_content(&before));

    harness.click_button("Add 8 relations");
    harness.settle();

    let sketch = harness.sketch(feature);
    assert_eq!(constraints_of_kind(sketch, "Coincident").len(), 4);
    assert_eq!(constraints_of_kind(sketch, "Horizontal").len(), 2);
    assert_eq!(constraints_of_kind(sketch, "Vertical").len(), 2);
    assert_eq!(
        harness.model.undo_label(),
        Some("Add 8 relations the drawing shows")
    );
    assert!(
        harness
            .model
            .settled_sketch(feature)
            .unwrap()
            .open_ends()
            .is_empty()
    );

    found(&mut harness);
    assert!(harness.shows(tidy_panel::NO_RELATIONS));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.settle();
    assert!(harness.sketch(feature).same_content(&before));
}

#[test]
fn a_line_drawn_twice_is_named_by_the_check_and_its_fix_removes_it() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_line(Point2::new(5.0, 5.0), Point2::new(40.0, 5.0));
    let copy = sketch.add_line(Point2::new(10.0, 5.0), Point2::new(30.0, 5.0));
    let feature = edit_free_sketch(&mut harness, sketch);
    harness.settle();

    run_from_palette(&mut harness, CHECK);
    found(&mut harness);

    assert!(harness.shows_containing(&format!("Line {copy} lies on")));
    harness.click_button("Fix 1 flaw");
    harness.settle();

    let sketch = harness.sketch(feature);
    assert_eq!(entities_of_kind(sketch, "Line").len(), 1);
    assert!(sketch.entity(copy).is_none());
    found(&mut harness);
    assert!(harness.shows(tidy_panel::NO_FLAWS));
}

#[test]
fn a_rectangle_dimensioned_from_its_selected_corner_becomes_fully_constrained() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let sides = loose_rectangle(&mut sketch);
    let related = sketch
        .inferred_relations(
            Tolerance::of(&sketch),
            &RelationKind::ALL.into_iter().collect(),
            &|_| Ok(Quantity::plain(0.0)),
            &|| false,
        )
        .unwrap();
    for constraint in related.constraints {
        sketch.add_constraint(constraint).unwrap();
    }
    let Some(&Entity::Line { start: corner, .. }) = sketch.entity(sides[0]) else {
        panic!("the side is a line");
    };
    let feature = edit_free_sketch(&mut harness, sketch);
    harness.settle();

    harness.select(entity_pickables(feature, &[corner]));
    run_from_palette(&mut harness, DIMENSION);
    found(&mut harness);

    assert_eq!(harness.workspace.tidying.datum, corner);
    assert!(harness.shows("Adding them leaves the sketch fully constrained."));
    harness.click_button("Add 4 dimensions");
    harness.settle();

    let sketch = harness.sketch(feature);
    assert_eq!(
        sketch
            .constraints()
            .filter(|(_, constraint)| constraint.dimension().is_some())
            .count(),
        4
    );
    assert!(
        harness
            .model
            .settled_solution(feature)
            .unwrap()
            .is_fully_constrained()
    );
    assert!(matches!(
        constraints_of_kind(sketch, "Horizontal distance").first(),
        Some(Constraint::HorizontalDistance { .. })
    ));
}
