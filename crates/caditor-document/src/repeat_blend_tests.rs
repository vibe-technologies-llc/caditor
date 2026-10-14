use std::collections::BTreeSet;

use caditor_expression::{Expression, ParameterId, Unit};
use caditor_geometry::{Plane, Point2};
use caditor_kernel::{EdgeReference, FaceOrigin, FaceReference, Made, Solid};
use caditor_sketch::Sketch;

use crate::{
    pattern_tests::{extrusion, failure, top},
    *,
};

const PLATE: f64 = 40.0 * 40.0 * 4.0;
const PEG_X: f64 = -12.0;

struct Peg {
    document: Document,
    count: ParameterId,
    plate: FeatureId,
    peg: FeatureId,
    rim: FeatureId,
    foot: FeatureId,
}

fn square() -> Sketch {
    let mut sketch = Sketch::new(Plane::XY);
    let corners = [
        Point2::new(-20.0, -20.0),
        Point2::new(20.0, -20.0),
        Point2::new(20.0, 20.0),
        Point2::new(-20.0, 20.0),
    ];
    for (index, corner) in corners.iter().enumerate() {
        sketch.add_line(*corner, corners[(index + 1) % 4]);
    }
    sketch
}

fn evaluate(document: &Document) -> Evaluation {
    Recompute::default().run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
}

fn exact_volume(evaluation: &Evaluation, body: FeatureId) -> f64 {
    evaluation
        .body_result(body)
        .unwrap()
        .solid()
        .unwrap()
        .exact_mass()
        .unwrap()
        .properties
        .volume
}

fn face_made_by(solid: &Solid, origin: FaceOrigin) -> FaceReference {
    let face = solid
        .faces()
        .find(|(_, face)| face.origin() == Some(origin))
        .map(|(id, _)| id)
        .unwrap();
    FaceReference::capture(solid, face).unwrap()
}

fn edge_between(solid: &Solid, first: FaceOrigin, second: FaceOrigin) -> EdgeReference {
    solid
        .edges()
        .filter_map(|(id, _)| EdgeReference::capture(solid, id))
        .find(|reference| {
            let origins = reference.origins();
            origins.contains(&Some(first)) && origins.contains(&Some(second))
        })
        .unwrap()
}

fn fillet(body: FeatureId, edges: Vec<EdgeReference>, groups: Vec<EdgeGroup>) -> FeatureKind {
    FeatureKind::Blend(Blend {
        kind: BlendKind::Fillet,
        body,
        edges,
        groups,
        size: Expression::measure(0.5, Unit::Millimetre),
        form: ChamferForm::Equal,
        flipped: false,
    })
}

fn filleted_peg() -> Peg {
    let mm = |value: f64| Expression::measure(value, Unit::Millimetre);
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let count = transaction.add_parameter("count", transaction.parse("3").unwrap());
    let outline = transaction.add_feature("Outline", FeatureKind::from(square()));
    let plate =
        transaction.add_feature("Plate", extrusion(outline, "4 mm", BodyOperation::NewBody));
    let peg = transaction.add_feature(
        "Peg",
        FeatureKind::Primitive(Primitive {
            shape: PrimitiveShape::Cylinder {
                diameter: mm(4.0),
                height: mm(6.0),
            },
            plane: PlaneReference::Principal(PrincipalPlane::Xy),
            at: [mm(PEG_X), mm(0.0)],
            anchor: PrimitiveAnchor::BaseCentre,
            reversed: false,
            operation: BodyOperation::Add(plate),
        }),
    );
    document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&document);
    let solid = evaluation.body(plate).unwrap();
    let top = face_made_by(solid, FaceOrigin::EndCap { feature: peg.raw() });
    let wall = FaceOrigin::Side {
        feature: peg.raw(),
        entity: 1,
    };
    let foot_edge = edge_between(
        solid,
        wall,
        FaceOrigin::EndCap {
            feature: plate.raw(),
        },
    );

    let mut transaction = document.transaction("Fillets");
    let rim = transaction.add_feature("Rim", fillet(plate, Vec::new(), vec![EdgeGroup::Face(top)]));
    let foot = transaction.add_feature("Foot", fillet(plate, vec![foot_edge], Vec::new()));
    document.apply(transaction.finish()).unwrap();
    Peg {
        document,
        count,
        plate,
        peg,
        rim,
        foot,
    }
}

fn add_pattern(peg: &mut Peg, kind: impl FnOnce(ParameterId) -> PatternKind) -> FeatureId {
    let repeated = vec![peg.peg, peg.rim, peg.foot];
    let pattern = Pattern::new(peg.plate, kind(peg.count)).repeating(repeated);
    let mut transaction = peg.document.transaction("Pattern");
    let pattern = transaction.add_feature("Pattern 1", FeatureKind::from(pattern));
    peg.document.apply(transaction.finish()).unwrap();
    pattern
}

