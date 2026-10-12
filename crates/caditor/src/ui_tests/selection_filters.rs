use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;
use egui::{Event, Key, Modifiers, PointerButton};

use super::{
    CAMERA_SETTLE, Harness, drag_screen, extruded_plate, is_edge, is_face, plate_on_screen,
    run_from_palette, selected_kinds, selected_of,
};
use crate::selection::{Pickable, SelectionFilter};

#[test]
fn a_selection_filter_makes_clicks_skip_everything_but_one_kind() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    let edge = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| matches!(pickable, Pickable::Edge { .. }))
        .expect("an edge is pickable");
    harness.select([]);

    run_from_palette(&mut harness, "select edges only");
    harness.frame();
    assert_eq!(harness.workspace.viewport.filter(), SelectionFilter::Edges);
    assert!(harness.shows("Selecting edges only"));

    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), top);
    assert!(harness.workspace.viewport.selection().is_empty());

    harness.click_pickable(Plane::XY, Point2::new(0.0, 0.0), edge);
    assert!(harness.workspace.viewport.selection().contains(edge));

    harness.click("Selecting edges only");
    harness.frame();
    assert_eq!(
        harness.workspace.viewport.filter(),
        SelectionFilter::Everything
    );
    assert!(!harness.shows("Selecting edges only"));

    harness.select([]);
    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), top);
    assert!(harness.workspace.viewport.selection().contains(top));
}

#[test]
fn the_bodies_filter_selects_a_whole_body_by_a_click_or_a_box_and_the_priority_cycles() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    harness.select([]);

    run_from_palette(&mut harness, "select whole bodies only");
    harness.frame();
    assert_eq!(harness.workspace.viewport.filter(), SelectionFilter::Bodies);
    assert!(harness.shows("Selecting whole bodies only"));

    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), top);
    assert_eq!(selected_kinds(&harness), (6, 0, 0));

    let position = harness.hover_pickable(Plane::XY, Point2::new(10.0, 10.0), top);
    harness
        .events
        .push(Event::ModifiersChanged(Modifiers::SHIFT));
    harness.frame();
    for pressed in [true, false] {
        harness.events.push(Event::PointerButton {
            pos: position,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::SHIFT,
        });
        harness.frame();
    }
    harness
        .events
        .push(Event::ModifiersChanged(Modifiers::NONE));
    harness.frame();
    assert_eq!(selected_kinds(&harness), (0, 0, 0));

    run_from_palette(&mut harness, "cycle the selection priority");
    harness.frame();
    assert_eq!(harness.workspace.viewport.filter(), SelectionFilter::Faces);
    run_from_palette(&mut harness, "cycle the selection priority");
    harness.frame();
    assert_eq!(harness.workspace.viewport.filter(), SelectionFilter::Edges);
    run_from_palette(&mut harness, "cycle the selection priority");
    harness.frame();
    assert_eq!(harness.workspace.viewport.filter(), SelectionFilter::Bodies);

    run_from_palette(&mut harness, "fit view");
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.frame();
    let (low, high) = plate_on_screen(&harness);
    let margin = egui::vec2(12.0, 12.0);
    drag_screen(&mut harness, low - margin, high + margin);
    assert_eq!(selected_kinds(&harness), (6, 0, 0));
}

#[test]
fn select_all_takes_every_face_or_edge_of_the_shown_bodies_by_the_selection_filter() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    harness.select([top]);

    harness
        .workspace
        .viewport
        .set_filter(SelectionFilter::Faces);
    harness.key(Key::A, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.frame();

    assert_eq!(selected_of(&harness, is_face), 6);
    assert_eq!(harness.workspace.viewport.selection().iter().count(), 6);

    harness
        .workspace
        .viewport
        .set_filter(SelectionFilter::Edges);
    harness.key(Key::A, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.frame();

    assert_eq!(selected_of(&harness, is_edge), 12);
    assert_eq!(harness.workspace.viewport.selection().iter().count(), 12);
}

#[test]
fn select_all_without_a_kind_to_take_says_what_to_choose() {
    let mut harness = Harness::new();
    let (_, _) = extruded_plate(&mut harness);
    harness.select([]);

    harness.key(Key::A, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.frame();

    harness.frame();

    assert!(harness.workspace.viewport.selection().is_empty());
    assert!(harness.shows_containing(crate::body_selection::NO_KIND_TO_SELECT));
}

#[test]
fn the_edges_around_a_face_replace_the_selected_face() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    harness.select([top]);

    harness.key(Key::E, Modifiers::ALT | Modifiers::SHIFT);
    harness.frame();

    assert_eq!(selected_of(&harness, is_edge), 4);
    assert_eq!(selected_of(&harness, is_face), 0);
}

#[test]
fn tangent_edges_join_the_selection_along_smooth_joins_and_a_corner_says_there_are_none() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_line(Point2::new(0.0, 0.0), Point2::new(20.0, 0.0));
    sketch.add_arc(
        Point2::new(20.0, 5.0),
        Point2::new(20.0, 0.0),
        Point2::new(20.0, 10.0),
    );
    sketch.add_line(Point2::new(20.0, 10.0), Point2::new(0.0, 10.0));
    sketch.add_arc(
        Point2::new(0.0, 5.0),
        Point2::new(0.0, 10.0),
        Point2::new(0.0, 0.0),
    );
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let edges: Vec<Pickable> = harness.built().picks.pickables().filter(is_edge).collect();
    let rim: Vec<Pickable> = edges
        .iter()
        .copied()
        .filter(|edge| {
            edge.describe(harness.document(), harness.model.evaluation())
                .contains("end face")
        })
        .collect();
    assert_eq!(rim.len(), 4);

    harness.select([rim[0]]);
    harness.key(Key::T, Modifiers::ALT);
    harness.frame();

    assert_eq!(selected_of(&harness, is_edge), 4);

    harness.key(Key::T, Modifiers::ALT);
    harness.frame();

    harness.frame();

    assert!(harness.shows_containing(crate::body_selection::NO_TANGENT_EDGES));
}

#[test]
fn tangent_faces_join_the_selection_around_rounded_ends_and_a_flat_end_says_there_are_none() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_line(Point2::new(0.0, 0.0), Point2::new(20.0, 0.0));
    sketch.add_arc(
        Point2::new(20.0, 5.0),
        Point2::new(20.0, 0.0),
        Point2::new(20.0, 10.0),
    );
    sketch.add_line(Point2::new(20.0, 10.0), Point2::new(0.0, 10.0));
    sketch.add_arc(
        Point2::new(0.0, 5.0),
        Point2::new(0.0, 10.0),
        Point2::new(0.0, 0.0),
    );
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let faces: Vec<Pickable> = harness.built().picks.pickables().filter(is_face).collect();
    let describe = |face: &Pickable| face.describe(harness.document(), harness.model.evaluation());
    let side = faces
        .iter()
        .copied()
        .find(|face| !describe(face).contains("end face"))
        .unwrap();
    let end = faces
        .iter()
        .copied()
        .find(|face| describe(face).contains("end face"))
        .unwrap();

    harness.select([side]);
    harness.key(Key::T, Modifiers::ALT | Modifiers::SHIFT);
    harness.frame();
    let around = selected_of(&harness, is_face);
    harness.select([end]);
    harness.key(Key::T, Modifiers::ALT | Modifiers::SHIFT);
    harness.frame();
    harness.frame();

    assert_eq!(faces.len(), 6);
    assert_eq!(around, 4);
    assert_eq!(selected_of(&harness, is_face), 1);
    assert!(harness.shows_containing(crate::body_selection::NO_TANGENT_FACES));
}
