use std::collections::BTreeSet;

use caditor_geometry::Point3;

use crate::{
    combine_tests::{Pair, evaluate, pair, volume},
    *,
};

fn moved(pair: &mut Pair, offset: [&str; 3], turn: [&str; 3]) -> FeatureId {
    let plate = pair.plate;
    let mut transaction = pair.document.transaction("Move");
    let expressions = |texts: [&str; 3]| texts.map(|text| transaction.parse(text).unwrap());
    let movement = Move {
        about: TurnCentre::Origin,
        body: plate,
        offset: expressions(offset),
        turn: expressions(turn),
        copy: false,
    };
    let feature = transaction.add_feature("Move 1", FeatureKind::Move(movement));
    pair.document.apply(transaction.finish()).unwrap();
    feature
}

fn bounds(evaluation: &Evaluation, body: FeatureId) -> (Point3, Point3) {
    let bounds = evaluation.body(body).unwrap().bounding_box().unwrap();
    (bounds.min(), bounds.max())
}

fn near(found: Point3, expected: [f64; 3]) -> bool {
    (found - Point3::from_array(expected)).length() < 1e-6
}

fn face_names(evaluation: &Evaluation, body: FeatureId) -> BTreeSet<caditor_kernel::FaceName> {
    evaluation
        .body(body)
        .unwrap()
        .faces()
        .map(|(_, face)| face.name())
        .collect()
}

#[test]
fn a_move_shifts_the_body_by_its_distances_and_keeps_its_volume_and_face_names() {
    let mut pair = pair();
    let mut engine = Recompute::default();
    let before = evaluate(&pair.document, &mut engine);
    let names = face_names(&before, pair.plate);
    let size = volume(&before, pair.plate);

    moved(
        &mut pair,
        ["5 mm", "-2 mm", "1 cm"],
        ["0 deg", "0 deg", "0 deg"],
    );
    let after = evaluate(&pair.document, &mut engine);

    assert_eq!(after.failed_count(), 0);
    let (low, high) = bounds(&after, pair.plate);
    assert!(near(low, [5.0, -2.0, 10.0]), "{low:?}");
    assert!(near(high, [25.0, 8.0, 14.0]), "{high:?}");
    assert!((volume(&after, pair.plate) - size).abs() < 1e-6 * size);
    assert_eq!(face_names(&after, pair.plate), names);
}

#[test]
fn a_move_turns_about_the_origin_before_it_shifts() {
    let mut pair = pair();
    let mut engine = Recompute::default();

    moved(
        &mut pair,
        ["0 mm", "0 mm", "0 mm"],
        ["0 deg", "0 deg", "90 deg"],
    );
    let after = evaluate(&pair.document, &mut engine);

    let (low, high) = bounds(&after, pair.plate);
    assert!(near(low, [-10.0, 0.0, 0.0]), "{low:?}");
    assert!(near(high, [0.0, 20.0, 4.0]), "{high:?}");
}

#[test]
fn a_move_follows_the_parameters_it_uses() {
    let mut pair = pair();
    let mut transaction = pair.document.transaction("Gap");
    let gap = transaction.add_parameter("gap", transaction.parse("3 mm").unwrap());
    pair.document.apply(transaction.finish()).unwrap();
    let movement = moved(
        &mut pair,
        ["gap * 2", "0 mm", "0 mm"],
        ["0 deg", "0 deg", "0 deg"],
    );
    let mut engine = Recompute::default();

    let first = evaluate(&pair.document, &mut engine);
    let expression = pair.document.parse("10 mm").unwrap();
    pair.document
        .apply(Transaction::single(
            "Edit",
            Edit::SetParameterExpression {
                id: gap,
                expression,
            },
        ))
        .unwrap();
    let second = evaluate(&pair.document, &mut engine);

    assert!(near(bounds(&first, pair.plate).0, [6.0, 0.0, 0.0]));
    assert!(near(bounds(&second, pair.plate).0, [20.0, 0.0, 0.0]));
    assert!(
        pair.document
            .feature(movement)
            .unwrap()
            .kind
            .uses_parameter(gap)
    );
}

