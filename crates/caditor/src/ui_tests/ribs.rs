use caditor_document::{
    BodyOperation, Extrude, ExtrudeExtent, FeatureId, FeatureKind, FeatureState, RegionChoice, Rib,
    RibDirection, RibEnds, RibSide, Shell, ShellSide, SolidFeature,
};
use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::{FaceReference, Surface};
use caditor_sketch::Sketch;
use egui::{Id, Key, Modifiers};

use super::{
    CAMERA_SETTLE, Harness, choose, drag_value_arrow, length_value, pipes::undone_and_redone,
    rectangle, run_from_palette,
};
use crate::{
    model::Action, move_manipulator::Handle, rib_panel, selection::Pickable, value_gauges::Measured,
};

const BOX_VOLUME: f64 = 100.0 * 60.0 * 40.0 - 90.0 * 50.0 * 35.0;
const WEB_AREA: f64 = 50.0 * 35.0;

fn length(text: &str) -> Expression {
    Expression::parse_stored(text).unwrap()
}

fn open_box(harness: &mut Harness) -> FeatureId {
    let mut outline = Sketch::new(Plane::XY);
    rectangle(
        &mut outline,
        Point2::new(0.0, 0.0),
        Point2::new(100.0, 60.0),
    );
    let outline = harness.add_sketch(outline);
    let mut transaction = harness.document().transaction("Box");
    let base = transaction.add_feature(
        "Base",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(length("40 mm"), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    );
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    let solid = harness
        .model
        .evaluation()
        .body(base)
        .expect("the box is built");
    let (top, _) = solid
        .faces()
        .find(|(_, face)| match face.surface() {
            Surface::Plane(plane) => plane.frame().normal() * face.sense().sign() == Vector3::Z,
            _ => false,
        })
        .expect("the box has a top");
    let open = vec![FaceReference::capture(solid, top).unwrap()];
    let mut transaction = harness.document().transaction("Hollow");
    transaction.add_feature(
        "Shell 1",
        FeatureKind::Shell(Shell {
            body: base,
            open,
            thickness: length("5 mm"),
            side: ShellSide::Inside,
            faces: Vec::new(),
        }),
    );
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    base
}

fn rib_of(harness: &Harness, feature: FeatureId) -> Rib {
    harness
        .document()
        .feature(feature)
        .and_then(|feature| feature.kind.rib())
        .expect("a rib")
        .clone()
}

fn near(found: f64, expected: f64) -> bool {
    (found - expected).abs() < 1e-3 * expected
}

#[test]
fn rib_from_the_palette_follows_the_selected_line_and_turns_into_a_web_from_its_panel() {
    let mut harness = Harness::new();
    let body = open_box(&mut harness);
    let plane = Plane::new(Point3::new(0.0, 0.0, 40.0), Vector3::Z).unwrap();
    let mut drawn = Sketch::new(plane);
    let line = drawn.add_line(Point2::new(50.0, 15.0), Point2::new(50.0, 45.0));
    let sketch = harness.add_sketch(drawn);
    harness.select([Pickable::SketchEntity {
        feature: sketch,
        entity: line,
    }]);

    run_from_palette(&mut harness, "rib");
    harness.settle();

    let rib = harness.workspace.editing.solid().expect("the rib is open");
    assert_eq!(harness.model.undo_label(), Some("Create Rib 1"));
    let made = rib_of(&harness, rib);
    assert_eq!((made.sketch, made.body), (sketch, body));
    assert_eq!(made.direction, RibDirection::Parallel);
    assert_eq!(made.side, RibSide::Symmetric);
    assert!(harness.shows(rib_panel::THICKNESS));
    let reason = match harness
        .model
        .evaluation()
        .feature(rib)
        .map(|status| status.state.clone())
    {
        Some(FeatureState::Failed(error)) => error.reason,
        other => panic!("a rib lying in the open top runs past the box, found {other:?}"),
    };
    assert!(reason.contains("not closed off"), "{reason}");
    assert!(harness.shows(&reason));

    choose(&mut harness, "Parallel to sketch", "Square to sketch");

    let web = rib_of(&harness, rib);
    assert_eq!(web.direction, RibDirection::Square);
    assert!(web.reversed, "the web runs down into the box");
    assert!(harness.shows(rib_panel::REVERSE_DIRECTION));
    assert!(near(harness.body_volume(body), BOX_VOLUME + 2.0 * WEB_AREA));

    harness.type_into_field(Id::unique(("rib-field", rib, "thickness")), "4 mm");
    harness.settle();
    assert!(near(harness.body_volume(body), BOX_VOLUME + 4.0 * WEB_AREA));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.settle();
    assert_eq!(rib_of(&harness, rib).thickness, length("2 mm"));
    assert!(near(harness.body_volume(body), BOX_VOLUME + 2.0 * WEB_AREA));
}

fn open_web(harness: &mut Harness) -> (FeatureId, FeatureId) {
    let body = open_box(harness);
    let plane = Plane::new(Point3::new(0.0, 0.0, 40.0), Vector3::Z).unwrap();
    let mut drawn = Sketch::new(plane);
    let line = drawn.add_line(Point2::new(50.0, 15.0), Point2::new(50.0, 45.0));
    let sketch = harness.add_sketch(drawn);
    harness.select([Pickable::SketchEntity {
        feature: sketch,
        entity: line,
    }]);
    run_from_palette(harness, "rib");
    harness.settle();
    choose(harness, "Parallel to sketch", "Square to sketch");
    harness.select([]);
    run_from_palette(harness, "Fit view");
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    (
        harness.workspace.editing.solid().expect("the rib is open"),
        body,
    )
}

#[test]
fn dragging_a_web_s_thickness_arrow_changes_its_thickness_in_one_undoable_change() {
    let mut harness = Harness::new();
    let (rib, body) = open_web(&mut harness);
    let thickness = |harness: &Harness| length_value(harness, &rib_of(harness, rib).thickness);
    let before = thickness(&harness);

    let step = drag_value_arrow(&mut harness, Measured::RibThickness, 3.0);
    let after = thickness(&harness);

    assert_eq!(before, 2.0);
    assert_eq!(harness.model.undo_label(), Some("Edit Rib 1"));
    assert!(
        (after - before - 6.0).abs() <= 2.0 * step,
        "{before} to {after} with steps of {step}"
    );
    assert!(near(
        harness.body_volume(body),
        BOX_VOLUME + after * WEB_AREA
    ));
    undone_and_redone(&mut harness, "the thickness drag", thickness);

    harness.type_into_field(Id::unique(("rib-field", rib, "thickness")), "6 mm");
    harness.settle();
    assert_eq!(thickness(&harness), 6.0);
}

#[test]
fn a_rib_parallel_to_its_sketch_has_a_thickness_arrow_across_the_sketch() {
    let mut harness = Harness::new();
    let (rib, _) = open_web(&mut harness);

    choose(&mut harness, "Square to sketch", "Parallel to sketch");
    harness.select([]);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();

    assert_eq!(rib_of(&harness, rib).direction, RibDirection::Parallel);
    assert!(
        harness
            .workspace
            .viewport
            .handle_position(Handle::Value(Measured::RibThickness), 0.0)
            .is_some()
    );
}

#[test]
fn a_web_s_draft_typed_in_its_panel_widens_it_toward_the_floor_and_undoes_in_one_step() {
    let mut harness = Harness::new();
    let (rib, body) = open_web(&mut harness);
    let draft = Id::unique(("rib-field", rib, "draft"));
    let widening = 35.0 * 35.0 * 2.0_f64.to_radians().tan();

    let offered = harness.shows(rib_panel::DRAFT) && harness.shows(rib_panel::ENDS);
    harness.type_into_field(draft, "2 deg");
    harness.settle();
    let drafted = rib_of(&harness, rib);
    let drafted_volume = harness.body_volume(body);
    harness.type_into_field(draft, "30 deg");
    let refused = harness
        .shows("Enter an angle from 0° up to below 30°; the web widens by it toward the body");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.settle();

    assert!(offered);
    assert_eq!(drafted.draft, length("2 deg"));
    assert!(near(
        drafted_volume,
        BOX_VOLUME + 50.0 * (2.0 * 35.0 + widening)
    ));
    assert!(refused);
    assert_eq!(rib_of(&harness, rib).draft, length("0 deg"));
    assert!(near(harness.body_volume(body), BOX_VOLUME + 2.0 * WEB_AREA));
}

#[test]
fn a_web_s_free_ends_chosen_in_its_panel_carry_on_round_its_arcs_and_the_choice_undoes() {
    let mut harness = Harness::new();
    let (rib, body) = open_web(&mut harness);

    choose(&mut harness, "Straight on", "Round the arc");
    let round = rib_of(&harness, rib);
    let label = harness.model.undo_label().map(str::to_owned);
    let volume = harness.body_volume(body);
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.settle();

    assert_eq!(round.ends, RibEnds::RoundArcs);
    assert_eq!(label.as_deref(), Some("Edit Rib 1"));
    assert!(near(volume, BOX_VOLUME + 2.0 * WEB_AREA));
    assert_eq!(rib_of(&harness, rib).ends, RibEnds::Straight);
}

#[test]
fn a_rib_parallel_to_its_sketch_offers_no_draft_or_free_ends() {
    let mut harness = Harness::new();
    let (_, _) = open_web(&mut harness);

    choose(&mut harness, "Square to sketch", "Parallel to sketch");

    assert!(!harness.shows(rib_panel::DRAFT));
    assert!(!harness.shows(rib_panel::ENDS));
}
