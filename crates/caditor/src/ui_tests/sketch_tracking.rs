use caditor_document::FeatureId;
use caditor_expression::{Expression, Unit};
use caditor_geometry::{Plane, Point2, Vector2};
use caditor_sketch::{Constraint, EllipseEnd, Entity, EntityId, PointTarget, Quadrant, Sketch};
use egui::{Event, Key, Modifiers, PointerButton};

use super::{
    Harness, constraints_of_kind, drag_in_sketch, edit_free_sketch, entities_of_kind, line_ends,
    near, rectangle, release_drag, sketch_entity,
};
use crate::{editing::Tool, selection::Pickable};

const HELD: f64 = 1e-6;

fn placed_alone(sketch: &Sketch, before: &[EntityId]) -> EntityId {
    let [placed] = entities_of_kind(sketch, "Point")
        .into_iter()
        .filter(|point| !before.contains(point))
        .collect::<Vec<_>>()[..]
    else {
        panic!("one point should be placed");
    };
    placed
}

fn constraints_on(sketch: &Sketch, point: EntityId) -> Vec<Constraint> {
    sketch
        .constraints()
        .filter(|(_, constraint)| constraint.entities().contains(&point))
        .map(|(_, constraint)| constraint.clone())
        .collect()
}

fn back_to_select(harness: &mut Harness) {
    for _ in 0..3 {
        if harness.tool() == Some(Tool::Select) {
            return;
        }
        harness.key(Key::Escape, Modifiers::NONE);
        harness.frame();
    }
    assert_eq!(harness.tool(), Some(Tool::Select));
}

fn drag_point(harness: &mut Harness, feature: FeatureId, point: EntityId, to: Point2) {
    back_to_select(harness);
    harness.settle();
    let from = harness.shown(feature).point(point).unwrap();
    let before = harness.sketch(feature).point(point);
    let grabbed = harness.hover_pickable(
        Plane::XY,
        from,
        Pickable::SketchEntity {
            feature,
            entity: point,
        },
    );
    drag_in_sketch(harness, grabbed, to);
    harness.wait_until("the drag is committed", |harness| {
        harness.sketch(feature).point(point) != before
    });
    harness.settle();
}

fn middle_of(sketch: &Sketch, line: EntityId) -> Point2 {
    let (start, end) = sketch.line_endpoints(line).unwrap();
    start.midpoint(end)
}

fn crossing_of(sketch: &Sketch, first: EntityId, second: EntityId) -> Point2 {
    let (a, b) = sketch.line_endpoints(first).unwrap();
    let (c, d) = sketch.line_endpoints(second).unwrap();
    let along = (c - a).perp_dot(d - c) / (b - a).perp_dot(d - c);
    a + (b - a) * along
}

fn triangle(sketch: &mut Sketch, corners: [Point2; 3]) -> [EntityId; 3] {
    let points = corners.map(|corner| sketch.add_point(corner));
    for (start, end) in points.iter().zip(points.iter().cycle().skip(1)) {
        let id = EntityId::from_raw(sketch.next_id());
        sketch
            .insert_entity(
                id,
                Entity::Line {
                    start: *start,
                    end: *end,
                },
            )
            .unwrap();
    }
    points
}

#[test]
fn a_line_drawn_after_hovering_the_middle_of_another_ends_level_with_that_middle_and_follows_it() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let base = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(60.0, 40.0));
    let (_, base_end) = line_ends(&sketch, base);
    let label = sketch.entity_label(base);
    let feature = edit_free_sketch(&mut harness, sketch);

    harness.use_tool(Key::L);
    harness.point_at(Point2::new(30.05, 20.05));
    assert!(harness.shows(&format!("Midpoint of {label}")));
    harness.click_at(Point2::new(10.0, 35.0));
    harness.point_at(Point2::new(55.0, 20.1));
    assert!(harness.shows(&format!("Horizontal from the midpoint of {label}")));
    harness.click_at(Point2::new(55.0, 20.1));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.settle();

    let sketch = harness.sketch(feature);
    let drawn = entities_of_kind(sketch, "Line")
        .into_iter()
        .find(|line| *line != base)
        .expect("a line is drawn");
    let (start, end) = line_ends(sketch, drawn);
    let landed = sketch.point(end).unwrap();
    assert!(near(sketch.point(start).unwrap(), Point2::new(10.0, 35.0)));
    assert!((landed.y - 20.0).abs() < 1e-9, "{landed}");
    assert!((landed.x - 55.0).abs() < 0.1, "{landed}");
    assert!(
        constraints_on(sketch, end).contains(&Constraint::HorizontalPoints(
            end,
            PointTarget::Middle(base)
        ))
    );

    drag_point(&mut harness, feature, base_end, Point2::new(60.0, 60.0));

    let shown = harness.shown(feature);
    let middle = middle_of(&shown, base);
    assert!((middle.y - 20.0).abs() > 1.0, "{middle}");
    assert!((shown.point(end).unwrap().y - middle.y).abs() < HELD);
}

