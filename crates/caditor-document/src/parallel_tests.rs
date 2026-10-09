use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_sketch::Sketch;

use crate::{
    combine_tests::block,
    pool::available_workers,
    solid_tests::{extrude, rectangle},
    *,
};

const PLATE_SIDE: f64 = 30.0;
const PLATE_GAP: f64 = 10.0;
const PLATE_HEIGHT: f64 = 6.0;
const WORKERS: usize = 8;
const TIMEOUT: Duration = Duration::from_secs(20);

struct Plate {
    body: FeatureId,
    holes: FeatureId,
    pockets: FeatureId,
}

fn holes_on(origin: f64, per_side: usize) -> Sketch {
    let top =
        Plane::from_frame(Point3::new(0.0, 0.0, PLATE_HEIGHT), Vector3::Z, Vector3::X).unwrap();
    let mut sketch = Sketch::new(top);
    let step = PLATE_SIDE / (per_side as f64 + 1.0);
    for row in 1..=per_side {
        for column in 1..=per_side {
            sketch.add_circle(
                Point2::new(origin + step * column as f64, step * row as f64),
                step * 0.3,
            );
        }
    }
    sketch
}

fn add_plate(transaction: &mut TransactionBuilder<'_>, index: usize, per_side: usize) -> Plate {
    let origin = index as f64 * (PLATE_SIDE + PLATE_GAP);
    let outline = transaction.add_feature(
        format!("Outline {index}"),
        FeatureKind::from(rectangle(
            Plane::XY,
            (origin, 0.0),
            (origin + PLATE_SIDE, PLATE_SIDE),
        )),
    );
    let body = transaction.add_feature(
        format!("Plate {index}"),
        extrude(outline, "6 mm", false, BodyOperation::NewBody),
    );
    let holes = transaction.add_feature(
        format!("Holes {index}"),
        FeatureKind::from(holes_on(origin, per_side)),
    );
    let pockets = transaction.add_feature(
        format!("Pockets {index}"),
        extrude(holes, "3 mm", true, BodyOperation::Remove(body)),
    );
    Plate {
        body,
        holes,
        pockets,
    }
}

fn plates(count: usize, per_side: usize) -> (Document, Vec<Plate>) {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let plates = (0..count)
        .map(|index| add_plate(&mut transaction, index, per_side))
        .collect();
    document.apply(transaction.finish()).unwrap();
    (document, plates)
}

struct Mixed {
    document: Document,
    plates: Vec<Plate>,
    combine: FeatureId,
    peg_cut: FeatureId,
    bad: FeatureId,
    after_bad: FeatureId,
}

fn mixed() -> Mixed {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let plates: Vec<Plate> = (0..4)
        .map(|index| add_plate(&mut transaction, index, 3))
        .collect();
    let peg = block(&mut transaction, "Peg", (5.0, 5.0), (9.0, 9.0), "12 mm");
    let combine = transaction.add_feature(
        "Join",
        FeatureKind::Combine(Combine::new(plates[0].body, peg, CombineOperation::Join)),
    );
    let peg_cut = transaction.add_feature(
        "Peg cut",
        extrude(plates[0].holes, "1 mm", true, BodyOperation::Remove(peg)),
    );
    let bad = transaction.add_feature(
        "Bad pocket",
        extrude(
            plates[2].holes,
            "0 mm",
            true,
            BodyOperation::Remove(plates[2].body),
        ),
    );
    let after_bad = transaction.add_feature(
        "After bad",
        extrude(
            plates[2].holes,
            "5 mm",
            true,
            BodyOperation::Remove(plates[2].body),
        ),
    );
    document.apply(transaction.finish()).unwrap();
    Mixed {
        document,
        plates,
        combine,
        peg_cut,
        bad,
        after_bad,
    }
}

fn evaluate(engine: &mut Recompute, document: &Document) -> Evaluation {
    engine.run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
}

#[test]
fn a_parallel_recompute_equals_the_sequential_one() {
    let mut model = mixed();
    let mut sequential = Recompute::default().with_workers(0);
    let mut parallel = Recompute::default().with_workers(WORKERS);

    let alone = evaluate(&mut sequential, &model.document);
    let together = evaluate(&mut parallel, &model.document);

    assert_eq!(together, alone);
    assert_eq!(together.recomputed(), alone.recomputed());
    assert!(together.is_complete());
    assert_eq!(together.failed_count(), 2);
    assert!(matches!(
        &together.feature(model.bad).unwrap().state,
        FeatureState::Failed(_)
    ));
    let FeatureState::Failed(consumed) = &together.feature(model.peg_cut).unwrap().state else {
        panic!("a feature cutting a body a combine consumed should fail");
    };
    assert!(consumed.reason.starts_with("Join combined"));
    assert_eq!(consumed.fix, Some(FixTarget::Feature(model.combine)));
    assert_eq!(
        together.feature(model.after_bad).unwrap().state,
        FeatureState::UpToDate
    );
    for plate in &model.plates {
        assert_eq!(
            together.feature(plate.pockets).unwrap().state,
            FeatureState::UpToDate
        );
    }

    model
        .document
        .apply(Transaction::single(
            "Taller",
            Edit::SetFeatureKind {
                id: model.plates[1].body,
                kind: extrude(
                    model.document.feature(model.plates[1].body).map_or_else(
                        || panic!("the plate should exist"),
                        |plate| plate.kind.solid().unwrap().sketch(),
                    ),
                    "7 mm",
                    false,
                    BodyOperation::NewBody,
                ),
            },
        ))
        .unwrap();
    let alone = evaluate(&mut sequential, &model.document);
    let together = evaluate(&mut parallel, &model.document);

    assert_eq!(together, alone);
    assert_eq!(together.recomputed(), alone.recomputed());
    assert_eq!(
        together.recomputed(),
        [model.plates[1].body, model.plates[1].pockets]
    );
}

