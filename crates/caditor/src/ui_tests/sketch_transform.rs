use caditor_geometry::{Plane, Point2};
use caditor_sketch::{EntityId, Sketch};
use egui::{Key, Modifiers};

use super::{DRAWN, Harness, edit_free_sketch, line_ends, run_from_palette};
use crate::{model::Action, selection::Pickable, sketch_toolbar, typed_point};

#[test]
fn the_sketch_bar_selects_everything_and_moves_it_to_a_typed_point() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(10.0, 10.0), Point2::new(20.0, 10.0));
    let feature = edit_free_sketch(&mut harness, sketch);
    let pickable = Pickable::SketchEntity {
        feature,
        entity: line,
    };

    harness.hover_button(sketch_toolbar::MOVE_LABEL);
    assert!(harness.shows(
        "Move the selected geometry to a typed position, or by a typed offset. Select sketch \
         geometry to move it (M)"
    ));
    harness.click_button(sketch_toolbar::SELECT_ALL_LABEL);
    harness.frame();
    assert!(harness.workspace.viewport.selection().contains(pickable));

    harness.click_button(sketch_toolbar::MOVE_LABEL);
    harness.frame();
    assert!(harness.shows(typed_point::MOVE_LABEL));
}

#[test]
fn a_waiting_move_or_rotate_field_acts_on_the_geometry_as_it_stands_at_enter() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(10.0, 10.0), Point2::new(20.0, 10.0));
    let (start, end) = line_ends(&sketch, line);
    let feature = edit_free_sketch(&mut harness, sketch);
    let selected = Pickable::SketchEntity {
        feature,
        entity: line,
    };
    let at = |harness: &Harness, point: EntityId| harness.sketch(feature).point(point).unwrap();

    harness.select([selected]);
    harness.key(Key::M, Modifiers::NONE);
    harness.frame();
    harness.frame();
    harness.type_text("30, 30");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.wait_until("the first move is committed", |harness| {
        harness.sketch(feature).point(start) == Some(Point2::new(30.0, 30.0))
    });

    harness.select([selected]);
    harness.key(Key::M, Modifiers::NONE);
    harness.frame();
    assert!(harness.shows(typed_point::MOVE_LABEL));
    harness.perform(Action::Undo);
    harness.settle();
    assert!(at(&harness, start).distance(Point2::new(10.0, 10.0)) < DRAWN);
    harness.type_text("40, 10");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.wait_until("the second move is committed", |harness| {
        harness.sketch(feature).point(start) != Some(Point2::new(10.0, 10.0))
    });

    assert!(at(&harness, start).distance(Point2::new(40.0, 10.0)) < DRAWN);
    assert!(at(&harness, end).distance(Point2::new(50.0, 10.0)) < DRAWN);

    harness.select([
        selected,
        Pickable::SketchEntity {
            feature,
            entity: EntityId::ORIGIN,
        },
    ]);
    run_from_palette(&mut harness, "rotate selected sketch");
    assert!(harness.shows(typed_point::ROTATE_FIELD.label));
    harness.perform(Action::Undo);
    harness.settle();
    assert!(at(&harness, start).distance(Point2::new(10.0, 10.0)) < DRAWN);
    harness.type_text("90");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.wait_until("the rotation is committed", |harness| {
        harness.sketch(feature).point(start) != Some(Point2::new(10.0, 10.0))
    });

    assert!(at(&harness, start).distance(Point2::new(-10.0, 10.0)) < DRAWN);
    assert!(at(&harness, end).distance(Point2::new(-10.0, 20.0)) < DRAWN);
}

#[test]
fn selected_geometry_moves_by_a_typed_offset_from_the_keyboard() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(10.0, 10.0), Point2::new(20.0, 10.0));
    let (start, end) = line_ends(&sketch, line);
    let feature = edit_free_sketch(&mut harness, sketch);

    harness.key(Key::A, Modifiers::COMMAND);
    harness.frame();
    harness.frame();
    assert!(
        harness
            .workspace
            .viewport
            .selection()
            .contains(Pickable::SketchEntity {
                feature,
                entity: line
            })
    );
    harness.key(Key::M, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert!(harness.shows(typed_point::MOVE_LABEL));
    harness.type_text("@5, -4");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.wait_until("the move is committed", |harness| {
        harness.sketch(feature).point(start) != Some(Point2::new(10.0, 10.0))
    });

    let moved = harness.sketch(feature);
    assert!(moved.point(start).unwrap().distance(Point2::new(15.0, 6.0)) < DRAWN);
    assert!(moved.point(end).unwrap().distance(Point2::new(25.0, 6.0)) < DRAWN);
    assert!(!harness.shows(typed_point::MOVE_LABEL));
    let label = format!("Move {}", harness.sketch(feature).entity_label(line));
    assert_eq!(harness.model.undo_label(), Some(label.as_str()));
}

#[test]
fn selected_geometry_rotates_about_a_selected_point_and_scales_about_its_centre_from_the_palette() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(10.0, 0.0), Point2::new(20.0, 0.0));
    let (start, end) = line_ends(&sketch, line);
    let circle = sketch.add_circle(Point2::new(40.0, 0.0), 5.0);
    let feature = edit_free_sketch(&mut harness, sketch);

    harness.select([
        Pickable::SketchEntity {
            feature,
            entity: line,
        },
        Pickable::SketchEntity {
            feature,
            entity: EntityId::ORIGIN,
        },
    ]);
    run_from_palette(&mut harness, "rotate selected sketch");
    let opened = harness.shows(typed_point::ROTATE_FIELD.label);
    harness.type_text("90");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.wait_until("the rotation is committed", |harness| {
        harness.sketch(feature).point(start) != Some(Point2::new(10.0, 0.0))
    });
    let rotated = harness.sketch(feature).clone();
    let rotate_label = harness.model.undo_label().map(str::to_owned);
    harness.select([Pickable::SketchEntity {
        feature,
        entity: circle,
    }]);
    run_from_palette(&mut harness, "scale selected sketch");
    harness.type_text("0");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();
    let refused = harness.shows("The factor must be a number above zero");
    harness.key(Key::A, Modifiers::COMMAND);
    harness.type_text("2");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.wait_until("the scale is committed", |harness| {
        harness
            .sketch(feature)
            .circle(circle)
            .map(|(_, radius)| radius)
            != Some(5.0)
    });

    assert!(opened);
    assert!(
        rotated
            .point(start)
            .unwrap()
            .distance(Point2::new(0.0, 10.0))
            < DRAWN
    );
    assert!(rotated.point(end).unwrap().distance(Point2::new(0.0, 20.0)) < DRAWN);
    assert_eq!(
        rotate_label.as_deref(),
        Some(format!("Rotate {}", rotated.entity_label(line)).as_str())
    );
    assert!(refused);
    let (centre, radius) = harness.sketch(feature).circle(circle).unwrap();
    assert!(centre.distance(Point2::new(40.0, 0.0)) < DRAWN);
    assert!((radius - 10.0).abs() < DRAWN);
}
