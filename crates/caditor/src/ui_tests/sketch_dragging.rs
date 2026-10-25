use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Constraint, EntityId, Sketch};
use egui::{Event, Key, Modifiers, PointerButton};

use super::{
    DRAWN, Harness, aligned_point_pairs, drag_in_sketch, edit_free_sketch, hold_drag, line_ends,
    release_drag,
};
use crate::{
    inference::Inference,
    model::Action,
    preferences::{PreferenceChange, PreferencesCommand},
    selection::Pickable,
};

#[test]
fn dragging_a_sketch_point_moves_it_as_its_constraints_allow_in_one_undoable_change() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
    let (start, end) = line_ends(&sketch, line);
    sketch
        .add_constraint(Constraint::Coincident(start, EntityId::ORIGIN))
        .unwrap();
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = harness.sketch(feature).clone();

    let grabbed = harness.hover_pickable(
        Plane::XY,
        Point2::new(40.0, 0.0),
        Pickable::SketchEntity {
            feature,
            entity: end,
        },
    );
    drag_in_sketch(&mut harness, grabbed, Point2::new(30.0, 12.0));
    harness.wait_until("the drag is committed", |harness| {
        harness.sketch(feature).point(end) != before.point(end)
    });
    harness.settle();

    let dragged = harness.sketch(feature).point(end).unwrap();
    assert!(
        dragged.distance(Point2::new(30.0, 12.0)) < 0.5,
        "{dragged:?}"
    );
    assert!(harness.sketch(feature).point(start).unwrap().length() < DRAWN);
    assert_eq!(
        harness.model.undo_label(),
        Some(format!("Drag {}", before.entity_label(end)).as_str())
    );
    let shown = harness.shown(feature).point(end).unwrap();
    assert!(shown.distance(dragged) < DRAWN);

    harness.perform(Action::Undo);
    harness.settle();
    assert_eq!(harness.sketch(feature).point(end), before.point(end));
}

#[test]
fn a_drag_pressed_before_its_pick_arrived_grabs_the_point_under_the_press() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
    let (start, end) = line_ends(&sketch, line);
    sketch
        .add_constraint(Constraint::Coincident(start, EntityId::ORIGIN))
        .unwrap();
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = harness.sketch(feature).clone();
    harness.point_at(Point2::new(20.0, 25.0));

    harness.picks_held = true;
    let grabbed = harness.on_screen(Point2::new(40.0, 0.0));
    harness.events.push(Event::PointerMoved(grabbed));
    harness.frame();
    drag_in_sketch(&mut harness, grabbed, Point2::new(30.0, 12.0));
    harness.picks_held = false;
    harness.wait_until("the drag is committed", |harness| {
        harness.sketch(feature).point(end) != before.point(end)
    });
    harness.settle();

    let dragged = harness.sketch(feature).point(end).unwrap();
    assert!(
        dragged.distance(Point2::new(30.0, 12.0)) < 0.5,
        "{dragged:?}"
    );
}

#[test]
fn a_point_dragged_onto_another_snaps_and_joins_it_in_the_same_change() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
    let (start, end) = line_ends(&sketch, line);
    sketch
        .add_constraint(Constraint::Coincident(start, EntityId::ORIGIN))
        .unwrap();
    let lone = sketch.add_point(Point2::new(30.0, 12.0));
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = harness.sketch(feature).clone();

    let grabbed = harness.hover_pickable(
        Plane::XY,
        Point2::new(40.0, 0.0),
        Pickable::SketchEntity {
            feature,
            entity: end,
        },
    );
    drag_in_sketch(&mut harness, grabbed, Point2::new(30.02, 12.02));
    harness.wait_until("the drag is committed", |harness| {
        harness.sketch(feature).point(end) != before.point(end)
    });
    harness.settle();

    let sketch = harness.sketch(feature);
    assert!(
        sketch
            .constraints()
            .any(|(_, constraint)| *constraint == Constraint::Coincident(end, lone))
    );
    assert!(sketch.point(end).unwrap().distance(Point2::new(30.0, 12.0)) < 1e-6);
    assert_eq!(
        harness.model.undo_label(),
        Some(format!("Drag {}", before.entity_label(end)).as_str())
    );
}

#[test]
fn a_point_dragged_near_another_stays_free_once_coincident_is_turned_off() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
    let (start, end) = line_ends(&sketch, line);
    sketch
        .add_constraint(Constraint::Coincident(start, EntityId::ORIGIN))
        .unwrap();
    let lone = sketch.add_point(Point2::new(30.0, 12.0));
    let feature = edit_free_sketch(&mut harness, sketch);
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Infer(Inference::Coincident, false),
    )));
    harness.settle();
    let before = harness.sketch(feature).clone();

    let grabbed = harness.hover_pickable(
        Plane::XY,
        Point2::new(40.0, 0.0),
        Pickable::SketchEntity {
            feature,
            entity: end,
        },
    );
    drag_in_sketch(&mut harness, grabbed, Point2::new(30.02, 12.02));
    harness.wait_until("the drag is committed", |harness| {
        harness.sketch(feature).point(end) != before.point(end)
    });
    harness.settle();

    let sketch = harness.sketch(feature);
    assert!(
        !sketch
            .constraints()
            .any(|(_, constraint)| *constraint == Constraint::Coincident(end, lone))
    );
    let dragged = sketch.point(end).unwrap();
    assert!(
        dragged.distance(Point2::new(30.02, 12.02)) < 0.01,
        "{dragged:?}"
    );
}

