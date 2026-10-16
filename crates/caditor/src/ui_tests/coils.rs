use std::f64::consts::{PI, TAU};

use caditor_document::{
    AxisReference, Coil, CoilLength, CoilTurning, FeatureId, FeatureState, PrincipalAxis,
};
use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2};
use caditor_kernel::CoilShape;
use egui::{Id, Key, Modifiers};

use super::{
    CAMERA_SETTLE, Harness, choose, drag_value_arrow, length_value, pipes::undone_and_redone,
    run_from_palette,
};
use crate::{
    coil_panel, coil_tools,
    move_manipulator::Handle,
    reference_picking::Slot,
    selection::{Axis, Pickable},
    value_gauges::Measured,
};

const RADIUS: f64 = 0.5 * coil_tools::DEFAULT_DIAMETER;
const WIRE: f64 = 0.5 * coil_tools::DEFAULT_SECTION;

fn coil_of(harness: &Harness, feature: FeatureId) -> Coil {
    harness
        .document()
        .feature(feature)
        .and_then(|feature| feature.kind.coil())
        .expect("a coil")
        .clone()
}

fn near(found: f64, expected: f64) -> bool {
    (found - expected).abs() < 1e-2 * expected
}

fn wire_volume(turns: f64) -> f64 {
    PI * WIRE * WIRE * TAU * RADIUS * turns
}

fn picking_slot(harness: &Harness) -> Option<Slot> {
    harness
        .workspace
        .editing
        .picking()
        .map(|picking| picking.slot)
}

#[test]
fn coil_with_nothing_selected_winds_round_z_and_takes_the_axis_clicked_next() {
    let mut harness = Harness::new();

    run_from_palette(&mut harness, "coil");
    harness.settle();

    let coil = harness.workspace.editing.solid().expect("the coil is open");
    assert_eq!(harness.model.undo_label(), Some("Create Coil 1"));
    let made = coil_of(&harness, coil);
    assert_eq!(made.axis, AxisReference::Principal(PrincipalAxis::Z));
    assert_eq!(made.shape, CoilShape::Round);
    assert_eq!(picking_slot(&harness), Some(Slot::CoilAxis));
    assert!(near(harness.body_volume(coil), wire_volume(5.0)));
    assert!(harness.shows(coil_panel::SIZED_BY));
    assert!(harness.shows(coil_panel::PITCH));

    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.click_pickable(Plane::XY, Point2::new(0.0, 0.0), Pickable::Axis(Axis::X));
    harness.settle();

    assert_eq!(
        coil_of(&harness, coil).axis,
        AxisReference::Principal(PrincipalAxis::X)
    );
    assert_eq!(picking_slot(&harness), None);
    assert!(near(harness.body_volume(coil), wire_volume(5.0)));
}

#[test]
fn a_coil_s_panel_sizes_it_by_any_two_lengths_and_switches_its_section_and_turning() {
    let mut harness = Harness::new();
    harness.select([Pickable::Axis(Axis::Y)]);

    run_from_palette(&mut harness, "coil");
    harness.settle();

    let coil = harness.workspace.editing.solid().expect("the coil is open");
    assert_eq!(
        coil_of(&harness, coil).axis,
        AxisReference::Principal(PrincipalAxis::Y)
    );
    assert_eq!(picking_slot(&harness), None);

    harness.type_into_field(Id::unique(("coil-field", coil, "turns")), "3");
    harness.settle();
    assert!(near(harness.body_volume(coil), wire_volume(3.0)));

    choose(&mut harness, "Turns and pitch", "Height and pitch");
    harness.select([]);
    harness.settle();
    let switched = coil_of(&harness, coil).length;
    assert!(
        matches!(
            &switched,
            CoilLength::HeightAndPitch { height, .. } if *height == Expression::parse_stored("15 mm").unwrap()
        ),
        "{switched:?}"
    );

    harness.type_into_field(Id::unique(("coil-field", coil, "height")), "20 mm");
    harness.settle();
    assert!(near(harness.body_volume(coil), wire_volume(4.0)));

    choose(&mut harness, "Round", "Square");
    assert_eq!(coil_of(&harness, coil).shape, CoilShape::Square);
    assert!(harness.shows("Section width"));
    assert!(near(
        harness.body_volume(coil),
        4.0 * WIRE * WIRE * TAU * RADIUS * 4.0
    ));

    choose(&mut harness, "Counter-clockwise", "Clockwise");
    assert_eq!(coil_of(&harness, coil).turning, CoilTurning::Clockwise);

    harness.type_into_field(Id::unique(("coil-field", coil, "pitch")), "1 mm");
    harness.settle();
    let reason = match harness
        .model
        .evaluation()
        .feature(coil)
        .map(|status| status.state.clone())
    {
        Some(FeatureState::Failed(error)) => error.reason(),
        other => panic!("turns closer than the section fail, found {other:?}"),
    };
    assert!(reason.contains("would touch"), "{reason}");
    assert!(harness.shows(&reason));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.settle();
    assert!(near(
        harness.body_volume(coil),
        4.0 * WIRE * WIRE * TAU * RADIUS * 4.0
    ));
}

