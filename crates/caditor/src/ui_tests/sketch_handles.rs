use caditor_geometry::{Plane, Point2};
use caditor_sketch::{EntityId, Sketch};
use egui::{Event, Key, Modifiers, PointerButton, Pos2};

use super::{DRAWN, Harness, edit_free_sketch, extruded_plate, line_ends, run_from_palette};
use crate::{model::Action, selection::Pickable, sketch_handles::SketchGrip};

fn drag_between(harness: &mut Harness, from: Pos2, to: Pos2, modifiers: Modifiers) {
    harness.hold(modifiers);
    harness.events.push(Event::PointerMoved(from));
    harness.frame();
    harness.events.push(Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers,
    });
    harness.frame();
    for step in 1..=6 {
        let fraction = step as f32 / 6.0;
        let at = Pos2::new(
            from.x + (to.x - from.x) * fraction,
            from.y + (to.y - from.y) * fraction,
        );

        harness.events.push(Event::PointerMoved(at));
        harness.frame();
    }
    harness.events.push(Event::PointerButton {
        pos: to,
        button: PointerButton::Primary,
        pressed: false,
        modifiers,
    });
    harness.frame();
    harness.hold(Modifiers::NONE);
}

#[test]
fn move_shows_2d_handles_that_drag_along_x_y_and_plane_in_one_undoable_step() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);

    let line = sketch.add_line(Point2::new(10.0, 10.0), Point2::new(30.0, 10.0));
    let (start, end) = line_ends(&sketch, line);
    let feature = edit_free_sketch(&mut harness, sketch);

    harness.select([Pickable::SketchEntity {
        feature,
        entity: line,
    }]);
    harness.frame();

    assert!(!harness.workspace.viewport.has_sketch_handles());

    harness.key(Key::M, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert!(harness.workspace.viewport.has_sketch_handles());

    let x_start = harness
        .workspace
        .viewport
        .sketch_handle_position(SketchGrip::AlongX, 0.0)
        .expect("X arrow should be shown");
    let x_end = harness
        .workspace
        .viewport
        .sketch_handle_position(SketchGrip::AlongX, 5.0)
        .expect("X destination");

    drag_between(&mut harness, x_start, x_end, Modifiers::COMMAND);
    harness.wait_until("the X drag commits", |harness| {
        harness.sketch(feature).point(start) != Some(Point2::new(10.0, 10.0))
    });
    harness.settle();

    let after_x = harness.sketch(feature);

    assert!(
        after_x
            .point(start)
            .unwrap()
            .distance(Point2::new(15.0, 10.0))
            < 0.5,
        "{:?}",
        after_x.point(start)
    );
    assert!(
        after_x
            .point(end)
            .unwrap()
            .distance(Point2::new(35.0, 10.0))
            < 0.5,
        "{:?}",
        after_x.point(end)
    );
    assert_eq!(
        harness.model.undo_label(),
        Some(format!("Move {}", after_x.entity_label(line)).as_str())
    );

    let y_start = harness
        .workspace
        .viewport
        .sketch_handle_position(SketchGrip::AlongY, 0.0)
        .expect("Y arrow should still be shown");
    let y_end = harness
        .workspace
        .viewport
        .sketch_handle_position(SketchGrip::AlongY, 5.0)
        .expect("Y destination");

    drag_between(&mut harness, y_start, y_end, Modifiers::COMMAND);
    harness.wait_until("the Y drag commits", |harness| {
        harness
            .sketch(feature)
            .point(start)
            .is_some_and(|at| (at.y - 15.0).abs() < 0.5)
    });
    harness.settle();

    let plane_start = harness
        .workspace
        .viewport
        .sketch_handle_position(SketchGrip::OnPlane, 0.0)
        .expect("XY plane grip should be shown");
    let plane_end = Pos2::new(plane_start.x + 30.0, plane_start.y - 30.0);

    drag_between(&mut harness, plane_start, plane_end, Modifiers::COMMAND);
    harness.wait_until("the plane drag commits", |harness| {
        harness
            .sketch(feature)
            .point(start)
            .is_some_and(|at| at.x > 15.2 && at.y > 15.2)
    });
    harness.settle();

    harness.perform(Action::Undo);
    harness.settle();

    assert!(
        harness
            .sketch(feature)
            .point(start)
            .unwrap()
            .distance(Point2::new(15.0, 15.0))
            < 0.5
    );
}

