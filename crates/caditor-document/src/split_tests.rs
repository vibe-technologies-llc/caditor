use caditor_expression::Expression;

use crate::{
    combine_tests::{block, evaluate, volume},
    *,
};

const HALF: f64 = 10.0 * 10.0 * 4.0;

struct Model {
    document: Document,
    plate: FeatureId,
    split: FeatureId,
}

fn split_plate(plane: PlaneReference, flipped: bool) -> Model {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let plate = block(
        &mut transaction,
        "Plate",
        (-10.0, -5.0),
        (10.0, 5.0),
        "4 mm",
    );
    let split = transaction.add_feature(
        "Split 1",
        FeatureKind::Split(Split {
            body: plate,
            plane,
            flipped,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    Model {
        document,
        plate,
        split,
    }
}

fn x_range(evaluation: &Evaluation, body: FeatureId) -> (f64, f64) {
    let bounds = evaluation.body(body).unwrap().bounding_box().unwrap();
    (bounds.min().x, bounds.max().x)
}

fn near(found: (f64, f64), expected: (f64, f64)) -> bool {
    (found.0 - expected.0).abs() < 1e-6 && (found.1 - expected.1).abs() < 1e-6
}

#[test]
fn a_split_keeps_the_side_the_plane_faces_and_makes_the_other_a_body_of_its_own() {
    let model = split_plate(PlaneReference::Principal(PrincipalPlane::Yz), false);

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    assert_eq!(evaluation.failed_count(), 0);
    assert_eq!(
        evaluation.bodies().collect::<Vec<_>>(),
        vec![(model.plate, model.split), (model.split, model.split)]
    );
    assert!(near(x_range(&evaluation, model.plate), (0.0, 10.0)));
    assert!(near(x_range(&evaluation, model.split), (-10.0, 0.0)));
    assert!((volume(&evaluation, model.plate) - HALF).abs() < 1e-6 * HALF);
    assert!((volume(&evaluation, model.split) - HALF).abs() < 1e-6 * HALF);
    assert_eq!(
        model.document.feature(model.split).unwrap().bodies(),
        vec![model.plate, model.split]
    );
}

#[test]
fn a_flipped_split_keeps_the_other_side() {
    let model = split_plate(PlaneReference::Principal(PrincipalPlane::Yz), true);

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    assert!(near(x_range(&evaluation, model.plate), (-10.0, 0.0)));
    assert!(near(x_range(&evaluation, model.split), (0.0, 10.0)));
}

#[test]
fn a_plane_that_misses_the_body_fails_the_split_alone_in_words() {
    let model = split_plate(PlaneReference::Principal(PrincipalPlane::Xy), false);

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    let (failed, error) = evaluation.failures().next().unwrap();
    assert_eq!(failed, model.split);
    assert!(
        error
            .reason
            .contains("does not pass through the body of Plate")
    );
    assert!(near(x_range(&evaluation, model.plate), (-10.0, 10.0)));
    assert!(evaluation.body(model.split).is_none());
}

#[test]
fn the_split_off_body_takes_later_features_of_its_own() {
    let mut model = split_plate(PlaneReference::Principal(PrincipalPlane::Yz), false);
    let mut transaction = model.document.transaction("Move");
    let moved = transaction.add_feature(
        "Move 1",
        FeatureKind::Move(Move {
            body: model.split,
            offset: [
                Expression::parse_stored("-5 mm").unwrap(),
                Expression::parse_stored("0 mm").unwrap(),
                Expression::parse_stored("0 mm").unwrap(),
            ],
            turn: [
                Expression::parse_stored("0 deg").unwrap(),
                Expression::parse_stored("0 deg").unwrap(),
                Expression::parse_stored("0 deg").unwrap(),
            ],
            copy: false,
        }),
    );
    model.document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();

    let evaluation = evaluate(&model.document, &mut engine);

    assert_eq!(evaluation.failed_count(), 0);
    assert!(near(x_range(&evaluation, model.split), (-15.0, -5.0)));
    assert!(near(x_range(&evaluation, model.plate), (0.0, 10.0)));
    assert_eq!(
        evaluation.bodies().collect::<Vec<_>>(),
        vec![(model.plate, model.split), (model.split, moved)]
    );
}

#[test]
fn a_failed_split_keeps_its_last_split_off_body_shown_as_stale() {
    let mut model = split_plate(PlaneReference::Principal(PrincipalPlane::Yz), false);
    let mut engine = Recompute::default();
    evaluate(&model.document, &mut engine);
    let mut transaction = model.document.transaction("Move the plane");
    transaction.edit(Edit::SetFeatureKind {
        id: model.split,
        kind: FeatureKind::Split(Split {
            body: model.plate,
            plane: PlaneReference::Principal(PrincipalPlane::Xy),
            flipped: false,
        }),
    });
    model.document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&model.document, &mut engine);

    assert_eq!(evaluation.failed_count(), 1);
    assert!(evaluation.is_stale(model.split));
    assert!(near(x_range(&evaluation, model.split), (-10.0, 0.0)));
    assert!(near(x_range(&evaluation, model.plate), (-10.0, 10.0)));
}
