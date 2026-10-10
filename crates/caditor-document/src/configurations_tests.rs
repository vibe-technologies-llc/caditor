use caditor_expression::{Expression, ParameterId};

use crate::{
    combine_tests::{Pair, pair},
    *,
};

fn parameter(document: &mut Document, name: &str, text: &str) -> ParameterId {
    let mut transaction = document.transaction("Add");
    let expression = transaction.parse(text).unwrap();
    let id = transaction.add_parameter(name, expression);
    document.apply(transaction.finish()).unwrap();
    id
}

fn expression(document: &Document, text: &str) -> Expression {
    document.transaction("Parse").parse(text).unwrap()
}

fn text(document: &Document, id: ParameterId) -> String {
    document
        .parameter(id)
        .unwrap()
        .expression
        .to_text(&|id| document.parameter_name(id))
}

fn add(editor: &mut Editor, name: &str) -> ConfigurationId {
    let (transaction, id) = editor.document().adding_configuration(Some(name));
    editor.apply(transaction).unwrap();
    id
}

fn set(editor: &mut Editor, id: ConfigurationId, value: ConfiguredValue, setting: Setting) {
    let transaction = editor
        .document()
        .setting_configuration(id, value, setting)
        .unwrap();
    editor.apply(transaction).unwrap();
}

fn switch(editor: &mut Editor, id: ConfigurationId) {
    let transaction = editor.document().activating(id).unwrap();
    editor.apply(transaction).unwrap();
}

#[test]
fn switching_configurations_sets_their_values_and_undo_switches_back() {
    let mut document = Document::default();
    let size = parameter(&mut document, "size", "4 mm");
    let mut editor = Editor::new(document);

    let small = add(&mut editor, "M4");
    let configure = editor
        .document()
        .configuring(ConfiguredValue::Parameter(size))
        .unwrap();
    editor.apply(configure).unwrap();
    let large = add(&mut editor, "M8");
    let eight = expression(editor.document(), "8 mm");
    set(
        &mut editor,
        large,
        ConfiguredValue::Parameter(size),
        Setting::Expression(eight.clone()),
    );

    assert_eq!(editor.document().configurations().active, Some(small));
    assert_eq!(text(editor.document(), size), "4 mm");

    switch(&mut editor, large);

    assert_eq!(editor.document().configurations().active, Some(large));
    assert_eq!(text(editor.document(), size), "8 mm");
    assert_eq!(
        editor
            .document()
            .configuration_setting(small, ConfiguredValue::Parameter(size)),
        Some(Setting::Expression(expression(editor.document(), "4 mm")))
    );

    editor.undo().unwrap();

    assert_eq!(editor.document().configurations().active, Some(small));
    assert_eq!(text(editor.document(), size), "4 mm");

    editor.redo().unwrap();

    assert_eq!(text(editor.document(), size), "8 mm");
}

#[test]
fn editing_a_value_changes_the_active_configuration_only() {
    let mut document = Document::default();
    let size = parameter(&mut document, "size", "4 mm");
    let mut editor = Editor::new(document);
    let first = add(&mut editor, "First");
    let configure = editor
        .document()
        .configuring(ConfiguredValue::Parameter(size))
        .unwrap();
    editor.apply(configure).unwrap();
    let second = add(&mut editor, "Second");

    let six = expression(editor.document(), "6 mm");
    editor
        .apply(Transaction::single(
            "Edit size",
            Edit::SetParameterExpression {
                id: size,
                expression: six.clone(),
            },
        ))
        .unwrap();

    assert_eq!(
        editor
            .document()
            .configurations()
            .row(first)
            .unwrap()
            .setting(ConfiguredValue::Parameter(size)),
        Some(&Setting::Expression(six.clone()))
    );
    assert_eq!(
        editor
            .document()
            .configurations()
            .row(second)
            .unwrap()
            .setting(ConfiguredValue::Parameter(size)),
        Some(&Setting::Expression(expression(editor.document(), "4 mm")))
    );

    switch(&mut editor, second);
    switch(&mut editor, first);

    assert_eq!(text(editor.document(), size), "6 mm");
}

