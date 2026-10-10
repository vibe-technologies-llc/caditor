use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Constraint, Entity, EntityId, Sketch};
use egui::{Key, Modifiers};

use super::{Harness, constraints_of_kind, entities_of_kind, extruded_plate, line_ends};

fn drawn_points(sketch: &Sketch) -> Vec<EntityId> {
    entities_of_kind(sketch, "Point")
        .into_iter()
        .filter(|point| !sketch.is_projected(*point))
        .collect()
}

#[test]
fn a_line_drawn_from_a_body_corner_to_an_edge_middle_projects_both_in_one_undoable_step() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);

    harness.point_at(Point2::new(40.3, 0.2));
    assert!(harness.shows("Corner of Extrude 1"));
    harness.click_at(Point2::new(40.3, 0.2));
    harness.point_at(Point2::new(40.2, 20.3));
    assert!(harness.shows_containing("Middle of an edge of Extrude 1"));
    harness.click_at(Point2::new(40.2, 20.3));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.settle();

    let sketch = harness.sketch(feature);
    let line = *entities_of_kind(sketch, "Line")
        .iter()
        .find(|line| !sketch.is_projected(**line))
        .expect("the drawn line");
    let (start, end) = line_ends(sketch, line);
    let corner = sketch
        .projected()
        .find(|id| matches!(sketch.entity(*id), Some(Entity::Point(at)) if at.distance(Point2::new(40.0, 0.0)) < 1e-9))
        .expect("the corner is projected");
    let edge = sketch
        .projected()
        .find(|id| matches!(sketch.entity(*id), Some(Entity::Line { .. })))
        .expect("the edge is projected");

    assert_eq!(sketch.projected().count(), 4);
    assert!(
        constraints_of_kind(sketch, "Coincident").contains(&Constraint::Coincident(start, corner))
    );
    assert_eq!(
        constraints_of_kind(sketch, "Midpoint"),
        vec![Constraint::Midpoint {
            point: end,
            curve: edge
        }]
    );
    assert_eq!(harness.model.undo_label(), Some("Draw line"));

    let shown = harness.shown(feature);
    let (from, to) = shown.line_endpoints(line).unwrap();
    assert!(from.distance(Point2::new(40.0, 0.0)) < 1e-6, "{from}");
    assert!(to.distance(Point2::new(40.0, 20.0)) < 1e-6, "{to}");

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    let sketch = harness.sketch(feature);
    assert_eq!(sketch.projected().count(), 0);
    assert!(entities_of_kind(sketch, "Line").is_empty());
}

#[test]
fn a_point_placed_on_a_body_edge_stays_on_its_projection() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::P);

    harness.point_at(Point2::new(40.2, 9.0));
    assert!(harness.shows("On an edge of Extrude 1"));
    harness.click_at(Point2::new(40.2, 9.0));
    harness.settle();

    let sketch = harness.sketch(feature);
    let point = *drawn_points(sketch).first().expect("the placed point");
    let edge = sketch
        .projected()
        .find(|id| matches!(sketch.entity(*id), Some(Entity::Line { .. })))
        .expect("the edge is projected");
    assert_eq!(
        constraints_of_kind(sketch, "Coincident"),
        vec![Constraint::Coincident(point, edge)]
    );
    let placed = harness.shown(feature).point(point).unwrap();
    assert!((placed.x - 40.0).abs() < 1e-6, "{placed}");
}

#[test]
fn a_circle_centred_on_a_round_edge_is_held_at_its_projected_centre() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_circle(Point2::new(60.0, 0.0), 10.0);
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click_tool("Extrude");
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::C);

    harness.point_at(Point2::new(60.3, 0.2));
    assert!(harness.shows("Centre of a round edge of Extrude 1"));
    harness.click_at(Point2::new(60.3, 0.2));
    harness.click_at(Point2::new(65.0, 0.0));
    harness.settle();

    let sketch = harness.sketch(feature);
    let circle = *entities_of_kind(sketch, "Circle")
        .iter()
        .find(|circle| !sketch.is_projected(**circle))
        .expect("the drawn circle");
    let Some(Entity::Circle { center, .. }) = sketch.entity(circle) else {
        panic!("expected a circle");
    };
    let projected_centre = sketch
        .projected()
        .find_map(|id| match sketch.entity(id) {
            Some(Entity::Circle { center, .. }) => Some(*center),
            _ => None,
        })
        .expect("the round edge is projected");
    assert!(
        constraints_of_kind(sketch, "Coincident")
            .contains(&Constraint::Coincident(*center, projected_centre))
    );
    assert_eq!(sketch.projected().count(), 2);
}
