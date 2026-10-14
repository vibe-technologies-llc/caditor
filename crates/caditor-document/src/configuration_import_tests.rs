use caditor_expression::ParameterId;

use crate::{
    body_pieces_tests::{evaluate, piece_of, three_rectangles},
    combine_tests::{Pair, pair},
    *,
};

fn file(headers: &[&str], rows: &[(&str, &[&str])]) -> ImportedConfigurations {
    ImportedConfigurations {
        columns: headers
            .iter()
            .map(|header| ImportedColumn::from_header(header))
            .collect(),
        rows: rows
            .iter()
            .map(|(name, cells)| ImportedConfiguration {
                name: (*name).to_owned(),
                cells: cells.iter().map(|cell| (*cell).to_owned()).collect(),
            })
            .collect(),
    }
}

fn model() -> (Document, ParameterId, FeatureId) {
    let Pair {
        mut document,
        plate,
        ..
    } = pair();
    let mut transaction = document.transaction("Size");
    let size = transaction.add_parameter("size", transaction.parse("4 mm").unwrap());
    document.apply(transaction.finish()).unwrap();
    (document, size, plate)
}

fn outcome<'a>(plan: &'a ConfigurationImport, name: &str) -> &'a RowOutcome {
    &plan
        .rows
        .iter()
        .find(|row| row.name == name)
        .unwrap()
        .outcome
}

fn text(document: &Document, name: &str) -> String {
    let parameter = document.parameter_named(name).unwrap();
    document.expression_text(&parameter.expression)
}

fn cell(document: &Document, row: &str, header: &str) -> String {
    let id = document
        .configurations()
        .row(
            document
                .configurations()
                .rows
                .iter()
                .find(|candidate| candidate.name == row)
                .unwrap()
                .id,
        )
        .unwrap()
        .id;
    document
        .configurations()
        .values
        .iter()
        .find(|value| document.configuration_column_header(**value).as_deref() == Some(header))
        .and_then(|value| document.configuration_cell_text(id, *value))
        .unwrap()
}

const HEADERS: [&str; 3] = ["Parameter: size", "Suppressed: Plate", "Colour: Plate"];

#[test]
fn rows_naming_new_configurations_add_them_and_configure_the_columns_they_name() {
    let (document, _, _) = model();
    let rows = file(
        &HEADERS,
        &[
            ("Small", &["4 mm", "no", "#ff0000"]),
            ("Large", &["8 mm", "yes", "default"]),
        ],
    );
    let mut editor = Editor::new(document);

    let plan = editor
        .document()
        .plan_configuration_import("Import", &rows, true)
        .unwrap();
    editor.apply(plan.transaction.unwrap()).unwrap();

    let document = editor.document();
    assert_eq!(document.configurations().rows.len(), 2);
    assert_eq!(document.configurations().values.len(), 3);
    assert_eq!(document.configurations().active, None);
    assert_eq!(cell(document, "Small", "Colour: Plate"), "#ff0000");
    assert_eq!(cell(document, "Large", "Parameter: size"), "8 mm");
    assert_eq!(cell(document, "Large", "Suppressed: Plate"), "yes");
    assert_eq!(cell(document, "Large", "Colour: Plate"), "default");
    assert_eq!(text(document, "size"), "4 mm");

    editor.undo().unwrap();
    assert!(editor.document().configurations().is_empty());
    assert_eq!(editor.undo_label(), None);
}

#[test]
fn the_plan_says_which_columns_are_known_new_or_refused_and_what_each_row_does() {
    let (document, size, _) = model();
    let mut editor = Editor::new(document);
    let configure = editor
        .document()
        .configuring(ConfiguredValue::Parameter(size))
        .unwrap();
    editor.apply(configure).unwrap();
    let (add, _) = editor.document().adding_configuration(Some("M4"));
    editor.apply(add).unwrap();
    let rows = file(
        &["Parameter: size", "Suppressed: Plate"],
        &[("m4", &["4 mm", ""]), ("M8", &["8 mm", "yes"])],
    );

    let plan = editor
        .document()
        .plan_configuration_import("Import", &rows, true)
        .unwrap();

    assert_eq!(plan.columns[0].outcome, ColumnOutcome::Known);
    assert_eq!(plan.columns[1].outcome, ColumnOutcome::Added);
    assert_eq!(outcome(&plan, "m4"), &RowOutcome::Unchanged);
    assert_eq!(outcome(&plan, "M8"), &RowOutcome::Added { values: 2 });
}