#[test]
fn values_using_each_other_swap_without_a_passing_cycle() {
    let mut document = Document::default();
    let a = parameter(&mut document, "a", "5 mm");
    let b = parameter(&mut document, "b", "a * 2");
    let mut editor = Editor::new(document);
    add(&mut editor, "Plain");
    for value in [a, b] {
        let configure = editor
            .document()
            .configuring(ConfiguredValue::Parameter(value))
            .unwrap();
        editor.apply(configure).unwrap();
    }
    let flipped = add(&mut editor, "Flipped");
    let half = expression(editor.document(), "b / 2");
    let ten = expression(editor.document(), "10 mm");
    set(
        &mut editor,
        flipped,
        ConfiguredValue::Parameter(a),
        Setting::Expression(half),
    );
    set(
        &mut editor,
        flipped,
        ConfiguredValue::Parameter(b),
        Setting::Expression(ten),
    );

    switch(&mut editor, flipped);

    let values = ParameterValues::evaluate(editor.document());
    assert_eq!(text(editor.document(), a), "b / 2");
    assert_eq!(values.value(a).unwrap().value, 5.0);
    assert_eq!(values.value(b).unwrap().value, 10.0);
}

#[test]
fn deleting_the_active_configuration_switches_to_the_next() {
    let mut document = Document::default();
    let size = parameter(&mut document, "size", "4 mm");
    let mut editor = Editor::new(document);
    let first = add(&mut editor, "First");
    let configure = editor
        .document()
        .configuring(ConfiguredValue::Parameter(size))
        .unwrap();
    editor.apply(configure).unwrap();
    let second = add(&mut editor, "Second");
    let nine = expression(editor.document(), "9 mm");
    set(
        &mut editor,
        second,
        ConfiguredValue::Parameter(size),
        Setting::Expression(nine),
    );

    let deletion = editor.document().deleting_configuration(first).unwrap();
    editor.apply(deletion).unwrap();

    assert_eq!(editor.document().configurations().active, Some(second));
    assert_eq!(editor.document().configurations().rows.len(), 1);
    assert_eq!(text(editor.document(), size), "9 mm");

    editor.undo().unwrap();

    assert_eq!(editor.document().configurations().active, Some(first));
    assert_eq!(text(editor.document(), size), "4 mm");
    assert_eq!(editor.document().configurations().rows.len(), 2);
}

#[test]
fn configurations_suppress_features_and_colour_bodies() {
    let Pair {
        document, plate, ..
    } = pair();
    let mut editor = Editor::new(document);
    add(&mut editor, "Plain");
    for value in [
        ConfiguredValue::Suppressed(plate),
        ConfiguredValue::Colour(plate),
    ] {
        let configure = editor.document().configuring(value).unwrap();
        editor.apply(configure).unwrap();
    }
    let red = Rgb::new(200, 30, 30);
    let other = add(&mut editor, "Without plate");
    set(
        &mut editor,
        other,
        ConfiguredValue::Suppressed(plate),
        Setting::Suppressed(true),
    );
    set(
        &mut editor,
        other,
        ConfiguredValue::Colour(plate),
        Setting::Colour(Some(red)),
    );

    switch(&mut editor, other);

    let feature = editor.document().feature(plate).unwrap();
    assert!(feature.suppressed);
    assert_eq!(feature.appearance.colour, Some(red));
}

#[test]
fn names_are_unique_and_ids_never_reused() {
    let mut editor = Editor::new(Document::default());
    let first = add(&mut editor, "Small");
    let (taken, _) = editor.document().adding_configuration(Some(" small "));

    assert_eq!(
        editor.apply(taken),
        Err(EditError::ConfigurationNameTaken("small".to_owned()))
    );

    let deletion = editor.document().deleting_configuration(first).unwrap();
    editor.apply(deletion).unwrap();
    let second = add(&mut editor, "Small");

    assert_ne!(first, second);
    assert_eq!(editor.document().configurations().active, Some(second));
}

