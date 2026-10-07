use caditor_expression::{Expression, Unit};

use crate::{
    combine_tests::{Pair, evaluate, pair, rectangle, volume},
    *,
};

const PLATE: f64 = 20.0 * 10.0 * 4.0;
const PEG: f64 = 10.0 * 6.0 * 6.0;
const FROM_PLATE: f64 = 6.0 * 2.0 * 4.0;
const FROM_PEG: f64 = 3.0 * 2.0 * 6.0;

fn cut_through_both(pair: &mut Pair, end: ExtrudeEnd, operation: BodyOperation) -> FeatureId {
    let mut transaction = pair.document.transaction("Cut");
    let slot = transaction.add_feature(
        "Slot sketch",
        FeatureKind::from(rectangle((12.0, 4.0), (18.0, 6.0))),
    );
    let cut = transaction.add_feature(
        "Slot",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: slot,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::OneSide {
                end,
                reversed: false,
            },
            operation,
            start: None,
            other_bodies: vec![pair.peg],
        })),
    );
    pair.document.apply(transaction.finish()).unwrap();
    cut
}

fn close(found: f64, expected: f64) -> bool {
    (found - expected).abs() < 1e-6 * expected
}

#[test]
fn one_cut_removes_its_shape_from_every_body_it_lists() {
    let mut pair = pair();
    let distance = ExtrudeEnd::Distance(Expression::Measure(10.0, Unit::Millimetre));
    let plate = pair.plate;
    let cut = cut_through_both(&mut pair, distance, BodyOperation::Remove(plate));

    let evaluation = evaluate(&pair.document, &mut Recompute::default());

    assert_eq!(evaluation.failed_count(), 0);
    assert!(close(volume(&evaluation, pair.plate), PLATE - FROM_PLATE));
    assert!(close(volume(&evaluation, pair.peg), PEG - FROM_PEG));
    assert_eq!(
        evaluation.bodies().collect::<Vec<_>>(),
        vec![(pair.plate, cut), (pair.peg, cut)]
    );
    assert_eq!(
        pair.document.feature(cut).unwrap().bodies(),
        vec![pair.plate, pair.peg]
    );
    assert_eq!(evaluation.cuts(cut).len(), 1);
}

#[test]
fn through_all_reaches_past_every_body_it_cuts() {
    let mut pair = pair();
    let plate = pair.plate;
    let cut = cut_through_both(
        &mut pair,
        ExtrudeEnd::ThroughAll,
        BodyOperation::Remove(plate),
    );

    let evaluation = evaluate(&pair.document, &mut Recompute::default());

    assert_eq!(
        evaluation.failed_count(),
        0,
        "{:?}",
        evaluation.failures().next()
    );
    assert!(close(volume(&evaluation, pair.peg), PEG - FROM_PEG));
    assert!(close(volume(&evaluation, pair.plate), PLATE - FROM_PLATE));
    assert!(pair.document.feature(cut).is_some());
}

#[test]
fn only_a_removal_takes_other_bodies() {
    let mut pair = pair();
    let distance = ExtrudeEnd::Distance(Expression::Measure(10.0, Unit::Millimetre));
    let plate = pair.plate;
    let cut = cut_through_both(&mut pair, distance, BodyOperation::Add(plate));

    let evaluation = evaluate(&pair.document, &mut Recompute::default());

    let (failed, error) = evaluation.failures().next().unwrap();
    assert_eq!(failed, cut);
    assert!(error.reason.contains("Only a removal cuts several bodies"));
    assert!(close(volume(&evaluation, pair.peg), PEG));
}

#[test]
fn a_cut_listing_its_target_twice_cuts_it_once() {
    let mut pair = pair();
    let mut transaction = pair.document.transaction("Cut");
    let slot = transaction.add_feature(
        "Slot sketch",
        FeatureKind::from(rectangle((12.0, 4.0), (18.0, 6.0))),
    );
    transaction.add_feature(
        "Slot",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: slot,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::Measure(10.0, Unit::Millimetre), false),
            operation: BodyOperation::Remove(pair.plate),
            start: None,
            other_bodies: vec![pair.plate, pair.peg, pair.peg],
        })),
    );
    pair.document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&pair.document, &mut Recompute::default());

    assert_eq!(evaluation.failed_count(), 0);
    assert!(close(volume(&evaluation, pair.plate), PLATE - FROM_PLATE));
    assert!(close(volume(&evaluation, pair.peg), PEG - FROM_PEG));
}

#[test]
fn a_cut_depends_on_every_body_it_cuts() {
    let mut pair = pair();
    let distance = ExtrudeEnd::Distance(Expression::Measure(10.0, Unit::Millimetre));
    let plate = pair.plate;
    let cut = cut_through_both(&mut pair, distance, BodyOperation::Remove(plate));
    assert_eq!(pair.document.dependents_of(&[pair.peg]), vec![cut]);
}