#[test]
fn importing_into_the_active_configuration_changes_the_model_and_undo_takes_it_back() {
    let (document, size, plate) = model();
    let mut editor = Editor::new(document);
    for value in [
        ConfiguredValue::Parameter(size),
        ConfiguredValue::Suppressed(plate),
    ] {
        let configure = editor.document().configuring(value).unwrap();
        editor.apply(configure).unwrap();
    }
    let (add, active) = editor.document().adding_configuration(Some("M4"));
    editor.apply(add).unwrap();
    let rows = file(
        &["Parameter: size", "Suppressed: Plate"],
        &[("M4", &["6 mm", "yes"])],
    );

    let plan = editor
        .document()
        .plan_configuration_import("Import", &rows, true)
        .unwrap();
    editor.apply(plan.transaction.unwrap()).unwrap();

    assert_eq!(text(editor.document(), "size"), "6 mm");
    assert!(editor.document().feature(plate).unwrap().suppressed);
    assert_eq!(editor.document().configurations().active, Some(active));
    assert_eq!(cell(editor.document(), "M4", "Parameter: size"), "6 mm");

    editor.undo().unwrap();
    assert_eq!(text(editor.document(), "size"), "4 mm");
    assert!(!editor.document().feature(plate).unwrap().suppressed);
    assert_eq!(cell(editor.document(), "M4", "Parameter: size"), "4 mm");
}

#[test]
fn a_changed_row_lists_each_cell_before_and_after_and_keeping_the_model_s_own_leaves_it_alone() {
    let (document, size, _) = model();
    let mut editor = Editor::new(document);
    let configure = editor
        .document()
        .configuring(ConfiguredValue::Parameter(size))
        .unwrap();
    editor.apply(configure).unwrap();
    let (first, _) = editor.document().adding_configuration(Some("M4"));
    editor.apply(first).unwrap();
    let (second, other) = editor.document().adding_configuration(Some("M8"));
    editor.apply(second).unwrap();
    let eight = editor
        .document()
        .transaction("Parse")
        .parse("8 mm")
        .unwrap();
    let set = editor
        .document()
        .setting_configuration(
            other,
            ConfiguredValue::Parameter(size),
            Setting::Expression(eight),
        )
        .unwrap();
    editor.apply(set).unwrap();
    let rows = file(&["Parameter: size"], &[("M8", &["10 mm"])]);

    let replacing = editor
        .document()
        .plan_configuration_import("Import", &rows, true)
        .unwrap();
    let keeping = editor
        .document()
        .plan_configuration_import("Import", &rows, false)
        .unwrap();

    let changes = vec![CellChange {
        column: "Parameter: size".to_owned(),
        before: "8 mm".to_owned(),
        after: "10 mm".to_owned(),
    }];
    assert_eq!(
        outcome(&replacing, "M8"),
        &RowOutcome::Changed {
            changes: changes.clone()
        }
    );
    assert_eq!(outcome(&keeping, "M8"), &RowOutcome::Kept { changes });
    assert!(replacing.transaction.is_some());
    assert!(keeping.transaction.is_none());
    assert_eq!(replacing.conflicts(), 1);
}

#[test]
fn blank_cells_leave_a_value_as_it_is_and_the_same_file_changes_nothing() {
    let (document, _, _) = model();
    let rows = file(&HEADERS, &[("Small", &["", "yes", ""])]);
    let mut editor = Editor::new(document);
    let plan = editor
        .document()
        .plan_configuration_import("Import", &rows, true)
        .unwrap();
    editor.apply(plan.transaction.unwrap()).unwrap();

    let again = editor
        .document()
        .plan_configuration_import("Import", &rows, true)
        .unwrap();

    assert_eq!(cell(editor.document(), "Small", "Parameter: size"), "4 mm");
    assert_eq!(cell(editor.document(), "Small", "Colour: Plate"), "default");
    assert_eq!(outcome(&again, "Small"), &RowOutcome::Unchanged);
    assert!(again.transaction.is_none());
}

