use crate::{tests::sample, *};

fn set(properties: ModelProperties) -> Transaction {
    Transaction::single(
        "Model properties",
        Edit::SetModelProperties {
            properties: Box::new(properties),
        },
    )
}

fn bracket() -> ModelProperties {
    ModelProperties {
        title: "Wall bracket".to_owned(),
        part_number: "BR-100".to_owned(),
        revision: "C".to_owned(),
        description: "Holds a shelf".to_owned(),
        notes: "Print with 40% infill.\nCountersink by hand.".to_owned(),
        ..ModelProperties::default()
    }
}

#[test]
fn a_new_model_has_no_properties() {
    let (document, _) = sample();

    assert!(document.properties().is_empty());
    assert_eq!(document.properties(), &ModelProperties::default());
}

#[test]
fn setting_properties_is_undoable_content() {
    let (document, _) = sample();
    let mut editor = Editor::new(document.clone());

    let changed = editor.apply(set(bracket())).unwrap();
    let with_properties = editor.document().clone();
    editor.undo().unwrap();

    assert!(changed);
    assert_eq!(with_properties.properties(), &bracket());
    assert!(!with_properties.same_content(&document));
    assert!(editor.document().same_content(&document));
    assert_eq!(editor.redo_label(), Some("Model properties"));
}

#[test]
fn setting_the_same_properties_again_is_no_change() {
    let (document, _) = sample();
    let mut editor = Editor::new(document);
    editor.apply(set(bracket())).unwrap();
    let revision = editor.revision();

    let changed = editor.apply(set(bracket())).unwrap();

    assert!(!changed);
    assert_eq!(editor.revision(), revision);
}

#[test]
fn properties_are_trimmed_and_single_line_ones_keep_one_line() {
    let (mut document, _) = sample();
    let typed = ModelProperties {
        title: "  Wall\r\n bracket \n".to_owned(),
        notes: "\n First line\nSecond line \n".to_owned(),
        ..ModelProperties::default()
    };

    document.apply(set(typed)).unwrap();

    assert_eq!(document.properties().title, "Wall bracket");
    assert_eq!(document.properties().notes, "First line\nSecond line");
}

#[test]
fn a_property_too_long_is_refused_naming_it() {
    let (mut document, _) = sample();
    let before = document.clone();
    let long = ModelProperties {
        part_number: "7".repeat(MAX_PROPERTY_CHARS + 1),
        ..ModelProperties::default()
    };

    let refused = document.apply(set(long));

    assert_eq!(
        refused,
        Err(EditError::PropertyTooLong {
            property: ModelProperty::PartNumber,
            length: MAX_PROPERTY_CHARS + 1,
        })
    );
    assert!(
        refused
            .unwrap_err()
            .to_string()
            .contains("part number may be at most 200 characters")
    );
    assert_eq!(document, before);
}

#[test]
fn restoring_a_version_brings_its_properties_back() {
    let (earlier, _) = sample();
    let mut later = earlier.clone();
    later.apply(set(bracket())).unwrap();

    let restore = later.transaction_to(&earlier, "Restore");
    let mut restored = later.clone();
    restored.apply(restore).unwrap();
    let forward = earlier.transaction_to(&later, "Restore");
    let mut forwarded = earlier.clone();
    forwarded.apply(forward).unwrap();

    assert!(restored.same_content(&earlier));
    assert!(forwarded.same_content(&later));
}

#[test]
fn a_properties_change_is_touched_and_counted() {
    let transaction = set(bracket());

    let touched = transaction.touched();

    assert!(touched.properties);
    assert!(touched.features.is_empty());
    assert!(transaction.approximate_size() > bracket().heap_size());
}