#[test]
fn sketch_handles_rotate_scale_cancel_with_escape_and_accept_typed_values() {
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

    assert!(harness.workspace.viewport.has_sketch_handles());

    let ring = harness
        .workspace
        .viewport
        .sketch_handle_position(SketchGrip::Rotate, 0.0)
        .expect("rotate ring should be shown");

    harness.events.push(Event::PointerMoved(ring));
    harness.frame();
    harness.events.push(Event::PointerButton {
        pos: ring,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    harness.events.push(Event::PointerButton {
        pos: ring,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    harness.frame();
    harness.type_text("90");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.wait_until("the typed handle rotation commits", |harness| {
        harness
            .sketch(feature)
            .point(start)
            .is_some_and(|at| at.distance(Point2::new(0.0, 10.0)) < DRAWN)
    });
    harness.settle();

    assert!(
        harness
            .sketch(feature)
            .point(end)
            .unwrap()
            .distance(Point2::new(0.0, 20.0))
            < DRAWN
    );

    harness.select([Pickable::SketchEntity {
        feature,
        entity: line,
    }]);
    harness.key(Key::M, Modifiers::NONE);
    harness.frame();
    harness.frame();

    let x_grip = harness
        .workspace
        .viewport
        .sketch_handle_position(SketchGrip::AlongX, 0.0)
        .expect("X grip should be shown");
    let x_away = harness
        .workspace
        .viewport
        .sketch_handle_position(SketchGrip::AlongX, 25.0)
        .expect("X away");

    harness.events.push(Event::PointerMoved(x_grip));
    harness.frame();
    harness.events.push(Event::PointerButton {
        pos: x_grip,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    harness.events.push(Event::PointerMoved(x_away));
    harness.frame();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.events.push(Event::PointerButton {
        pos: x_away,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    harness.settle();

    assert!(
        harness
            .sketch(feature)
            .point(start)
            .unwrap()
            .distance(Point2::new(0.0, 10.0))
            < DRAWN
    );

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    assert!(!harness.workspace.viewport.has_sketch_handles());

    harness.select([Pickable::SketchEntity {
        feature,
        entity: circle,
    }]);
    harness.key(Key::M, Modifiers::NONE);
    harness.frame();
    harness.frame();

    let scale_grip = harness
        .workspace
        .viewport
        .sketch_handle_position(SketchGrip::Scale, 0.0)
        .expect("scale grip should be shown for a circle");

    harness.events.push(Event::PointerMoved(scale_grip));
    harness.frame();
    harness.events.push(Event::PointerButton {
        pos: scale_grip,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    harness.events.push(Event::PointerButton {
        pos: scale_grip,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    harness.frame();
    harness.type_text("2");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.wait_until("the typed scale commits", |harness| {
        harness
            .sketch(feature)
            .circle(circle)
            .is_some_and(|(_, radius)| (radius - 10.0).abs() < DRAWN)
    });
    harness.settle();

    let (centre, radius) = harness.sketch(feature).circle(circle).unwrap();

    assert!(centre.distance(Point2::new(40.0, 0.0)) < DRAWN);
    assert!((radius - 10.0).abs() < DRAWN);
}

#[test]
fn move_refuses_projected_geometry_in_words_until_it_is_kept_as_ordinary_curves() {
    let mut harness = Harness::new();

    let (_, top) = extruded_plate(&mut harness);
    let sketch = harness.draw_on_new_sketch();

    harness.use_tool_with(Key::P, Modifiers::ALT);
    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), top);
    harness.settle();
    harness.use_tool(Key::Escape);

    let projected: Vec<Pickable> = harness
        .sketch(sketch)
        .projected()
        .map(|entity| Pickable::SketchEntity {
            feature: sketch,
            entity,
        })
        .collect();
    let before = harness.sketch(sketch).clone();

    harness.select(projected);
    harness.frame();
    harness.key(Key::M, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert!(harness.shows_containing("keep the selected projections as ordinary curves to move"));
    assert!(!harness.workspace.viewport.has_sketch_handles());
    assert_eq!(harness.sketch(sketch), &before);

    run_from_palette(&mut harness, "keep the selected projections");
    harness.settle();
    harness.key(Key::M, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert_eq!(harness.sketch(sketch).projected().count(), 0);
    assert!(harness.workspace.viewport.has_sketch_handles());
}
