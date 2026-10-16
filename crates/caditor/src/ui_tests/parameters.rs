use caditor_document::{ExtrudeExtent, SolidFeature};
use egui::{Event, Id, Key, Modifiers, PointerButton};

use super::{
    Harness, assert_failure_shown_once, choose, extruded_plate, feature_named, offer,
    run_from_palette,
};
use crate::{
    commands::Command, editing::EditingCommand, files::FileCommand, model::Action, panels::Focus,
};

#[test]
fn editing_parameters_breaking_a_feature_and_undoing_it_works_through_the_panels() {
    let mut harness = Harness::new();
    assert!(harness.shows("Up to date"));
    let width = harness.parameter("width");
    let height = harness.parameter("height");

    harness.type_into(
        Focus::ParameterValue(height),
        "400 mm * 1 mm / (width - 30 mm)",
    );
    assert_eq!(
        harness.expression_text("height"),
        "400 mm * 1 mm / (width - 30 mm)"
    );
    harness.settle();
    assert!(harness.shows("40 mm"));

    harness.type_into(Focus::ParameterValue(width), "30 mm");
    harness.settle();
    assert!(harness.shows("1 feature failed"));
    assert_failure_shown_once(&harness, "Side sketch");
    assert!(harness.shows(
        "Distance between Point 0 and Point 1 cannot be evaluated: it uses height, which has an \
         error."
    ));

    harness.click("Go to height");
    harness.frame();
    assert_eq!(
        harness.focused(),
        Some(Focus::ParameterValue(height).field_id())
    );

    harness.type_into(Focus::ParameterValue(height), "wdth");
    assert!(harness.shows("There is no parameter named 'wdth'"));
    assert_eq!(
        harness.expression_text("height"),
        "400 mm * 1 mm / (width - 30 mm)"
    );

    harness.focus(Focus::ParameterValue(height));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert!(!harness.shows("There is no parameter named 'wdth'"));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert_eq!(harness.expression_text("width"), "40 mm");
    harness.settle();
    assert!(harness.shows("Up to date"));
    assert_eq!(harness.model.undo_label(), Some("Edit height"));
}

#[test]
fn a_rejected_draft_gives_way_to_a_value_changed_by_undo() {
    let mut harness = Harness::new();
    let height = harness.parameter("height");
    let original = harness.expression_text("height");

    harness.type_into(Focus::ParameterValue(height), "30 mm");
    harness.settle();
    harness.type_into(Focus::ParameterValue(height), "wdth");
    let rejected = harness.shows("There is no parameter named 'wdth'");
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.frame();

    assert!(rejected);
    assert_eq!(harness.expression_text("height"), original);
    assert!(!harness.shows("There is no parameter named 'wdth'"));
    assert!(!harness.shows("wdth"));
}

#[test]
fn a_dimension_edited_in_the_tree_is_undoable_and_rejects_the_wrong_kind() {
    let mut harness = Harness::new();
    let base = harness.document().features().next().unwrap().id();
    let sketch = harness
        .document()
        .feature(base)
        .unwrap()
        .kind
        .sketch()
        .unwrap();
    let (constraint, _) = sketch
        .constraints()
        .find(|(_, constraint)| constraint.dimension().is_some())
        .unwrap();
    let focus = Focus::Dimension {
        feature: base,
        constraint,
    };

    harness.type_into(focus, "width * width");
    assert!(harness.shows("It gives an area, but a length is needed"));

    harness.type_into(focus, "width / 4");
    harness.settle();
    assert!(harness.shows("= 10 mm"));
    assert_eq!(
        harness.model.undo_label(),
        Some("Edit dimension in Base sketch")
    );
}

