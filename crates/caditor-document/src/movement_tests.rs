use std::collections::BTreeSet;

use caditor_expression::Expression;
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
        frame: None,
        body: plate,
        more_bodies: Vec::new(),
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
        error.reason().contains("distance along Y"),
        "{}",
        error.reason()
    );
    assert_eq!(evaluation.failed_count(), 1);
    assert!(evaluation.body(pair.peg).is_some());
}

#[test]
fn a_move_past_the_largest_size_fails_the_move_alone_saying_it_is_too_far() {
    let mut pair = pair();

    let movement = moved(
        &mut pair,
        ["100000000000 mm", "0 mm", "0 mm"],
        ["0 deg", "0 deg", "0 deg"],
    );
    let mut engine = Recompute::default();

    let evaluation = evaluate(&pair.document, &mut engine);

    let FeatureState::Failed(error) = &evaluation.feature(movement).unwrap().state else {
        panic!("the move should fail");
    };

    assert!(
        error.reason().contains("too far from the origin"),
        "{}",
        error.reason()
    );
    assert_eq!(error.remedy(), "Enter smaller distances.");
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
        frame: None,
        body: plate,
        more_bodies: Vec::new(),
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

    assert!(matches!(refused, Err(EditError::BodyInUse { .. })));
}

#[test]
fn a_copy_others_use_can_be_edited_while_it_stays_a_copy() {
    let mut pair = pair();

    let copy = copied(&mut pair, ["0 mm", "30 mm", "0 mm"]);
    let mut transaction = pair.document.transaction("Use");

    transaction.add_feature("Remove 1", FeatureKind::Remove(Remove { body: copy }));
    pair.document.apply(transaction.finish()).unwrap();

    let FeatureKind::Move(mut movement) = pair.document.feature(copy).unwrap().kind.clone() else {
        panic!("the copy is a move");
    };

    movement.offset[1] = Expression::parse_stored("45 mm").unwrap();

    let edited = pair.document.apply(Transaction::single(
        "Move the copy",
        Edit::SetFeatureKind {
            id: copy,
            kind: FeatureKind::Move(movement),
        },
    ));

    assert!(edited.is_ok());
}

fn upright_edge(evaluation: &Evaluation, body: FeatureId, x: f64, y: f64) -> AxisReference {
    let solid = evaluation.body(body).unwrap();
    let edge = solid
        .edges()
        .map(|(id, _)| id)
        .find(|id| {
            crate::datum::edge_ray(solid, *id).is_some_and(|ray| {
                ray.direction().z.abs() > 0.99
                    && (ray.origin().x - x).abs() < 1e-9
                    && (ray.origin().y - y).abs() < 1e-9
            })
        })
        .unwrap();
    AxisReference::capture_edge(body, solid, edge).unwrap()
}

fn turned_about(pair: &mut Pair, axis: AxisReference, angle: &str) -> FeatureId {
    let plate = pair.plate;
    let mut transaction = pair.document.transaction("Turn");

    let zero = |unit: &str| std::array::from_fn(|_| transaction.parse(unit).unwrap());
    let movement = Move {
        body: plate,
        more_bodies: Vec::new(),
        offset: zero("0 mm"),
        turn: zero("0 deg"),
        copy: false,
        about: TurnCentre::Axis(Box::new(AxisTurn {
            axis,
            angle: transaction.parse(angle).unwrap(),
        })),
        frame: None,
    };
    let feature = transaction.add_feature("Move 1", FeatureKind::Move(movement));

    pair.document.apply(transaction.finish()).unwrap();
    feature
}

#[test]
fn a_move_turns_its_body_about_an_edge_of_its_own() {
    let mut pair = pair();
    let mut engine = Recompute::default();

    let before = evaluate(&pair.document, &mut engine);
    let names = face_names(&before, pair.plate);
    let axis = upright_edge(&before, pair.plate, 20.0, 0.0);

    let upward = crate::datum::displayed_axis(&before, pair.plate, &axis)
        .unwrap()
        .direction()
        .z
        > 0.0;

    let movement = turned_about(&mut pair, axis, "90 deg");
    let after = evaluate(&pair.document, &mut engine);

    assert_eq!(after.failed_count(), 0);

    let (low, high) = bounds(&after, pair.plate);
    let (expected_low, expected_high) = if upward {
        ([10.0, -20.0, 0.0], [20.0, 0.0, 4.0])
    } else {
        ([20.0, 0.0, 0.0], [30.0, 20.0, 4.0])
    };

    assert!(near(low, expected_low), "{low:?}");
    assert!(near(high, expected_high), "{high:?}");
    assert_eq!(face_names(&after, pair.plate), names);

    let feature = pair.document.feature(movement).unwrap();

    assert_eq!(feature.kind.bodies_used(), BTreeSet::from([pair.plate]));
}

