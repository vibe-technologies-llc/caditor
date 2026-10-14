use std::f64::consts::TAU;

use caditor_document::{AxisReference, FeatureId, Helix, PrincipalAxis, SweepPath};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_sketch::Sketch;
use egui::{Id, Key, Modifiers};

use super::{
    CAMERA_SETTLE, Harness, drag_value_arrow, length_value, pipes::undone_and_redone,
    run_from_palette,
};
use crate::{
    coil_panel, coil_tools,
    editing::EditingCommand,
    helix_panel, helix_tools,
    model::Action,
    selection::{Axis, Pickable},
    solid_panel,
    value_gauges::Measured,
};

const RADIUS: f64 = 0.5 * coil_tools::DEFAULT_DIAMETER;

fn helix_of(harness: &Harness, feature: FeatureId) -> Helix {
    harness
        .document()
        .feature(feature)
        .and_then(|feature| feature.kind.helix())
        .expect("a helix")
        .clone()
}

fn triangle_on_the_helix_start() -> Sketch {
    let plane = Plane::from_frame(Point3::ZERO, Vector3::Y, Vector3::X).unwrap();
    let mut sketch = Sketch::new(plane);
    let corners = [
        Point2::new(11.0, -1.0),
        Point2::new(11.0, 1.0),
        Point2::new(9.0, 0.0),
    ];
    for (from, to) in corners.iter().zip(corners.iter().cycle().skip(1)) {
        sketch.add_line(*from, *to);
    }
    sketch
}

#[test]
fn helix_winds_round_z_from_the_palette_and_a_triangle_swept_along_it_makes_a_thread() {
    let mut harness = Harness::new();
    harness.select([Pickable::Axis(Axis::Z)]);

    run_from_palette(&mut harness, "helix");
    harness.settle();

    let helix = harness
        .workspace
        .editing
        .solid()
        .expect("the helix is open");
    assert_eq!(harness.model.undo_label(), Some("Create Helix 1"));
    assert_eq!(
        helix_of(&harness, helix).axis,
        AxisReference::Principal(PrincipalAxis::Z)
    );
    assert!(harness.workspace.editing.picking().is_none());
    assert!(harness.shows(helix_panel::DESCRIPTION));
    assert!(harness.shows(coil_panel::SIZED_BY));
    assert!(!harness.document().feature(helix).unwrap().makes_body());

    harness.type_into_field(Id::new(("helix-field", helix, "turns")), "2");
    harness.settle();
    assert_eq!(
        helix_of(&harness, helix).length.turns(),
        Some(&caditor_expression::Expression::number(2.0))
    );

    harness.perform(Action::Editing(EditingCommand::CloseSolid));
    let profile = harness.add_sketch(triangle_on_the_helix_start());
    harness.select([Pickable::Datum(helix)]);

    run_from_palette(&mut harness, "sweep");
    harness.settle();

    let sweep = harness
        .workspace
        .editing
        .solid()
        .expect("the sweep is open");
    let made = harness
        .document()
        .feature(sweep)
        .and_then(|feature| feature.kind.solid())
        .expect("a sweep")
        .clone();
    assert_eq!(made.sketch(), profile);
    assert_eq!(made.path(), Some(&SweepPath::Helix { helix }));
    assert!(harness.shows(solid_panel::PATH));
    assert!(harness.shows("Helix 1"));
    let expected = 2.0 * TAU * 2.0 * (31.0 / 3.0);
    let found = harness.body_volume(sweep);
    assert!(
        (found - expected).abs() < 1e-2 * expected,
        "{found} against {expected}"
    );

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.settle();
    assert!(harness.document().feature(sweep).is_none());
}

fn open_vertical_helix(harness: &mut Harness) -> FeatureId {
    harness.select([Pickable::Axis(Axis::Z)]);
    run_from_palette(harness, "helix");
    harness.settle();
    harness.select([]);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness
        .workspace
        .editing
        .solid()
        .expect("the helix is open")
}

fn start_of(harness: &Harness, helix: FeatureId) -> Point3 {
    helix_tools::points(harness.model.evaluation(), helix)
        .first()
        .copied()
        .expect("the helix is built")
}

#[test]
fn typing_a_start_angle_starts_the_helix_further_round_its_axis_in_one_undoable_change() {
    let mut harness = Harness::new();
    let helix = open_vertical_helix(&mut harness);
    let before = start_of(&harness, helix);

    harness.type_into_field(Id::new(("helix-field", helix, "start")), "90");
    harness.settle();
    let after = start_of(&harness, helix);

    assert!(harness.shows(helix_panel::START_ANGLE));
    assert!(
        before.distance(Point3::new(RADIUS, 0.0, 0.0)) < 1e-9,
        "{before}"
    );
    assert!(
        after.distance(Point3::new(0.0, RADIUS, 0.0)) < 1e-9,
        "{after}"
    );
    assert_eq!(harness.model.undo_label(), Some("Edit Helix 1"));
    let angle = |harness: &Harness| {
        harness
            .model
            .parameters()
            .evaluate_expression(&helix_of(harness, helix).start)
            .unwrap()
            .value
    };
    undone_and_redone(&mut harness, "the start angle", angle);
    assert_eq!(angle(&harness), 90.0);
}

#[test]
fn dragging_a_helix_s_diameter_and_pitch_arrows_changes_them_in_one_undoable_change_each() {
    let mut harness = Harness::new();
    let helix = open_vertical_helix(&mut harness);
    let diameter = |harness: &Harness| length_value(harness, &helix_of(harness, helix).diameter);
    let pitch = |harness: &Harness| {
        let length = helix_of(harness, helix).length;
        length_value(harness, length.pitch().expect("a pitch"))
    };
    let before = diameter(&harness);

    let step = drag_value_arrow(&mut harness, Measured::CoilDiameter, 3.0);
    let after = diameter(&harness);

    assert_eq!(before, coil_tools::DEFAULT_DIAMETER);
    assert_eq!(harness.model.undo_label(), Some("Edit Helix 1"));
    assert!(
        (after - before - 6.0).abs() <= 2.0 * step,
        "{before} to {after} with steps of {step}"
    );
    undone_and_redone(&mut harness, "the diameter drag", diameter);

    let before = pitch(&harness);
    let step = drag_value_arrow(&mut harness, Measured::CoilPitch, 2.0);
    let after = pitch(&harness);

    assert_eq!(before, caditor_document::DEFAULT_PITCH);
    assert!(
        (after - before - 2.0).abs() <= step,
        "{before} to {after} with steps of {step}"
    );
    undone_and_redone(&mut harness, "the pitch drag", pitch);
}
