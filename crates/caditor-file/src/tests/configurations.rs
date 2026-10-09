use caditor_document::{ConfiguredValue, Editor, Rgb, Setting};

use super::*;

fn configured_model() -> (Document, Transaction, Transaction) {
    let (document, base, _) = solid_model();
    let depth = document.parameter_named("depth").unwrap().id();
    let mut editor = Editor::new(document);
    let (add, small) = editor.document().adding_configuration(Some("Short"));
    editor.apply(add).unwrap();
    for value in [
        ConfiguredValue::Parameter(depth),
        ConfiguredValue::Suppressed(base),
        ConfiguredValue::Colour(base),
    ] {
        let configure = editor.document().configuring(value).unwrap();
        editor.apply(configure).unwrap();
    }
    let (add, tall) = editor.document().adding_configuration(Some("Tall"));
    editor.apply(add).unwrap();
    let nine = editor
        .document()
        .transaction("Parse")
        .parse("9 mm")
        .unwrap();
    let settings = [
        (ConfiguredValue::Parameter(depth), Setting::Expression(nine)),
        (
            ConfiguredValue::Colour(base),
            Setting::Colour(Some(Rgb::new(10, 20, 30))),
        ),
    ];
    for (value, setting) in settings {
        let change = editor
            .document()
            .setting_configuration(tall, value, setting)
            .unwrap();
        editor.apply(change).unwrap();
    }
    let change = editor.document().activating(tall).unwrap();
    let mut document = editor.document().clone();
    let undo = document.apply(change.clone()).unwrap();
    assert_eq!(document.configurations().active, Some(tall));
    assert_ne!(small, tall);
    (document, change, undo)
}

#[test]
fn configurations_survive_saving_the_journal_and_its_snapshot() {
    let (document, change, undo) = configured_model();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let journaled: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&change)).unwrap());
    let undone: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&undo)).unwrap());
    let recovered = journal::decode_journal(
        &journal::encode_journal(
            &journal::JournalHead {
                file: None,
                on_disk: None,
                loaded_with_problems: false,
                folded: 0,
            },
            &document,
            &[],
        )
        .unwrap(),
    )
    .unwrap();

    assert!(text.contains("\"configurations\""), "{text}");
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert_eq!(format::restore_transaction(journaled), Some(change));
    assert_eq!(format::restore_transaction(undone), Some(undo));
    assert_eq!(recovered.issues, Vec::<String>::new());
    assert_eq!(recovered.base, document);
}

#[test]
fn a_model_without_configurations_writes_no_record() {
    let (document, _, _) = solid_model();

    let text = encode(&document).unwrap();

    assert!(!text.contains("\"configurations\""));
}

#[test]
fn configurations_that_cannot_be_read_are_repaired_and_reported() {
    let (document, _, _) = solid_model();
    let depth = document.parameter_named("depth").unwrap().id().raw();
    let configurations = format!(
        r#"{{"configurations":{{"values":[{{"parameter":{depth}}},{{"cog":1}}],"rows":[{{"id":3,"name":"Small","settings":[{{"value":{{"parameter":{depth}}},"expression":"4 mm"}},{{"value":{{"parameter":{depth}}},"expression":"(("}}]}},{{"id":4,"name":"small","settings":[]}},{{"id":3,"name":"Again"}},{{"name":"Broken"}}],"active":9,"next_id":5}}}}"#
    );
    let text = format!("{}\n{configurations}", encode(&document).unwrap());

    let loaded = decode_text(&text);

    let configurations = loaded.document.configurations();
    let names: Vec<&str> = configurations
        .rows
        .iter()
        .map(|row| row.name.as_str())
        .collect();
    assert_eq!(names, ["Small", "small 2"]);
    assert_eq!(configurations.active, None);
    assert_eq!(configurations.next_id, 5);
    assert!(issues_mention(
        &loaded,
        "One value set by the configurations could not be read"
    ));
    assert!(issues_mention(
        &loaded,
        "One value of the configuration “Small” could not be read"
    ));
    assert!(issues_mention(
        &loaded,
        "Two configurations were called “small”"
    ));
    assert!(issues_mention(
        &loaded,
        "The configuration “Again” could not be told apart"
    ));
    assert!(issues_mention(&loaded, "A configuration could not be read"));
    assert!(issues_mention(
        &loaded,
        "Which configuration was active could not be read"
    ));
}