#[test]
fn a_move_about_an_axis_follows_the_angle_and_shifts_after_turning() {
    let mut pair = pair();
    let mut engine = Recompute::default();

    let movement = turned_about(
        &mut pair,
        AxisReference::Principal(PrincipalAxis::Z),
        "180 deg",
    );
    let mut changed = pair.document.feature(movement).unwrap().kind.clone();

    if let FeatureKind::Move(moved) = &mut changed {
        moved.offset[2] = pair.document.parse("5 mm").unwrap();
    }
    pair.document
        .apply(Transaction::single(
            "Shift",
            Edit::SetFeatureKind {
                id: movement,
                kind: changed,
            },
        ))
        .unwrap();

    let after = evaluate(&pair.document, &mut engine);

    let (low, high) = bounds(&after, pair.plate);

    assert!(near(low, [-20.0, -10.0, 5.0]), "{low:?}");
    assert!(near(high, [0.0, 0.0, 9.0]), "{high:?}");
}

#[test]
fn a_move_about_a_datum_axis_depends_on_it_and_fails_alone_without_it() {
    let mut pair = pair();

    let mut transaction = pair.document.transaction("Axis");

    let datum = transaction.add_feature(
        "Axis 1",
        FeatureKind::Datum(Datum::Axis(DatumAxis::Along(AxisReference::Principal(
            PrincipalAxis::X,
        )))),
    );

    pair.document.apply(transaction.finish()).unwrap();

    let movement = turned_about(&mut pair, AxisReference::Datum(datum), "90 deg");
    let mut engine = Recompute::default();

    assert!(pair.document.dependents_of(&[datum]).contains(&movement));

    pair.document
        .apply(Transaction::single(
            "Delete",
            Edit::RemoveFeature { id: datum },
        ))
        .unwrap();

    let evaluation = evaluate(&pair.document, &mut engine);

    assert!(matches!(
        evaluation.feature(movement).unwrap().state,
        FeatureState::Failed(_)
    ));
    assert_eq!(evaluation.failed_count(), 1);
}

#[test]
fn a_move_about_an_axis_inlines_its_angle() {
    let mut pair = pair();

    let mut transaction = pair.document.transaction("Angle");

    let quarter = transaction.add_parameter("quarter", transaction.parse("90 deg").unwrap());

    pair.document.apply(transaction.finish()).unwrap();

    let movement = turned_about(
        &mut pair,
        AxisReference::Principal(PrincipalAxis::Y),
        "quarter",
    );

    let kind = &pair.document.feature(movement).unwrap().kind;

    assert!(kind.uses_parameter(quarter));

    let inlined = pair.document.inline_parameter(quarter).unwrap();

    pair.document.apply(inlined).unwrap();

    assert!(
        !pair
            .document
            .feature(movement)
            .unwrap()
            .kind
            .uses_parameter(quarter)
    );
}

fn moved_both(
    pair: &mut Pair,
    offset: [&str; 3],
    turn: [&str; 3],
    about: TurnCentre,
    copy: bool,
) -> FeatureId {
    let (plate, peg) = (pair.plate, pair.peg);
    let mut transaction = pair.document.transaction("Move both");

    let expressions = |texts: [&str; 3]| texts.map(|text| transaction.parse(text).unwrap());
    let movement = Move {
        body: plate,
        more_bodies: vec![peg, plate],
        offset: expressions(offset),
        turn: expressions(turn),
        copy,
        about,
        frame: None,
    };
    let name = if copy { "Copy 1" } else { "Move 1" };

    let feature = transaction.add_feature(name, FeatureKind::Move(movement));

    pair.document.apply(transaction.finish()).unwrap();
    feature
}

fn move_of(document: &Document, feature: FeatureId) -> Move {
    match &document.feature(feature).unwrap().kind {
        FeatureKind::Move(movement) => movement.clone(),
        other => panic!("expected a move, found {other:?}"),
    }
}

#[test]
fn one_move_shifts_and_turns_several_bodies_together_about_their_combined_centre() {
    let mut pair = pair();
    let mut engine = Recompute::default();

    let before = evaluate(&pair.document, &mut engine);
    let plate_names = face_names(&before, pair.plate);
    let peg_names = face_names(&before, pair.peg);
    let combined = before
        .body(pair.plate)
        .unwrap()
        .bounding_box()
        .unwrap()
        .union(before.body(pair.peg).unwrap().bounding_box().unwrap());
    let centre = combined.center();

    let movement = moved_both(
        &mut pair,
        ["10 mm", "-5 mm", "0 mm"],
        ["0 deg", "0 deg", "180 deg"],
        TurnCentre::Body,
        false,
    );
    let after = evaluate(&pair.document, &mut engine);

    assert_eq!(move_of(&pair.document, movement).more_bodies, [pair.peg]);
    assert_eq!(after.failed_count(), 0);
    assert_eq!(
        pair.document.feature(movement).unwrap().bodies(),
        [pair.plate, pair.peg]
    );
    assert_eq!(pair.document.bodies_standing(), vec![pair.plate, pair.peg]);
    assert_eq!(face_names(&after, pair.plate), plate_names);
    assert_eq!(face_names(&after, pair.peg), peg_names);

    let after_combined = after
        .body(pair.plate)
        .unwrap()
        .bounding_box()
        .unwrap()
        .union(after.body(pair.peg).unwrap().bounding_box().unwrap());
    let expected_centre = centre + caditor_geometry::Vector3::new(10.0, -5.0, 0.0);

    assert!(
        after_combined.center().distance(expected_centre) < 1e-6,
        "combined centre {:?} != {:?}",
        after_combined.center(),
        expected_centre
    );
}

