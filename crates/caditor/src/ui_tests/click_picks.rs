use caditor_geometry::{Plane, Point2, Point3, Vector3};
use egui::Event;

use super::{Harness, extruded_plate};

#[test]
fn a_click_waits_for_the_pick_under_the_cursor_rather_than_using_an_old_one() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    harness.select([]);
    harness.hover_pickable(Plane::XY, Point2::new(20.0, 20.0), top);
    harness.frame();

    harness.picks_held = true;
    let beside = harness
        .workspace
        .viewport
        .screen_position(Plane::XY, Point2::new(-30.0, 20.0))
        .unwrap();
    harness.events.push(Event::PointerMoved(beside));
    harness.press(beside);
    harness.frame();
    assert!(harness.workspace.viewport.selection().is_empty());

    harness.picks_held = false;
    harness.frame();
    harness.frame();
    assert!(harness.workspace.viewport.selection().is_empty());

    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), top);
    assert!(harness.workspace.viewport.selection().contains(top));
    harness.picks_held = true;
    harness.events.push(Event::PointerMoved(beside));
    harness.press(beside);
    harness.picks_held = false;
    harness.frame();
    harness.frame();
    assert!(harness.workspace.viewport.selection().is_empty());
}

#[test]
fn a_click_waiting_for_its_pick_acts_on_what_was_clicked_when_the_pointer_moves_on_or_leaves() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    let face = Plane::from_frame(Point3::new(0.0, 0.0, 10.0), Vector3::Z, Vector3::X).unwrap();
    let on_top = harness
        .workspace
        .viewport
        .screen_position(face, Point2::new(20.0, 20.0))
        .unwrap();
    let beside = harness
        .workspace
        .viewport
        .screen_position(Plane::XY, Point2::new(-30.0, 20.0))
        .unwrap();
    let outside = harness.workspace.viewport.rect().unwrap().left_top() - egui::vec2(10.0, 10.0);

    for away in [beside, outside] {
        harness.select([]);
        harness.events.push(Event::PointerMoved(beside));
        harness.frame();
        harness.picks_held = true;
        harness.events.push(Event::PointerMoved(on_top));
        harness.press(on_top);
        let waiting = harness.workspace.viewport.selection().is_empty();
        harness.events.push(Event::PointerMoved(away));
        harness.frame();
        harness.picks_held = false;
        harness.frame();
        harness.frame();

        assert!(waiting);
        assert!(harness.workspace.viewport.selection().contains(top));
    }
}
