use std::f64::consts::PI;

use caditor_document::{EmbossForm, FeatureId};
use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Entity, Sketch};
use egui::{Key, Modifiers};

use super::{Harness, delete_face::holed_plate, rectangle, volume_about};
use crate::selection::Pickable;

const PLATE: f64 = 40.0 * 40.0 * 10.0;
const BORE: f64 = PI * 25.0 * 10.0;
const LABEL: f64 = 10.0 * 6.0;

fn emboss_of(harness: &Harness, feature: FeatureId) -> &caditor_document::Emboss {
    harness
        .document()
        .feature(feature)
        .and_then(|feature| feature.kind.emboss())
        .unwrap()
}

fn label(harness: &mut Harness) -> (FeatureId, Pickable) {
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::new(5.0, 5.0), Point2::new(15.0, 11.0));
    let edge = sketch
        .entities()
        .find(|(_, entity)| matches!(entity, Entity::Line { .. }))
        .map(|(id, _)| id)
        .unwrap();
    let feature = harness.add_sketch(sketch);
    (
        feature,
        Pickable::SketchEntity {
            feature,
            entity: edge,
        },
    )
}

#[test]
fn emboss_raises_a_sketch_from_a_face_and_recesses_it_when_asked() {
    let mut harness = Harness::new();
    let (plate, _, top) = holed_plate(&mut harness);
    let (label, edge) = label(&mut harness);
    assert!(volume_about(&harness, plate, PLATE - BORE));

    harness.select([top, edge]);
    harness.use_tool_with(Key::Y, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();
    let emboss = harness
        .workspace
        .editing
        .solid()
        .expect("the emboss is open");
    assert_eq!(harness.model.undo_label(), Some("Create Emboss 1"));
    assert_eq!(emboss_of(&harness, emboss).sketch, Some(label));
    assert_eq!(emboss_of(&harness, emboss).form, EmbossForm::Raised);
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert!(volume_about(&harness, plate, PLATE - BORE + LABEL * 0.5));
    assert!(harness.shows("Click faces to emboss, and again to leave one out"));
    assert!(harness.shows("Faces to emboss"));

    harness.click_button("Recess");
    harness.settle();
    assert_eq!(emboss_of(&harness, emboss).form, EmbossForm::Recessed);
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert!(volume_about(&harness, plate, PLATE - BORE - LABEL * 0.5));

    for _ in 0..2 {
        harness.key(Key::Escape, Modifiers::NONE);
        harness.frame();
        harness.frame();
    }
    assert_eq!(harness.workspace.editing.solid(), None);
}

#[test]
fn the_emboss_command_needs_a_face_and_undoes_in_one_step() {
    let mut harness = Harness::new();
    let (plate, _, top) = holed_plate(&mut harness);
    let (_, edge) = label(&mut harness);

    harness.select([edge]);
    harness.use_tool_with(Key::Y, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();
    assert_eq!(harness.workspace.editing.solid(), None);
    assert_ne!(harness.model.undo_label(), Some("Create Emboss 1"));

    harness.select([top, edge]);
    harness.use_tool_with(Key::Y, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();
    assert!(volume_about(&harness, plate, PLATE - BORE + LABEL * 0.5));
    for _ in 0..2 {
        harness.key(Key::Escape, Modifiers::NONE);
        harness.frame();
        harness.frame();
    }

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.settle();
    assert!(volume_about(&harness, plate, PLATE - BORE));
}