#[test]
fn a_point_placed_square_to_a_line_through_its_middle_stays_on_its_perpendicular_bisector() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let base = sketch.add_line(Point2::new(10.0, 10.0), Point2::new(40.0, 50.0));
    let (_, base_end) = line_ends(&sketch, base);
    let label = sketch.entity_label(base);
    let middle = Point2::new(25.0, 30.0);
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = entities_of_kind(harness.sketch(feature), "Point");

    harness.use_tool(Key::P);
    harness.point_at(middle + Vector2::new(0.05, 0.05));
    assert!(harness.shows(&format!("Midpoint of {label}")));
    harness.point_at(Point2::new(5.04, 45.03));
    assert!(harness.shows(&format!("On the perpendicular bisector of {label}")));
    harness.click_at(Point2::new(5.04, 45.03));
    harness.settle();

    let sketch = harness.sketch(feature);
    let placed = placed_alone(sketch, &before);
    let at = sketch.point(placed).unwrap();
    assert!(
        (at - middle).dot(Vector2::new(3.0, 4.0)).abs() < 1e-9,
        "{at} is off the bisector"
    );
    assert!(at.distance(Point2::new(5.0, 45.0)) < 0.1, "{at}");
    assert_eq!(
        constraints_on(sketch, placed),
        vec![Constraint::OnDirection {
            point: placed,
            through: PointTarget::Middle(base),
            line: base,
            square: true,
        }]
    );

    drag_point(&mut harness, feature, base_end, Point2::new(55.0, 35.0));

    let shown = harness.shown(feature);
    let (start, end) = shown.line_endpoints(base).unwrap();
    let moved = shown.point(placed).unwrap();
    assert!(end.distance(Point2::new(40.0, 50.0)) > 1.0, "{end}");
    assert!(
        (moved - start.midpoint(end))
            .dot((end - start).normalize())
            .abs()
            < HELD,
        "{moved} is off the moved bisector"
    );
}

#[test]
fn a_point_upright_with_a_crossing_follows_the_crossing_when_a_line_moves() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let rising = sketch.add_line(Point2::ZERO, Point2::new(30.0, 30.0));
    let falling = sketch.add_line(Point2::new(0.0, 20.0), Point2::new(30.0, -10.0));
    let (falling_start, _) = line_ends(&sketch, falling);
    let words = format!(
        "the crossing of {} and {}",
        sketch.entity_label(rising),
        sketch.entity_label(falling)
    );
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = entities_of_kind(harness.sketch(feature), "Point");

    harness.use_tool(Key::P);
    harness.point_at(Point2::new(10.1, 10.05));
    assert!(harness.shows_containing("Crossing of"));
    harness.point_at(Point2::new(10.03, 28.0));
    assert!(harness.shows(&format!("Vertical from {words}")));
    harness.click_at(Point2::new(10.03, 28.0));
    harness.settle();

    let sketch = harness.sketch(feature);
    let placed = placed_alone(sketch, &before);
    assert!((sketch.point(placed).unwrap().x - 10.0).abs() < 1e-9);
    assert_eq!(
        constraints_on(sketch, placed),
        vec![Constraint::VerticalPoints(
            placed,
            PointTarget::Crossing(rising, falling)
        )]
    );

    drag_point(&mut harness, feature, falling_start, Point2::new(0.0, 32.0));

    let shown = harness.shown(feature);
    let crossing = crossing_of(&shown, rising, falling);
    assert!((crossing.x - 10.0).abs() > 1.0, "{crossing}");
    assert!((shown.point(placed).unwrap().x - crossing.x).abs() < HELD);
}

