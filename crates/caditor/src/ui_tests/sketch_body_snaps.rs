use caditor_document::{Document, FeatureId};
use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Constraint, Entity, EntityId, PointTarget, Sketch};
use egui::{Event, Key, Modifiers, PointerButton, Pos2};

use super::{
    CAMERA_SETTLE, Harness, aligned_point_pairs, constraints_of_kind, drag_in_sketch,
    edit_free_sketch, entities_of_kind, extruded_plate, line_ends, pickable_described, rectangle,
    release_drag, run_from_palette, sketch_entity,
};

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

#[test]
fn a_circle_on_a_new_cylinder_s_top_face_snaps_to_its_edge_and_its_centre() {
    let mut harness = Harness::starting(None, Document::default(), crate::app::Workspace::new());

    harness.select([]);
    run_from_palette(&mut harness, "Cylinder");
    harness.settle();

    let top = pickable_described(&mut harness, "Cylinder 1 › Cylinder 1 top face");

    harness.click_pickable(Plane::XY, Point2::new(0.0, 0.0), top);
    harness.settle();

    assert_eq!(harness.workspace.editing.picking(), None);
    assert!(harness.workspace.viewport.selection().contains(top));

    harness.click("New sketch");
    harness.settle();

    let sketch = harness.editing().expect("the sketch on the top face");

    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.use_tool(Key::C);

    harness.point_at(Point2::new(5.3, 5.2));

    assert!(harness.shows("Centre of a round edge of Cylinder 1"));

    harness.click_at(Point2::new(5.3, 5.2));
    harness.point_at(Point2::new(8.5, 5.0));
    harness.click_at(Point2::new(8.5, 5.0));
    harness.settle();

    let sketch = harness.sketch(sketch);
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
}

fn extruded_block(harness: &mut Harness, far_corner: Point2) {
    let mut sketch = Sketch::new(Plane::XY);

    rectangle(&mut sketch, Point2::ZERO, far_corner);
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click_tool("Extrude");
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
}

fn millimetres_per_point(harness: &Harness, at: Point2) -> f64 {
    let one_millimetre = harness
        .on_screen(at + Point2::new(1.0, 0.0))
        .distance(harness.on_screen(at));
    1.0 / f64::from(one_millimetre)
}

#[test]
fn holding_alt_pulls_a_point_onto_a_body_corner_beyond_the_ordinary_reach() {
    let mut harness = Harness::new();
    let corner = Point2::new(45.0, 25.0);

    extruded_block(&mut harness, corner);

    let feature = harness.draw_on_new_sketch();

    harness.use_tool(Key::P);

    let along = 14.0 * millimetres_per_point(&harness, corner) / 2.0_f64.sqrt();
    let pointer = corner + Point2::splat(along);

    harness.point_at(pointer);

    assert!(!harness.shows("Corner of Extrude 1"));

    harness.hold(Modifiers::ALT);
    harness.point_at(pointer);

    assert!(harness.shows("Corner of Extrude 1"));

    harness.click_at(pointer);
    harness.hold(Modifiers::NONE);
    harness.settle();

    let sketch = harness.sketch(feature);
    let point = *drawn_points(sketch).first().expect("the placed point");
    let projected = sketch
        .projected()
        .find(|id| matches!(sketch.entity(*id), Some(Entity::Point(at)) if at.distance(corner) < 1e-9))
        .expect("the corner is projected");
    let placed = harness.shown(feature).point(point).unwrap();

    assert_eq!(
        constraints_of_kind(sketch, "Coincident"),
        vec![Constraint::Coincident(point, projected)]
    );
    assert!(placed.distance(corner) < 1e-6, "{placed}");
}