#[test]
fn a_parameter_is_deleted_and_a_failed_feature_found_from_the_keyboard() {
    let mut harness = Harness::new();
    assert_eq!(
        offer(&harness, Command::DeleteParameter).availability,
        Err(
            "Click or tab into a parameter's name or expression in the Parameters section first"
                .into()
        )
    );
    let width = harness.parameter("width");
    harness.focus(Focus::ParameterValue(width));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let offered = offer(&harness, Command::DeleteParameter);
    assert_eq!(offered.title(), "Delete parameter: width");
    assert!(offered.availability.is_ok());

    run_from_palette(&mut harness, "add parameter");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert!(harness.document().parameter_named("parameter1").is_some());
    run_from_palette(&mut harness, "delete parameter");
    harness.settle();
    assert!(harness.document().parameter_named("parameter1").is_none());
    assert_eq!(harness.model.undo_label(), Some("Delete parameter1"));

    assert!(
        offer(&harness, Command::ShowNextFailed)
            .availability
            .is_err()
    );
    let height = harness.parameter("height");
    harness.type_into(
        Focus::ParameterValue(height),
        "400 mm * 1 mm / (width - 30 mm)",
    );
    harness.type_into(Focus::ParameterValue(width), "30 mm");
    harness.settle();
    assert!(harness.shows("1 feature failed"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.key(Key::F8, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert_eq!(
        harness.workspace.panels.selected,
        Some(feature_named(&harness, "Side sketch"))
    );
}

#[test]
fn a_used_parameter_is_deleted_by_writing_its_expression_into_its_uses() {
    let mut harness = Harness::new();
    assert!(!harness.shows(crate::icons::DELETE));
    harness.hover_button("Delete width");
    let explained = harness.shows_containing("Delete width and write 40 mm in its place")
        && harness.shows_containing("Used by height and Base sketch.");

    let mut transaction = harness.document().transaction("Add spare");
    transaction.add_parameter("spare", transaction.parse("3 mm").unwrap());
    harness.perform(Action::Apply(transaction.finish()));
    harness.frame();
    let spare_marked = harness.describes("spare is unused: nothing refers to it yet.");
    let width_marked = harness.describes("width is unused: nothing refers to it yet.");
    harness.click_button("Delete width");
    harness.settle();
    let document = harness.document();
    let height = document.parameter_named("height").unwrap();
    let base = feature_named(&harness, "Base sketch");
    let base_value = document
        .feature(base)
        .and_then(|feature| feature.kind.sketch())
        .and_then(|sketch| sketch.constraints().find_map(|(_, held)| held.dimension()))
        .map(|value| document.expression_text(value));

    assert!(explained);
    assert!(spare_marked);
    assert!(!width_marked);
    assert!(document.parameter_named("width").is_none());
    assert_eq!(document.expression_text(&height.expression), "40 mm / 2");
    assert_eq!(base_value.as_deref(), Some("40 mm"));
    assert_eq!(harness.model.undo_label(), Some("Delete width"));
    assert_eq!(
        harness.model.notice().map(|notice| notice.text.as_str()),
        Some("Deleted width and wrote 40 mm into its 2 uses. Undo brings it back.")
    );
    assert_eq!(harness.model.evaluation().failed_count(), 0);

    harness.perform(Action::Undo);
    harness.settle();
    assert!(harness.document().parameter_named("width").is_some());
}

#[test]
fn a_parameter_s_menu_lists_what_uses_it_and_goes_to_it() {
    let mut harness = Harness::new();
    harness.command(FileCommand::OpenSample(crate::samples::Sample::Bracket));
    harness.settle();
    let hole_sketch = harness
        .document()
        .features()
        .find(|feature| feature.name == "Hole sketch")
        .map(caditor_document::Feature::id)
        .unwrap();
    let spot = harness.position_of("hole");

    harness.events.push(Event::PointerMoved(spot));
    harness.frame();
    for pressed in [true, false] {
        harness.events.push(Event::PointerButton {
            pos: spot,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        harness.frame();
    }
    harness.show_new_windows();
    let listed = harness.shows(crate::parameter_table::USED_BY);
    harness.click("Hole sketch");
    harness.frame();
    harness.frame();

    assert!(listed);
    assert_eq!(harness.workspace.panels.selected, Some(hole_sketch));
}

#[test]
fn parameters_are_reordered_and_noted_from_the_keyboard() {
    let mut harness = Harness::new();
    let order = |harness: &Harness| -> Vec<String> {
        harness
            .document()
            .parameters()
            .iter()
            .map(|parameter| parameter.name.clone())
            .collect()
    };
    let height = harness.parameter("height");
    harness.focus(Focus::ParameterName(height));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    let at_bottom = offer(&harness, Command::MoveParameterDown).availability;
    run_from_palette(&mut harness, "move parameter up");
    harness.frame();
    let moved = order(&harness);
    let at_top = offer(&harness, Command::MoveParameterUp).availability;
    run_from_palette(&mut harness, "parameter's note");
    harness.frame();
    let focused = harness.focused() == Some(Id::unique("parameter-note-field"));
    harness.type_text("Half the width, for the side");
    harness.click(crate::parameter_table::SAVE_NOTE_LABEL);
    harness.context.enable_accesskit();
    harness.frame();
    harness.frame();

    assert_eq!(
        at_bottom,
        Err("height is already the last parameter".into())
    );
    assert_eq!(moved, ["height", "width"]);
    assert_eq!(at_top, Err("height is already the first parameter".into()));
    assert!(focused);
    assert_eq!(
        harness.document().parameter(height).unwrap().note,
        "Half the width, for the side"
    );
    assert!(harness.describes("Note on height: Half the width, for the side"));
    assert_eq!(harness.model.undo_label(), Some("Note on height"));
}

#[test]
fn values_named_in_their_fields_are_listed_as_model_parameters_and_used_by_name() {
    let mut harness = Harness::new();
    let base = harness.document().features().next().unwrap().id();
    let (constraint, _) = harness
        .sketch(base)
        .constraints()
        .find(|(_, constraint)| constraint.dimension().is_some())
        .map(|(id, constraint)| (id, constraint.clone()))
        .unwrap();
    let dimension = Focus::Dimension {
        feature: base,
        constraint,
    };
    let (extrude, _) = extruded_plate(&mut harness);
    let distance = Id::unique(("solid-field", "distance", extrude));
    let held = |harness: &Harness| match &harness.solid(extrude) {
        SolidFeature::Extrude(caditor_document::Extrude {
            extent:
                caditor_document::ExtrudeExtent::OneSide {
                    end: caditor_document::ExtrudeEnd::Distance(distance),
                    ..
                },
            ..
        }) => harness.document().expression_text(distance),
        _ => String::new(),
    };

    harness.perform(Action::Editing(EditingCommand::OpenSolid(extrude)));
    harness.type_into_field(distance, "depth = 12 mm");
    harness.settle();
    let depth = harness.parameter("depth");
    let named_label = harness.model.undo_label().map(str::to_owned);
    let shown_in_field = harness.shows("depth = 12 mm");
    harness.type_into(dimension, "length = depth * 2");
    harness.settle();
    let length = harness.parameter("length");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.focus(Focus::ParameterName(length));
    harness.frame();
    let listed = harness.shows(crate::parameter_table::MODEL_PARAMETERS)
        && harness.shows("Extrude 1 · Distance")
        && harness.shows("Base sketch · Distance");

    assert_eq!(named_label.as_deref(), Some("Name depth"));
    assert!(shown_in_field);
    assert_eq!(held(&harness), "depth");
    assert!(listed);
    assert!(harness.shows("length = depth * 2"));
    assert_eq!(harness.expression_text("length"), "depth * 2");

    harness.type_into(Focus::ParameterName(depth), "deep");
    harness.settle();
    harness.perform(Action::Editing(EditingCommand::OpenSolid(extrude)));
    harness.type_into_field(distance, "deep = 15 mm");
    harness.settle();

    assert_eq!(harness.expression_text("length"), "deep * 2");
    assert_eq!(held(&harness), "deep");
    assert_eq!(harness.expression_text("deep"), "15 mm");
    assert_eq!(harness.model.undo_label(), Some("Edit deep"));

    harness.type_into_field(distance, "deep = length");
    harness.settle();

    assert!(harness.shows("This would make deep depend on itself (deep → length → deep)"));
    assert_eq!(harness.expression_text("deep"), "15 mm");

    harness.type_into_field(distance, "20 mm");
    harness.settle();
    let deep = harness.document().parameter_named("deep").unwrap();

    assert_eq!(held(&harness), "20 mm");
    assert_eq!(deep.owner, None);
    assert_eq!(harness.expression_text("length"), "deep * 2");

    harness.perform(Action::Undo);
    harness.settle();

    assert_eq!(held(&harness), "deep");
    assert!(
        harness
            .document()
            .parameter_named("deep")
            .unwrap()
            .is_model_parameter()
    );
}

#[test]
fn a_named_value_follows_its_field_into_another_mode_and_goes_when_a_mode_drops_it() {
    use caditor_document::ParameterOwner;

    let mut harness = Harness::new();
    let (extrude, _) = extruded_plate(&mut harness);
    let field = |salt: &str| Id::unique(("solid-field", salt, extrude));
    let owner = |harness: &Harness, name: &str| {
        harness
            .document()
            .parameter_named(name)
            .and_then(|parameter| parameter.owner.clone())
    };
    let caption = |value: &str| {
        Some(ParameterOwner::Feature {
            feature: extrude,
            value: value.to_owned(),
        })
    };
    let backward = |harness: &Harness| match harness.solid(extrude) {
        SolidFeature::Extrude(caditor_document::Extrude {
            extent:
                ExtrudeExtent::TwoSides {
                    backward: caditor_document::ExtrudeEnd::Distance(distance),
                    ..
                },
            ..
        }) => harness.document().expression_text(distance),
        _ => String::new(),
    };

    harness.perform(Action::Editing(EditingCommand::OpenSolid(extrude)));
    harness.type_into_field(field("distance"), "h = 20 mm");
    harness.settle();
    choose(&mut harness, "One side", "Symmetric");
    let symmetric = owner(&harness, "h");
    let shown = harness.shows("h = 20 mm");
    harness.type_into_field(field("distance"), "h = 30 mm");
    harness.settle();

    assert_eq!(symmetric, caption("Total distance"));
    assert!(shown);
    assert_eq!(harness.expression_text("h"), "30 mm");
    assert_eq!(harness.model.undo_label(), Some("Edit h"));

    choose(&mut harness, "Symmetric", "Two sides");
    let forward = owner(&harness, "h");
    let copied = backward(&harness);
    harness.type_into_field(field("backward"), "b = 5 mm");
    harness.settle();
    let named_backward = owner(&harness, "b");
    choose(&mut harness, "Two sides", "One side");

    assert_eq!(forward, caption("Forward distance"));
    assert_eq!(copied, "30 mm");
    assert_eq!(named_backward, caption("Backward distance"));
    assert_eq!(owner(&harness, "h"), caption("Distance"));
    assert!(harness.document().parameter_named("b").is_none());

    harness.perform(Action::Undo);
    harness.settle();

    assert_eq!(owner(&harness, "b"), caption("Backward distance"));
    assert_eq!(owner(&harness, "h"), caption("Forward distance"));
}

#[test]
fn parameters_are_added_renamed_given_expressions_and_deleted_in_their_table() {
    let mut harness = Harness::new();
    harness.click_beside(crate::icons::ADD, "Parameters");
    harness.frame();
    let added = harness.parameter("parameter1");
    let added_label = harness.model.undo_label().map(str::to_owned);
    let focused_on_name = harness.focused() == Some(Focus::ParameterName(added).field_id());
    harness.key(Key::A, Modifiers::COMMAND);
    harness.events.push(Event::Text("depth".to_owned()));
    harness.frame();
    harness.key(Key::Enter, Modifiers::NONE);
    harness.settle();
    let renamed = harness.document().parameter(added).map(|p| p.name.clone());
    harness.type_into(Focus::ParameterValue(added), "width / 4");
    harness.settle();
    let quarter_shown = harness.shows("10 mm");
    let width = harness.parameter("width");
    harness.type_into(Focus::ParameterName(width), "span");
    harness.settle();
    let height_text = harness.expression_text("height");
    let depth_text = harness.expression_text("depth");
    harness.click_button("Delete depth");
    harness.settle();
    let deleted_label = harness.model.undo_label().map(str::to_owned);
    let deleted = harness.document().parameter(added).is_none();
    harness.perform(Action::Undo);
    harness.settle();

    assert_eq!(added_label.as_deref(), Some("Add parameter1"));
    assert!(focused_on_name);
    assert_eq!(renamed.as_deref(), Some("depth"));
    assert!(quarter_shown);
    assert_eq!(height_text, "span / 2");
    assert_eq!(depth_text, "span / 4");
    assert!(harness.shows("span / 2"));
    assert_eq!(deleted_label.as_deref(), Some("Delete depth"));
    assert!(deleted);
    assert_eq!(harness.expression_text("depth"), "span / 4");
}
