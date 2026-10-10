use caditor_document::{ExtrudeEnd, ExtrudeExtent, FeatureId, SolidFeature, Transaction};
use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;
use egui::{Event, Id, Key, Modifiers, accesskit::Role};

use super::{Harness, edit_base_sketch, rectangle};
use crate::{model::Action, panels::Focus};

const WIDTH_LABEL: &str = "width = 40 mm";

fn with_parameters(harness: &mut Harness) {
    let mut transaction = harness.document().transaction("Add parameters");
    let wall = transaction.parse("3 mm").unwrap();
    transaction.add_parameter("wall", wall);
    let hole = transaction.parse("5 mm").unwrap();
    transaction.add_parameter("hole", hole);
    let transaction: Transaction = transaction.finish();
    harness.perform(Action::Apply(transaction));
    harness.settle();
}

fn open_extrusion(harness: &mut Harness) -> FeatureId {
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::new(0.0, 0.0), Point2::new(40.0, 40.0));
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    harness.frame();
    harness
        .workspace
        .editing
        .solid()
        .expect("the extrusion is open")
}

fn distance_field(extrude: FeatureId) -> Id {
    Id::new(("solid-field", "distance", extrude))
}

fn end_distance(harness: &Harness, extrude: FeatureId) -> Expression {
    match harness.solid(extrude) {
        SolidFeature::Extrude(extrude) => match &extrude.extent {
            ExtrudeExtent::OneSide {
                end: ExtrudeEnd::Distance(distance),
                ..
            } => distance.clone(),
            other => panic!("the extrusion runs a distance one way, not {other:?}"),
        },
        SolidFeature::Revolve(_) => panic!("the feature is an extrusion"),
    }
}

fn suggestions(harness: &mut Harness) -> Vec<String> {
    harness.read_screen();
    harness
        .accessible
        .iter()
        .filter(|(_, node)| node.role() == Role::ListBoxOption)
        .filter_map(|(_, node)| node.label().map(str::to_owned))
        .collect()
}

