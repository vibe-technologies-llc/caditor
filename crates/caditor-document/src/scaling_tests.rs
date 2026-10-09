use std::collections::BTreeSet;

use caditor_geometry::Point3;
use caditor_kernel::FaceName;

use crate::{
    combine_tests::{Pair, evaluate, pair, volume},
    *,
};

const PLATE_VOLUME: f64 = 20.0 * 10.0 * 4.0;

fn scaled(pair: &mut Pair, factor: &str, center: [&str; 3]) -> FeatureId {
    let plate = pair.plate;
    let mut transaction = pair.document.transaction("Scale");
    let scale = Scale {
        body: plate,
        factor: transaction.parse(factor).unwrap(),
        center: center.map(|text| transaction.parse(text).unwrap()),
        frame: None,
    };
    let feature = transaction.add_feature("Scale 1", FeatureKind::Scale(scale));
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

fn face_names(evaluation: &Evaluation, body: FeatureId) -> BTreeSet<FaceName> {
    evaluation
        .body(body)
        .unwrap()
        .faces()
        .map(|(_, face)| face.name())
        .collect()
}

fn reason(evaluation: &Evaluation, feature: FeatureId) -> String {
    match &evaluation.feature(feature).unwrap().state {
        FeatureState::Failed(error) => error.reason.clone(),
        state => panic!("the scale should fail, not {state:?}"),
    }
}

const ORIGIN: [&str; 3] = ["0 mm", "0 mm", "0 mm"];

#[test]
fn a_scale_about_the_origin_stretches_the_body_and_keeps_its_face_names() {
    let mut pair = pair();
    let mut engine = Recompute::default();
    let before = evaluate(&pair.document, &mut engine);
    let names = face_names(&before, pair.plate);

    scaled(&mut pair, "2", ORIGIN);
    let after = evaluate(&pair.document, &mut engine);

    assert_eq!(after.failed_count(), 0);
    let (low, high) = bounds(&after, pair.plate);
    assert!(near(low, [0.0, 0.0, 0.0]), "{low:?}");
    assert!(near(high, [40.0, 20.0, 8.0]), "{high:?}");
    assert!((volume(&after, pair.plate) - 8.0 * PLATE_VOLUME).abs() < 1e-6 * PLATE_VOLUME);
    assert_eq!(face_names(&after, pair.plate), names);
}

#[test]
fn a_scale_keeps_its_centre_in_place() {
    let mut pair = pair();
    let mut engine = Recompute::default();

    scaled(&mut pair, "0.5", ["10 mm", "5 mm", "0 mm"]);
    let after = evaluate(&pair.document, &mut engine);

    let (low, high) = bounds(&after, pair.plate);
    assert!(near(low, [5.0, 2.5, 0.0]), "{low:?}");
    assert!(near(high, [15.0, 7.5, 2.0]), "{high:?}");
}

#[test]
fn an_inch_part_scaled_to_millimetres_follows_its_parameter() {
    let mut pair = pair();
    let mut transaction = pair.document.transaction("Factor");
    let factor = transaction.add_parameter("factor", transaction.parse("25.4").unwrap());
    pair.document.apply(transaction.finish()).unwrap();
    let scale = scaled(&mut pair, "factor", ORIGIN);
    let mut engine = Recompute::default();

    let first = evaluate(&pair.document, &mut engine);
    let expression = pair.document.parse("2").unwrap();
    pair.document
        .apply(Transaction::single(
            "Edit",
            Edit::SetParameterExpression {
                id: factor,
                expression,
            },
        ))
        .unwrap();
    let second = evaluate(&pair.document, &mut engine);

    assert!(near(bounds(&first, pair.plate).1, [508.0, 254.0, 101.6]));
    assert!(near(bounds(&second, pair.plate).1, [40.0, 20.0, 8.0]));
    assert!(
        pair.document
            .feature(scale)
            .unwrap()
            .kind
            .uses_parameter(factor)
    );
}

#[test]
fn a_factor_of_zero_or_a_length_fails_the_scale_alone_saying_why() {
    let mut pair = pair();
    let zero = scaled(&mut pair, "0", ORIGIN);
    let mut engine = Recompute::default();

    let after = evaluate(&pair.document, &mut engine);

    assert_eq!(
        reason(&after, zero),
        "The scale factor must be more than zero, and 0 is not."
    );
    assert_eq!(after.failed_count(), 1);
    assert!(after.body(pair.peg).is_some());

    let mut pair = self::pair();
    let length = scaled(&mut pair, "2 mm", ORIGIN);
    let after = evaluate(&pair.document, &mut engine);
    assert!(
        reason(&after, length).contains("scale factor"),
        "{}",
        reason(&after, length)
    );
}

#[test]
fn a_scale_reaching_beyond_a_kilometre_fails_saying_so() {
    let mut pair = pair();
    let scale = scaled(&mut pair, "100000", ORIGIN);
    let mut engine = Recompute::default();

    let after = evaluate(&pair.document, &mut engine);

    assert!(
        reason(&after, scale).contains("farther than a kilometre"),
        "{}",
        reason(&after, scale)
    );
}

#[test]
fn a_scale_changes_its_body_without_hiding_the_result_and_is_dependent_on_it() {
    let mut pair = pair();
    let scale = scaled(&mut pair, "3", ORIGIN);
    let feature = pair.document.feature(scale).unwrap();

    assert!(!feature.kind.modifies_body());
    assert!(!feature.makes_body());
    assert_eq!(feature.body(), Some(pair.plate));
    assert_eq!(pair.document.dependents_of(&[pair.plate]), vec![scale]);
}
