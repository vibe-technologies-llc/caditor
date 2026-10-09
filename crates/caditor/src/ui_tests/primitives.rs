use std::f64::consts::PI;

use caditor_document::{PrimitiveKind, PrimitiveShape};
use egui::{Id, Key, Modifiers};

use super::{Harness, primitive_of, run_from_palette};

fn near(found: f64, expected: f64) -> bool {
    (found - expected).abs() < 0.01 * expected
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
