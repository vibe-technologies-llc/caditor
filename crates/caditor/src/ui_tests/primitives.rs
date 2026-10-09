use std::f64::consts::PI;

use caditor_document::{PrimitiveKind, PrimitiveShape};
use egui::{Id, Key, Modifiers};

use super::{Harness, primitive_of, run_from_palette};

fn near(found: f64, expected: f64) -> bool {
    (found - expected).abs() < 0.01 * expected
}

fn drawn_length(harness: &mut Harness) -> Option<f64> {
    let built = harness.built();
    built
        .scene
        .meshes
        .iter()
        .chain(&built.scene.translucent_meshes)
        .find_map(|instance| instance.mesh.bounds())
        .map(|bounds| bounds.max().x - bounds.min().x)
}

#[test]
fn a_box_length_being_typed_is_drawn_before_it_is_entered() {
    let mut harness = Harness::new();
    harness.select([]);
    run_from_palette(&mut harness, "Box");
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let made = harness.workspace.editing.solid().expect("the box is open");
    let length = Id::new(("primitive-field", made, ("size", 0)));

    harness.draft_into_field(length, "45 mm");
    harness.wait_until("the typed length is drawn", |harness| {
        harness.model.draft_body_result().is_some()
    });
    harness.settle();

    assert_eq!(harness.model.undo_label(), Some("Create Box 1"));
    assert!(near(harness.body_volume(made), 20.0 * 20.0 * 10.0));
    assert!(drawn_length(&mut harness).is_some_and(|drawn| near(drawn, 45.0)));

    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.settle();

    assert_eq!(harness.model.undo_label(), Some("Edit Box 1"));
    assert!(near(harness.body_volume(made), 45.0 * 20.0 * 10.0));
    harness.wait_until("the preview gives way to the result", |harness| {
        harness.model.draft_evaluation().is_none()
    });
    assert!(drawn_length(&mut harness).is_some_and(|drawn| near(drawn, 45.0)));
}

#[test]
fn a_prism_and_a_cone_are_made_from_the_palette_and_sized_in_their_panel() {
    let mut harness = Harness::new();
    harness.select([]);

    run_from_palette(&mut harness, "Prism");
    harness.settle();
    let prism = harness
        .workspace
        .editing
        .solid()
        .expect("the prism is open");
    let hexagon = 3.0 * 3.0_f64.sqrt() / 2.0 * 100.0 * 10.0;

    assert_eq!(harness.model.undo_label(), Some("Create Prism 1"));
    assert_eq!(
        primitive_of(&harness, prism).shape.kind(),
        PrimitiveKind::Prism
    );
    assert!(near(harness.body_volume(prism), hexagon));

    let sides = Id::new(("primitive-field", prism, ("size", 0)));
    harness.type_into_field(sides, "8");
    harness.settle();
    let octagon = 2.0 * 2.0_f64.sqrt() * 100.0 * 10.0;

    assert!(near(harness.body_volume(prism), octagon));

    harness.type_into_field(sides, "2");
    harness.settle();

    assert!(harness.shows_containing("Enter a whole number from 3 to 64"));
    assert!(matches!(
        &primitive_of(&harness, prism).shape,
        PrimitiveShape::Prism { sides, .. } if harness.document().expression_text(sides) == "8"
    ));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.select([]);
    run_from_palette(&mut harness, "Cone");
    harness.settle();
    let cone = harness.workspace.editing.solid().expect("the cone is open");

    assert_eq!(harness.model.undo_label(), Some("Create Cone 1"));
    assert!(near(harness.body_volume(cone), PI * 100.0 * 20.0 / 3.0));
}