#[test]
fn restoring_another_document_brings_its_configurations() {
    let mut document = Document::default();
    let size = parameter(&mut document, "size", "4 mm");
    let mut editor = Editor::new(document.clone());
    add(&mut editor, "First");
    let configure = editor
        .document()
        .configuring(ConfiguredValue::Parameter(size))
        .unwrap();
    editor.apply(configure).unwrap();
    let second = add(&mut editor, "Second");
    let nine = expression(editor.document(), "9 mm");
    set(
        &mut editor,
        second,
        ConfiguredValue::Parameter(size),
        Setting::Expression(nine),
    );
    switch(&mut editor, second);
    let configured = editor.document().clone();

    let back = configured.transaction_to(&document, "Restore");
    let mut restored = configured.clone();
    restored.apply(back).unwrap();

    assert!(restored.same_content(&document));

    let forward = restored.transaction_to(&configured, "Restore");
    restored.apply(forward).unwrap();

    assert!(restored.same_content(&configured));
}

fn sized_by_another_in_one_configuration() -> (Editor, ParameterId, ParameterId, ConfigurationId) {
    let mut document = Document::default();
    let unit = parameter(&mut document, "unit", "3 mm");
    let size = parameter(&mut document, "size", "4 mm");
    let mut editor = Editor::new(document);
    add(&mut editor, "Plain");
    let configure = editor
        .document()
        .configuring(ConfiguredValue::Parameter(size))
        .unwrap();
    editor.apply(configure).unwrap();
    let scaled = add(&mut editor, "Scaled");
    let twice = expression(editor.document(), "unit * 2");
    set(
        &mut editor,
        scaled,
        ConfiguredValue::Parameter(size),
        Setting::Expression(twice),
    );
    (editor, unit, size, scaled)
}

#[test]
fn a_parameter_named_only_by_an_inactive_configuration_is_in_use_and_cannot_be_removed() {
    let (mut editor, unit, _, _) = sized_by_another_in_one_configuration();

    let used = editor.document().used_parameters();
    let removal = editor.apply(Transaction::single(
        "Delete unit",
        Edit::RemoveParameter { id: unit },
    ));

    assert!(used.contains(&unit));
    assert!(matches!(removal, Err(EditError::ParameterInUse { .. })));
    assert_eq!(
        editor.document().parameter_users(unit),
        ["Scaled configuration"]
    );
}

#[test]
fn deleting_a_parameter_named_by_an_inactive_configuration_writes_its_value_into_the_cell() {
    let (mut editor, unit, size, scaled) = sized_by_another_in_one_configuration();

    let deletion = editor.document().inline_parameter(unit).unwrap();
    editor.apply(deletion).unwrap();
    switch(&mut editor, scaled);

    let values = ParameterValues::evaluate(editor.document());
    assert!(editor.document().parameter(unit).is_none());
    assert_eq!(values.value(size).unwrap().value, 6.0);
}

#[test]
fn deleting_a_configured_parameter_drops_its_column_so_switching_still_works() {
    let (mut editor, _, size, scaled) = sized_by_another_in_one_configuration();
    let plain = editor.document().configurations().rows[0].id;
    let used = editor.document().used_parameters();

    let deletion = editor.document().inline_parameter(size).unwrap();
    editor.apply(deletion).unwrap();
    switch(&mut editor, scaled);
    switch(&mut editor, plain);

    assert!(used.contains(&size));
    assert!(editor.document().parameter(size).is_none());
    assert!(editor.document().configurations().values.is_empty());
    assert!(
        editor
            .document()
            .configurations()
            .rows
            .iter()
            .all(|row| row.settings.is_empty())
    );
}