#[test]
fn holding_alt_pulls_a_point_onto_a_body_edge_beyond_the_ordinary_reach() {
    let mut harness = Harness::new();

    extruded_block(&mut harness, Point2::new(45.0, 25.0));

    let feature = harness.draw_on_new_sketch();

    harness.use_tool(Key::P);

    let on_edge = Point2::new(45.0, 20.0);
    let pointer = on_edge + Point2::new(12.0 * millimetres_per_point(&harness, on_edge), 0.0);

    harness.point_at(pointer);

    assert!(!harness.shows("On an edge of Extrude 1"));

    harness.hold(Modifiers::ALT);
    harness.point_at(pointer);

    assert!(harness.shows("On an edge of Extrude 1"));

    harness.click_at(pointer);
    harness.hold(Modifiers::NONE);
    harness.settle();

    let sketch = harness.sketch(feature);
    let point = *drawn_points(sketch).first().expect("the placed point");
    let edge = sketch
        .projected()
        .find(|id| matches!(sketch.entity(*id), Some(Entity::Line { .. })))
        .expect("the edge is projected");
    let placed = harness.shown(feature).point(point).unwrap();

    assert_eq!(
        constraints_of_kind(sketch, "Coincident"),
        vec![Constraint::Coincident(point, edge)]
    );
    assert!((placed.x - 45.0).abs() < 1e-6, "{placed}");
}

#[test]
fn a_point_dragged_with_alt_held_is_pulled_onto_a_body_corner_and_joins_its_projection() {
    let mut harness = Harness::new();
    let corner = Point2::new(45.0, 25.0);

    extruded_block(&mut harness, corner);

    let mut sketch = Sketch::new(Plane::XY);

    let point = sketch.add_point(Point2::new(70.0, 40.0));
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = harness.sketch(feature).clone();
    let along = 14.0 * millimetres_per_point(&harness, corner) / 2.0_f64.sqrt();

    let grabbed = harness.hover_pickable(
        Plane::XY,
        Point2::new(70.0, 40.0),
        sketch_entity(feature, point),
    );

    harness.hold(Modifiers::ALT);
    drag_in_sketch(&mut harness, grabbed, corner + Point2::splat(along));
    harness.hold(Modifiers::NONE);
    harness.wait_until("the drag is committed", |harness| {
        harness.sketch(feature).point(point) != before.point(point)
    });
    harness.settle();

    let sketch = harness.sketch(feature);
    let projected = sketch
        .projected()
        .find(|id| matches!(sketch.entity(*id), Some(Entity::Point(at)) if at.distance(corner) < 1e-9))
        .expect("the corner is projected");
    let moved = sketch.point(point).unwrap();

    assert_eq!(
        constraints_of_kind(sketch, "Coincident"),
        vec![Constraint::Coincident(point, projected)]
    );
    assert!(moved.distance(corner) < 1e-6, "{moved}");
}

#[test]
fn a_circle_drawn_from_its_centre_snaps_tangent_to_a_body_edge_and_projects_it() {
    let mut harness = Harness::new();

    extruded_plate(&mut harness);

    let feature = harness.draw_on_new_sketch();

    harness.use_tool(Key::C);
    harness.click_at(Point2::new(25.0, 20.0));
    harness.point_at(Point2::new(40.1, 12.0));

    assert!(harness.shows("Tangent to an edge of Extrude 1"));

    harness.click_at(Point2::new(40.1, 12.0));
    harness.settle();

    let sketch = harness.sketch(feature);
    let circle = *entities_of_kind(sketch, "Circle")
        .iter()
        .find(|id| !sketch.is_projected(**id))
        .expect("a tangent circle should be drawn");
    let edge = sketch
        .projected()
        .find(|id| matches!(sketch.entity(*id), Some(Entity::Line { .. })))
        .expect("the body edge should be projected");

    let (center, radius) = sketch.circle(circle).unwrap();

    assert!(center.distance(Point2::new(25.0, 20.0)) < 1e-3, "{center}");
    assert!((radius - 15.0).abs() < 1e-3, "{radius}");
    assert!(constraints_of_kind(sketch, "Tangent").contains(&Constraint::Tangent(circle, edge)));
}

fn plate_beside_a_cylinder(harness: &mut Harness) {
    extruded_plate(harness);

    let mut sketch = Sketch::new(Plane::XY);

    sketch.add_circle(Point2::new(60.0, 30.0), 5.0);
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click_tool("Extrude");
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
}

fn projected_point_at(sketch: &Sketch, at: Point2) -> EntityId {
    sketch
        .projected()
        .find(|id| matches!(sketch.entity(*id), Some(Entity::Point(point)) if point.distance(at) < 1e-9))
        .expect("the corner is projected")
}

fn projected_centre(sketch: &Sketch) -> EntityId {
    sketch
        .projected()
        .find_map(|id| match sketch.entity(id) {
            Some(Entity::Circle { center, .. }) => Some(*center),
            _ => None,
        })
        .expect("the round edge is projected")
}

