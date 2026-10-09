use caditor_document::{Edit, ExtrudeEnd, ExtrudeExtent, FeatureId, SolidFeature, Transaction};
use caditor_expression::{Expression, ParameterId};
use caditor_geometry::Point3;
use egui::{Id, Key, Modifiers};

use super::{Harness, extruded_plate, offer, run_from_palette, top_edge_along_x, vertex_at};
use crate::{
    commands::Command, editing::EditingCommand, measure_panel, model::Action, panels::Focus,
    parameter_table, selection::Pickable, units::LengthUnit,
};

fn distance_text(harness: &Harness, extrude: FeatureId) -> String {
    match harness.solid(extrude) {
        SolidFeature::Extrude(caditor_document::Extrude {
            extent:
                ExtrudeExtent::OneSide {
                    end: ExtrudeEnd::Distance(distance),
                    ..
                },
            ..
        }) => harness.document().expression_text(distance),
        _ => String::new(),
    }
}

fn distance_field(extrude: FeatureId) -> Id {
    Id::new(("solid-field", "distance", extrude))
}

fn notice(harness: &Harness) -> String {
    harness
        .model
        .notice()
        .map(|notice| notice.text.clone())
        .unwrap_or_default()
}

fn add_parameters(harness: &mut Harness, rows: &[(&str, &str)]) -> Vec<ParameterId> {
    let mut ids = Vec::new();
    for (name, text) in rows {
        let document = harness.document();
        let expression = Expression::parse(text, &|name| {
            document
                .parameter_named(name)
                .map(|parameter| parameter.id())
        })
        .unwrap();
        let mut transaction = document.transaction(format!("Add {name}"));
        ids.push(transaction.add_parameter(*name, expression));
        harness.perform(Action::Apply(transaction.finish()));
    }
    harness.settle();
    ids
}

#[test]
fn cancelling_a_new_fillet_takes_it_away_and_redo_brings_it_back() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let front = top_edge_along_x(&harness, plate, 0.0);
    harness.select([Pickable::Edge {
        body: plate,
        edge: front,
    }]);
    let before = harness.model.undo_label().map(str::to_owned);

    harness.click("Fillet");
    harness.settle();
    let fillet = harness
        .workspace
        .editing
        .solid()
        .expect("the fillet is open");
    harness.type_into_field(Id::new(("blend-size", fillet)), "3 mm");
    harness.settle();
    let edited = harness.model.undo_label().map(str::to_owned);
    harness.click_button("Cancel the new Fillet 1");
    harness.settle();

    assert_eq!(edited.as_deref(), Some("Edit Fillet 1"));
    assert!(harness.document().feature(fillet).is_none());
    assert_eq!(harness.workspace.editing.solid(), None);
    assert_eq!(harness.model.undo_label().map(str::to_owned), before);
    assert_eq!(harness.model.redo_label(), Some("Create Fillet 1"));

    harness.perform(Action::Redo);
    harness.perform(Action::Redo);
    harness.settle();

    assert!(harness.document().feature(fillet).is_some());
    assert_eq!(harness.model.undo_label(), Some("Edit Fillet 1"));
}

#[test]
fn cancelling_an_opened_feature_undoes_its_changes_and_its_named_value_but_keeps_it() {
    let mut harness = Harness::new();
    let (extrude, _) = extruded_plate(&mut harness);
    let original = distance_text(&harness, extrude);
    let before = harness.model.undo_label().map(str::to_owned);

    harness.perform(Action::Editing(EditingCommand::OpenSolid(extrude)));
    harness.frame();
    let unchanged = offer(&harness, Command::CancelFeature).availability;
    harness.type_into_field(distance_field(extrude), "depth = 12 mm");
    harness.settle();
    harness.type_into_field(distance_field(extrude), "depth = 14 mm");
    harness.settle();
    let named = harness.document().parameter_named("depth").is_some();
    harness.perform(Action::Editing(EditingCommand::OpenSolid(extrude)));
    run_from_palette(&mut harness, "Cancel the changes");
    harness.settle();

    assert!(unchanged.is_ok());
    assert!(named);
    assert!(harness.document().feature(extrude).is_some());
    assert!(harness.document().parameter_named("depth").is_none());
    assert_eq!(distance_text(&harness, extrude), original);
    assert_eq!(harness.workspace.editing.solid(), None);
    assert_eq!(harness.model.undo_label().map(str::to_owned), before);
    assert_eq!(harness.model.redo_label(), Some("Name depth"));
}

