use caditor_document::FeatureId;
use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;
use egui::{Key, Modifiers};

use super::{Harness, extruded_plate, rectangle, run_from_palette};
fn side_body_without_a_shape(harness: &mut Harness) -> FeatureId {
    extruded_plate(harness);
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::new(60.0, 0.0), Point2::new(80.0, 20.0));
    harness.add_sketch(sketch);
    harness.select([]);
    harness.model.hold_evaluations(true);
    harness.click_tool("Extrude");
    harness.frame();
    let side = harness
        .workspace
        .editing
        .solid()
        .expect("the extrusion is open");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    side
}

fn ask_to_move(harness: &mut Harness, body: FeatureId) {
    harness.workspace.panels.selected = Some(body);
    harness.workspace.panels.choose_only(body);
    run_from_palette(harness, "move body");
    harness.frame();
    harness.frame();
}

fn moves(harness: &Harness) -> usize {
    harness
        .document()
        .features()
        .filter(|feature| feature.name.starts_with("Move"))
        .count()
}

#[test]
fn a_command_refused_for_want_of_a_shape_runs_when_the_recompute_lands() {
    let mut harness = Harness::new();
    let side = side_body_without_a_shape(&mut harness);

    ask_to_move(&mut harness, side);
    let waiting = harness.shows_containing("waiting for the model to recompute");
    let meanwhile = moves(&harness);

    harness.model.hold_evaluations(false);
    harness.settle();

    assert!(waiting);
    assert_eq!(meanwhile, 0);
    assert_eq!(moves(&harness), 1);
    assert!(harness.workspace.editing.solid().is_some());
    assert!(!harness.shows("waiting for the model to recompute"));
}

#[test]
fn escape_drops_a_queued_command_so_the_recompute_landing_runs_nothing() {
    let mut harness = Harness::new();
    let side = side_body_without_a_shape(&mut harness);

    ask_to_move(&mut harness, side);
    let waiting = harness.shows_containing("waiting for the model to recompute");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let dropped = !harness.shows("waiting for the model to recompute");

    harness.model.hold_evaluations(false);
    harness.settle();

    assert!(waiting);
    assert!(dropped);
    assert_eq!(moves(&harness), 0);
    assert_eq!(harness.workspace.editing.solid(), None);
}