fn projected_line(sketch: &Sketch) -> EntityId {
    sketch
        .projected()
        .find(|id| matches!(sketch.entity(*id), Some(Entity::Line { .. })))
        .expect("the edge is projected")
}

fn placed_point(harness: &mut Harness, feature: FeatureId, at: Point2) -> (EntityId, Point2) {
    harness.click_at(at);
    harness.settle();

    let point = *drawn_points(harness.sketch(feature))
        .last()
        .expect("the placed point");
    (point, harness.shown(feature).point(point).unwrap())
}

#[test]
fn a_point_level_with_a_hovered_body_corner_is_held_level_with_its_projection_in_one_undo() {
    let mut harness = Harness::new();

    extruded_plate(&mut harness);

    let feature = harness.draw_on_new_sketch();

    harness.use_tool(Key::P);
    harness.point_at(Point2::new(40.2, 40.2));
    harness.point_at(Point2::new(60.0, 40.3));

    assert!(harness.shows("Horizontal from a corner of Extrude 1"));

    let (point, placed) = placed_point(&mut harness, feature, Point2::new(60.0, 40.3));
    let sketch = harness.sketch(feature);

    let corner = projected_point_at(sketch, Point2::new(40.0, 40.0));

    assert_eq!(
        aligned_point_pairs(sketch),
        vec![Constraint::HorizontalPoints(point, corner.into())]
    );
    assert_eq!(sketch.projected().count(), 1);
    assert!((placed.y - 40.0).abs() < 1e-6, "{placed}");
    assert_eq!(harness.model.undo_label(), Some("Draw point"));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();

    let sketch = harness.sketch(feature);

    assert_eq!(sketch.projected().count(), 0);
    assert!(drawn_points(sketch).is_empty());
}

#[test]
fn a_point_level_with_a_hovered_body_edge_middle_is_held_level_with_the_projected_edge_middle() {
    let mut harness = Harness::new();

    extruded_plate(&mut harness);

    let feature = harness.draw_on_new_sketch();

    harness.use_tool(Key::P);
    harness.point_at(Point2::new(40.2, 20.2));
    harness.point_at(Point2::new(60.0, 20.3));

    assert!(harness.shows("Horizontal from the middle of an edge of Extrude 1"));

    let (point, placed) = placed_point(&mut harness, feature, Point2::new(60.0, 20.3));
    let sketch = harness.sketch(feature);

    let edge = projected_line(sketch);

    assert_eq!(
        aligned_point_pairs(sketch),
        vec![Constraint::HorizontalPoints(
            point,
            PointTarget::Middle(edge)
        )]
    );
    assert!((placed.y - 20.0).abs() < 1e-6, "{placed}");
}

#[test]
fn a_point_under_a_hovered_round_edge_centre_is_held_under_the_projected_centre() {
    let mut harness = Harness::new();

    plate_beside_a_cylinder(&mut harness);

    let feature = harness.draw_on_new_sketch();

    harness.use_tool(Key::P);
    harness.point_at(Point2::new(60.2, 30.2));

    assert!(harness.shows_containing("Centre of a round edge of Extrude"));

    harness.point_at(Point2::new(60.3, 10.0));

    assert!(harness.shows_containing("Vertical from the centre of a round edge of Extrude"));

    let (point, placed) = placed_point(&mut harness, feature, Point2::new(60.3, 10.0));
    let sketch = harness.sketch(feature);

    let centre = projected_centre(sketch);

    assert_eq!(
        aligned_point_pairs(sketch),
        vec![Constraint::VerticalPoints(point, centre.into())]
    );
    assert!((placed.x - 60.0).abs() < 1e-6, "{placed}");
}