#[test]
fn cancelling_with_nothing_changed_only_closes_the_feature() {
    let mut harness = Harness::new();
    let (extrude, _) = extruded_plate(&mut harness);
    let revision = harness.model.revision();

    harness.perform(Action::Editing(EditingCommand::OpenSolid(extrude)));
    harness.frame();
    harness.click_button("Cancel the changes to Extrude 1");
    harness.frame();

    assert_eq!(harness.workspace.editing.solid(), None);
    assert_eq!(harness.model.revision(), revision);
    assert!(harness.document().feature(extrude).is_some());
}

#[test]
fn cancel_refuses_in_words_when_another_change_was_made_meanwhile() {
    let mut harness = Harness::new();
    let (extrude, _) = extruded_plate(&mut harness);

    harness.perform(Action::Editing(EditingCommand::OpenSolid(extrude)));
    harness.type_into_field(distance_field(extrude), "12 mm");
    harness.settle();
    add_parameters(&mut harness, &[("gap", "2 mm")]);
    harness.perform(Action::Editing(EditingCommand::OpenSolid(extrude)));
    harness.frame();
    let offered = offer(&harness, Command::CancelFeature).availability;
    harness.click_button("Cancel the changes to Extrude 1");
    harness.frame();

    assert!(offered.is_err());
    assert!(
        notice(&harness).contains("\"Add gap\", made while Extrude 1 was open, changed more"),
        "{}",
        notice(&harness)
    );
    assert_eq!(harness.workspace.editing.solid(), Some(extrude));
    assert_eq!(distance_text(&harness, extrude), "12 mm");
    assert!(harness.document().parameter_named("gap").is_some());
}

#[test]
fn cancel_refuses_once_undo_went_back_past_where_the_feature_was_opened() {
    let mut harness = Harness::new();
    let (extrude, _) = extruded_plate(&mut harness);
    add_parameters(&mut harness, &[("gap", "2 mm")]);

    harness.perform(Action::Editing(EditingCommand::OpenSolid(extrude)));
    harness.perform(Action::Undo);
    harness.settle();
    harness.perform(Action::Editing(EditingCommand::OpenSolid(extrude)));
    harness.frame();
    harness.click_button("Cancel the changes to Extrude 1");
    harness.frame();

    assert!(
        notice(&harness).contains("the undo history no longer goes back"),
        "{}",
        notice(&harness)
    );
    assert_eq!(harness.workspace.editing.solid(), Some(extrude));
}

#[test]
fn the_parameter_filter_keeps_rows_matching_a_name_or_expression() {
    let mut harness = Harness::new();
    let ids = add_parameters(
        &mut harness,
        &[
            ("span", "40 mm"),
            ("rise", "10 mm"),
            ("depth", "5 mm"),
            ("gap", "span / 4"),
            ("wall", "2 mm"),
            ("pitch", "7 mm"),
        ],
    );
    let span = ids.first().copied().unwrap();
    harness.focus(Focus::ParameterName(span));
    let filter_shown = harness.shows(parameter_table::FILTER_HINT);

    harness.type_into_field(Id::new("parameter-filter"), "SPAN");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert!(filter_shown);
    assert_eq!(harness.workspace.panels.parameter_filter, "SPAN");
    assert!(harness.shows("span"));
    assert!(harness.shows("gap"));
    assert!(!harness.shows("rise"));
    assert!(!harness.shows("width"));

    harness.type_into_field(Id::new("parameter-filter"), "nothing like it");
    harness.frame();
    assert!(harness.shows(parameter_table::CLEAR_FILTER_LABEL));
    harness.click(parameter_table::CLEAR_FILTER_LABEL);
    harness.frame();
    harness.frame();
    assert!(harness.shows("rise"));
    assert!(harness.workspace.panels.parameter_filter.is_empty());
}