#[test]
fn a_line_dragged_by_its_middle_lands_its_nearer_end_on_a_point_and_joins_it() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(20.0, 0.0));
    let (start, end) = line_ends(&sketch, line);
    let lone = sketch.add_point(Point2::new(40.0, 20.0));
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = harness.sketch(feature).clone();

    let grabbed = harness.hover_pickable(
        Plane::XY,
        Point2::new(15.0, 0.0),
        Pickable::SketchEntity {
            feature,
            entity: line,
        },
    );
    drag_in_sketch(&mut harness, grabbed, Point2::new(35.03, 20.02));
    harness.wait_until("the drag is committed", |harness| {
        harness.sketch(feature).point(end) != before.point(end)
    });
    harness.settle();

    let sketch = harness.sketch(feature);
    assert!(
        sketch
            .constraints()
            .any(|(_, constraint)| *constraint == Constraint::Coincident(end, lone))
    );
    assert!(sketch.point(end).unwrap().distance(Point2::new(40.0, 20.0)) < 1e-6);
    assert!(
        sketch
            .point(start)
            .unwrap()
            .distance(Point2::new(20.0, 20.0))
            < 1e-3
    );
    assert_eq!(
        harness.model.undo_label(),
        Some(format!("Drag {}", before.entity_label(line)).as_str())
    );
}

#[test]
fn a_selection_dragged_by_one_point_lands_another_on_a_point_it_comes_near_and_joins_it() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_line(Point2::ZERO, Point2::new(20.0, 0.0));
    let second = sketch.add_line(Point2::new(0.0, 10.0), Point2::new(20.0, 10.0));
    let (grabbed_end, _) = line_ends(&sketch, first);
    let (_, far_end) = line_ends(&sketch, second);
    let lone = sketch.add_point(Point2::new(50.0, 40.0));
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = harness.sketch(feature).clone();
    harness.select([first, second].map(|entity| Pickable::SketchEntity { feature, entity }));
    harness.frame();

    let grabbed = harness.hover_pickable(
        Plane::XY,
        Point2::new(1.0, 0.0),
        Pickable::SketchEntity {
            feature,
            entity: first,
        },
    );
    drag_in_sketch(&mut harness, grabbed, Point2::new(31.03, 29.98));
    harness.wait_until("the drag is committed", |harness| {
        harness.sketch(feature).point(far_end) != before.point(far_end)
    });
    harness.settle();

    let sketch = harness.sketch(feature);
    assert!(
        sketch
            .constraints()
            .any(|(_, constraint)| *constraint == Constraint::Coincident(far_end, lone))
    );
    assert!(
        sketch
            .point(far_end)
            .unwrap()
            .distance(Point2::new(50.0, 40.0))
            < 1e-6
    );
    assert!(
        sketch
            .point(grabbed_end)
            .unwrap()
            .distance(Point2::new(30.0, 30.0))
            < 1e-3
    );
}

#[test]
fn a_dragged_point_tracks_the_other_end_of_its_line_and_is_kept_level_with_it() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(0.0, 10.0), Point2::new(20.0, 25.0));
    let (start, end) = line_ends(&sketch, line);
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = harness.sketch(feature).clone();

    let grabbed = harness.hover_pickable(
        Plane::XY,
        Point2::new(20.0, 25.0),
        Pickable::SketchEntity {
            feature,
            entity: end,
        },
    );
    let to = harness.on_screen(Point2::new(12.0, 10.03));
    hold_drag(&mut harness, grabbed, to);
    assert!(harness.shows(&format!("Horizontal from {}", before.entity_label(start))));
    release_drag(&mut harness, to);
    harness.wait_until("the drag is committed", |harness| {
        harness.sketch(feature).point(end) != before.point(end)
    });
    harness.settle();

    let sketch = harness.sketch(feature);
    assert_eq!(
        aligned_point_pairs(sketch),
        vec![Constraint::HorizontalPoints(end, start.into())]
    );
    let (from, to) = (sketch.point(start).unwrap(), sketch.point(end).unwrap());
    assert!((from.y - to.y).abs() < 1e-6, "{from} {to}");
}

