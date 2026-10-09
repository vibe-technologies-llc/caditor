use caditor_document::{ConfiguredValue, Transaction};

use super::{Harness, run_from_palette};
use crate::{
    configurations::{ADD_LABEL, cell_id},
    model::Action,
};

fn add_size(harness: &mut Harness) {
    let mut transaction = harness.document().transaction("Add size");
    let expression = transaction.parse("10 mm").unwrap();
    transaction.add_parameter("size", expression);
    let transaction: Transaction = transaction.finish();
    harness.perform(Action::Apply(transaction));
    harness.settle();
}

#[test]
fn the_configurations_dialog_adds_rows_sets_values_and_the_palette_switches_between_them() {
    let mut harness = Harness::new();
    harness.context.enable_accesskit();
    add_size(&mut harness);
    let size = ConfiguredValue::Parameter(harness.parameter("size"));

    run_from_palette(&mut harness, "configurations");
    harness.frame();
    harness.click_button(ADD_LABEL);
    harness.frame();
    let configure = harness.document().configuring(size).unwrap();
    harness.perform(Action::Apply(configure));
    harness.frame();
    harness.click_button(ADD_LABEL);
    harness.frame();
    let rows: Vec<_> = harness
        .document()
        .configurations()
        .rows
        .iter()
        .map(|row| (row.id, row.name.clone()))
        .collect();
    harness.type_into_field(cell_id(rows[1].0, size), "20 mm");
    harness.frame();
    let unchanged = harness.expression_text("size");
    harness.perform(Action::Preferences(
        crate::preferences::PreferencesCommand::CloseConfigurations,
    ));
    harness.frame();

    run_from_palette(&mut harness, "switch to configuration 2");
    harness.settle();
    let switched = harness.expression_text("size");
    let active = harness.document().configurations().active;
    harness.perform(Action::Undo);
    harness.settle();

    assert_eq!(
        rows.iter()
            .map(|(_, name)| name.as_str())
            .collect::<Vec<_>>(),
        ["Configuration 1", "Configuration 2"]
    );
    assert_eq!(unchanged, "10 mm");
    assert_eq!(switched, "20 mm");
    assert_eq!(active, Some(rows[1].0));
    assert_eq!(harness.expression_text("size"), "10 mm");
    assert_eq!(harness.document().configurations().active, Some(rows[0].0));
}
