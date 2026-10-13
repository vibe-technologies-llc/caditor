use std::f64::consts::{PI, TAU};

use caditor_document::{
    AxisReference, Coil, CoilLength, CoilTurning, FeatureId, FeatureState, PrincipalAxis,
};
use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2};
use caditor_kernel::CoilShape;
use egui::{Id, Key, Modifiers};

use super::{CAMERA_SETTLE, Harness, choose, run_from_palette};
use crate::{
    coil_panel, coil_tools,
    reference_picking::Slot,
    selection::{Axis, Pickable},
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

    harness.type_into_field(Id::new(("coil-field", coil, "turns")), "3");
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

    harness.type_into_field(Id::new(("coil-field", coil, "height")), "20 mm");
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

    harness.type_into_field(Id::new(("coil-field", coil, "pitch")), "1 mm");
    harness.settle();
    let reason = match harness
        .model
        .evaluation()
        .feature(coil)
        .map(|status| status.state.clone())
    {
        Some(FeatureState::Failed(error)) => error.reason,
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
