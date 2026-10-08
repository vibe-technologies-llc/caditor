use caditor_geometry::Plane;
use caditor_sketch::Sketch;

use crate::*;

fn sketches(count: usize) -> (Document, Vec<FeatureId>) {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let ids = (1..=count)
        .map(|number| {
            transaction.add_feature(
                format!("Sketch {number}"),
                FeatureKind::from(Sketch::new(Plane::XY)),
            )
        })
        .collect();
    document.apply(transaction.finish()).unwrap();
    (document, ids)
}

fn groups(document: &Document) -> Vec<Option<String>> {
    document
        .features()
        .map(|feature| feature.group.clone())
        .collect()
}

fn named(names: &[Option<&str>]) -> Vec<Option<String>> {
    names.iter().map(|name| name.map(str::to_owned)).collect()
}

fn order(document: &Document) -> Vec<FeatureId> {
    document.features().map(Feature::id).collect()
}

#[test]
fn grouping_names_the_chosen_features_and_undoing_leaves_them_ungrouped() {
    let (mut document, ids) = sketches(4);
    let name = document.unused_group_name();

    let grouping = document
        .grouping(&ids[1..3], &name, "Group features")
        .unwrap();
    let undo = document.apply(grouping).unwrap();
    let grouped = groups(&document);
    let run = document.group_run(ids[2]);
    document.apply(undo).unwrap();

    assert_eq!(name, "Group 1");
    assert_eq!(
        grouped,
        named(&[None, Some("Group 1"), Some("Group 1"), None])
    );
    assert_eq!(run, ids[1..3]);
    assert_eq!(groups(&document), vec![None; 4]);
}

#[test]
fn grouping_features_apart_brings_them_together_under_the_first() {
    let (mut document, ids) = sketches(4);

    let grouping = document
        .grouping(&[ids[0], ids[3]], "Holes", "Group features")
        .unwrap();
    document.apply(grouping).unwrap();

    assert_eq!(order(&document), [ids[0], ids[3], ids[1], ids[2]]);
    assert_eq!(
        groups(&document),
        named(&[Some("Holes"), Some("Holes"), None, None])
    );
    assert_eq!(document.group_run(ids[0]), [ids[0], ids[3]]);
    assert_eq!(document.unused_group_name(), "Group 1");
}

#[test]
fn moving_into_the_middle_of_a_group_joins_it_and_moving_out_leaves_it() {
    let (mut document, ids) = sketches(4);
    let grouping = document
        .grouping(&ids[0..2], "Base", "Group features")
        .unwrap();
    document.apply(grouping).unwrap();

    let joining = document
        .move_row(TreeRow::Feature(ids[3]), 1, "Move")
        .unwrap();
    document.apply(joining).unwrap();
    let joined = groups(&document);
    let leaving = document
        .move_row(TreeRow::Feature(ids[0]), 4, "Move")
        .unwrap();
    document.apply(leaving).unwrap();

    assert_eq!(
        joined,
        named(&[Some("Base"), Some("Base"), Some("Base"), None])
    );
    assert_eq!(order(&document), [ids[3], ids[1], ids[2], ids[0]]);
    assert_eq!(
        groups(&document),
        named(&[Some("Base"), Some("Base"), None, None])
    );
}

#[test]
fn a_group_name_is_one_trimmed_line_and_a_long_one_is_refused() {
    let (mut document, ids) = sketches(2);
    let set = |group: &str| {
        Transaction::single(
            "Group",
            Edit::SetFeatureGroup {
                id: ids[0],
                group: Some(group.to_owned()),
            },
        )
    };

    document.apply(set("  Mounting\nholes  ")).unwrap();
    let trimmed = groups(&document);
    document.apply(set("   ")).unwrap();
    let blank = groups(&document);
    let refused = document.apply(set(&"x".repeat(MAX_GROUP_NAME_CHARS + 1)));

    assert_eq!(trimmed, named(&[Some("Mounting holes"), None]));
    assert_eq!(blank, [None, None]);
    assert_eq!(
        refused,
        Err(EditError::GroupNameTooLong(MAX_GROUP_NAME_CHARS + 1))
    );
}

#[test]
fn renaming_or_ungrouping_a_run_changes_only_its_members_and_counts_as_content() {
    let (mut document, ids) = sketches(3);
    let before = document.clone();
    let grouping = document.grouping(&ids[0..2], "A", "Group").unwrap();
    document.apply(grouping).unwrap();
    let grouped = document.clone();

    let renaming = document
        .regrouping(&document.group_run(ids[0]), Some("B".to_owned()), "Rename")
        .unwrap();
    document.apply(renaming).unwrap();
    let renamed = groups(&document);
    let ungrouping = document
        .regrouping(&document.group_run(ids[1]), None, "Ungroup")
        .unwrap();
    document.apply(ungrouping).unwrap();

    assert!(!grouped.same_content(&before));
    assert_eq!(renamed, named(&[Some("B"), Some("B"), None]));
    assert_eq!(groups(&document), vec![None; 3]);
    assert!(document.same_content(&before));
}
