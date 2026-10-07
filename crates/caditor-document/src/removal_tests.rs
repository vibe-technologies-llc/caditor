use crate::{
    combine_tests::{Pair, pair},
    *,
};

fn evaluate(document: &Document) -> Evaluation {
    Recompute::default().run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
}

fn add(document: &mut Document, name: &str, kind: FeatureKind) -> FeatureId {
    let mut transaction = document.transaction("Add");
    let feature = transaction.add_feature(name, kind);
    document.apply(transaction.finish()).unwrap();
    feature
}

fn remove_peg(pair: &mut Pair) -> FeatureId {
    add(
        &mut pair.document,
        "Remove 1",
        FeatureKind::Remove(Remove { body: pair.peg }),
    )
}

#[test]
fn a_removed_body_no_longer_stands_and_comes_back_when_the_removal_is_suppressed() {
    let mut pair = pair();
    let removal = remove_peg(&mut pair);

    let evaluation = evaluate(&pair.document);
    let standing = pair.document.bodies_standing();
    pair.document
        .apply(Transaction::single(
            "Suppress",
            Edit::SetFeatureSuppressed {
                id: removal,
                suppressed: true,
            },
        ))
        .unwrap();
    let restored = evaluate(&pair.document);

    assert_eq!(standing, vec![pair.plate]);
    assert!(evaluation.body(pair.peg).is_none());
    assert!(evaluation.body(pair.plate).is_some());
    assert_eq!(evaluation.failed_count(), 0);
    assert!(restored.body(pair.peg).is_some());
    assert_eq!(
        pair.document
            .feature(removal)
            .unwrap()
            .kind
            .consumed_bodies(),
        vec![pair.peg]
    );
}

#[test]
fn a_feature_using_a_removed_body_fails_naming_the_removal() {
    let mut pair = pair();
    remove_peg(&mut pair);
    let moved = add(
        &mut pair.document,
        "Move 1",
        FeatureKind::Move(Move {
            body: pair.peg,
            offset: [0.0, 0.0, 1.0].map(|value| {
                caditor_expression::Expression::Measure(value, caditor_expression::Unit::Millimetre)
            }),
            turn: [0.0; 3].map(|value| {
                caditor_expression::Expression::Measure(value, caditor_expression::Unit::Degree)
            }),
        }),
    );

    let evaluation = evaluate(&pair.document);

    let FeatureState::Failed(error) = &evaluation.feature(moved).unwrap().state else {
        panic!("moving a removed body fails");
    };
    assert_eq!(
        error.reason,
        "Remove 1 removed the body made by Peg, so it no longer stands."
    );
}

#[test]
fn a_removal_keeps_its_body_used_so_it_cannot_move_above_it() {
    let mut pair = pair();
    let removal = remove_peg(&mut pair);

    let above = pair.document.apply(Transaction::single(
        "Move up",
        Edit::MoveFeature {
            id: removal,
            index: 0,
        },
    ));

    assert!(above.is_err());
    assert!(pair.document.dependents_of(&[pair.peg]).contains(&removal));
}

#[test]
fn a_body_keeps_a_name_of_its_own_beside_its_feature() {
    let mut pair = pair();
    let name = |appearance: &str| BodyAppearance {
        name: Some(appearance.to_owned()),
        ..BodyAppearance::default()
    };
    let set =
        |id, appearance| Transaction::single("Name", Edit::SetBodyAppearance { id, appearance });

    pair.document
        .apply(set(pair.peg, name("  Locating\npeg ")))
        .unwrap();
    let refused = pair
        .document
        .apply(set(pair.peg, name(&"x".repeat(MAX_BODY_NAME_CHARS + 1))));

    assert_eq!(pair.document.body_name(pair.peg), Some("Locating peg"));
    assert_eq!(pair.document.body_name(pair.plate), Some("Plate"));
    assert_eq!(
        refused,
        Err(EditError::BodyNameTooLong(MAX_BODY_NAME_CHARS + 1))
    );
}