#[test]
fn a_drag_in_a_conflicting_sketch_names_the_conflict() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
    let (start, end) = line_ends(&sketch, line);
    for constraint in [
        Constraint::Coincident(start, EntityId::ORIGIN),
        Constraint::Horizontal(line),
        Constraint::Distance {
            from: start,
            to: end,
            value: Expression::parse_stored("40 mm").unwrap(),
        },
        Constraint::Distance {
            from: start,
            to: end,
            value: Expression::parse_stored("50 mm").unwrap(),
        },
    ] {
        sketch.add_constraint(constraint).unwrap();
    }
    let feature = edit_free_sketch(&mut harness, sketch);
    harness.settle();
    let conflict = harness.model.sketch_conflict(feature);

    let grabbed = harness.hover_pickable(
        Plane::XY,
        Point2::new(40.0, 0.0),
        Pickable::SketchEntity {
            feature,
            entity: end,
        },
    );
    harness.events.push(Event::PointerButton {
        pos: grabbed,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    let target = harness.on_screen(Point2::new(30.0, 12.0));
    for step in 1..=4 {
        harness.events.push(Event::PointerMoved(
            grabbed + (target - grabbed) * (step as f32 / 4.0),
        ));
        harness.frame();
    }
    harness.wait_until("the cue shows", |harness| {
        harness.shows_containing(crate::viewport::DRAG_CONFLICT)
    });
    harness.events.push(Event::PointerButton {
        pos: target,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.wait_until("the drag ends", |harness| !harness.model.drag_blocked());
    harness.frame();

    let conflict = conflict.expect("the sketch conflicts");
    assert!(conflict.contains("Distance"), "{conflict}");
    assert!(harness.shows_containing(&conflict));
}

#[test]
fn a_drag_the_constraints_cannot_follow_says_so_until_it_ends() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
    let (start, end) = line_ends(&sketch, line);
    for constraint in [
        Constraint::Coincident(start, EntityId::ORIGIN),
        Constraint::Horizontal(line),
        Constraint::Vertical(line),
    ] {
        sketch.add_constraint(constraint).unwrap();
    }
    let feature = edit_free_sketch(&mut harness, sketch);

    let grabbed = harness.hover_pickable(
        Plane::XY,
        Point2::new(40.0, 0.0),
        Pickable::SketchEntity {
            feature,
            entity: end,
        },
    );
    harness.events.push(Event::PointerButton {
        pos: grabbed,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    let target = harness.on_screen(Point2::new(30.0, 12.0));
    for step in 1..=4 {
        harness.events.push(Event::PointerMoved(
            grabbed + (target - grabbed) * (step as f32 / 4.0),
        ));
        harness.frame();
    }
    let cued = |harness: &Harness| {
        harness.shows(crate::viewport::DRAG_BLOCKED)
            || harness.shows_containing(crate::viewport::DRAG_CONFLICT)
    };
    harness.wait_until("the cue shows", cued);
    assert!(harness.model.drag_blocked());

    harness.events.push(Event::PointerButton {
        pos: target,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.wait_until("the drag ends", |harness| !harness.model.drag_blocked());
    harness.frame();
    assert!(!cued(&harness));
}

#[test]
fn dragging_across_empty_space_selects_what_the_box_takes_in() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let near = sketch.add_line(Point2::new(10.0, 10.0), Point2::new(20.0, 10.0));
    let far = sketch.add_line(Point2::new(15.0, 30.0), Point2::new(60.0, 30.0));
    let feature = edit_free_sketch(&mut harness, sketch);
    let selected = |harness: &Harness| {
        let mut entities =
            crate::sketch_tools::selected_entities(harness.workspace.viewport.selection(), feature);
        entities.sort();
        entities
    };

    let corner = harness.on_screen(Point2::new(5.0, 25.0));
    harness.events.push(Event::PointerMoved(corner));
    harness.frame();
    drag_in_sketch(&mut harness, corner, Point2::new(25.0, 5.0));
    assert_eq!(selected(&harness), vec![near]);

    let corner = harness.on_screen(Point2::new(25.0, 5.0));
    harness.events.push(Event::PointerMoved(corner));
    harness.frame();
    drag_in_sketch(&mut harness, corner, Point2::new(5.0, 35.0));
    let mut both = vec![near, far];
    both.sort();
    assert_eq!(selected(&harness), both);
}

#[test]
fn escape_during_a_drag_puts_the_geometry_back_and_changes_nothing() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::new(20.0, 20.0), 10.0);
    let feature = edit_free_sketch(&mut harness, sketch);
    let revision = harness.model.revision();

    let grabbed = harness.hover_pickable(
        Plane::XY,
        Point2::new(30.0, 20.0),
        Pickable::SketchEntity {
            feature,
            entity: circle,
        },
    );
    harness.events.push(Event::PointerButton {
        pos: grabbed,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    let outside = harness.on_screen(Point2::new(35.0, 20.0));
    for step in 1..=4 {
        let position = grabbed + (outside - grabbed) * (step as f32 / 4.0);
        harness.events.push(Event::PointerMoved(position));
        harness.frame();
    }
    harness.wait_until("the dragged circle is shown", |harness| {
        harness
            .shown(feature)
            .circle(circle)
            .is_some_and(|(_, radius)| (radius - 15.0).abs() < 0.5)
    });
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.events.push(Event::PointerButton {
        pos: outside,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    harness.settle();

    assert_eq!(harness.model.revision(), revision);
    assert_eq!(harness.shown(feature).circle(circle).unwrap().1, 10.0);
    assert_eq!(harness.sketch(feature).circle(circle).unwrap().1, 10.0);
    assert_eq!(harness.editing(), Some(feature));
}
