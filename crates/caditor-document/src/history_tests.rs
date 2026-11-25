use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use caditor_expression::{Expression, ParameterId};
use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;

use crate::{
    history::SharedStorage,
    solid_tests::{extrude, rectangle},
    *,
};

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
            direction: None,
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

#[test]
fn earlier_results_are_held_at_the_size_of_their_solids_and_meshes() {
    let mut model = model();
    let mut engine = Recompute::default();

    let first = evaluate(&model.document, &mut engine);
    let undo = set_depth(&mut model, "6 mm");

    evaluate(&model.document, &mut engine);
    model.document.apply(undo).unwrap();
    evaluate(&model.document, &mut engine);
    set_depth(&mut model, "7 mm");
    evaluate(&model.document, &mut engine);

    let base = first.body_result(model.base).unwrap().solid().unwrap();
    let mesh = base.mesh().unwrap().approximate_size();
    let (held, measured) = engine.earlier_results_held();

    assert_eq!(held, measured);
    assert!(held >= 2 * (base.solid.approximate_size() + mesh), "{held}");

    let mut cleared = Recompute::default().keeping_earlier_results_within(0);

    evaluate(&model.document, &mut cleared);
    set_depth(&mut model, "8 mm");
    evaluate(&model.document, &mut cleared);

    assert_eq!(cleared.earlier_results_held(), (0, 0));
}

#[test]
fn a_draft_copy_shares_every_cache_entry_and_changes_none_of_the_originals() {
    let mut model = model();
    let mut engine = Recompute::default();

    evaluate(&model.document, &mut engine);

    let mut draft = engine.draft_copy();

    let copied = draft.entries_shared_with(&engine);

    set_depth(&mut model, "6 mm");

    let drafted = evaluate(&model.document, &mut draft);
    let original = evaluate(&model.document, &mut engine);

    assert_eq!(copied, (2, 2));
    assert!(drafted.recomputed().contains(&model.base));
    assert!(original.recomputed().contains(&model.base));
    assert_eq!(draft.results_kept(model.base), 2);
    assert!(draft.threads_shared_with(&engine));
}

#[test]
fn a_draft_copy_shares_the_cache_until_it_writes_and_its_writes_stay_its_own() {
    let mut model = model();
    let mut engine = Recompute::default();

    for depth in ["5 mm", "6 mm", "7 mm"] {
        set_depth(&mut model, depth);
        evaluate(&model.document, &mut engine);
    }

    let kept = engine.results_kept(model.base);
    let held = engine.earlier_results_held();
    let shared = |results, recency, holdings| SharedStorage {
        results,
        recency,
        holdings,
    };

    let mut draft = engine.draft_copy();

    let copied = draft.cache_storage_shared_with(&engine);

    evaluate(&model.document, &mut draft);

    let reused = draft.cache_storage_shared_with(&engine);

    let mut extended = model.document.clone();

    let mut transaction = extended.transaction("Add");

    transaction.add_feature(
        "Another outline",
        FeatureKind::from(rectangle(Plane::XY, (20.0, 0.0), (24.0, 4.0))),
    );
    extended.apply(transaction.finish()).unwrap();

    let mut previewing = engine.draft_copy();

    evaluate(&extended, &mut previewing);

    let added = previewing.cache_storage_shared_with(&engine);

    set_depth(&mut model, "8 mm");
    evaluate(&model.document, &mut draft);

    let changed = draft.cache_storage_shared_with(&engine);

    assert_eq!(kept, 3);
    assert_eq!(copied, shared(true, true, true));
    assert_eq!(reused, shared(true, false, true));
    assert_eq!(added, shared(false, false, true));
    assert_eq!(changed, shared(false, false, false));
    assert_eq!(engine.results_kept(model.base), kept);
    assert_eq!(engine.earlier_results_held(), held);
    assert_eq!(draft.results_kept(model.base), kept + 1);
    assert_eq!(
        draft.earlier_results_held().0,
        draft.earlier_results_held().1
    );
    assert!(draft.earlier_results_held().0 > held.0);
}

#[test]
fn retrying_failures_in_a_draft_copy_leaves_the_original_cache_alone() {
    let mut model = model();
    let mut engine = Recompute::default();

    set_depth(&mut model, "0 mm");

    let failed = evaluate(&model.document, &mut engine);
    let mut draft = engine.draft_copy();

    draft.retry_failures();

    let retried = evaluate(&model.document, &mut draft);
    let again = evaluate(&model.document, &mut engine);

    assert!(matches!(
        failed.feature(model.base).unwrap().state,
        FeatureState::Failed(_)
    ));
    assert!(retried.recomputed().contains(&model.base));
    assert!(!again.recomputed().contains(&model.base));
}

fn drilled_plate(holes: usize) -> (Document, ParameterId, Vec<FeatureId>) {
    let mut document = Document::default();

    let mut transaction = document.transaction("Build");

    let depth = transaction.add_parameter("depth", transaction.parse("6 mm").unwrap());
    let outline = transaction.add_feature(
        "Outline",
        FeatureKind::from(rectangle(Plane::XY, (0.0, 0.0), (40.0, 40.0))),
    );

    let plate = transaction.add_feature(
        "Plate",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::Parameter(depth), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    );
    let mut states = vec![plate];

    for index in 0..holes {
        let mut sketch = Sketch::new(Plane::XY);
        let column = (index % 6) as f64;
        let row = (index / 6) as f64;

        sketch.add_circle(Point2::new(5.0 + 6.0 * column, 5.0 + 6.0 * row), 1.5);

        let circle = transaction.add_feature(format!("Circle {index}"), FeatureKind::from(sketch));

        states.push(transaction.add_feature(
            format!("Hole {index}"),
            extrude(circle, "20 mm", false, BodyOperation::Remove(plate)),
        ));
    }
    document.apply(transaction.finish()).unwrap();
    (document, depth, states)
}