#[test]
fn a_point_where_tracks_from_two_body_items_cross_is_held_by_both_projections() {
    let mut harness = Harness::new();

    plate_beside_a_cylinder(&mut harness);

    let feature = harness.draw_on_new_sketch();

    harness.use_tool(Key::P);
    harness.point_at(Point2::new(40.2, 40.2));
    harness.point_at(Point2::new(60.2, 30.2));
    harness.point_at(Point2::new(60.3, 40.3));

    assert!(harness.shows_containing(
        "Horizontal from a corner of Extrude 1, vertical from the centre of a round edge of Extrude"
    ));

    let (point, placed) = placed_point(&mut harness, feature, Point2::new(60.3, 40.3));
    let sketch = harness.sketch(feature);

    let corner = projected_point_at(sketch, Point2::new(40.0, 40.0));
    let centre = projected_centre(sketch);

    assert_eq!(
        aligned_point_pairs(sketch),
        vec![
            Constraint::HorizontalPoints(point, corner.into()),
            Constraint::VerticalPoints(point, centre.into()),
        ]
    );
    assert!(placed.distance(Point2::new(60.0, 40.0)) < 1e-6, "{placed}");

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();

    assert_eq!(harness.sketch(feature).projected().count(), 0);
}

#[test]
fn a_track_from_a_body_item_crosses_a_body_edge_and_keeps_both() {
    let mut harness = Harness::new();

    plate_beside_a_cylinder(&mut harness);

    let feature = harness.draw_on_new_sketch();

    harness.use_tool(Key::P);
    harness.point_at(Point2::new(60.2, 30.2));
    harness.point_at(Point2::new(40.2, 30.4));

    assert!(harness.shows_containing(
        "On an edge of Extrude 1, horizontal from the centre of a round edge of Extrude"
    ));

    let (point, placed) = placed_point(&mut harness, feature, Point2::new(40.2, 30.4));
    let sketch = harness.sketch(feature);

    let edge = projected_line(sketch);
    let centre = projected_centre(sketch);

    assert_eq!(
        constraints_of_kind(sketch, "Coincident"),
        vec![Constraint::Coincident(point, edge)]
    );
    assert_eq!(
        aligned_point_pairs(sketch),
        vec![Constraint::HorizontalPoints(point, centre.into())]
    );
    assert!(placed.distance(Point2::new(40.0, 30.0)) < 1e-6, "{placed}");
}

#[test]
fn a_line_end_on_a_body_edge_keeps_the_direction_where_it_crosses_the_edge() {
    let mut harness = Harness::new();

    extruded_plate(&mut harness);

    let feature = harness.draw_on_new_sketch();

    harness.use_tool(Key::L);
    harness.click_at(Point2::new(55.0, 10.0));
    harness.point_at(Point2::new(40.2, 10.4));

    assert!(harness.shows("On an edge of Extrude 1, horizontal"));

    harness.click_at(Point2::new(40.2, 10.4));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.settle();

    let sketch = harness.sketch(feature);
    let line = *entities_of_kind(sketch, "Line")
        .iter()
        .find(|line| !sketch.is_projected(**line))
        .expect("the drawn line");
    let edge = projected_line(sketch);

    let (start, end) = line_ends(sketch, line);
    let shown = harness.shown(feature);

    let started = shown.point(start).unwrap();
    let reached = shown.point(end).unwrap();

    assert_eq!(
        constraints_of_kind(sketch, "Coincident"),
        vec![Constraint::Coincident(end, edge)]
    );
    assert_eq!(
        constraints_of_kind(sketch, "Horizontal"),
        vec![Constraint::Horizontal(line)]
    );
    assert!((reached.x - 40.0).abs() < 1e-6, "{reached}");
    assert!((reached.y - started.y).abs() < 1e-9, "{reached} {started}");
}

#[test]
fn a_body_corner_hovered_with_ctrl_held_or_snapping_off_is_not_acquired() {
    let mut harness = Harness::new();

    extruded_plate(&mut harness);

    let feature = harness.draw_on_new_sketch();

    harness.use_tool(Key::P);
    harness.hold(Modifiers::COMMAND);
    harness.point_at(Point2::new(40.2, 40.2));
    harness.point_at(Point2::new(60.0, 40.3));
    harness.hold(Modifiers::NONE);

    assert!(!harness.shows_containing("from a corner"));

    let (_, placed) = placed_point(&mut harness, feature, Point2::new(60.0, 40.3));

    assert!(aligned_point_pairs(harness.sketch(feature)).is_empty());
    assert_eq!(harness.sketch(feature).projected().count(), 0);
    assert!((placed.y - 40.3).abs() < 0.05, "{placed}");

    run_from_palette(&mut harness, "snapping");
    harness.frame();

    assert!(!harness.workspace.viewport.snapping());

    harness.point_at(Point2::new(40.2, 20.2));
    harness.point_at(Point2::new(70.0, 20.3));
    run_from_palette(&mut harness, "snapping");
    harness.frame();

    assert!(harness.workspace.viewport.snapping());
    assert!(!harness.shows_containing("from the middle"));

    placed_point(&mut harness, feature, Point2::new(70.0, 20.3));

    assert!(aligned_point_pairs(harness.sketch(feature)).is_empty());
    assert_eq!(harness.sketch(feature).projected().count(), 0);
}