#[test]
fn columns_naming_nothing_the_model_has_are_listed_and_refused_in_words() {
    let (document, _, _) = model();
    let rows = file(
        &[
            "Parameter: size",
            "Parameter: nothing",
            "Weight: Plate",
            "Suppressed: Ghost",
            "Colour: Plate outline",
            "Colour: Ghost",
            "Parameter: size",
        ],
        &[("Small", &["5 mm", "1", "1", "yes", "#fff", "#fff", "6 mm"])],
    );

    let plan = document
        .plan_configuration_import("Import", &rows, true)
        .unwrap();

    let refusals: Vec<String> = plan
        .columns
        .iter()
        .map(|column| match &column.outcome {
            ColumnOutcome::Refused(refusal) => refusal.to_string(),
            other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(
        refusals,
        [
            "Added",
            "the model has no parameter named “nothing”",
            "its heading does not start with “Parameter:”, “Suppressed:” or “Colour:”",
            "the model has no feature named “Ghost”",
            "“Plate outline” is not a feature that makes a body, so it has no colour to configure",
            "the model has no body named “Ghost”",
            "an earlier column already sets the same value",
        ]
    );
    assert_eq!(outcome(&plan, "Small"), &RowOutcome::Added { values: 1 });
    assert_eq!(
        plan.count_columns(|c| matches!(c, ColumnOutcome::Refused(_))),
        6
    );
}

#[test]
fn a_row_with_a_bad_cell_name_or_circle_is_left_out_alone_with_its_reason() {
    let (mut document, _, _) = model();
    let mut transaction = document.transaction("Circle");
    transaction.add_parameter("a", transaction.parse("1 mm").unwrap());
    transaction.add_parameter("b", transaction.parse("2 mm").unwrap());
    document.apply(transaction.finish()).unwrap();
    let rows = file(
        &[
            "Parameter: size",
            "Suppressed: Plate",
            "Colour: Plate",
            "Parameter: a",
            "Parameter: b",
        ],
        &[
            ("Fine", &["7 mm", "no", "#00ff00", "", ""]),
            ("Unreadable", &["7 mm +", "", "", "", ""]),
            ("Maybe", &["", "perhaps", "", "", ""]),
            ("Shade", &["", "", "#12", "", ""]),
            ("Circle", &["", "", "", "b + 1 mm", "a + 1 mm"]),
            ("Nothing", &["1 mm / 0", "", "", "", ""]),
            ("", &["1 mm", "", "", "", ""]),
            ("fine", &["1 mm", "", "", "", ""]),
        ],
    );

    let plan = document
        .plan_configuration_import("Import", &rows, true)
        .unwrap();

    assert_eq!(outcome(&plan, "Fine"), &RowOutcome::Added { values: 3 });
    let reasons: Vec<String> = plan
        .rows
        .iter()
        .filter_map(|row| match &row.outcome {
            RowOutcome::Refused(refusal) => Some(refusal.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(reasons.len(), 7);
    assert!(reasons[0].starts_with("Parameter: size: “7 mm +” cannot be read as an expression"));
    assert_eq!(reasons[1], "Suppressed: Plate: “perhaps” is not yes or no");
    assert_eq!(
        reasons[2],
        "Colour: Plate: “#12” is not a colour; write it as #rrggbb, or default"
    );
    assert!(reasons[3].starts_with("its parameters depend on each other in a circle"));
    assert!(reasons[4].starts_with("Parameter: size cannot be worked out"));
    assert_eq!(reasons[5], "it has no configuration name");
    assert_eq!(reasons[6], "the name appears on an earlier row");
    let mut document = document;
    document.apply(plan.transaction.unwrap()).unwrap();
    assert_eq!(document.configurations().rows.len(), 1);
}

#[test]
fn a_model_past_the_configuration_limit_leaves_the_extra_rows_out() {
    let (document, _, _) = model();
    let names: Vec<String> = (0..=MAX_CONFIGURATIONS)
        .map(|n| format!("Row {n}"))
        .collect();
    let rows: Vec<(&str, &[&str])> = names
        .iter()
        .map(|name| (name.as_str(), &["5 mm"][..]))
        .collect();
    let rows = file(&["Parameter: size"], &rows);

    let plan = document
        .plan_configuration_import("Import", &rows, true)
        .unwrap();

    assert_eq!(
        plan.count_rows(|outcome| matches!(outcome, RowOutcome::Refused(RowRefusal::TooMany))),
        1
    );
    let mut document = document;
    document.apply(plan.transaction.unwrap()).unwrap();
    assert_eq!(document.configurations().rows.len(), MAX_CONFIGURATIONS);
}

#[test]
fn headers_read_in_either_spelling_of_colour_and_any_case() {
    let colour = ImportedColumn::from_header("  color :  Body 1 ");
    let plain = ImportedColumn::from_header("Width");

    assert_eq!(colour.kind, Some(ColumnKind::Colour));
    assert_eq!(colour.name, "Body 1");
    assert_eq!(plain.kind, None);
    assert_eq!(
        ImportedColumn::header_of(ColumnKind::Suppressed, "Fillet 2"),
        "Suppressed: Fillet 2"
    );
}

#[test]
fn a_colour_column_names_a_separate_piece_by_the_body_name_the_app_shows() {
    let mut model = three_rectangles();
    let evaluation = evaluate(&model.document, &mut model.engine);
    let second = piece_of(model.extrusion, &model.rectangles[1]);
    let name = model.document.body_name(second).unwrap().into_owned();
    let header = format!("Colour: {name}");
    let rows = file(&[header.as_str()], &[("Red", &["#ff0000"])]);

    let blind = model
        .document
        .plan_configuration_import("Import", &rows, true)
        .unwrap();
    let plan = model
        .document
        .plan_configuration_import_in("Import", &rows, true, &evaluation)
        .unwrap();
    let mut editor = Editor::new(model.document);
    editor.apply(plan.transaction.unwrap()).unwrap();
    let document = editor.document();

    assert_eq!(
        blind.columns[0].outcome,
        ColumnOutcome::Refused(ColumnRefusal::MissingBody { name: name.clone() })
    );
    assert_eq!(plan.columns[0].outcome, ColumnOutcome::Added);
    assert_eq!(
        document.configurations().values,
        [ConfiguredValue::Colour(second)]
    );
    assert_eq!(cell(document, "Red", &header), "#ff0000");
}