fn open_vertical_coil(harness: &mut Harness) -> FeatureId {
    harness.select([Pickable::Axis(Axis::Z)]);
    run_from_palette(harness, "coil");
    harness.settle();
    harness.select([]);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.workspace.editing.solid().expect("the coil is open")
}

#[test]
fn dragging_a_coil_s_diameter_arrow_changes_its_diameter_and_a_typed_value_still_works() {
    let mut harness = Harness::new();
    let coil = open_vertical_coil(&mut harness);
    let diameter = |harness: &Harness| length_value(harness, &coil_of(harness, coil).diameter);
    let before = diameter(&harness);

    let step = drag_value_arrow(&mut harness, Measured::CoilDiameter, 3.0);
    let after = diameter(&harness);

    assert_eq!(before, coil_tools::DEFAULT_DIAMETER);
    assert_eq!(harness.model.undo_label(), Some("Edit Coil 1"));
    assert!(
        (after - before - 6.0).abs() <= 2.0 * step,
        "{before} to {after} with steps of {step}"
    );
    undone_and_redone(&mut harness, "the diameter drag", diameter);

    harness.type_into_field(Id::unique(("coil-field", coil, "diameter")), "30 mm");
    harness.settle();
    assert_eq!(diameter(&harness), 30.0);
    assert!(near(
        harness.body_volume(coil),
        PI * WIRE * WIRE * TAU * 15.0 * 5.0
    ));
}

#[test]
fn dragging_a_coil_s_section_arrow_changes_the_size_of_its_wire() {
    let mut harness = Harness::new();
    let coil = open_vertical_coil(&mut harness);
    let size = |harness: &Harness| length_value(harness, &coil_of(harness, coil).size);
    let before = size(&harness);

    let step = drag_value_arrow(&mut harness, Measured::CoilSection, 0.5);
    let after = size(&harness);

    assert_eq!(before, coil_tools::DEFAULT_SECTION);
    assert_eq!(harness.model.undo_label(), Some("Edit Coil 1"));
    assert!(
        (after - before - 1.0).abs() <= 2.0 * step,
        "{before} to {after} with steps of {step}"
    );
    undone_and_redone(&mut harness, "the section drag", size);
}

#[test]
fn dragging_a_coil_s_pitch_arrow_spreads_its_turns() {
    let mut harness = Harness::new();
    let coil = open_vertical_coil(&mut harness);
    let pitch = |harness: &Harness| {
        let length = coil_of(harness, coil).length;
        length_value(harness, length.pitch().expect("a pitch"))
    };
    let before = pitch(&harness);

    let step = drag_value_arrow(&mut harness, Measured::CoilPitch, 2.0);
    let after = pitch(&harness);

    assert_eq!(before, caditor_document::DEFAULT_PITCH);
    assert_eq!(harness.model.undo_label(), Some("Edit Coil 1"));
    assert!(
        (after - before - 2.0).abs() <= step,
        "{before} to {after} with steps of {step}"
    );
    undone_and_redone(&mut harness, "the pitch drag", pitch);
}

#[test]
fn a_coil_sized_by_height_has_a_height_arrow_and_no_pitch_arrow_when_sized_by_height_and_turns() {
    let mut harness = Harness::new();
    let coil = open_vertical_coil(&mut harness);
    let shown = |harness: &Harness, measured: Measured| {
        harness
            .workspace
            .viewport
            .handle_position(Handle::Value(measured), 0.0)
            .is_some()
    };
    assert!(shown(&harness, Measured::CoilPitch));
    assert!(!shown(&harness, Measured::CoilHeight));

    choose(&mut harness, "Turns and pitch", "Height and turns");
    harness.select([]);
    run_from_palette(&mut harness, "Fit view");
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    assert!(!shown(&harness, Measured::CoilPitch));
    assert!(shown(&harness, Measured::CoilHeight));

    let height = |harness: &Harness| {
        let length = coil_of(harness, coil).length;
        length_value(harness, length.height().expect("a height"))
    };
    let before = height(&harness);
    let step = drag_value_arrow(&mut harness, Measured::CoilHeight, 5.0);
    let after = height(&harness);

    assert!(
        (after - before - 5.0).abs() <= step,
        "{before} to {after} with steps of {step}"
    );
    undone_and_redone(&mut harness, "the height drag", height);
}
