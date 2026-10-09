use std::sync::Arc;

use caditor_expression::{Expression, ParameterId};
use caditor_geometry::Plane;

use crate::{solid_tests::rectangle, *};

struct Model {
    document: Document,
    depth: ParameterId,
    base: FeatureId,
}

fn model() -> Model {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let depth = transaction.add_parameter("depth", transaction.parse("4 mm").unwrap());
    let outline = transaction.add_feature(
        "Outline",
        FeatureKind::from(rectangle(Plane::XY, (0.0, 0.0), (10.0, 8.0))),
    );
    let base = transaction.add_feature(
        "Base",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::Parameter(depth), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
        })),
    );
    document.apply(transaction.finish()).unwrap();
    Model {
        document,
        depth,
        base,
    }
}

fn evaluate(document: &Document, engine: &mut Recompute) -> Evaluation {
    engine.run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
}

fn set_depth(model: &mut Model, text: &str) -> Transaction {
    let expression = model.document.parse(text).unwrap();
    model
        .document
        .apply(Transaction::single(
            "Depth",
            Edit::SetParameterExpression {
                id: model.depth,
                expression,
            },
        ))
        .unwrap()
}

#[test]
fn undoing_a_change_takes_the_earlier_result_back_without_recomputing() {
    let mut model = model();
    let mut engine = Recompute::default();
    let first = evaluate(&model.document, &mut engine);
    let undo = set_depth(&mut model, "6 mm");
    let changed = evaluate(&model.document, &mut engine);

    model.document.apply(undo).unwrap();
    let undone = evaluate(&model.document, &mut engine);

    let result = |evaluation: &Evaluation| evaluation.body_result(model.base).cloned().unwrap();
    assert!(changed.recomputed().contains(&model.base));
    assert!(!undone.recomputed().contains(&model.base));
    assert!(Arc::ptr_eq(&result(&first), &result(&undone)));
    assert_eq!(engine.results_kept(model.base), 2);
}

#[test]
fn results_kept_per_feature_are_bounded_by_count_and_earlier_ones_by_size() {
    let mut model = model();
    let mut engine = Recompute::default();
    let mut tight = Recompute::default().keeping_earlier_results_within(0);
    for depth in 1..=8 {
        set_depth(&mut model, &format!("{depth} mm"));
        evaluate(&model.document, &mut engine);
        evaluate(&model.document, &mut tight);
    }

    assert_eq!(
        engine.results_kept(model.base),
        crate::history::RESULTS_KEPT_PER_FEATURE
    );
    assert_eq!(tight.results_kept(model.base), 1);
}