#[test]
fn a_point_placed_at_the_centre_of_a_triangle_is_held_at_the_centre_of_its_corners() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let corners = triangle(
        &mut sketch,
        [
            Point2::new(0.0, 0.0),
            Point2::new(30.0, 0.0),
            Point2::new(0.0, 30.0),
        ],
    );
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = entities_of_kind(harness.sketch(feature), "Point");

    harness.use_tool(Key::P);
    harness.point_at(Point2::new(10.1, 9.95));
    assert!(harness.shows_containing("Centre of the outline of Line"));
    assert!(!harness.shows_containing("not kept there"));
    harness.click_at(Point2::new(10.1, 9.95));
    harness.settle();

    let sketch = harness.sketch(feature);
    let placed = placed_alone(sketch, &before);
    assert!(
        sketch
            .point(placed)
            .unwrap()
            .distance(Point2::new(10.0, 10.0))
            < 1e-9
    );
    let [
        Constraint::Centroid {
            point,
            corners: ref held,
        },
    ] = constraints_on(sketch, placed)[..]
    else {
        panic!("one centre constraint is expected");
    };
    let mut held = held.clone();
    held.sort_unstable();
    assert_eq!(point, placed);
    assert_eq!(held, corners.to_vec());

    drag_point(&mut harness, feature, corners[1], Point2::new(45.0, 0.0));

    let shown = harness.shown(feature);
    let centre = corners
        .iter()
        .map(|corner| shown.point(*corner).unwrap())
        .fold(Vector2::ZERO, |sum, corner| sum + corner)
        / 3.0;
    assert!(centre.distance(Point2::new(10.0, 10.0)) > 1.0, "{centre}");
    assert!(shown.point(placed).unwrap().distance(centre) < HELD);
}

#[test]
fn a_point_level_with_the_top_of_a_circle_follows_the_circle() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::new(30.0, 20.0), 10.0);
    let centre = sketch.center_of(circle).unwrap();
    sketch
        .add_constraint(Constraint::Radius {
            entity: circle,
            value: Expression::Measure(10.0, Unit::Millimetre),
        })
        .unwrap();
    let label = sketch.entity_label(circle);
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = entities_of_kind(harness.sketch(feature), "Point");

    harness.use_tool(Key::P);
    harness.point_at(Point2::new(30.05, 30.05));
    assert!(harness.shows(&format!("Top of {label}")));
    harness.point_at(Point2::new(45.0, 30.04));
    assert!(harness.shows(&format!("Horizontal from the top of {label}")));
    harness.click_at(Point2::new(45.0, 30.04));
    harness.settle();

    let sketch = harness.sketch(feature);
    let placed = placed_alone(sketch, &before);
    assert_eq!(
        constraints_on(sketch, placed),
        vec![Constraint::HorizontalPoints(
            placed,
            PointTarget::Quadrant {
                curve: circle,
                side: Quadrant::Top,
            }
        )]
    );

    drag_point(&mut harness, feature, centre, Point2::new(25.0, 26.0));

    let shown = harness.shown(feature);
    let (moved, radius) = shown.circle(circle).unwrap();
    assert!(moved.y > 21.0, "{moved}");
    assert!((shown.point(placed).unwrap().y - (moved.y + radius)).abs() < HELD);
}

#[test]
fn a_point_upright_with_the_far_end_of_an_ellipse_follows_it_as_the_axis_moves() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = sketch.add_ellipse(Point2::new(40.0, 30.0), Point2::new(60.0, 30.0), 8.0);
    let (centre, major) = match sketch.entity(ellipse) {
        Some(Entity::Ellipse { center, major, .. }) => (*center, *major),
        other => panic!("expected an ellipse, found {other:?}"),
    };
    sketch
        .add_constraint(Constraint::Fix {
            point: centre,
            at: Point2::new(40.0, 30.0),
        })
        .unwrap();
    let label = sketch.entity_label(ellipse);
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = entities_of_kind(harness.sketch(feature), "Point");

    harness.use_tool(Key::P);
    harness.point_at(Point2::new(19.95, 30.05));
    assert!(harness.shows(&format!("Far end of the major axis of {label}")));
    harness.point_at(Point2::new(19.96, 45.0));
    assert!(harness.shows(&format!(
        "Vertical from the far end of the major axis of {label}"
    )));
    harness.click_at(Point2::new(19.96, 45.0));
    harness.settle();

    let sketch = harness.sketch(feature);
    let placed = placed_alone(sketch, &before);
    assert_eq!(
        constraints_on(sketch, placed),
        vec![Constraint::VerticalPoints(
            placed,
            PointTarget::EllipseEnd {
                ellipse,
                end: EllipseEnd::FarMajor,
            }
        )]
    );

    drag_point(&mut harness, feature, major, Point2::new(66.0, 36.0));

    let shown = harness.shown(feature);
    let far = shown.target_places(&PointTarget::EllipseEnd {
        ellipse,
        end: EllipseEnd::FarMajor,
    })[0];
    assert!((far.x - 20.0).abs() > 1.0, "{far}");
    assert!((shown.point(placed).unwrap().x - far.x).abs() < HELD);
}

