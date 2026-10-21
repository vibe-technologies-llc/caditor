use std::{
    collections::BTreeSet,
    fs,
    sync::{Arc, atomic::AtomicUsize},
};

use caditor_document::{CancelToken, ConfiguredValue, Setting, Transaction};
use caditor_file::{ExportFormat, MeshOptions, MeshResolution, StlEncoding};
use caditor_geometry::Point2;
use egui::{Key, Modifiers};
use tempfile::TempDir;

use super::{Harness, add_block, canonical, extruded_plate, run_from_palette};
use crate::{
    configuration_export::{self, ConfigurationJob},
    configurations::{ADD_LABEL, cell_id},
    export::ExportCommand,
    files::FileCommand,
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

#[test]
fn escape_in_a_configuration_s_rename_cell_keeps_the_name_and_the_dialog() {
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
    harness.click_button("Rename it");
    harness.frame();
    harness.type_text("Changed");
    assert!(harness.has_button("Keep the name (Esc)"));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert!(harness.workspace.configurations.is_some());
    assert_eq!(
        harness.document().configurations().rows[0].name,
        "Configuration 1"
    );
    assert!(!harness.has_button("Keep the name (Esc)"));
    assert!(harness.has_button("Rename it"));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert!(harness.workspace.configurations.is_none());
}

#[test]
fn exporting_every_configuration_writes_a_file_for_each_with_its_own_values() {
    let mut harness = Harness::new();
    add_size(&mut harness);
    let size = harness.parameter("size");
    add_block(
        &mut harness,
        "Block",
        [Point2::new(0.0, 0.0), Point2::new(10.0, 10.0)],
        &format!("${}", size.raw()),
    );
    let value = ConfiguredValue::Parameter(size);
    let (first, _) = harness.document().adding_configuration(Some("Short"));
    harness.perform(Action::Apply(first));
    let configure = harness.document().configuring(value).unwrap();
    harness.perform(Action::Apply(configure));
    let (second, tall) = harness.document().adding_configuration(Some("Tall"));
    harness.perform(Action::Apply(second));
    let thirty = harness
        .document()
        .transaction("Parse")
        .parse("30 mm")
        .unwrap();
    let change = harness
        .document()
        .setting_configuration(tall, value, Setting::Expression(thirty))
        .unwrap();
    harness.perform(Action::Apply(change));
    harness.settle();
    let folder = tempfile::tempdir().unwrap();
    let job = ConfigurationJob {
        document: harness.document().clone(),
        files: configuration_export::files_for(
            &folder.path().join("bracket.stl"),
            configuration_export::exportable(harness.document()),
            ExportFormat::Stl,
        ),
        left_out: BTreeSet::new(),
        format: ExportFormat::Stl,
        done: Arc::new(AtomicUsize::new(0)),
    };
    let options = MeshOptions {
        resolution: MeshResolution::default(),
        stl: StlEncoding::Text,
        thumbnail: None,
    };

    let outcomes = job
        .run(
            &options,
            harness.document().properties(),
            &CancelToken::never(),
        )
        .unwrap();

    let short = std::fs::read_to_string(folder.path().join("bracket Short.stl")).unwrap();
    let long = std::fs::read_to_string(folder.path().join("bracket Tall.stl")).unwrap();
    assert_eq!(outcomes.len(), 2);
    assert_ne!(short, long);
    assert!(outcomes.iter().all(|outcome| outcome.result.is_ok()));
    assert!(
        short.contains("1.000000e1") || short.contains(" 10"),
        "{short}"
    );
    assert!(
        long.contains("3.000000e1") || long.contains(" 30"),
        "{long}"
    );
    assert_eq!(harness.expression_text("size"), "10 mm");
}

fn notice_starts(harness: &Harness, text: &str) -> bool {
    harness
        .model
        .notice()
        .is_some_and(|notice| notice.text.starts_with(text))
}

#[test]
fn configurations_whose_names_make_the_same_file_are_written_apart_once_replacing_is_confirmed() {
    let dir = TempDir::new().unwrap();
    let root = canonical(&dir);
    let mut harness = Harness::with_directories(Some(root.as_path()));
    extruded_plate(&mut harness);
    let (first, _) = harness.document().adding_configuration(Some("M4/20"));
    harness.perform(Action::Apply(first));
    let (second, _) = harness.document().adding_configuration(Some("M4-20"));
    harness.perform(Action::Apply(second));
    harness.settle();
    harness.frame();

    let slashed = root.join("bracket M4-20.stl");
    let dashed = root.join("bracket M4-20 (2).stl");
    fs::write(&slashed, b"kept").unwrap();
    fs::write(&dashed, b"kept").unwrap();

    harness.command(FileCommand::Export(ExportCommand::Show));
    harness.click("STL");
    harness.command(FileCommand::Export(ExportCommand::SetEveryConfiguration(
        true,
    )));
    harness.answer_dialog(Some(root.join("bracket")));
    harness.click("Export…");
    harness.wait_until("the replacement is asked about", |harness| {
        harness.shows("Replace 2 files?")
    });
    let listed = harness.shows("bracket M4-20.stl") && harness.shows("bracket M4-20 (2).stl");
    harness.click("Cancel");

    assert!(listed);
    assert_eq!(fs::read(&slashed).unwrap(), b"kept");
    assert_eq!(fs::read(&dashed).unwrap(), b"kept");

    harness.click("Export…");
    harness.wait_until("the replacement is asked about again", |harness| {
        harness.shows("Replace 2 files?")
    });
    harness.click("Replace");
    harness.wait_until("both configurations are written", |harness| {
        notice_starts(
            harness,
            "Exported 2 configurations to “bracket M4-20.stl” and “bracket M4-20 (2).stl”.",
        )
    });

    assert_ne!(fs::read(&slashed).unwrap(), b"kept");
    assert_ne!(fs::read(&dashed).unwrap(), b"kept");
    assert!(!root.join("bracket.stl").exists());

    harness.perform(Action::DismissNotice);
    run_from_palette(&mut harness, "export again");
    harness.wait_until("both configurations are written again", |harness| {
        notice_starts(harness, "Exported 2 configurations")
    });

    assert!(!harness.shows("Replace 2 files?"));

    fs::write(&dashed, b"changed elsewhere").unwrap();
    run_from_palette(&mut harness, "export again");
    harness.wait_until("the changed file is asked about", |harness| {
        harness.shows("Replace “bracket M4-20 (2).stl”?")
    });
    harness.click("Cancel");

    assert_eq!(fs::read(&dashed).unwrap(), b"changed elsewhere");
}
