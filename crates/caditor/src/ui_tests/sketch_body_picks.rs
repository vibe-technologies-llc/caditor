use std::collections::BTreeSet;

use caditor_document::FeatureId;
use caditor_expression::{Expression, Unit};
use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Constraint, Entity, EntityId, Sketch};
use egui::{Event, Key, Modifiers, PointerButton};

use super::{
    FRAME_SECONDS, Harness, constraints_of_kind, drag_in_sketch, edit_free_sketch, extruded_plate,
    sketch_entity,
};
use crate::{
    body_snap::BodyItem, canvas_guidance::HOVER_DELAY, editing::Tool, model::Action,
    selection::Pickable,
};

const MOST_HIGHLIGHT_STEPS: usize = 80;

fn plate_and_point(harness: &mut Harness) -> (FeatureId, EntityId) {
    extruded_plate(harness);
    let mut sketch = Sketch::new(Plane::XY);
    let point = sketch.add_point(Point2::new(60.0, 10.0));
    let feature = edit_free_sketch(harness, sketch);
    (feature, point)
}

fn rest_pointer(harness: &mut Harness) {
    let delay_frames = (HOVER_DELAY / FRAME_SECONDS).ceil() as usize + 1;
    for _ in 0..delay_frames {
        harness.frame();
    }
}

fn projected_line(sketch: &Sketch) -> EntityId {
    sketch
        .projected()
        .find(|id| matches!(sketch.entity(*id), Some(Entity::Line { .. })))
        .expect("the edge is projected")
}

fn projected_corner(sketch: &Sketch, at: Point2) -> EntityId {
    sketch
        .projected()
        .find(|id| {
            matches!(sketch.entity(*id), Some(Entity::Point(position)) if position.distance(at) < 1e-9)
        })
        .expect("the corner is projected")
}

fn highlight_until(harness: &mut Harness, wanted: impl Fn(Pickable) -> bool) -> Pickable {
    for _ in 0..MOST_HIGHLIGHT_STEPS {
        if let Some(highlight) = harness.workspace.viewport.keyboard_highlight()
            && wanted(highlight)
        {
            return highlight;
        }
        harness.key(Key::N, Modifiers::NONE);
        harness.frame();
    }
    panic!("the highlight keys never reached what was wanted");
}

fn activate(harness: &mut Harness) {
    harness.key(Key::Space, Modifiers::NONE);
    harness.frame();
    harness.frame();
}

fn distance_to_line(point: Point2, (start, end): (Point2, Point2)) -> f64 {
    let along = (end - start).normalize();
    let offset = point - start;
    (offset.x * along.y - offset.y * along.x).abs()
}

#[test]
fn the_highlight_keys_reach_a_body_edge_and_smart_dimension_measures_from_it_without_a_pointer() {
    let mut harness = Harness::new();
    let (feature, point) = plate_and_point(&mut harness);
    harness.use_tool(Key::D);
    let own_point = sketch_entity(feature, point);

    let edge = highlight_until(&mut harness, |pickable| {
        matches!(
            pickable,
            Pickable::BodyItem {
                item: BodyItem::Edge(_),
                ..
            }
        )
    });
    assert!(harness.shows_containing("Edge of Extrude 1"));
    activate(&mut harness);
    assert!(harness.workspace.viewport.selection().contains(edge));
    highlight_until(&mut harness, |pickable| pickable == own_point);
    activate(&mut harness);
    harness.settle();

    let sketch = harness.sketch(feature);
    let projected = projected_line(sketch);
    let ends = sketch.line_endpoints(projected).unwrap();
    let expected = distance_to_line(Point2::new(60.0, 10.0), ends);
    let [Constraint::Distance { from, to, value }] = &constraints_of_kind(sketch, "Distance")[..]
    else {
        panic!("one distance should be added");
    };

    assert_eq!(sketch.projected().count(), 3);
    assert_eq!(
        BTreeSet::from([*from, *to]),
        BTreeSet::from([point, projected])
    );
    assert!(
        matches!(value, Expression::Measure(measured, Unit::Millimetre) if (measured - expected).abs() < 1e-6),
        "{value:?} against {expected}"
    );
    assert_eq!(harness.model.undo_label(), Some("Add Distance"));
}

#[test]
fn a_body_corner_highlighted_and_chosen_from_the_keyboard_takes_the_coincident_tool() {
    let mut harness = Harness::new();
    let (feature, point) = plate_and_point(&mut harness);
    let own_point = sketch_entity(feature, point);

    let corner = highlight_until(&mut harness, |pickable| {
        matches!(
            pickable,
            Pickable::BodyItem {
                item: BodyItem::Corner(_),
                ..
            }
        )
    });
    assert!(harness.shows_containing("Corner of Extrude 1"));
    activate(&mut harness);
    highlight_until(&mut harness, |pickable| pickable == own_point);
    activate(&mut harness);
    assert!(harness.workspace.viewport.selection().contains(corner));
    assert!(harness.workspace.viewport.selection().contains(own_point));
    harness.key(Key::C, Modifiers::SHIFT);
    harness.frame();
    harness.settle();

    let sketch = harness.sketch(feature);
    let [Constraint::Coincident(joined, projected)] = constraints_of_kind(sketch, "Coincident")[..]
    else {
        panic!("one coincident should be added");
    };

    assert_eq!(sketch.projected().count(), 1);
    assert_eq!(joined, point);
    assert!(sketch.is_projected(projected));

    harness.perform(Action::Undo);
    harness.settle();
    assert_eq!(harness.sketch(feature).projected().count(), 0);
}