fn grab_through(harness: &mut Harness, from: Pos2, stops: &[Point2]) {
    harness.events.push(Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    for stop in stops {
        let position = harness.on_screen(*stop);

        harness.events.push(Event::PointerMoved(position));
        harness.frame();
    }
}

fn released_at(harness: &mut Harness, feature: FeatureId, point: EntityId, at: Point2) -> Point2 {
    let before = harness.sketch(feature).point(point);
    let on_screen = harness.on_screen(at);

    release_drag(harness, on_screen);
    harness.wait_until("the drag is committed", |harness| {
        harness.sketch(feature).point(point) != before
    });
    harness.settle();
    harness.shown(feature).point(point).unwrap()
}

fn grabbable_point(harness: &mut Harness, at: Point2) -> (FeatureId, EntityId, Pos2) {
    let mut sketch = Sketch::new(Plane::XY);

    let point = sketch.add_point(at);
    let feature = edit_free_sketch(harness, sketch);

    harness.settle();

    let grabbed = harness.hover_pickable(Plane::XY, at, sketch_entity(feature, point));
    (feature, point, grabbed)
}

#[test]
fn a_point_grabbed_level_with_a_hovered_body_corner_is_held_level_with_its_projection_in_one_undo()
{
    let mut harness = Harness::new();
    let start = Point2::new(70.0, 60.0);

    extruded_plate(&mut harness);

    let (feature, point, grabbed) = grabbable_point(&mut harness, start);

    grab_through(
        &mut harness,
        grabbed,
        &[Point2::new(40.2, 40.2), Point2::new(60.0, 40.3)],
    );

    assert!(harness.shows("Horizontal from a corner of Extrude 1"));

    let placed = released_at(&mut harness, feature, point, Point2::new(60.0, 40.3));
    let sketch = harness.sketch(feature);

    let corner = projected_point_at(sketch, Point2::new(40.0, 40.0));

    assert_eq!(
        aligned_point_pairs(sketch),
        vec![Constraint::HorizontalPoints(point, corner.into())]
    );
    assert_eq!(sketch.projected().count(), 1);
    assert!((placed.y - 40.0).abs() < 1e-6, "{placed}");
    assert!((placed.x - 60.0).abs() < 0.5, "{placed}");

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.settle();

    let sketch = harness.sketch(feature);

    assert_eq!(sketch.projected().count(), 0);
    assert!(aligned_point_pairs(sketch).is_empty());
    assert_eq!(sketch.point(point), Some(start));
}

#[test]
fn a_grabbed_point_where_a_track_from_a_body_item_crosses_a_body_edge_keeps_both() {
    let mut harness = Harness::new();

    plate_beside_a_cylinder(&mut harness);

    let (feature, point, grabbed) = grabbable_point(&mut harness, Point2::new(75.0, 45.0));

    grab_through(
        &mut harness,
        grabbed,
        &[Point2::new(60.2, 30.2), Point2::new(40.2, 30.3)],
    );

    assert!(harness.shows_containing(
        "On an edge of Extrude 1, horizontal from the centre of a round edge of Extrude"
    ));

    let placed = released_at(&mut harness, feature, point, Point2::new(40.2, 30.3));
    let sketch = harness.sketch(feature);

    let edge = projected_line(sketch);
    let centre = projected_centre(sketch);

    assert_eq!(
        constraints_of_kind(sketch, "Coincident"),
        vec![Constraint::Coincident(point, edge)]
    );
    assert_eq!(
        aligned_point_pairs(sketch),
        vec![Constraint::HorizontalPoints(point, centre.into())]
    );
    assert!(placed.distance(Point2::new(40.0, 30.0)) < 1e-6, "{placed}");

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.settle();

    assert_eq!(harness.sketch(feature).projected().count(), 0);
}