#[test]
fn a_distance_given_as_an_angle_fails_the_move_alone_saying_which() {
    let mut pair = pair();
    let movement = moved(
        &mut pair,
        ["0 mm", "5 deg", "0 mm"],
        ["0 deg", "0 deg", "0 deg"],
    );
    let mut engine = Recompute::default();

    let evaluation = evaluate(&pair.document, &mut engine);

    let FeatureState::Failed(error) = &evaluation.feature(movement).unwrap().state else {
        panic!("the move should fail");
    };
    assert!(
        error.reason.contains("distance along Y"),
        "{}",
        error.reason
    );
    assert_eq!(evaluation.failed_count(), 1);
    assert!(evaluation.body(pair.peg).is_some());
}

#[test]
fn a_move_modifies_its_body_and_is_dependent_on_it() {
    let mut pair = pair();
    let movement = moved(
        &mut pair,
        ["1 mm", "0 mm", "0 mm"],
        ["0 deg", "0 deg", "0 deg"],
    );
    let feature = pair.document.feature(movement).unwrap();

    assert!(feature.kind.modifies_body());
    assert!(!feature.makes_body());
    assert_eq!(feature.body(), Some(pair.plate));
    assert_eq!(pair.document.dependents_of(&[pair.plate]), vec![movement]);
}

fn copied(pair: &mut Pair, offset: [&str; 3]) -> FeatureId {
    let plate = pair.plate;
    let mut transaction = pair.document.transaction("Copy");
    let expressions = |texts: [&str; 3]| texts.map(|text| transaction.parse(text).unwrap());
    let copy = Move {
        about: TurnCentre::Origin,
        body: plate,
        offset: expressions(offset),
        turn: expressions(["0 deg", "0 deg", "0 deg"]),
        copy: true,
    };
    let feature = transaction.add_feature("Copy 1", FeatureKind::Move(copy));
    pair.document.apply(transaction.finish()).unwrap();
    feature
}

#[test]
fn a_copy_makes_a_new_body_of_its_own_and_leaves_the_original_where_it_was() {
    let mut pair = pair();
    let mut engine = Recompute::default();

    let copy = copied(&mut pair, ["0 mm", "30 mm", "0 mm"]);
    let after = evaluate(&pair.document, &mut engine);

    let feature = pair.document.feature(copy).unwrap();
    assert!(feature.makes_body());
    assert_eq!(feature.body(), Some(copy));
    assert_eq!(after.failed_count(), 0);
    let (low, _) = bounds(&after, pair.plate);
    let (copy_low, copy_high) = bounds(&after, copy);
    assert!(near(low, [0.0, 0.0, 0.0]), "{low:?}");
    assert!(near(copy_low, [0.0, 30.0, 0.0]), "{copy_low:?}");
    assert!(near(copy_high, [20.0, 40.0, 4.0]), "{copy_high:?}");
    assert!((volume(&after, copy) - volume(&after, pair.plate)).abs() < 1e-6);
    assert_eq!(
        pair.document.bodies_standing(),
        vec![pair.plate, pair.peg, copy]
    );
}

#[test]
fn a_copy_others_use_cannot_stop_being_a_copy() {
    let mut pair = pair();
    let copy = copied(&mut pair, ["0 mm", "30 mm", "0 mm"]);
    let mut transaction = pair.document.transaction("Use");
    transaction.add_feature("Remove 1", FeatureKind::Remove(Remove { body: copy }));
    pair.document.apply(transaction.finish()).unwrap();
    let FeatureKind::Move(mut movement) = pair.document.feature(copy).unwrap().kind.clone() else {
        panic!("the copy is a move");
    };
    movement.copy = false;

    let refused = pair.document.apply(Transaction::single(
        "Stop copying",
        Edit::SetFeatureKind {
            id: copy,
            kind: FeatureKind::Move(movement),
        },
    ));

    assert!(refused.is_err());
}