#[test]
fn unused_parameters_are_deleted_in_one_undoable_change() {
    let mut harness = Harness::new();
    let ids = add_parameters(
        &mut harness,
        &[("lonely", "1 mm"), ("base", "4 mm"), ("double", "base * 2")],
    );
    let lonely = ids.first().copied().unwrap();
    harness.focus(Focus::ParameterName(lonely));
    let label = parameter_table::delete_unused_label(2);

    harness.click(&label);
    harness.settle();

    assert!(harness.document().parameter_named("lonely").is_none());
    assert!(harness.document().parameter_named("double").is_none());
    assert!(harness.document().parameter_named("base").is_some());
    assert_eq!(harness.model.undo_label(), Some(label.as_str()));
    assert!(
        notice(&harness).contains("lonely and double"),
        "{}",
        notice(&harness)
    );

    harness.perform(Action::Undo);
    harness.settle();
    assert!(harness.document().parameter_named("lonely").is_some());
    assert!(harness.document().parameter_named("double").is_some());

    harness.perform(Action::Apply(Transaction::new(
        "Keep only base",
        vec![
            Edit::RemoveParameter { id: lonely },
            Edit::RemoveParameter {
                id: ids.get(2).copied().unwrap(),
            },
        ],
    )));
    harness.settle();
    harness.frame();
    let base_only = offer(&harness, Command::DeleteUnusedParameters).availability;
    assert!(base_only.is_ok(), "base is unused once double is gone");
}

#[test]
fn deleting_unused_parameters_is_unavailable_when_every_one_is_in_use() {
    let mut harness = Harness::new();
    let (extrude, _) = extruded_plate(&mut harness);
    harness.perform(Action::Editing(EditingCommand::OpenSolid(extrude)));
    harness.type_into_field(distance_field(extrude), "depth = 12 mm");
    harness.settle();
    harness.frame();

    let refused = offer(&harness, Command::DeleteUnusedParameters).availability;

    assert!(refused.is_err());
}

#[test]
fn a_model_parameter_s_owner_line_goes_to_its_feature() {
    let mut harness = Harness::new();
    let (extrude, _) = extruded_plate(&mut harness);
    harness.perform(Action::Editing(EditingCommand::OpenSolid(extrude)));
    harness.type_into_field(distance_field(extrude), "depth = 12 mm");
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let depth = harness.parameter("depth");
    harness.focus(Focus::ParameterName(depth));
    harness.workspace.panels.choose_nothing();
    harness.context.enable_accesskit();
    harness.frame();
    let linked = harness.accessible_named(egui::accesskit::Role::Link, "Extrude 1 · Distance");

    harness.click("Extrude 1 · Distance");
    harness.frame();
    harness.frame();

    assert!(linked);
    assert_eq!(harness.workspace.panels.selected, Some(extrude));
}

#[test]
fn a_measured_distance_is_copied_or_made_a_parameter_from_its_row_menu() {
    let mut harness = Harness::new();
    let (body, _) = extruded_plate(&mut harness);
    let far_point = Point3::new(40.0, 40.0, 10.0);
    let corner = vertex_at(&harness, body, Point3::ZERO);
    let far = vertex_at(&harness, body, far_point);
    harness.key(Key::I, Modifiers::NONE);
    harness.frame();
    harness.select([corner, far]);
    let distance = Point3::ZERO.distance(far_point);
    let expected = LengthUnit::Millimetre.measured_length(distance);
    harness.wait_until("the distance is measured", |harness| {
        harness.shows(&expected)
    });

    harness.click_button("More for Distance");
    harness.show_new_windows();
    harness.click(measure_panel::COPY_VALUE);
    harness.frame();
    assert_eq!(harness.clipboard.as_deref(), Some(expected.as_str()));

    harness.click_button("More for Distance");
    harness.show_new_windows();
    harness.click(measure_panel::NEW_PARAMETER);
    harness.settle();
    harness.frame();
    harness.frame();

    let parameter = harness.parameter("distance1");
    let shown = harness
        .document()
        .expression_text(&LengthUnit::Millimetre.measured(distance));
    assert_eq!(harness.expression_text("distance1"), shown);
    assert_eq!(harness.model.undo_label(), Some("Add distance1"));
    assert_eq!(
        harness.focused(),
        Some(Focus::ParameterName(parameter).field_id())
    );
    assert!(harness.model.revision() > 0);
}

#[test]
fn a_model_parameter_s_owner_line_never_widens_the_panel() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    harness.select([top]);
    harness.click("Move body");
    harness.settle();
    let movement = harness.workspace.editing.solid().expect("the move is open");
    harness.type_into_field(Id::new(("move-field", "offset", 0, movement)), "dx = 5 mm");
    harness.settle();
    harness.frame();
    let before = harness.workspace.viewport.rect();

    for step in 0..5 {
        let across = 1000.0 + 40.0 * step as f32;
        harness
            .events
            .push(egui::Event::PointerMoved(egui::pos2(across, 500.0)));
        harness.frame();
    }

    assert_eq!(harness.workspace.viewport.rect(), before);
}
