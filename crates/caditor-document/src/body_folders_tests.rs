use crate::{
    body_pieces_tests::{piece_of, three_rectangles},
    combine_tests::pair,
    *,
};

fn path(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn folders(document: &Document, bodies: &[FeatureId]) -> Vec<Vec<String>> {
    bodies
        .iter()
        .map(|body| document.body_folder(*body))
        .collect()
}

fn regroup(document: &mut Document, made: Result<Transaction, EditError>) {
    document.apply(made.unwrap()).unwrap();
}

#[test]
fn grouping_bodies_names_their_group_and_undoing_leaves_them_loose() {
    let mut pair = pair();
    let bodies = [pair.plate, pair.peg];

    let grouping = pair
        .document
        .body_grouping(&bodies, &bodies, "Group 2 bodies")
        .unwrap();
    let undo = pair.document.apply(grouping.transaction).unwrap();
    let grouped = folders(&pair.document, &bodies);
    let members = pair.document.body_folder_members(&path(&["Group 1"]));
    let next = pair.document.unused_body_folder_name();
    pair.document.apply(undo).unwrap();

    assert_eq!(grouping.folder, path(&["Group 1"]));
    assert_eq!(grouped, [path(&["Group 1"]), path(&["Group 1"])]);
    assert_eq!(members, bodies);
    assert_eq!(next, "Group 2");
    assert_eq!(folders(&pair.document, &bodies), [path(&[]), path(&[])]);
}

#[test]
fn grouping_part_of_a_group_nests_inside_it_and_grouping_all_of_it_wraps_it() {
    let mut model = three_rectangles();
    let pieces: Vec<FeatureId> = model
        .rectangles
        .iter()
        .map(|drawn| piece_of(model.extrusion, drawn))
        .collect();
    let blocks = model
        .document
        .bodies_moving(&pieces, &path(&["Blocks"]), "Group");
    regroup(&mut model.document, blocks);

    let inner = model
        .document
        .body_grouping(&pieces[..2], &pieces, "Group")
        .unwrap();
    model.document.apply(inner.transaction).unwrap();
    let nested = folders(&model.document, &pieces);
    let outer = model
        .document
        .body_grouping(&pieces, &pieces, "Group")
        .unwrap();
    model.document.apply(outer.transaction).unwrap();

    assert_eq!(inner.folder, path(&["Blocks", "Group 1"]));
    assert_eq!(
        nested,
        [
            path(&["Blocks", "Group 1"]),
            path(&["Blocks", "Group 1"]),
            path(&["Blocks"])
        ]
    );
    assert_eq!(outer.folder, path(&["Group 2"]));
    assert_eq!(
        folders(&model.document, &pieces),
        [
            path(&["Group 2", "Blocks", "Group 1"]),
            path(&["Group 2", "Blocks", "Group 1"]),
            path(&["Group 2", "Blocks"])
        ]
    );
}

#[test]
fn renaming_ungrouping_and_moving_a_group_carry_everything_inside_it() {
    let mut pair = pair();
    let frame = pair
        .document
        .bodies_moving(&[pair.plate], &path(&["Frame", "Plates"]), "Group");
    regroup(&mut pair.document, frame);
    let pins = pair
        .document
        .bodies_moving(&[pair.peg], &path(&["Pins"]), "Group");
    regroup(&mut pair.document, pins);

    let renamed = pair
        .document
        .body_group_renaming(&path(&["Frame"]), "Chassis", "Rename");
    regroup(&mut pair.document, renamed);
    let after_rename = pair.document.body_folder(pair.plate);
    let moved =
        pair.document
            .body_group_moving(&path(&["Pins"]), &path(&["Chassis", "Plates"]), "Move");
    regroup(&mut pair.document, moved);
    let after_move = pair.document.body_folder(pair.peg);
    let into_itself =
        pair.document
            .body_group_moving(&path(&["Chassis"]), &path(&["Chassis", "Plates"]), "Move");
    let ungrouped = pair
        .document
        .body_ungrouping(&path(&["Chassis", "Plates"]), "Ungroup");
    regroup(&mut pair.document, ungrouped);
    let out = pair.document.bodies_moving_out(&[pair.peg], "Out");
    regroup(&mut pair.document, out);

    assert_eq!(after_rename, path(&["Chassis", "Plates"]));
    assert_eq!(after_move, path(&["Chassis", "Plates", "Pins"]));
    assert!(matches!(into_itself, Err(EditError::BodyFolderIntoItself)));
    assert_eq!(pair.document.body_folder(pair.plate), path(&["Chassis"]));
    assert_eq!(pair.document.body_folder(pair.peg), path(&["Chassis"]));
}

#[test]
fn group_names_are_trimmed_lines_and_too_long_or_too_deep_is_refused() {
    let mut pair = pair();

    let trimmed =
        pair.document
            .bodies_moving(&[pair.plate], &path(&["  Base\nparts ", " "]), "Group");
    regroup(&mut pair.document, trimmed);
    let long = "x".repeat(MAX_GROUP_NAME_CHARS + 1);
    let too_long = pair.document.bodies_moving(&[pair.peg], &[long], "Group");
    let deep = vec!["Deep".to_owned(); MAX_BODY_FOLDER_DEPTH + 1];
    let too_deep = pair.document.bodies_moving(&[pair.peg], &deep, "Group");

    assert_eq!(pair.document.body_folder(pair.plate), path(&["Base parts"]));
    assert!(matches!(
        too_long,
        Err(EditError::GroupNameTooLong(length)) if length == MAX_GROUP_NAME_CHARS + 1
    ));
    assert!(matches!(
        too_deep,
        Err(EditError::BodyFolderTooDeep(depth)) if depth == MAX_BODY_FOLDER_DEPTH + 1
    ));
    assert_eq!(folder_label(&path(&["Frame", "Plates"])), "Frame › Plates");
}

#[test]
fn pieces_take_their_feature_group_until_grouped_on_their_own() {
    let mut model = three_rectangles();
    let pieces: Vec<FeatureId> = model
        .rectangles
        .iter()
        .map(|drawn| piece_of(model.extrusion, drawn))
        .collect();

    let whole = model
        .document
        .bodies_moving(&[model.extrusion], &path(&["Blocks"]), "Group");
    regroup(&mut model.document, whole);
    let inherited = folders(&model.document, &pieces);
    let own = model
        .document
        .bodies_moving(&pieces[..1], &path(&["Left"]), "Group");
    regroup(&mut model.document, own);
    let renamed = model
        .document
        .body_group_renaming(&path(&["Blocks"]), "Rest", "Rename");
    regroup(&mut model.document, renamed);

    assert_eq!(inherited, vec![path(&["Blocks"]); 3]);
    assert_eq!(
        model.document.body_folder_members(&path(&["Left"])),
        pieces[..1]
    );
    assert_eq!(
        model
            .document
            .body_folder_names()
            .into_iter()
            .collect::<Vec<_>>(),
        ["Left", "Rest"]
    );
    assert_eq!(
        folders(&model.document, &pieces),
        [path(&["Left"]), path(&["Rest"]), path(&["Rest"])]
    );
}