#[test]
fn the_smart_dimension_measures_a_point_from_a_body_edge_projecting_it_in_the_same_change() {
    let mut harness = Harness::new();
    let (feature, point) = plate_and_point(&mut harness);
    harness.use_tool(Key::D);
    assert_eq!(harness.tool(), Some(Tool::Dimension));

    harness.point_at(Point2::new(40.2, 20.0));
    assert!(harness.shows_containing("Edge of Extrude 1"));
    harness.click_at(Point2::new(40.2, 20.0));
    assert!(
        harness
            .workspace
            .viewport
            .selection()
            .iter()
            .any(|pickable| matches!(pickable, Pickable::BodyItem { .. }))
    );
    assert_eq!(harness.sketch(feature).projected().count(), 0);

    harness.click_pickable(
        Plane::XY,
        Point2::new(60.0, 10.0),
        sketch_entity(feature, point),
    );
    harness.frame();
    harness.frame();

    let sketch = harness.sketch(feature);
    let edge = projected_line(sketch);
    assert_eq!(sketch.projected().count(), 3);
    assert_eq!(
        constraints_of_kind(sketch, "Distance"),
        vec![Constraint::Distance {
            from: point,
            to: edge,
            value: Expression::Measure(20.0, Unit::Millimetre),
        }]
    );
    assert_eq!(harness.model.undo_label(), Some("Add Distance"));
    assert!(harness.workspace.viewport.selection().is_empty());

    harness.perform(Action::Undo);
    harness.settle();
    let sketch = harness.sketch(feature);
    assert_eq!(sketch.projected().count(), 0);
    assert_eq!(sketch.constraints().count(), 0);
}

#[test]
fn a_body_corner_selected_with_a_point_takes_the_coincident_tool_projecting_it_once() {
    let mut harness = Harness::new();
    let (feature, point) = plate_and_point(&mut harness);

    harness.point_at(Point2::new(40.3, 0.2));
    rest_pointer(&mut harness);
    assert!(harness.shows_containing("Corner of Extrude 1"));
    let position = harness.on_screen(Point2::new(40.3, 0.2));
    harness
        .events
        .push(Event::ModifiersChanged(Modifiers::SHIFT));
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
    harness
        .workspace
        .viewport
        .selection_mut()
        .extend([sketch_entity(feature, point)]);
    harness.frame();
    assert_eq!(harness.workspace.viewport.selection().len(), 2);

    harness.click_button("Coincident");
    harness.settle();

    let sketch = harness.sketch(feature);
    let corner = projected_corner(sketch, Point2::new(40.0, 0.0));
    assert_eq!(sketch.projected().count(), 1);
    assert_eq!(
        constraints_of_kind(sketch, "Coincident"),
        vec![Constraint::Coincident(point, corner)]
    );
    let moved = harness.shown(feature).point(point).unwrap();
    assert!(moved.distance(Point2::new(40.0, 0.0)) < 1e-6, "{moved}");

    harness.perform(Action::Undo);
    harness.settle();
    assert_eq!(harness.sketch(feature).projected().count(), 0);
}

#[test]
fn a_point_dragged_onto_a_body_corner_lands_on_it_and_joins_its_projection() {
    let mut harness = Harness::new();
    let (feature, point) = plate_and_point(&mut harness);
    let before = harness.sketch(feature).clone();

    let grabbed = harness.hover_pickable(
        Plane::XY,
        Point2::new(60.0, 10.0),
        sketch_entity(feature, point),
    );
    drag_in_sketch(&mut harness, grabbed, Point2::new(40.2, 0.3));
    harness.wait_until("the drag is committed", |harness| {
        harness.sketch(feature).point(point) != before.point(point)
    });
    harness.settle();

    let sketch = harness.sketch(feature);
    let corner = projected_corner(sketch, Point2::new(40.0, 0.0));
    assert_eq!(
        constraints_of_kind(sketch, "Coincident"),
        vec![Constraint::Coincident(point, corner)]
    );
    assert!(
        sketch
            .point(point)
            .unwrap()
            .distance(Point2::new(40.0, 0.0))
            < 1e-6
    );
    assert_eq!(
        harness.model.undo_label(),
        Some(format!("Drag {}", before.entity_label(point)).as_str())
    );

    harness.perform(Action::Undo);
    harness.settle();
    assert_eq!(harness.sketch(feature).projected().count(), 0);
}
