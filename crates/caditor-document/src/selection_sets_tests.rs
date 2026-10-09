use caditor_kernel::{EdgeReference, FaceOrigin, FaceReference, Solid};

use crate::{
    combine_tests::{Pair, evaluate, pair},
    *,
};

fn set(sets: SelectionSets) -> Transaction {
    Transaction::single(
        "Selection sets",
        Edit::SetSelectionSets {
            sets: Box::new(sets),
        },
    )
}

fn named(name: &str, members: Vec<SetMember>) -> SelectionSets {
    SelectionSets {
        sets: vec![SelectionSet {
            name: name.to_owned(),
            members,
        }],
    }
}

fn top_face(solid: &Solid, feature: FeatureId) -> FaceReference {
    let (top, _) = solid
        .faces()
        .find(|(_, face)| {
            face.origin()
                == Some(FaceOrigin::EndCap {
                    feature: feature.raw(),
                })
        })
        .unwrap();
    FaceReference::capture(solid, top).unwrap()
}

fn plate_top(pair: &Pair) -> SetMember {
    let evaluation = evaluate(&pair.document, &mut Recompute::default());
    let solid = &evaluation
        .body_result(pair.plate)
        .unwrap()
        .solid()
        .unwrap()
        .solid;
    SetMember::Face {
        body: pair.plate,
        face: top_face(solid, pair.plate),
    }
}

#[test]
fn saving_selection_sets_is_undoable_content() {
    let pair = pair();
    let mut editor = Editor::new(pair.document.clone());
    let sets = named("Plate", vec![SetMember::Body(pair.plate)]);

    let changed = editor.apply(set(sets.clone())).unwrap();
    let with_sets = editor.document().clone();
    let again = editor.apply(set(sets.clone())).unwrap();
    editor.undo().unwrap();

    assert!(changed);
    assert!(!again);
    assert_eq!(with_sets.selection_sets(), &sets);
    assert!(!with_sets.same_content(&pair.document));
    assert!(editor.document().same_content(&pair.document));
    assert_eq!(editor.redo_label(), Some("Selection sets"));
    assert!(set(sets).touched().selection_sets);
}

#[test]
fn restoring_an_earlier_version_restores_its_selection_sets() {
    let pair = pair();
    let mut later = pair.document.clone();
    later
        .apply(set(named("Plate", vec![SetMember::Body(pair.plate)])))
        .unwrap();

    let mut restored = later.clone();
    restored
        .apply(later.transaction_to(&pair.document, "Restore"))
        .unwrap();

    assert!(restored.selection_sets().is_empty());
    assert!(restored.same_content(&pair.document));
}

#[test]
fn set_names_are_trimmed_to_one_line() {
    let mut pair = pair();
    let top = plate_top(&pair);
    let sets = named(
        "  Mating\r\n faces \n",
        vec![top.clone(), SetMember::Body(pair.peg)],
    );

    pair.document.apply(set(sets)).unwrap();

    let kept = &pair.document.selection_sets().sets[0];
    assert_eq!(kept.name, "Mating faces");
    assert_eq!(kept.members, vec![top, SetMember::Body(pair.peg)]);
}

#[test]
fn a_blank_taken_or_long_name_an_empty_set_and_too_many_sets_are_refused() {
    let mut pair = pair();
    let before = pair.document.clone();
    let body = || vec![SetMember::Body(pair.plate)];
    let twice = SelectionSets {
        sets: vec![
            named("Bolts", body()).sets.remove(0),
            named("bolts", body()).sets.remove(0),
        ],
    };
    let too_many = SelectionSets {
        sets: (0..=MAX_SELECTION_SETS)
            .map(|index| named(&format!("Set {index}"), body()).sets.remove(0))
            .collect(),
    };

    let blank = pair.document.apply(set(named(" \n ", body())));
    let taken = pair.document.apply(set(twice));
    let long = pair
        .document
        .apply(set(named(&"s".repeat(MAX_SET_NAME_CHARS + 1), body())));
    let empty = pair.document.apply(set(named("Nothing", Vec::new())));
    let many = pair.document.apply(set(too_many));

    assert_eq!(blank, Err(EditError::SetNameEmpty));
    assert_eq!(taken, Err(EditError::SetNameTaken("bolts".to_owned())));
    assert_eq!(
        long,
        Err(EditError::SetNameTooLong {
            length: MAX_SET_NAME_CHARS + 1
        })
    );
    assert_eq!(empty, Err(EditError::SetEmpty("Nothing".to_owned())));
    assert_eq!(many, Err(EditError::TooManySets));
    assert_eq!(pair.document, before);
}

#[test]
fn a_set_naming_a_deleted_body_keeps_new_features_from_taking_its_id() {
    let mut pair = pair();
    let gone = FeatureId::from_raw(pair.document.next_feature_id() + 5);

    pair.document
        .apply(set(named("Gone", vec![SetMember::Body(gone)])))
        .unwrap();

    assert_eq!(pair.document.next_feature_id(), gone.raw() + 1);
}

#[test]
fn a_body_member_resolves_to_every_face_and_a_lost_body_is_named() {
    let pair = pair();
    let evaluation = evaluate(&pair.document, &mut Recompute::default());
    let missing = FeatureId::from_raw(pair.document.next_feature_id() + 1);
    let chosen = SelectionSet {
        name: "Both".to_owned(),
        members: vec![SetMember::Body(pair.peg), SetMember::Body(missing)],
    };

    let resolved = chosen.resolve(&evaluation);

    assert_eq!(resolved.found.len(), 6);
    assert!(
        resolved
            .found
            .iter()
            .all(|item| matches!(item, SetItem::Face { body, .. } if *body == pair.peg))
    );
    assert_eq!(resolved.lost, vec![SetLoss::Body(missing)]);
}

#[test]
fn a_face_and_an_edge_are_found_again_after_a_later_cut_changes_the_body() {
    let mut pair = pair();
    let evaluation = evaluate(&pair.document, &mut Recompute::default());
    let solid = &evaluation
        .body_result(pair.plate)
        .unwrap()
        .solid()
        .unwrap()
        .solid;
    let top = top_face(solid, pair.plate);
    let (edge, _) = solid.edges().next().unwrap();
    let edge = EdgeReference::capture(solid, edge).unwrap();
    let chosen = SelectionSet {
        name: "Top".to_owned(),
        members: vec![
            SetMember::Face {
                body: pair.plate,
                face: top.clone(),
            },
            SetMember::Edge {
                body: pair.plate,
                edge,
            },
        ],
    };
    let mut transaction = pair.document.transaction("Cut");
    transaction.add_feature(
        "Combine 1",
        FeatureKind::Combine(Combine::new(pair.plate, pair.peg, CombineOperation::Cut)),
    );
    pair.document.apply(transaction.finish()).unwrap();

    let cut = evaluate(&pair.document, &mut Recompute::default());
    let resolved = chosen.resolve(&cut);

    let solid = &cut.body_result(pair.plate).unwrap().solid().unwrap().solid;
    let notched = top.resolve(solid).unwrap();
    assert!(resolved.lost.is_empty(), "{:?}", resolved.lost);
    assert_eq!(resolved.found.len(), 2);
    assert_eq!(
        resolved.found[0],
        SetItem::Face {
            body: pair.plate,
            face: notched
        }
    );
    assert!(matches!(resolved.found[1], SetItem::Edge { body, .. } if body == pair.plate));
}
