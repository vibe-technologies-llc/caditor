use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
    time::{Duration, Instant},
};

use caditor_geometry::{Plane, Point3, Vector3};

use crate::{
    solid_tests::{extrude, rectangle},
    *,
};

const SLOW: &str = "Pocket";
const TIMEOUT: Duration = Duration::from_secs(20);

struct SlowPocket {
    release: Arc<AtomicBool>,
}

impl Evaluator for SlowPocket {
    fn evaluate(
        &self,
        feature: &Feature,
        inputs: &Inputs<'_>,
        cancel: &CancelToken,
    ) -> Result<FeatureResult, Failure> {
        while feature.name == SLOW && !self.release.load(Ordering::SeqCst) {
            if cancel.is_cancelled() {
                return Err(Failure::Cancelled);
            }
            thread::yield_now();
        }
        ModelEvaluator.evaluate(feature, inputs, cancel)
    }
}

struct TwoBodies {
    document: Document,
    right: FeatureId,
    first: FeatureId,
    second: FeatureId,
    hole: FeatureId,
    pocket: FeatureId,
}

fn two_bodies() -> TwoBodies {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let left = transaction.add_feature(
        "Left outline",
        FeatureKind::from(rectangle(Plane::XY, (0.0, 0.0), (10.0, 8.0))),
    );
    let first = transaction.add_feature(
        "First",
        extrude(left, "4 mm", false, BodyOperation::NewBody),
    );
    let right = transaction.add_feature(
        "Right outline",
        FeatureKind::from(rectangle(Plane::XY, (20.0, 0.0), (30.0, 8.0))),
    );
    let second = transaction.add_feature(
        "Second",
        extrude(right, "4 mm", false, BodyOperation::NewBody),
    );
    let top = Plane::from_frame(Point3::new(0.0, 0.0, 4.0), Vector3::Z, Vector3::X).unwrap();
    let hole = transaction.add_feature(
        "Hole outline",
        FeatureKind::from(rectangle(top, (22.0, 2.0), (24.0, 4.0))),
    );
    let pocket = transaction.add_feature(
        SLOW,
        extrude(hole, "2 mm", true, BodyOperation::Remove(second)),
    );
    document.apply(transaction.finish()).unwrap();
    TwoBodies {
        document,
        right,
        first,
        second,
        hole,
        pocket,
    }
}

fn slow_pocket() -> (SlowPocket, Arc<AtomicBool>) {
    let release = Arc::new(AtomicBool::new(false));
    let evaluator = SlowPocket {
        release: Arc::clone(&release),
    };
    (evaluator, release)
}

fn run_shown_at_once(
    mut recompute: Recompute,
    document: Document,
    evaluator: SlowPocket,
) -> (Receiver<Evaluation>, thread::JoinHandle<Evaluation>) {
    let (reports, received) = mpsc::channel();
    let running = thread::spawn(move || {
        recompute.report_features_done_after(Duration::ZERO);
        recompute.run_reporting(
            &document,
            &evaluator,
            &CancelToken::never(),
            &|_, _| {},
            &|shown| {
                let _ = reports.send(shown);
            },
        )
    });
    (received, running)
}

fn shown_once(received: &Receiver<Evaluation>, wanted: impl Fn(&Evaluation) -> bool) -> Evaluation {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let shown = received
            .recv_timeout(left)
            .expect("the evaluation wanted should be shown");
        if wanted(&shown) {
            return shown;
        }
    }
}

fn meshed(evaluation: &Evaluation, body: FeatureId) -> bool {
    evaluation
        .body_result(body)
        .and_then(|result| result.solid())
        .is_some_and(SolidResult::is_meshed)
}

#[test]
fn a_body_no_later_feature_changes_is_meshed_and_shown_while_a_slow_feature_runs() {
    let model = two_bodies();
    let (evaluator, release) = slow_pocket();

    let (received, running) =
        run_shown_at_once(Recompute::default(), model.document.clone(), evaluator);
    let shown = shown_once(&received, |shown| {
        meshed(shown, model.first) && !shown.is_pending(model.second)
    });
    release.store(true, Ordering::SeqCst);
    let evaluation = running.join().unwrap();

    assert!(!shown.is_complete());
    assert!(shown.is_pending(model.pocket));
    assert!(shown.feature(model.pocket).is_none());
    assert_eq!(
        shown.feature(model.second).unwrap().state,
        FeatureState::UpToDate
    );
    assert!(!meshed(&shown, model.second));
    assert!(evaluation.is_complete());
    assert!(!evaluation.is_pending(model.pocket));
    assert!(meshed(&evaluation, model.second));
    assert!(Arc::ptr_eq(
        shown.body_result(model.first).unwrap(),
        evaluation.body_result(model.first).unwrap()
    ));
}