#[test]
fn a_point_placed_parallel_to_a_line_from_another_point_stays_parallel_when_the_line_turns() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(30.0, 40.0));
    let (start, end) = line_ends(&sketch, line);
    let other = sketch.add_point(Point2::new(50.0, 0.0));
    for (point, at) in [(start, Point2::ZERO), (other, Point2::new(50.0, 0.0))] {
        sketch
            .add_constraint(Constraint::Fix { point, at })
            .unwrap();
    }
    let words = format!(
        "Parallel to {} from {}",
        sketch.entity_label(line),
        sketch.entity_label(other)
    );
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = entities_of_kind(harness.sketch(feature), "Point");
    let aside = Vector2::new(-0.8, 0.6) * 0.03;

    harness.use_tool(Key::P);
    harness.point_at(Point2::new(30.05, 40.05));
    harness.point_at(Point2::new(50.05, 0.05));
    harness.point_at(Point2::new(56.0, 8.0) + aside);
    assert!(harness.shows(&words));
    harness.click_at(Point2::new(56.0, 8.0) + aside);
    harness.settle();

    let sketch = harness.sketch(feature);
    let placed = placed_alone(sketch, &before);
    let at = sketch.point(placed).unwrap();
    assert!(
        Vector2::new(3.0, 4.0)
            .perp_dot(at - Point2::new(50.0, 0.0))
            .abs()
            < 1e-9
    );
    assert_eq!(
        constraints_on(sketch, placed),
        vec![Constraint::OnDirection {
            point: placed,
            through: PointTarget::Point(other),
            line,
            square: false,
        }]
    );

    drag_point(&mut harness, feature, end, Point2::new(40.0, 30.0));

    let shown = harness.shown(feature);
    let direction = shown.line_direction(line).unwrap().normalize();
    let moved = shown.point(placed).unwrap();
    assert!(direction.perp_dot(Vector2::new(3.0, 4.0)).abs() > 0.1);
    assert!(direction.perp_dot(moved - Point2::new(50.0, 0.0)).abs() < HELD);
}

#[test]
fn hovering_the_centre_of_a_rectangle_highlights_all_of_it_and_the_corners_holding_the_point() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::R);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(50.0, 30.0));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.settle();
    let sides = entities_of_kind(harness.sketch(feature), "Line");
    let corners = entities_of_kind(harness.sketch(feature), "Point");

    harness.use_tool(Key::P);
    harness.point_at(Point2::new(30.2, 20.1));
    assert!(harness.shows_containing("Centre of the outline of Line"));
    let highlighted = harness.workspace.viewport.snap_highlight();
    harness.click_at(Point2::new(30.2, 20.1));
    harness.settle();

    let symmetric = constraints_of_kind(harness.sketch(feature), "Symmetric");
    let [Constraint::Symmetric { first, second, .. }] = symmetric[..] else {
        panic!("one symmetry about the centre is expected");
    };
    assert_eq!(sides.len(), 4);
    assert!(sides.iter().all(|side| highlighted.contains(side)));
    assert!(highlighted.contains(&first) && highlighted.contains(&second));
    assert_eq!(
        highlighted
            .iter()
            .filter(|entity| corners.contains(entity))
            .count(),
        2
    );
}

#[test]
fn dragging_a_point_to_the_centre_of_a_rectangle_highlights_all_of_it_and_the_corners_holding_the_point()
 {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(
        &mut sketch,
        Point2::new(10.0, 10.0),
        Point2::new(50.0, 30.0),
    );
    let lone = sketch.add_point(Point2::new(80.0, 60.0));
    let feature = edit_free_sketch(&mut harness, sketch);
    let sides = entities_of_kind(harness.sketch(feature), "Line");
    let corners: Vec<EntityId> = entities_of_kind(harness.sketch(feature), "Point")
        .into_iter()
        .filter(|point| *point != lone)
        .collect();

    let grabbed = harness.hover_pickable(
        Plane::XY,
        Point2::new(80.0, 60.0),
        sketch_entity(feature, lone),
    );
    harness.events.push(Event::PointerButton {
        pos: grabbed,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    for step in 1..=4 {
        let target = harness.on_screen(Point2::new(30.2, 20.1));
        let position = grabbed + (target - grabbed) * (step as f32 / 4.0);
        harness.events.push(Event::PointerMoved(position));
        harness.frame();
    }
    let highlighted = harness
        .workspace
        .viewport
        .grab_highlight(&harness.shown(feature));
    let at = harness.on_screen(Point2::new(30.2, 20.1));
    release_drag(&mut harness, at);

    assert_eq!(sides.len(), 4);
    assert!(sides.iter().all(|side| highlighted.contains(side)));
    assert_eq!(
        highlighted
            .iter()
            .filter(|entity| corners.contains(entity))
            .count(),
        2
    );
    assert!(!highlighted.contains(&lone));
}
