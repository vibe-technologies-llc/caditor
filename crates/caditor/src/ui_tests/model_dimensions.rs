use caditor_document::{Edit, FeatureId, SolidFeature};
use egui::{Event, Key, Modifiers};

use super::{CAMERA_SETTLE, Harness, distance_of, extruded_plate, run_from_palette};
use crate::{
    feature_values::ValueSlot, model::Action, model_dimensions, move_manipulator::Reach,
    selection::Pickable,
};

const WIDTH_LABEL: &str = "width = 40 mm";
const HEIGHT_LABEL: &str = "height = 20 mm";
const DISTANCE_LABEL: &str = "Distance 10 mm";
const HIGHLIGHT_STEPS: usize = 200;

fn show_dimensions(harness: &mut Harness) {
    run_from_palette(harness, "dimensions on the model");
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.frame();
    assert!(harness.workspace.viewport.dimensions_shown());
}

fn plate_with_distance_shown(harness: &mut Harness) -> FeatureId {
    let (extrude, top) = extruded_plate(harness);
    run_from_palette(harness, "fit view");
    show_dimensions(harness);
    harness.select([top]);
    harness.frame();
    harness.frame();
    extrude
}

fn distance_value(extrude: FeatureId) -> Pickable {
    Pickable::FeatureValue {
        feature: extrude,
        value: ValueSlot::Extrude(Reach::Only),
    }
}

fn enter(harness: &mut Harness, text: &str) {
    harness.events.push(Event::Text(text.to_owned()));
    harness.frame();
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.settle();
}

#[test]
fn show_dimensions_puts_the_shown_sketches_dimensions_on_the_model_and_edits_their_parameter() {
    let mut harness = Harness::new();
    harness.settle();
    run_from_palette(&mut harness, "fit view");
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    assert!(!harness.shows(WIDTH_LABEL));

    show_dimensions(&mut harness);
    assert!(harness.shows(WIDTH_LABEL));
    assert!(harness.shows(HEIGHT_LABEL));

    harness.double_click(WIDTH_LABEL);
    harness.frame();
    harness.key(Key::A, Modifiers::COMMAND);
    enter(&mut harness, "width = 50 mm");
    assert_eq!(harness.expression_text("width"), "50 mm");
    assert!(harness.shows("width = 50 mm"));
    assert!(harness.shows("height = 25 mm"));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.settle();
    assert_eq!(harness.expression_text("width"), "40 mm");

    run_from_palette(&mut harness, "dimensions on the model");
    harness.frame();
    assert!(!harness.workspace.viewport.dimensions_shown());
    assert!(!harness.shows(WIDTH_LABEL));
}

#[test]
fn a_selected_extrusion_shows_its_distance_on_the_model_and_a_double_click_changes_it_in_one_step()
{
    let mut harness = Harness::new();
    let extrude = plate_with_distance_shown(&mut harness);
    assert!(harness.shows(DISTANCE_LABEL));
    assert!(
        harness
            .workspace
            .viewport
            .model_labels()
            .contains(&distance_value(extrude))
    );

    harness.double_click(DISTANCE_LABEL);
    harness.frame();
    assert_eq!(
        harness.focused(),
        Some(model_dimensions::value_field_id(
            extrude,
            ValueSlot::Extrude(Reach::Only)
        ))
    );
    harness.key(Key::A, Modifiers::COMMAND);
    enter(&mut harness, "25");
    assert_eq!(distance_of(&harness, extrude), "25");
    assert_eq!(harness.model.undo_label(), Some("Edit Extrude 1"));
    assert!(harness.shows("Distance 25 mm"));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.settle();
    assert_eq!(distance_of(&harness, extrude), "10 mm");
}

#[test]
fn a_distance_driven_by_a_parameter_reads_name_equals_value_and_edits_the_parameter() {
    let mut harness = Harness::new();
    let extrude = plate_with_distance_shown(&mut harness);
    let mut transaction = harness.document().transaction("Name the depth");
    let depth = transaction.parse("12 mm").unwrap();
    let id = transaction.add_parameter("depth", depth);
    let SolidFeature::Extrude(mut extrusion) = harness.solid(extrude).clone() else {
        panic!("expected an extrusion");
    };
    extrusion.extent = caditor_document::ExtrudeExtent::one_side(
        caditor_expression::Expression::Parameter(id),
        false,
    );
    transaction.edit(Edit::SetFeatureKind {
        id: extrude,
        kind: caditor_document::FeatureKind::Solid(SolidFeature::Extrude(extrusion)),
    });
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    harness.frame();
    assert!(harness.shows("depth = 12 mm"));

    harness.double_click("depth = 12 mm");
    harness.frame();
    harness.key(Key::A, Modifiers::COMMAND);
    enter(&mut harness, "depth = 30 mm");
    assert_eq!(harness.expression_text("depth"), "30 mm");
    assert_eq!(distance_of(&harness, extrude), "depth");
    assert!(harness.shows("depth = 30 mm"));
}

#[test]
fn the_highlight_keys_reach_a_features_value_and_enter_edits_it() {
    let mut harness = Harness::new();
    let extrude = plate_with_distance_shown(&mut harness);
    let target = distance_value(extrude);

    for _ in 0..HIGHLIGHT_STEPS {
        if harness.workspace.viewport.keyboard_highlight() == Some(target) {
            break;
        }
        harness.key(Key::N, Modifiers::NONE);
        harness.frame();
    }
    assert_eq!(
        harness.workspace.viewport.keyboard_highlight(),
        Some(target)
    );

    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert_eq!(
        harness.focused(),
        Some(model_dimensions::value_field_id(
            extrude,
            ValueSlot::Extrude(Reach::Only)
        ))
    );
    harness.key(Key::A, Modifiers::COMMAND);
    enter(&mut harness, "18 mm");
    assert_eq!(distance_of(&harness, extrude), "18 mm");
}

#[test]
fn a_refused_value_keeps_the_field_open_and_changes_nothing() {
    let mut harness = Harness::new();
    let extrude = plate_with_distance_shown(&mut harness);
    let undo = harness.model.undo_label().map(str::to_owned);

    harness.double_click(DISTANCE_LABEL);
    harness.frame();
    harness.key(Key::A, Modifiers::COMMAND);
    enter(&mut harness, "-4");
    assert_eq!(distance_of(&harness, extrude), "10 mm");
    assert!(harness.shows_containing("Enter a value above zero"));
    assert_eq!(harness.model.undo_label().map(str::to_owned), undo);
}
