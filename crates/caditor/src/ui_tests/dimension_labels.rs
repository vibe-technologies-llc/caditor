use egui::{Key, Modifiers, vec2};

use super::{Harness, constraint_of_kind, edit_base_sketch, hold_drag, release_drag, type_point};
use crate::annotations;

const WIDTH_LABEL: &str = "width = 40 mm";

#[test]
fn a_dragged_dimension_label_stays_where_it_was_dropped_in_one_undoable_step() {
    let mut harness = Harness::new();
    let base = edit_base_sketch(&mut harness);
    let distance = constraint_of_kind(harness.sketch(base), "Distance");
    let before = harness.sketch(base).clone();

    let from = harness.position_of(WIDTH_LABEL);
    let to = from + vec2(60.0, -50.0);
    hold_drag(&mut harness, from, to);
    release_drag(&mut harness, to);

    let sketch = harness.sketch(base);
    assert!(sketch.label_offset(distance).is_some());
    assert!(sketch.same_geometry(&before));
    assert!(harness.position_of(WIDTH_LABEL).distance(to) < 2.0);
    assert_eq!(
        harness.model.undo_label(),
        Some(annotations::MOVE_LABEL_TRANSACTION)
    );

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.settle();
    assert_eq!(harness.sketch(base).label_offset(distance), None);
    assert!(harness.position_of(WIDTH_LABEL).distance(from) < 2.0);
}

#[test]
fn the_move_command_types_a_selected_dimensions_label_to_a_new_place() {
    let mut harness = Harness::new();
    let base = edit_base_sketch(&mut harness);
    let distance = constraint_of_kind(harness.sketch(base), "Distance");
    let from = harness.position_of(WIDTH_LABEL);

    harness.click(WIDTH_LABEL);
    harness.key(Key::M, Modifiers::NONE);
    harness.frame();
    harness.frame();
    type_point(&mut harness, "@0, 10");

    let offset = harness.sketch(base).label_offset(distance).unwrap();
    let moved = harness.position_of(WIDTH_LABEL);
    assert!(offset.length() > 0.0);
    assert!((moved.x - from.x).abs() < 2.0);
    assert!(moved.y < from.y - 5.0);
}
