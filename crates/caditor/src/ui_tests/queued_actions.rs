use caditor_document::{Blend, BlendKind, ChamferForm, FeatureId, FeatureKind, FilletSizing};
use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;
use egui::{Key, Modifiers};

use super::{Harness, extruded_plate, rectangle, run_from_palette};
use crate::files::{FileCommand, GuardChoice};

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

#[test]
fn a_queued_command_coming_due_under_an_open_dialog_runs_once_it_closes() {
    let mut harness = Harness::new();

    let side = side_body_without_a_shape(&mut harness);

    ask_to_move(&mut harness, side);
    harness.key(Key::Comma, Modifiers::COMMAND);
    harness.frame();
    harness.show_new_windows();

    let opened = harness.workspace.preferences_open;

    harness.model.hold_evaluations(false);
    harness.settle();

    let meanwhile = moves(&harness);

    harness.workspace.preferences_open = false;
    harness.settle();

    assert!(opened);
    assert_eq!(meanwhile, 0);
    assert_eq!(moves(&harness), 1);
}

fn holes(harness: &Harness) -> usize {
    harness
        .document()
        .features()
        .filter(|feature| matches!(feature.kind, FeatureKind::Hole(_)))
        .count()
}

#[test]
fn a_hole_on_a_face_of_a_body_still_recomputing_waits_for_it_instead_of_refusing() {
    let mut harness = Harness::new();

    let (plate, top) = extruded_plate(&mut harness);

    harness.select([top]);
    harness.model.hold_evaluations(true);

    let mut transaction = harness.document().transaction("Add a fillet");

    transaction.add_feature(
        "Fillet 1",
        FeatureKind::Blend(Blend {
            kind: BlendKind::Fillet,
            body: plate,
            edges: Vec::new(),
            groups: Vec::new(),
            size: harness.document().parse("1 mm").unwrap(),
            form: ChamferForm::Equal,
            flipped: false,
            sizing: FilletSizing::Radius,
        }),
    );
    harness.perform(crate::model::Action::Apply(transaction.finish()));
    harness.frame();

    run_from_palette(&mut harness, "hole");
    harness.frame();
    harness.frame();

    let waiting = harness.shows_containing("waiting for the model to recompute");
    let meanwhile = holes(&harness);

    harness.model.hold_evaluations(false);
    harness.settle();

    assert!(waiting);
    assert_eq!(meanwhile, 0);
    assert_eq!(holes(&harness), 1);
    assert!(!harness.shows_containing("not recomputed up to there"));
}

#[test]
fn quitting_drops_a_queued_command_so_the_recompute_landing_while_closing_changes_nothing() {
    let mut harness = Harness::new();

    let side = side_body_without_a_shape(&mut harness);

    ask_to_move(&mut harness, side);

    let waiting = harness.model.queued().is_some();

    harness.files.perform(FileCommand::Quit, &mut harness.model);
    harness.files.perform(
        FileCommand::Guard(GuardChoice::DontSave),
        &mut harness.model,
    );

    let dropped = harness.model.queued().is_none();
    let revision = harness.model.revision();

    harness.model.hold_evaluations(false);
    harness.wait_until("caditor quits", |harness| harness.files.should_quit());

    assert!(waiting);
    assert!(dropped);
    assert_eq!(moves(&harness), 0);
    assert_eq!(harness.model.revision(), revision);
}