struct SlowPockets {
    started: Arc<AtomicUsize>,
    spinning: Arc<AtomicUsize>,
}

impl Evaluator for SlowPockets {
    fn evaluate(
        &self,
        feature: &Feature,
        inputs: &Inputs<'_>,
        cancel: &CancelToken,
    ) -> Result<FeatureResult, Failure> {
        if !feature.name.starts_with("Pockets") {
            return ModelEvaluator.evaluate(feature, inputs, cancel);
        }
        self.started.fetch_add(1, Ordering::SeqCst);
        self.spinning.fetch_add(1, Ordering::SeqCst);
        while !cancel.is_cancelled() {
            thread::yield_now();
        }
        self.spinning.fetch_sub(1, Ordering::SeqCst);
        Err(Failure::Cancelled)
    }
}

#[test]
fn independent_features_run_at_once_and_a_cancel_stops_them_all_promptly() {
    let (document, plates) = plates(6, 2);
    let started = Arc::new(AtomicUsize::new(0));
    let spinning = Arc::new(AtomicUsize::new(0));
    let evaluator = SlowPockets {
        started: Arc::clone(&started),
        spinning: Arc::clone(&spinning),
    };
    let stop = Arc::new(AtomicBool::new(false));
    let cancel = {
        let stop = Arc::clone(&stop);
        CancelToken::new(move || stop.load(Ordering::SeqCst))
    };
    let (finished, evaluated) = mpsc::channel();

    let running = thread::spawn(move || {
        let evaluation = Recompute::default().with_workers(WORKERS).run(
            &document,
            &evaluator,
            &cancel,
            &|_, _| {},
        );
        let _ = finished.send(evaluation);
    });
    let deadline = Instant::now() + TIMEOUT;
    while spinning.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
        thread::yield_now();
    }
    let concurrent = spinning.load(Ordering::SeqCst);
    stop.store(true, Ordering::SeqCst);
    let cancelled_at = Instant::now();
    let evaluation = evaluated.recv_timeout(TIMEOUT).unwrap();
    let stopped_after = cancelled_at.elapsed();
    let started_in_all = started.load(Ordering::SeqCst);
    running.join().unwrap();

    assert!(concurrent >= 2);
    assert!(stopped_after < Duration::from_secs(2));
    assert!(started_in_all <= plates.len());
    assert!(!evaluation.is_complete());
    for plate in &plates {
        assert_eq!(
            evaluation.feature(plate.pockets).unwrap().state,
            FeatureState::Outdated
        );
    }
}

fn timed(runs: usize, mut run: impl FnMut()) -> Duration {
    let mut best = Duration::MAX;
    for _ in 0..runs {
        let started = Instant::now();
        run();
        best = best.min(started.elapsed());
    }
    best
}

#[test]
#[ignore = "a timing benchmark: cargo test --release -p caditor-document parallel_recompute_costs -- --ignored --nocapture"]
fn parallel_recompute_costs() {
    let (document, plates) = plates(8, 6);
    let evaluation = evaluate(&mut Recompute::default(), &document);
    assert_eq!(evaluation.failed_count(), 0);
    assert!(evaluation.is_complete());
    assert_eq!(evaluation.bodies().count(), plates.len());

    for workers in [0, available_workers()] {
        let shown = timed(5, || {
            evaluate(&mut Recompute::default().with_workers(workers), &document);
        });
        let unshown = timed(5, || {
            Recompute::default()
                .with_workers(workers)
                .run_without_display(&document, &ModelEvaluator, &CancelToken::never());
        });
        println!(
            "8 independent plates of 36 pockets on {workers} workers: {shown:.1?} with meshes, \
             {unshown:.1?} without"
        );
    }
}

fn blocks(count: usize) -> Document {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    for index in 0..count {
        let at = index as f64 * 2.0;
        block(
            &mut transaction,
            &format!("Block {index}"),
            (at, 0.0),
            (at + 1.0, 1.0),
            "1 mm",
        );
    }
    document.apply(transaction.finish()).unwrap();
    document
}

#[test]
#[ignore = "a timing benchmark: cargo test --release -p caditor-document long_tree_recompute_costs -- --ignored --nocapture"]
fn long_tree_recompute_costs() {
    for count in [250, 1_000] {
        let document = blocks(count);
        let mut longer = document.clone();
        let mut transaction = longer.transaction("Add");
        block(&mut transaction, "Last", (-4.0, 0.0), (-3.0, 1.0), "1 mm");
        longer.apply(transaction.finish()).unwrap();

        let shown = timed(3, || {
            evaluate(&mut Recompute::default(), &document);
        });
        let unshown = timed(3, || {
            Recompute::default().run_without_display(
                &document,
                &ModelEvaluator,
                &CancelToken::never(),
            );
        });
        let mut engine = Recompute::default();
        evaluate(&mut engine, &document);
        let unchanged = timed(3, || {
            evaluate(&mut engine, &document);
        });
        let appended = timed(3, || {
            evaluate(&mut engine.clone(), &longer);
        });
        println!(
            "{} features: {shown:.1?} with meshes, {unshown:.1?} without, {unchanged:.1?} again \
             unchanged, {appended:.1?} with one block appended",
            2 * count
        );
    }
}