fn linear(count: ParameterId) -> PatternKind {
    PatternKind::Linear {
        first: LinearDirection {
            axis: AxisReference::Principal(PrincipalAxis::X),
            count: Expression::Parameter(count),
            spacing: Expression::measure(10.0, Unit::Millimetre),
            measured: LinearSpacing::BetweenCopies,
            reversed: false,
        },
        second: None,
    }
}

fn circular(count: ParameterId) -> PatternKind {
    PatternKind::Circular(CircularPattern {
        axis: Some(AxisReference::Principal(PrincipalAxis::Z)),
        count: Expression::Parameter(count),
        angle: Expression::measure(360.0, Unit::Degree),
        reversed: false,
    })
}

fn one_peg(evaluation: &Evaluation, peg: &Peg) -> f64 {
    let filleted = evaluation
        .feature(peg.foot)
        .and_then(|status| status.result.as_deref())
        .and_then(FeatureResult::solid)
        .unwrap()
        .exact_mass()
        .unwrap()
        .properties
        .volume;
    filleted - PLATE
}

fn blended_copies(solid: &Solid, repeating: FeatureId, blend: FeatureId) -> BTreeSet<[u32; 2]> {
    solid
        .faces()
        .filter_map(|(_, face)| face.origin())
        .filter_map(|origin| match origin {
            FaceOrigin::Copy {
                copy,
                feature,
                made: Made::Fillet,
            } if copy.pattern == repeating.raw() && feature == blend.raw() => Some(copy.index),
            _ => None,
        })
        .collect()
}

fn assert_close(found: f64, expected: f64) {
    assert!(
        (found - expected).abs() < 1e-3,
        "volume {found} instead of {expected}"
    );
}

fn bare_peg() -> f64 {
    std::f64::consts::PI * 4.0 * 2.0
}

#[test]
fn a_linear_pattern_gives_every_copy_of_a_filleted_peg_its_fillets() {
    let mut peg = filleted_peg();
    let pattern = add_pattern(&mut peg, linear);
    let evaluation = evaluate(&peg.document);

    assert_eq!(evaluation.failed_count(), 0);
    let one = one_peg(&evaluation, &peg);
    assert!((one - bare_peg()).abs() > 0.05);
    assert_close(exact_volume(&evaluation, peg.plate), PLATE + 3.0 * one);
    let solid = evaluation.body(peg.plate).unwrap();
    for blend in [peg.rim, peg.foot] {
        assert_eq!(
            blended_copies(solid, pattern, blend),
            BTreeSet::from([[1, 0], [2, 0]])
        );
    }
}

#[test]
fn raising_the_count_gives_the_new_copies_their_fillets_too() {
    let mut peg = filleted_peg();
    let pattern = add_pattern(&mut peg, linear);
    let mut engine = Recompute::default();
    engine.run(
        &peg.document,
        &ModelEvaluator,
        &CancelToken::never(),
        &|_, _| {},
    );

    let expression = peg.document.parse("4").unwrap();
    peg.document
        .apply(Transaction::single(
            "Count",
            Edit::SetParameterExpression {
                id: peg.count,
                expression,
            },
        ))
        .unwrap();
    let evaluation = engine.run(
        &peg.document,
        &ModelEvaluator,
        &CancelToken::never(),
        &|_, _| {},
    );

    assert_eq!(evaluation.failed_count(), 0);
    let one = one_peg(&evaluation, &peg);
    assert_close(exact_volume(&evaluation, peg.plate), PLATE + 4.0 * one);
    let solid = evaluation.body(peg.plate).unwrap();
    for blend in [peg.rim, peg.foot] {
        assert_eq!(
            blended_copies(solid, pattern, blend),
            BTreeSet::from([[1, 0], [2, 0], [3, 0]])
        );
    }
}

#[test]
fn a_circular_pattern_turns_the_fillets_with_their_peg() {
    let mut peg = filleted_peg();
    let pattern = add_pattern(&mut peg, circular);
    let evaluation = evaluate(&peg.document);

    assert_eq!(evaluation.failed_count(), 0);
    let one = one_peg(&evaluation, &peg);
    assert_close(exact_volume(&evaluation, peg.plate), PLATE + 3.0 * one);
    let solid = evaluation.body(peg.plate).unwrap();
    assert_eq!(
        blended_copies(solid, pattern, peg.foot),
        BTreeSet::from([[1, 0], [2, 0]])
    );
}