#[test]
fn typing_a_name_lists_the_parameters_it_starts_and_tab_inserts_the_chosen_one() {
    let mut harness = Harness::new();
    with_parameters(&mut harness);
    let extrude = open_extrusion(&mut harness);

    harness.draft_into_field(distance_field(extrude), "w");

    assert_eq!(suggestions(&mut harness), ["wall, 3 mm", "width, 40 mm"]);
    assert!(harness.accessible_named(Role::ListBox, crate::completion::LIST_NAME));

    harness.key(Key::ArrowDown, Modifiers::NONE);
    harness.frame();
    harness.key(Key::Tab, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert!(suggestions(&mut harness).is_empty());
    assert_eq!(harness.focused(), Some(distance_field(extrude)));

    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.settle();

    assert_eq!(
        end_distance(&harness, extrude),
        Expression::Parameter(harness.parameter("width"))
    );
}

#[test]
fn up_and_down_choose_a_suggestion_instead_of_stepping_and_enter_without_a_choice_commits() {
    let mut harness = Harness::new();
    with_parameters(&mut harness);
    let extrude = open_extrusion(&mut harness);

    harness.draft_into_field(distance_field(extrude), "w");
    harness.key(Key::ArrowUp, Modifiers::NONE);
    harness.frame();

    assert_eq!(suggestions(&mut harness).len(), 2);
    assert!(!harness.shows_containing(crate::stepping::Refusal::NotPlain.message()));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert!(harness.shows_containing("no parameter named 'w'"));
}

#[test]
fn escape_closes_the_suggestions_and_keeps_the_text_and_focus() {
    let mut harness = Harness::new();
    with_parameters(&mut harness);
    let extrude = open_extrusion(&mut harness);

    harness.draft_into_field(distance_field(extrude), "wa");
    assert_eq!(suggestions(&mut harness), ["wall, 3 mm"]);

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert!(suggestions(&mut harness).is_empty());
    assert_eq!(harness.focused(), Some(distance_field(extrude)));

    harness.type_text("l");

    assert_eq!(suggestions(&mut harness), ["wall, 3 mm"]);
}

#[test]
fn a_name_that_would_form_a_cycle_is_never_suggested() {
    let mut harness = Harness::new();
    with_parameters(&mut harness);
    edit_base_sketch(&mut harness);
    let width = Focus::ParameterValue(harness.parameter("width"));

    harness.focus(width);
    harness.type_text("h");

    assert_eq!(suggestions(&mut harness), ["hole, 5 mm"]);

    let hole = Focus::ParameterValue(harness.parameter("hole"));
    harness.focus(hole);
    harness.type_text("he");

    assert_eq!(suggestions(&mut harness), ["height, 20 mm"]);
}

#[test]
fn clicking_a_suggestion_inserts_it_without_leaving_the_field() {
    let mut harness = Harness::new();
    with_parameters(&mut harness);
    let extrude = open_extrusion(&mut harness);

    harness.draft_into_field(distance_field(extrude), "wa");
    harness.read_screen();
    let row = harness
        .accessible
        .iter()
        .find(|(_, node)| node.label() == Some("wall, 3 mm"))
        .map(|(_, node)| node.bounds().unwrap())
        .unwrap();
    let centre = egui::pos2(
        ((row.x0 + row.x1) / 2.0) as f32,
        ((row.y0 + row.y1) / 2.0) as f32,
    );
    harness.click_screen(centre);
    harness.frame();

    assert_eq!(harness.focused(), Some(distance_field(extrude)));
    assert_eq!(harness.model.undo_label(), Some("Create Extrude 1"));

    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.settle();

    assert_eq!(
        end_distance(&harness, extrude),
        Expression::Parameter(harness.parameter("wall"))
    );
}

#[test]
fn clicking_a_dimension_in_the_view_inserts_its_name_instead_of_selecting_it() {
    let mut harness = Harness::new();
    edit_base_sketch(&mut harness);
    let height = Focus::ParameterValue(harness.parameter("height"));

    harness.focus(height);
    harness.type_text("2 * ");
    let label = harness.position_of(WIDTH_LABEL);
    harness.events.push(Event::PointerMoved(label));
    harness.frame();
    harness.frame();
    harness.press(label);
    harness.frame();

    assert!(harness.workspace.viewport.selection().is_empty());
    assert_eq!(harness.focused(), Some(height.field_id()));

    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.settle();

    assert_eq!(harness.expression_text("height"), "2 * width");
    assert!(harness.workspace.viewport.selection().is_empty());
}

#[test]
fn a_dimension_click_with_no_field_focused_still_selects_it() {
    let mut harness = Harness::new();
    edit_base_sketch(&mut harness);

    harness.click(WIDTH_LABEL);
    harness.frame();

    assert!(!harness.workspace.viewport.selection().is_empty());
}

fn with_derived_parameters(harness: &mut Harness) {
    let mut transaction = harness.document().transaction("Add parameters");
    let wall = transaction.parse("3 mm").unwrap();
    transaction.add_parameter("wall", wall);
    let hole = transaction.parse("wall + 2 mm").unwrap();
    transaction.add_parameter("hole", hole);
    let gap = transaction.parse("7 mm").unwrap();
    transaction.add_parameter("gap", gap);
    let transaction: Transaction = transaction.finish();
    harness.perform(Action::Apply(transaction));
    harness.settle();
}

fn press_value(harness: &mut Harness, shown: &str) {
    let value = harness.position_of(shown);
    harness.events.push(Event::PointerMoved(value));
    harness.frame();
    harness.frame();
    harness.press(value);
    harness.frame();
}

#[test]
fn clicking_a_value_in_the_parameters_panel_inserts_its_name_into_the_focused_field() {
    let mut harness = Harness::new();
    with_derived_parameters(&mut harness);
    let gap = Focus::ParameterValue(harness.parameter("gap"));

    harness.focus(gap);
    harness.type_text("2 * ");
    press_value(&mut harness, "5 mm");

    assert_eq!(harness.focused(), Some(gap.field_id()));

    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.settle();

    assert_eq!(harness.expression_text("gap"), "2 * hole");
}

#[test]
fn a_parameter_value_that_would_form_a_cycle_is_not_inserted() {
    let mut harness = Harness::new();
    with_derived_parameters(&mut harness);
    let wall = Focus::ParameterValue(harness.parameter("wall"));

    harness.focus(wall);
    harness.type_text("2 * ");
    press_value(&mut harness, "5 mm");

    assert_ne!(harness.focused(), Some(wall.field_id()));
    assert_eq!(harness.expression_text("hole"), "wall + 2 mm");
}
