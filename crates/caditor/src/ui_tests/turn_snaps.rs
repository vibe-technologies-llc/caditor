use caditor_document::{
    ExtrudeEnd, ExtrudeExtent, FeatureId, FeatureKind, MoveAxis, RevolveExtent, SolidFeature,
};
use caditor_expression::Dimension;
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_sketch::Sketch;
use egui::{Event, Id, Key, Modifiers, PointerButton, Pos2};

use super::{
    CAMERA_SETTLE, Harness, choose, extruded_plate, length_value, open_solid, rectangle,
    run_from_palette,
};
use crate::{
    move_manipulator::{Handle, Reach},
    turn_handles::TurnEnd,
};

const EXACT: f64 = 1e-6;
const ANGLE_AS_STORED: f64 = 1e-3;
const WITHIN_A_PIXEL_DEGREES: f64 = 0.05;
const STEP_DEGREES: f64 = 5.0;

fn block(harness: &mut Harness, min: Point2, max: Point2) -> FeatureId {
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, min, max);
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click_tool("Extrude");
    harness.settle();
    let block = open_solid(harness);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    block
}

fn fit_and_settle(harness: &mut Harness) {
    run_from_palette(harness, "Fit view");
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
}

fn screen_of(harness: &Harness, point: Point3) -> Pos2 {
    harness
        .workspace
        .viewport
        .screen_of(point)
        .unwrap_or_else(|| panic!("{point:?} is in view"))
}

fn grip(harness: &Harness, handle: Handle, along: f64) -> Pos2 {
    harness
        .workspace
        .viewport
        .handle_position(handle, along)
        .unwrap_or_else(|| panic!("{handle:?} is shown"))
}