#[test]
fn a_mirror_of_features_reflects_the_fillets_of_its_peg() {
    let mut peg = filleted_peg();
    let mirror = Mirror::new(peg.plate, PlaneReference::Principal(PrincipalPlane::Yz))
        .mirroring(vec![peg.peg, peg.rim, peg.foot]);
    let mut transaction = peg.document.transaction("Mirror");
    let mirror = transaction.add_feature("Mirror 1", FeatureKind::Mirror(mirror));
    peg.document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&peg.document);

    assert_eq!(evaluation.failed_count(), 0);
    let one = one_peg(&evaluation, &peg);
    assert_close(exact_volume(&evaluation, peg.plate), PLATE + 2.0 * one);
    let solid = evaluation.body(peg.plate).unwrap();
    for blend in [peg.rim, peg.foot] {
        assert_eq!(
            blended_copies(solid, mirror, blend),
            BTreeSet::from([MIRROR_IMAGE])
        );
    }
}

#[test]
fn a_fillet_repeated_without_the_feature_it_rounds_is_refused_in_words() {
    let mut peg = filleted_peg();
    let pattern = Pattern::new(peg.plate, linear(peg.count)).repeating(vec![peg.rim]);
    let mut transaction = peg.document.transaction("Pattern");
    let pattern = transaction.add_feature("Pattern 1", FeatureKind::from(pattern));
    peg.document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&peg.document);

    let error = failure(&evaluation, pattern);
    assert_eq!(
        error.reason,
        "Rim blends no edge of the features this pattern repeats, so its fillet has nothing to \
         follow on the copies."
    );
    assert_eq!(
        error.remedy,
        "Also pattern the feature whose edges Rim blends, or leave Rim out of the pattern."
    );
    assert_eq!(
        repeatable_on(&peg.document.feature(peg.rim).unwrap().kind),
        Some(peg.plate)
    );
}

#[test]
fn a_fillet_on_the_faces_of_an_earlier_repeated_fillet_follows_them_onto_the_copies() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let count = transaction.add_parameter("count", transaction.parse("3").unwrap());
    let outline = transaction.add_feature("Outline", FeatureKind::from(square()));
    let plate =
        transaction.add_feature("Plate", extrusion(outline, "4 mm", BodyOperation::NewBody));
    let mut boss_outline = Sketch::new(top());
    let corners = [
        Point2::new(-15.0, -2.0),
        Point2::new(-11.0, -2.0),
        Point2::new(-11.0, 2.0),
        Point2::new(-15.0, 2.0),
    ];
    for (index, corner) in corners.iter().enumerate() {
        boss_outline.add_line(*corner, corners[(index + 1) % 4]);
    }
    let boss_outline = transaction.add_feature("Boss outline", FeatureKind::from(boss_outline));
    let boss = transaction.add_feature(
        "Boss",
        extrusion(boss_outline, "2 mm", BodyOperation::Add(plate)),
    );
    document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&document);
    let solid = evaluation.body(plate).unwrap();
    let upright: Vec<EdgeReference> = solid
        .edges()
        .filter_map(|(id, _)| EdgeReference::capture(solid, id))
        .filter(|reference| {
            reference.origins().iter().all(|origin| {
                matches!(origin, Some(FaceOrigin::Side { feature, .. }) if *feature == boss.raw())
            })
        })
        .collect();
    let mut transaction = document.transaction("Corners");
    let corners = transaction.add_feature("Corners", fillet(plate, upright.clone(), Vec::new()));
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document);
    let solid = evaluation.body(plate).unwrap();
    let cap = face_made_by(
        solid,
        FaceOrigin::EndCap {
            feature: boss.raw(),
        },
    );
    let mut transaction = document.transaction("Top");
    let rim = transaction.add_feature("Top", fillet(plate, Vec::new(), vec![EdgeGroup::Face(cap)]));
    let pattern = Pattern::new(plate, linear(count)).repeating(vec![boss, corners, rim]);
    let pattern = transaction.add_feature("Pattern 1", FeatureKind::from(pattern));
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document);

    assert_eq!(upright.len(), 4);
    assert_eq!(evaluation.failed_count(), 0);
    let filleted = evaluation
        .feature(rim)
        .and_then(|status| status.result.as_deref())
        .and_then(FeatureResult::solid)
        .unwrap()
        .exact_mass()
        .unwrap()
        .properties
        .volume;
    assert_close(
        exact_volume(&evaluation, plate),
        PLATE + 3.0 * (filleted - PLATE),
    );
    let solid = evaluation.body(plate).unwrap();
    for blend in [corners, rim] {
        assert_eq!(
            blended_copies(solid, pattern, blend),
            BTreeSet::from([[1, 0], [2, 0]])
        );
    }
}
