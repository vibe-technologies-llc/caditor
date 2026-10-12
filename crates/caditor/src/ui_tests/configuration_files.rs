use std::fs;

use caditor_document::ConfiguredValue;
use tempfile::TempDir;

use super::{Harness, run_from_palette};
use crate::{
    configurations::ADD_LABEL,
    files::{
        FileCommand,
        configurations::{ConfigurationFileCommand, EXPORT_LABEL, IMPORT_LABEL},
    },
    model::Action,
    preferences::PreferencesCommand,
};

fn notice_says(harness: &Harness, text: &str) -> bool {
    harness
        .model
        .notice()
        .is_some_and(|notice| notice.text.contains(text))
}

fn configured_widths(harness: &mut Harness) {
    let width = ConfiguredValue::Parameter(harness.parameter("width"));
    let (first, _) = harness.document().adding_configuration(Some("M4"));
    harness.perform(Action::Apply(first));
    let configure = harness.document().configuring(width).unwrap();
    harness.perform(Action::Apply(configure));
    let (second, large) = harness.document().adding_configuration(Some("M8"));
    harness.perform(Action::Apply(second));
    let wide = harness
        .document()
        .transaction("Parse")
        .parse("60 mm")
        .unwrap();
    let change = harness
        .document()
        .setting_configuration(large, width, caditor_document::Setting::Expression(wide))
        .unwrap();
    harness.perform(Action::Apply(change));
    harness.settle();
}

#[test]
fn configurations_export_to_csv_and_import_back_after_a_preview_as_one_change() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::new();
    configured_widths(&mut harness);

    harness.answer_dialog(Some(dir.path().join("variants")));
    run_from_palette(&mut harness, "Export configurations");
    harness.wait_until("the configurations are exported", |harness| {
        notice_says(harness, "Exported 2 configurations")
    });
    let written = fs::read_to_string(dir.path().join("variants.csv")).unwrap();
    assert_eq!(
        written,
        "Configuration,Parameter: width\r\nM4,40 mm\r\nM8,60 mm\r\n"
    );

    let edited = dir.path().join("edited.csv");
    fs::write(
        &edited,
        "Configuration,Parameter: width,Parameter: depth\nM8,70 mm,1\nM12,90 mm,\nM16,1 mm +,\n",
    )
    .unwrap();
    harness.answer_dialog(Some(edited));
    run_from_palette(&mut harness, "Import configurations");
    harness.wait_until("the preview opens", |harness| {
        harness.shows("Import configurations from “edited.csv”")
    });
    assert!(harness.shows("1 configuration is already in the model with other values."));
    assert!(harness.shows(
        "1 column name something the model does not have and will be left out; the reason is \
         beside each."
    ));
    assert!(
        harness.shows("1 row cannot be imported and will be left out; the reason is beside each.")
    );
    assert!(harness.shows("The model has no parameter named “depth”"));
    assert_eq!(harness.document().configurations().rows.len(), 2);

    harness.command(FileCommand::ConfigurationImport(
        ConfigurationFileCommand::Apply,
    ));
    harness.settle();

    let document = harness.document();
    let names: Vec<_> = document
        .configurations()
        .rows
        .iter()
        .map(|row| row.name.as_str())
        .collect();
    assert_eq!(names, ["M4", "M8", "M12"]);
    assert_eq!(
        harness.model.undo_label(),
        Some("Import configurations from edited.csv")
    );
    assert!(notice_says(
        &harness,
        "added 1 configuration, changed 1, left out 1 row that could not be read, left out 1 \
         column the model has no match for."
    ));
    assert!(!harness.shows("Import configurations from “edited.csv”"));

    harness.perform(Action::Undo);
    harness.settle();
    assert_eq!(harness.document().configurations().rows.len(), 2);
}

#[test]
fn the_configurations_dialog_exports_and_imports_through_its_buttons() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::new();
    configured_widths(&mut harness);
    run_from_palette(&mut harness, "configurations");
    harness.frame();

    harness.answer_dialog(Some(dir.path().join("table")));
    harness.click_button(EXPORT_LABEL);
    harness.wait_until("the configurations are exported", |harness| {
        notice_says(harness, "Exported 2 configurations to “table.csv”")
    });
    harness.answer_dialog(Some(dir.path().join("table.csv")));
    harness.click_button(IMPORT_LABEL);
    harness.wait_until("the preview opens", |harness| {
        harness.shows("Import configurations from “table.csv”")
    });

    assert!(harness.shows("2 rows and 1 column in the file. Nothing changes until you import."));
    harness.command(FileCommand::ConfigurationImport(
        ConfigurationFileCommand::Apply,
    ));
    harness.settle();
    assert!(notice_says(
        &harness,
        "match the model's, so nothing changed."
    ));
    harness.perform(Action::Preferences(PreferencesCommand::CloseConfigurations));
    assert_eq!(harness.document().configurations().rows.len(), 2);
}

#[test]
fn exporting_without_configurations_and_importing_a_file_without_the_column_say_why() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::new();

    harness.command(FileCommand::ExportConfigurations);
    harness.settle();
    assert!(notice_says(
        &harness,
        "The model has no configurations to export."
    ));

    let file = dir.path().join("list.csv");
    fs::write(&file, "a,b\n1,2\n").unwrap();
    harness.answer_dialog(Some(file));
    harness.command(FileCommand::ImportConfigurations);
    harness.wait_until("the file is refused", |harness| {
        notice_says(
            harness,
            "Could not import configurations from “list.csv”: its first row does not name a \
             “Configuration” column.",
        )
    });
}

#[test]
fn a_file_naming_a_new_value_configures_it_and_the_add_button_is_still_offered() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::new();
    let file = dir.path().join("sizes.csv");
    fs::write(
        &file,
        "Configuration;Parameter: width\nSmall;20 mm\nBig;80 mm\n",
    )
    .unwrap();

    harness.answer_dialog(Some(file));
    harness.command(FileCommand::ImportConfigurations);
    harness.wait_until("the preview opens", |harness| {
        harness.shows("Import configurations from “sizes.csv”")
    });
    assert!(harness.shows("New value"));
    harness.command(FileCommand::ConfigurationImport(
        ConfigurationFileCommand::Apply,
    ));
    harness.settle();

    let document = harness.document();
    assert_eq!(document.configurations().rows.len(), 2);
    assert_eq!(document.configurations().values.len(), 1);
    assert_eq!(document.configurations().active, None);
    assert_eq!(harness.expression_text("width"), "40 mm");
    run_from_palette(&mut harness, "configurations");
    harness.frame();
    assert!(harness.shows(ADD_LABEL));
}
