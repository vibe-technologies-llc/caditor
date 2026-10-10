use caditor_expression::Quantity;

use crate::*;

fn row(name: &str, expression: &str) -> ImportedParameter {
    ImportedParameter {
        name: name.to_owned(),
        expression: expression.to_owned(),
        note: String::new(),
        reading: ExpressionReading::Typed,
    }
}

fn model() -> Document {
    let mut document = Document::default();
    let mut transaction = document.transaction("Parameters");
    transaction.add_parameter("width", transaction.parse("40 mm").unwrap());
    transaction.add_parameter("height", transaction.parse("width / 2").unwrap());
    document.apply(transaction.finish()).unwrap();
    document
}

fn outcome<'a>(plan: &'a ParameterImport, name: &str) -> &'a ImportOutcome {
    &plan
        .rows
        .iter()
        .find(|row| row.name == name)
        .unwrap()
        .outcome
}

#[test]
fn an_import_adds_new_names_changes_existing_ones_and_is_one_undoable_change() {
    let mut document = model();
    let rows = [
        row("depth", "gap * 2"),
        row("width", "50 mm"),
        row("height", "width / 2"),
        ImportedParameter {
            note: "between the plates".to_owned(),
            ..row("gap", "3 mm")
        },
    ];

    let plan = document
        .plan_parameter_import("Import parameters", &rows, true)
        .unwrap();

    assert_eq!(outcome(&plan, "depth"), &ImportOutcome::Added);
    assert_eq!(
        outcome(&plan, "width"),
        &ImportOutcome::Changed {
            before: "40 mm".to_owned()
        }
    );
    assert_eq!(outcome(&plan, "height"), &ImportOutcome::Unchanged);
    assert_eq!(outcome(&plan, "gap"), &ImportOutcome::Added);
    assert_eq!(plan.conflicts(), 1);
    let before = document.clone();
    let undo = document.apply(plan.transaction.unwrap()).unwrap();
    let values = ParameterValues::evaluate(&document);
    let value = |name: &str| values.value(document.parameter_named(name).unwrap().id());
    assert_eq!(value("depth"), Ok(Quantity::length(6.0)));
    assert_eq!(value("height"), Ok(Quantity::length(25.0)));
    assert_eq!(
        document.parameter_named("gap").unwrap().note,
        "between the plates"
    );
    let names: Vec<&str> = document
        .parameters()
        .iter()
        .map(|parameter| parameter.name.as_str())
        .collect();
    assert_eq!(names, ["width", "height", "depth", "gap"]);
    document.apply(undo).unwrap();
    assert_eq!(document.parameters(), before.parameters());
}

#[test]
fn conflicting_values_can_be_kept_and_bad_rows_are_refused_alone_in_words() {
    let document = model();
    let rows = [
        row("width", "60 mm"),
        row("2wide", "1 mm"),
        row("loop_a", "loop_b + 1 mm"),
        row("loop_b", "loop_a"),
        row("broken", "3 mm +"),
        row("mixed", "width + 3 deg"),
        row("user", "broken * 2"),
        row("fine", "4 mm"),
        row("fine", "5 mm"),
    ];

    let plan = document
        .plan_parameter_import("Import parameters", &rows, false)
        .unwrap();

    assert_eq!(
        outcome(&plan, "width"),
        &ImportOutcome::Kept {
            theirs: "60 mm".to_owned()
        }
    );
    let refusal = |name: &str| match outcome(&plan, name) {
        ImportOutcome::Refused(refusal) => refusal.to_string(),
        other => panic!("{name} should be refused, got {other:?}"),
    };
    assert!(refusal("2wide").contains("must start with a letter"));
    assert!(refusal("loop_a").contains("depends on itself"));
    assert!(refusal("loop_b").contains("depends on itself"));
    assert!(refusal("broken").starts_with("its expression cannot be read"));
    assert!(refusal("mixed").starts_with("its value cannot be worked out"));
    assert!(refusal("user").contains("broken"));
    assert_eq!(plan.rows[7].outcome, ImportOutcome::Added);
    assert_eq!(
        plan.rows[8].outcome,
        ImportOutcome::Refused(ImportRefusal::Repeated)
    );
    let mut applied = document.clone();
    applied.apply(plan.transaction.unwrap()).unwrap();
    assert_eq!(applied.parameters().len(), 3);
}