#[test]
fn earlier_results_count_the_geometry_their_solids_share_once() {
    let (mut document, depth, states) = drilled_plate(12);
    let mut engine = Recompute::default();

    let first = evaluate(&document, &mut engine);
    let deeper = document.parse("7 mm").unwrap();
    let undo = document
        .apply(Transaction::single(
            "Depth",
            Edit::SetParameterExpression {
                id: depth,
                expression: deeper,
            },
        ))
        .unwrap();
    let changed = evaluate(&document, &mut engine);

    let (held, measured) = engine.earlier_results_held();
    let in_full = engine.earlier_results_in_full();

    assert_eq!(changed.failed_count(), 0);
    assert!(
        states
            .iter()
            .all(|state| changed.recomputed().contains(state))
    );
    assert_eq!(held, measured);
    assert!(2 * held < in_full, "{held} of {in_full}");

    document.apply(undo).unwrap();

    let undone = evaluate(&document, &mut engine);
    let body = |evaluation: &Evaluation| evaluation.body_result(states[0]).cloned().unwrap();
    let (held, measured) = engine.earlier_results_held();

    assert!(Arc::ptr_eq(&body(&first), &body(&undone)));
    assert_eq!(held, measured);
    assert!(2 * held < engine.earlier_results_in_full());
}

fn block_of_depth(
    transaction: &mut TransactionBuilder<'_>,
    index: usize,
    depth: ParameterId,
) -> FeatureId {
    let at = index as f64 * 2.0;
    let outline = transaction.add_feature(
        format!("Outline {index}"),
        FeatureKind::from(rectangle(Plane::XY, (at, 0.0), (at + 1.0, 1.0))),
    );

    transaction.add_feature(
        format!("Block {index}"),
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::Parameter(depth), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    )
}

fn blocks_of_one_depth(count: usize) -> (Document, ParameterId) {
    let mut document = Document::default();

    let mut transaction = document.transaction("Build");

    let depth = transaction.add_parameter("depth", transaction.parse("1 mm").unwrap());

    for index in 0..count {
        block_of_depth(&mut transaction, index, depth);
    }
    document.apply(transaction.finish()).unwrap();
    (document, depth)
}

fn set_parameter(document: &mut Document, id: ParameterId, text: &str) {
    let expression = document.parse(text).unwrap();

    document
        .apply(Transaction::single(
            "Depth",
            Edit::SetParameterExpression { id, expression },
        ))
        .unwrap();
}

fn filled_history(count: usize) -> (Document, ParameterId, Recompute) {
    let (mut document, depth) = blocks_of_one_depth(count);
    let mut engine = Recompute::default();

    for millimetres in 1..=crate::history::RESULTS_KEPT_PER_FEATURE {
        set_parameter(&mut document, depth, &format!("{millimetres} mm"));
        evaluate(&document, &mut engine);
    }
    (document, depth, engine)
}

fn earlier_results_after_both_blocks_change(budget: Option<usize>) -> (Recompute, [FeatureId; 2]) {
    let mut document = Document::default();

    let mut transaction = document.transaction("Build");

    let first_depth = transaction.add_parameter("first", transaction.parse("1 mm").unwrap());
    let second_depth = transaction.add_parameter("second", transaction.parse("1 mm").unwrap());
    let first = block_of_depth(&mut transaction, 0, first_depth);
    let second = block_of_depth(&mut transaction, 1, second_depth);

    document.apply(transaction.finish()).unwrap();

    let mut engine = match budget {
        Some(budget) => Recompute::default().keeping_earlier_results_within(budget),
        None => Recompute::default(),
    };

    evaluate(&document, &mut engine);
    set_parameter(&mut document, second_depth, "2 mm");
    evaluate(&document, &mut engine);
    set_parameter(&mut document, first_depth, "2 mm");
    evaluate(&document, &mut engine);
    (engine, [first, second])
}

#[test]
fn an_entry_reused_as_its_features_latest_counts_as_used_when_it_becomes_earlier() {
    let (unbounded, _) = earlier_results_after_both_blocks_change(None);
    let (both, _) = unbounded.earlier_results_held();

    let (bounded, [first, second]) = earlier_results_after_both_blocks_change(Some(both - 1));

    assert_eq!(unbounded.earlier_results_kept(), 2);
    assert_eq!(bounded.results_kept(first), 2);
    assert_eq!(bounded.results_kept(second), 1);
}

fn best_of(runs: usize, mut run: impl FnMut()) -> Duration {
    let mut best = Duration::MAX;

    for _ in 0..runs {
        let started = Instant::now();

        run();
        best = best.min(started.elapsed());
    }
    best
}

#[test]
#[ignore = "a timing benchmark: cargo test --release -p caditor-document draft_copy_costs -- --ignored --nocapture"]
fn draft_copy_costs() {
    for count in [250, 1_000] {
        let (mut document, depth, engine) = filled_history(count);

        let (held, _) = engine.earlier_results_held();
        let copy = best_of(20, || drop(engine.draft_copy()));
        let unchanged = best_of(5, || {
            evaluate(&document, &mut engine.draft_copy());
        });

        set_parameter(&mut document, depth, "9 mm");

        let changed = best_of(5, || {
            evaluate(&document, &mut engine.draft_copy());
        });

        println!(
            "{} features, {} earlier results of {} KiB in {} buffers: a draft copy {copy:.1?}, \
             a draft run {unchanged:.1?} unchanged and {changed:.1?} with every block changed",
            2 * count,
            engine.earlier_results_kept(),
            held / 1024,
            engine.earlier_buffers_held(),
        );
    }
}