#[test]
fn one_copy_of_several_bodies_makes_a_body_of_each_copy_named_after_its_original() {
    let mut pair = pair();
    let mut engine = Recompute::default();

    let before = evaluate(&pair.document, &mut engine);
    let plate_volume = volume(&before, pair.plate);
    let peg_volume = volume(&before, pair.peg);

    let copy = moved_both(
        &mut pair,
        ["0 mm", "40 mm", "0 mm"],
        ["0 deg", "0 deg", "0 deg"],
        TurnCentre::Origin,
        true,
    );
    let after = evaluate(&pair.document, &mut engine);

    let feature = pair.document.feature(copy).unwrap();

    assert!(feature.makes_body() && feature.separates_pieces());
    assert_eq!(after.failed_count(), 0);

    let copies = after.pieces_made_by(copy);
    let plate_copy = copy.piece(Move::copy_key(pair.plate)).unwrap();
    let peg_copy = copy.piece(Move::copy_key(pair.peg)).unwrap();

    assert_eq!(copies, [plate_copy, peg_copy]);
    assert!(after.body(copy).is_none());
    assert!((volume(&after, plate_copy) - plate_volume).abs() < 1e-6 * plate_volume);
    assert!((volume(&after, peg_copy) - peg_volume).abs() < 1e-6 * peg_volume);

    let (plate_low, _) = bounds(&after, plate_copy);
    let (peg_low, _) = bounds(&after, peg_copy);

    assert!(near(plate_low, [0.0, 40.0, 0.0]), "{plate_low:?}");
    assert!(
        (peg_low.y - (bounds(&before, pair.peg).0.y + 40.0)).abs() < 1e-6,
        "{peg_low:?}"
    );
    assert_eq!(
        pair.document.body_name(peg_copy).as_deref(),
        Some("Copy 1 (copy of Peg)")
    );
    assert_eq!(
        pair.document.bodies_standing_in(&after),
        vec![pair.plate, pair.peg, plate_copy, peg_copy]
    );
}

#[test]
fn a_copy_of_several_bodies_in_use_keeps_making_bodies_and_fails_users_when_left_out() {
    let mut pair = pair();
    let mut engine = Recompute::default();

    let third = {
        let plate = pair.plate;
        let mut transaction = pair.document.transaction("Third");

        let expressions = |texts: [&str; 3]| texts.map(|text| transaction.parse(text).unwrap());
        let feature = transaction.add_feature(
            "Third",
            FeatureKind::Move(Move {
                about: TurnCentre::Origin,
                frame: None,
                body: plate,
                more_bodies: Vec::new(),
                offset: expressions(["50 mm", "0 mm", "0 mm"]),
                turn: expressions(["0 deg", "0 deg", "0 deg"]),
                copy: true,
            }),
        );

        pair.document.apply(transaction.finish()).unwrap();
        feature
    };

    let copy = moved_both(
        &mut pair,
        ["0 mm", "40 mm", "0 mm"],
        ["0 deg", "0 deg", "0 deg"],
        TurnCentre::Origin,
        true,
    );
    let peg_copy = copy.piece(Move::copy_key(pair.peg)).unwrap();
    let mut transaction = pair.document.transaction("Use");

    let removal =
        transaction.add_feature("Remove 1", FeatureKind::Remove(Remove { body: peg_copy }));

    pair.document.apply(transaction.finish()).unwrap();

    let both = move_of(&pair.document, copy);

    let mut as_move = both.clone();

    as_move.copy = false;

    let stopping_copy = pair.document.apply(Transaction::single(
        "Move instead",
        Edit::SetFeatureKind {
            id: copy,
            kind: FeatureKind::Move(as_move),
        },
    ));
    let plate_only = pair.document.apply(Transaction::single(
        "Plate only",
        Edit::SetFeatureKind {
            id: copy,
            kind: FeatureKind::Move(both.clone().with_more_bodies(Vec::new())),
        },
    ));

    pair.document
        .apply(Transaction::single(
            "Swap peg for third",
            Edit::SetFeatureKind {
                id: copy,
                kind: FeatureKind::Move(both.with_more_bodies(vec![third])),
            },
        ))
        .unwrap();

    let after = evaluate(&pair.document, &mut engine);

    assert!(matches!(stopping_copy, Err(EditError::BodyInUse { .. })));
    assert!(matches!(plate_only, Err(EditError::PiecesInUse { .. })));
    assert_eq!(after.failed_count(), 1);

    let FeatureState::Failed(error) = &after.feature(removal).unwrap().state else {
        panic!("the removal should fail");
    };

    assert_eq!(
        error.reason(),
        "Copy 1 no longer copies Peg, so there is no copy to work on."
    );
    assert_eq!(
        error.remedy(),
        "Copy that body again in Copy 1, or choose another body."
    );
}
