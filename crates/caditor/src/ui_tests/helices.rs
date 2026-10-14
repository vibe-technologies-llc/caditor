use std::f64::consts::TAU;

use caditor_document::{AxisReference, FeatureId, Helix, PrincipalAxis, SweepPath};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_sketch::Sketch;
use egui::{Id, Key, Modifiers};

use super::{Harness, run_from_palette};
use crate::{
    coil_panel,
    editing::EditingCommand,
    helix_panel,
    model::Action,
    selection::{Axis, Pickable},
    solid_panel,
};

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
