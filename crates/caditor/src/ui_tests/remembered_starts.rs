use caditor_document::{BlendKind, FeatureId};
use egui::{Id, Key, Modifiers};

use super::{Harness, extruded_plate, offer, run_from_palette, top_edge_along_x};
use crate::{commands::Command, hole_panel, last_values::Remembered, selection::Pickable};

fn size_of(harness: &Harness, feature: FeatureId) -> f64 {
    let blend = harness
        .document()
        .feature(feature)
        .and_then(|feature| feature.kind.blend())
        .expect("the feature is a blend");
    harness
        .model
        .parameters()
        .evaluate_expression(&blend.size)
        .expect("the size evaluates")
        .value
}

fn open_fillet(harness: &mut Harness, body: FeatureId, y: f64) -> FeatureId {
    let edge = top_edge_along_x(harness, body, y);
    harness.select([Pickable::Edge { body, edge }]);
    harness.click("Fillet");
    harness.settle();
    harness
        .workspace
        .editing
        .solid()
        .expect("the fillet is open")
}

fn finish(harness: &mut Harness) {
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
}

#[test]
fn a_new_fillet_starts_from_the_radius_typed_for_the_last_one() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);

    let first = open_fillet(&mut harness, plate, 0.0);
    assert_eq!(size_of(&harness, first), 1.0);

    harness.type_into_field(Id::new(("blend-size", first)), "3 mm");
    harness.settle();
    finish(&mut harness);

    let stored = harness.workspace.preferences.settings();
    assert_eq!(stored.text("modelling.last.fillet_size"), Some("3 mm"));
    assert_eq!(
        harness.model.last_values(),
        &harness.workspace.preferences.last_values
    );

    let second = open_fillet(&mut harness, plate, 40.0);

    assert_eq!(size_of(&harness, second), 3.0);
    assert_eq!(
        harness
            .document()
            .feature(second)
            .and_then(|feature| feature.kind.blend())
            .map(|blend| blend.kind),
        Some(BlendKind::Fillet)
    );
}

#[test]
fn a_remembered_radius_naming_a_missing_parameter_gives_way_to_the_default() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);

    harness
        .workspace
        .preferences
        .last_values
        .keep(Remembered::FilletSize, "wall".to_owned());
    harness
        .model
        .set_last_values(&harness.workspace.preferences.last_values);

    let fillet = open_fillet(&mut harness, plate, 0.0);

    assert_eq!(size_of(&harness, fillet), 1.0);
}

#[test]
fn an_extrusion_panel_and_a_selected_face_open_the_sketch_they_came_from() {
    let mut harness = Harness::new();
    let (extrude, top) = extruded_plate(&mut harness);
    let sketch = harness
        .document()
        .feature(extrude)
        .and_then(|feature| feature.kind.solid())
        .map(|solid| solid.sketch())
        .expect("the extrusion has a sketch");

    harness.select([top]);
    harness.frame();
    assert!(offer(&harness, Command::EditSketch).availability.is_ok());

    run_from_palette(&mut harness, "edit the sketch");
    harness.settle();
    assert_eq!(harness.editing(), Some(sketch));

    run_from_palette(&mut harness, "finish sketch");
    harness.settle();
    assert_eq!(harness.editing(), None);

    harness.select([]);
    harness.click("Extrude 1");
    harness.key(Key::E, Modifiers::NONE);
    harness.settle();
    assert_eq!(harness.workspace.editing.solid(), Some(extrude));

    harness.click_button(hole_panel::EDIT_SKETCH);
    harness.settle();
    assert_eq!(harness.editing(), Some(sketch));
}

#[test]
fn a_face_made_by_a_fillet_refuses_edit_the_sketch_in_words() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    open_fillet(&mut harness, plate, 0.0);
    finish(&mut harness);

    let rounded = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| {
            matches!(pickable, Pickable::Face { .. })
                && pickable
                    .describe(harness.document(), harness.model.evaluation())
                    .contains("Fillet 1")
        })
        .expect("the rounded face is pickable");
    harness.select([rounded]);
    harness.frame();

    let reason = offer(&harness, Command::EditSketch)
        .availability
        .expect_err("a rounded face has no sketch");

    assert!(reason.contains("Fillet 1"), "{reason}");
    assert!(reason.contains("not made from a sketch"), "{reason}");
    assert_eq!(harness.editing(), None);
}