#[test]
fn features_not_reached_yet_keep_what_they_last_showed() {
    let model = two_bodies();
    let (evaluator, release) = slow_pocket();
    let mut recompute = Recompute::default();
    release.store(true, Ordering::SeqCst);
    let before = recompute.run(
        &model.document,
        &evaluator,
        &CancelToken::never(),
        &|_, _| {},
    );
    release.store(false, Ordering::SeqCst);
    let mut document = model.document.clone();
    document
        .apply(Transaction::single(
            "Taller",
            Edit::SetFeatureKind {
                id: model.second,
                kind: extrude(model.right, "5 mm", false, BodyOperation::NewBody),
            },
        ))
        .unwrap();

    let (received, running) = run_shown_at_once(recompute, document, evaluator);
    let shown = shown_once(&received, |shown| shown.is_pending(model.pocket));
    release.store(true, Ordering::SeqCst);
    let after = running.join().unwrap();

    assert_eq!(shown.feature(model.pocket), before.feature(model.pocket));
    assert!(Arc::ptr_eq(
        shown.body_result(model.second).unwrap(),
        before.body_result(model.second).unwrap()
    ));
    assert!(meshed(&shown, model.second));
    assert_ne!(shown.feature(model.second), before.feature(model.second));
    assert!(!Arc::ptr_eq(
        after.body_result(model.second).unwrap(),
        before.body_result(model.second).unwrap()
    ));
    assert!(after.is_complete());
}

#[test]
fn a_run_that_changes_nothing_before_a_slow_feature_shows_nothing_until_it_ends() {
    let model = two_bodies();
    let (evaluator, release) = slow_pocket();
    let mut recompute = Recompute::default();
    release.store(true, Ordering::SeqCst);
    recompute.run(
        &model.document,
        &evaluator,
        &CancelToken::never(),
        &|_, _| {},
    );
    release.store(false, Ordering::SeqCst);
    let mut document = model.document.clone();
    document
        .apply(Transaction::single(
            "Deeper",
            Edit::SetFeatureKind {
                id: model.pocket,
                kind: extrude(
                    model.hole,
                    "3 mm",
                    true,
                    BodyOperation::Remove(model.second),
                ),
            },
        ))
        .unwrap();

    let (received, running) = run_shown_at_once(recompute, document, evaluator);
    thread::sleep(Duration::from_millis(50));
    let early: Vec<Evaluation> = received.try_iter().collect();
    release.store(true, Ordering::SeqCst);
    running.join().unwrap();

    assert!(early.is_empty());
}

#[test]
fn a_changed_body_meshes_again_only_the_faces_that_changed() {
    let model = two_bodies();
    let mut recompute = Recompute::default();
    let run = |recompute: &mut Recompute, document: &Document| {
        recompute.run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
    };
    let reused = |evaluation: &Evaluation| {
        let solid = evaluation
            .body_result(model.second)
            .unwrap()
            .solid()
            .unwrap();
        let faces = solid.solid.faces().count();
        (solid.display_mesh().unwrap().reused_faces(), faces)
    };
    let mesh = |evaluation: &Evaluation| {
        let solid = evaluation
            .body_result(model.second)
            .unwrap()
            .solid()
            .unwrap();
        solid.mesh().unwrap().clone()
    };
    let deeper = Transaction::single(
        "Deeper",
        Edit::SetFeatureKind {
            id: model.pocket,
            kind: extrude(
                model.hole,
                "3 mm",
                true,
                BodyOperation::Remove(model.second),
            ),
        },
    );

    let before = run(&mut recompute, &model.document);
    let mut document = model.document.clone();
    document.apply(deeper).unwrap();
    let after = run(&mut recompute, &document);
    let fresh = run(&mut Recompute::default(), &document);

    assert_eq!(reused(&before), (0, 11));
    assert_eq!(reused(&after), (6, 11));
    assert_eq!(mesh(&after), mesh(&fresh));
}