fn hold_at(harness: &mut Harness, from: Pos2, to: Pos2) {
    harness.events.push(Event::PointerMoved(from));
    harness.frame();
    harness.events.push(Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    for step in 1..=4 {
        let position = from + (to - from) * (step as f32 / 4.0);
        harness.events.push(Event::PointerMoved(position));
        harness.frame();
    }
    harness.frame();
}

fn release_at(harness: &mut Harness, to: Pos2) {
    harness.events.push(Event::PointerButton {
        pos: to,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    harness.settle();
}

fn angle_value(harness: &Harness, expression: &caditor_expression::Expression) -> f64 {
    let parameters = harness.model.parameters();
    expression
        .evaluate_as(Dimension::ANGLE, &|id| parameters.value(id))
        .unwrap()
}

#[test]
fn a_revolve_s_angle_arrow_dropped_on_a_corner_turns_its_end_into_the_corner() {
    let mut harness = Harness::new();
    block(
        &mut harness,
        Point2::new(-50.0, 7.0),
        Point2::new(-30.0, 27.0),
    );
    let mut section = Sketch::new(Plane::XZ);
    rectangle(
        &mut section,
        Point2::new(15.0, 0.0),
        Point2::new(25.0, 10.0),
    );
    harness.add_sketch(section);
    harness.select([]);
    harness.click("Revolve");
    harness.settle();
    let revolve = open_solid(&harness);
    choose(&mut harness, "Full turn", "Two angles");
    fit_and_settle(&mut harness);
    let forward = Handle::Revolve(TurnEnd::Forward);
    let from = grip(&harness, forward, 0.0);
    let corner = screen_of(&harness, Point3::new(-30.0, 7.0, 10.0));

    hold_at(&mut harness, from, corner);
    let readout = harness.shows_containing("to a corner");
    release_at(&mut harness, corner);
    let SolidFeature::Revolve(turned) = harness.solid(revolve) else {
        panic!("the revolve stays a revolve");
    };
    let RevolveExtent::TwoSides { forward: angle, .. } = &turned.extent else {
        panic!("the revolve keeps two angles");
    };
    let angle = angle_value(&harness, angle);
    let into_the_corner = 180.0 - 7.0_f64.atan2(30.0).to_degrees();

    assert!(readout);
    assert_eq!(harness.model.undo_label(), Some("Edit Revolve 1"));
    assert!(
        (angle - into_the_corner).abs() < ANGLE_AS_STORED
            || (angle - (360.0 - into_the_corner)).abs() < ANGLE_AS_STORED,
        "{angle} does not bring the end into the corner at {into_the_corner}"
    );
}

#[test]
fn a_move_s_ring_dropped_on_a_corner_turns_the_grabbed_point_to_it() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    block(
        &mut harness,
        Point2::new(-10.0, 50.0),
        Point2::new(5.0, 65.0),
    );
    harness.select([top]);
    harness.click("Move body");
    harness.settle();
    let movement = open_solid(&harness);
    fit_and_settle(&mut harness);
    let ring = Handle::Turn(MoveAxis::Z);
    let grabbed = harness
        .workspace
        .viewport
        .handle_grip(ring, 3.0)
        .expect("the Z ring is shown");
    let from = grip(&harness, ring, 3.0);
    let corner_at = Point3::new(5.0, 50.0, 10.0);
    let corner = screen_of(&harness, corner_at);
    let centre = Point3::new(20.0, 20.0, 5.0);
    let (from_centre, to_corner) = (grabbed - centre, corner_at - centre);
    let to_the_corner = from_centre
        .cross(to_corner)
        .dot(Vector3::Z)
        .atan2(from_centre.x * to_corner.x + from_centre.y * to_corner.y)
        .to_degrees();
    let off_the_steps = (to_the_corner / STEP_DEGREES - (to_the_corner / STEP_DEGREES).round())
        .abs()
        * STEP_DEGREES;

    hold_at(&mut harness, from, corner);
    let readout = harness.shows_containing("to a corner");
    release_at(&mut harness, corner);
    let Some(FeatureKind::Move(moved)) = harness
        .document()
        .feature(movement)
        .map(|feature| feature.kind.clone())
    else {
        panic!("the move stays a move");
    };
    let about_z = angle_value(&harness, &moved.turn[2]);

    assert!(off_the_steps > 10.0 * WITHIN_A_PIXEL_DEGREES);
    assert!(readout);
    assert_eq!(harness.model.undo_label(), Some("Edit Move body 1"));
    assert!(
        (about_z - to_the_corner).abs() < WITHIN_A_PIXEL_DEGREES,
        "turned {about_z} instead of {to_the_corner}"
    );
}

#[test]
fn an_extrusion_arrow_dropped_on_a_cylinder_stops_where_its_line_meets_it() {
    let mut harness = Harness::new();
    let mut base = Sketch::new(Plane::XY);
    base.add_circle(Point2::new(0.0, 40.37), 10.0);
    harness.add_sketch(base);
    harness.select([]);
    harness.click_tool("Extrude");
    harness.settle();
    let cylinder = open_solid(&harness);
    harness.type_into_field(Id::new(("solid-field", "distance", cylinder)), "30");
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let facing_the_cylinder =
        Plane::from_frame(Point3::new(0.0, 0.0, 15.0), Vector3::Y, Vector3::X).unwrap();
    let mut profile = Sketch::new(facing_the_cylinder);
    rectangle(&mut profile, Point2::new(-3.0, -3.0), Point2::new(3.0, 3.0));
    harness.add_sketch(profile);
    harness.select([]);
    harness.click_tool("Extrude");
    harness.settle();
    let extrude = open_solid(&harness);
    harness.type_into_field(Id::new(("solid-field", "distance", extrude)), "10");
    harness.settle();
    fit_and_settle(&mut harness);
    let only = Handle::Reach(Reach::Only);
    let from = grip(&harness, only, 0.0);
    let viewport = &harness.workspace.viewport;
    let grabbed_past_the_end = viewport.handle_grip(only, 0.0).expect("the arrow is shown")
        - viewport.handle_foot(only).expect("the arrow has a foot");
    let meeting = screen_of(
        &harness,
        Point3::new(0.0, 30.37, 15.0) + grabbed_past_the_end,
    );

    hold_at(&mut harness, from, meeting);
    let readout = harness.shows_containing("to a face");
    release_at(&mut harness, meeting);
    let SolidFeature::Extrude(dragged) = harness.solid(extrude) else {
        panic!("the extrusion stays an extrusion");
    };
    let ExtrudeExtent::OneSide {
        end: ExtrudeEnd::Distance(distance),
        ..
    } = &dragged.extent
    else {
        panic!("the extrusion keeps one distance");
    };
    let distance = length_value(&harness, distance);

    assert!(readout);
    assert_eq!(harness.model.undo_label(), Some("Edit Extrude 2"));
    assert!(
        (distance - 30.37).abs() < EXACT,
        "the arrow stopped at {distance}"
    );
}
