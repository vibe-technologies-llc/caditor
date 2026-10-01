use std::{
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, TryRecvError},
    },
    thread,
};

use caditor_kernel::{MeshQuality, interruptible};
use parking_lot::Mutex;

use crate::{
    document::Document,
    recompute::{CancelToken, Evaluation, Evaluator, FeatureResult, Recompute},
};

const NO_JOB: u64 = u64::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Finished,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone)]
pub struct Update {
    pub revision: u64,
    pub outcome: Outcome,
    pub evaluation: Evaluation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the recompute worker stopped")]
pub struct WorkerStopped;

struct Job {
    sequence: u64,
    cancels: u64,
    revision: u64,
    retry_failures: bool,
    document: Document,
}

enum Message {
    Recompute(Job),
    Mesh {
        result: Arc<FeatureResult>,
        name: String,
    },
    MeshQuality(MeshQuality),
}

struct Shared {
    latest: AtomicU64,
    cancels: AtomicU64,
    progress: Mutex<Option<Progress>>,
}

impl Shared {
    fn is_cancelled(&self, sequence: u64, cancels: u64) -> bool {
        self.latest.load(Ordering::SeqCst) != sequence
            || self.cancels.load(Ordering::SeqCst) != cancels
    }
}

pub struct Recomputer {
    jobs: mpsc::Sender<Message>,
    updates: mpsc::Receiver<Update>,
    shared: Arc<Shared>,
    next_sequence: u64,
}

impl Recomputer {
    pub fn spawn(
        evaluator: impl Evaluator,
        wake: impl Fn() + Send + 'static,
    ) -> std::io::Result<Self> {
        let (jobs, queue) = mpsc::channel();
        let (sender, updates) = mpsc::channel();
        let shared = Arc::new(Shared {
            latest: AtomicU64::new(NO_JOB),
            cancels: AtomicU64::new(0),
            progress: Mutex::new(None),
        });
        let worker = Arc::clone(&shared);
        thread::Builder::new()
            .name("recompute".to_owned())
            .spawn(move || work(&evaluator, &queue, &sender, &worker, &wake))?;
        Ok(Self {
            jobs,
            updates,
            shared,
            next_sequence: 0,
        })
    }

    pub fn submit(&mut self, document: Document, revision: u64) -> Result<(), WorkerStopped> {
        self.enqueue(document, revision, false)
    }

    pub fn submit_retrying_failures(
        &mut self,
        document: Document,
        revision: u64,
    ) -> Result<(), WorkerStopped> {
        self.enqueue(document, revision, true)
    }

    fn enqueue(
        &mut self,
        document: Document,
        revision: u64,
        retry_failures: bool,
    ) -> Result<(), WorkerStopped> {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        self.shared.latest.store(sequence, Ordering::SeqCst);
        self.jobs
            .send(Message::Recompute(Job {
                sequence,
                cancels: self.shared.cancels.load(Ordering::SeqCst),
                revision,
                retry_failures,
                document,
            }))
            .map_err(|_| WorkerStopped)
    }

    pub fn mesh(&self, result: Arc<FeatureResult>, name: String) -> Result<(), WorkerStopped> {
        self.jobs
            .send(Message::Mesh { result, name })
            .map_err(|_| WorkerStopped)
    }

    pub fn set_mesh_quality(&self, quality: MeshQuality) -> Result<(), WorkerStopped> {
        self.jobs
            .send(Message::MeshQuality(quality))
            .map_err(|_| WorkerStopped)
    }

    pub fn cancel(&self) {
        self.shared.cancels.fetch_add(1, Ordering::SeqCst);
    }

    pub fn progress(&self) -> Option<Progress> {
        *self.shared.progress.lock()
    }

    pub fn poll(&self) -> Result<Option<Update>, WorkerStopped> {
        let mut latest = None;
        loop {
            match self.updates.try_recv() {
                Ok(update) => latest = Some(update),
                Err(TryRecvError::Empty) => return Ok(latest),
                Err(TryRecvError::Disconnected) => return latest.map(Some).ok_or(WorkerStopped),
            }
        }
    }

    #[cfg(test)]
    fn wait(&self) -> Update {
        self.updates
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the worker should report within the timeout")
    }
}

struct PendingMesh {
    result: Arc<FeatureResult>,
    name: String,
}

fn work(
    evaluator: &dyn Evaluator,
    queue: &mpsc::Receiver<Message>,
    updates: &mpsc::Sender<Update>,
    shared: &Arc<Shared>,
    wake: &dyn Fn(),
) {
    let mut recompute = Recompute::default();
    let mut reported = Evaluation::default();
    let mut meshes: Vec<PendingMesh> = Vec::new();
    while let Ok(first) = queue.recv() {
        let mut latest: Option<Job> = None;
        for message in std::iter::once(first).chain(std::iter::from_fn(|| queue.try_recv().ok())) {
            match message {
                Message::Recompute(mut job) => {
                    job.retry_failures |= latest.as_ref().is_some_and(|job| job.retry_failures);
                    latest = Some(job);
                }
                Message::Mesh { result, name } => {
                    meshes.retain(|pending| !Arc::ptr_eq(&pending.result, &result));
                    meshes.push(PendingMesh { result, name });
                }
                Message::MeshQuality(quality) => recompute.set_mesh_quality(quality),
            }
        }
        if let Some(job) = latest {
            if job.retry_failures {
                recompute.retry_failures();
            }
            let Some(ran) = run_contained(&mut recompute, &job, evaluator, shared) else {
                continue;
            };
            let (outcome, evaluation) = match ran {
                Ok(evaluation) => {
                    reported = evaluation.clone();
                    let outcome = if evaluation.is_complete() {
                        Outcome::Finished
                    } else {
                        Outcome::Cancelled
                    };
                    (outcome, evaluation)
                }
                Err(Panicked) => (Outcome::Failed, reported.clone()),
            };
            let update = Update {
                revision: job.revision,
                outcome,
                evaluation,
            };
            if updates.send(update).is_err() {
                break;
            }
            wake();
        }
        mesh_pending(&mut meshes, &recompute.mesh_quality(), shared, wake);
    }
}

struct Panicked;

fn run_contained(
    recompute: &mut Recompute,
    job: &Job,
    evaluator: &dyn Evaluator,
    shared: &Arc<Shared>,
) -> Option<Result<Evaluation, Panicked>> {
    let (sequence, cancels) = (job.sequence, job.cancels);
    let watched = Arc::clone(shared);
    let cancel = CancelToken::new(move || watched.is_cancelled(sequence, cancels));
    let report = |done, total| *shared.progress.lock() = Some(Progress { done, total });
    let evaluation = contained(recompute, |recompute| {
        recompute.run(&job.document, evaluator, &cancel, &report)
    });
    *shared.progress.lock() = None;

    (shared.latest.load(Ordering::SeqCst) == sequence).then_some(evaluation)
}

fn contained(
    recompute: &mut Recompute,
    run: impl Fn(&mut Recompute) -> Evaluation,
) -> Result<Evaluation, Panicked> {
    let attempt = |recompute: &mut Recompute| {
        panic::catch_unwind(AssertUnwindSafe(|| run(recompute))).map_err(|_| Panicked)
    };
    attempt(recompute).or_else(|Panicked| {
        log::error!("recompute panicked, so it runs again without its cache");
        recompute.clear_cache();
        attempt(recompute).inspect_err(|Panicked| {
            log::error!("recompute panicked again");
            recompute.clear_cache();
        })
    })
}

fn mesh_pending(
    meshes: &mut Vec<PendingMesh>,
    quality: &MeshQuality,
    shared: &Arc<Shared>,
    wake: &dyn Fn(),
) {
    let sequence = shared.latest.load(Ordering::SeqCst);
    let cancels = shared.cancels.load(Ordering::SeqCst);
    let watched = Arc::clone(shared);
    let cancel = CancelToken::new(move || watched.is_cancelled(sequence, cancels));
    while let Some(pending) = meshes.last() {
        if cancel.is_cancelled() {
            return;
        }
        if let Some(solid) = pending.result.solid() {
            interruptible(cancel.interrupt(), || {
                solid.tessellate(&pending.name, quality)
            });
            if !solid.is_meshed() {
                return;
            }
            wake();
        }
        meshes.pop();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize},
    };

    use caditor_kernel::Solid;

    use super::*;
    use crate::{
        document::{Feature, FeatureId},
        recompute::{Failure, FeatureResult, FeatureState, Inputs, ModelEvaluator},
        solid::SolidResult,
        tests::sample,
    };

    fn empty_body(raw: u64) -> Arc<FeatureResult> {
        Arc::new(FeatureResult::Solid(SolidResult::new(
            FeatureId::from_raw(raw),
            Solid::default(),
        )))
    }

    fn is_meshed(result: &Arc<FeatureResult>) -> bool {
        result.solid().is_some_and(SolidResult::is_meshed)
    }

    fn idle_shared() -> Arc<Shared> {
        Arc::new(Shared {
            latest: AtomicU64::new(NO_JOB),
            cancels: AtomicU64::new(0),
            progress: Mutex::new(None),
        })
    }

    struct Blocking {
        started: Arc<AtomicUsize>,
        release: Arc<AtomicBool>,
    }

    impl Evaluator for Blocking {
        fn evaluate(
            &self,
            feature: &Feature,
            inputs: &Inputs<'_>,
            cancel: &CancelToken,
        ) -> Result<FeatureResult, Failure> {
            self.started.fetch_add(1, Ordering::SeqCst);
            while !self.release.load(Ordering::SeqCst) {
                if cancel.is_cancelled() {
                    return Err(Failure::Cancelled);
                }
                thread::yield_now();
            }
            ModelEvaluator.evaluate(feature, inputs, cancel)
        }
    }

    struct Panicking;

    impl Evaluator for Panicking {
        fn evaluate(
            &self,
            feature: &Feature,
            _inputs: &Inputs<'_>,
            _cancel: &CancelToken,
        ) -> Result<FeatureResult, Failure> {
            panic!("kernel bug in {}", feature.name)
        }
    }

    fn blocking() -> (Blocking, Arc<AtomicUsize>, Arc<AtomicBool>) {
        let started = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(AtomicBool::new(false));
        (
            Blocking {
                started: Arc::clone(&started),
                release: Arc::clone(&release),
            },
            started,
            release,
        )
    }

    #[test]
    fn a_recompute_reports_its_revision_and_wakes_the_ui() {
        let woken = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&woken);
        let mut worker = Recomputer::spawn(ModelEvaluator, move || {
            counter.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        let (document, _) = sample();

        worker.submit(document, 7).unwrap();
        let update = worker.wait();

        assert_eq!(update.revision, 7);
        assert_eq!(update.outcome, Outcome::Finished);
        assert_eq!(update.evaluation.failed_count(), 0);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while woken.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(woken.load(Ordering::SeqCst), 1);
        assert_eq!(worker.progress(), None);
    }

    #[test]
    fn cancelling_stops_the_running_recompute_and_reports_what_is_outdated() {
        let (evaluator, started, _release) = blocking();
        let mut worker = Recomputer::spawn(evaluator, || {}).unwrap();
        let (document, ids) = sample();

        worker.submit(document, 1).unwrap();
        while started.load(Ordering::SeqCst) == 0 {
            thread::yield_now();
        }
        assert!(worker.progress().is_some());
        worker.cancel();
        let update = worker.wait();

        assert_eq!(update.outcome, Outcome::Cancelled);
        let status = update.evaluation.feature(ids.base).unwrap();
        assert_eq!(status.state, FeatureState::Outdated);
        assert_eq!(status.result, None);
    }

    #[test]
    fn a_newer_document_supersedes_the_running_one_without_reporting_it() {
        let (evaluator, started, release) = blocking();
        let mut worker = Recomputer::spawn(evaluator, || {}).unwrap();
        let (document, _) = sample();

        worker.submit(document.clone(), 1).unwrap();
        while started.load(Ordering::SeqCst) == 0 {
            thread::yield_now();
        }
        worker.submit(document, 2).unwrap();
        release.store(true, Ordering::SeqCst);

        let update = worker.wait();
        assert_eq!(update.revision, 2);
        assert_eq!(update.outcome, Outcome::Finished);
        assert!(worker.poll().unwrap().is_none());
    }

    #[test]
    fn a_panicking_feature_fails_alone_and_the_worker_keeps_running() {
        let mut worker = Recomputer::spawn(Panicking, || {}).unwrap();
        let (document, ids) = sample();

        worker.submit(document.clone(), 1).unwrap();
        let update = worker.wait();
        let FeatureState::Failed(error) = &update.evaluation.feature(ids.base).unwrap().state
        else {
            panic!("the panicking feature should fail");
        };
        assert!(error.reason.contains("internal error"));

        worker.submit(document, 2).unwrap();
        assert_eq!(worker.wait().revision, 2);
    }

    #[test]
    fn a_recompute_that_panics_runs_again_without_its_cache_and_fails_alone() {
        let (document, _) = sample();
        let runs = AtomicUsize::new(0);

        let mut recompute = Recompute::default();
        let once = contained(&mut recompute, |recompute| {
            if runs.fetch_add(1, Ordering::SeqCst) == 0 {
                panic!("stale cache");
            }
            recompute.run(
                &document,
                &ModelEvaluator,
                &CancelToken::never(),
                &|_, _| {},
            )
        });
        let always = contained(&mut recompute, |_| panic!("bug in parameter evaluation"));
        let after = contained(&mut recompute, |recompute| {
            recompute.run(
                &document,
                &ModelEvaluator,
                &CancelToken::never(),
                &|_, _| {},
            )
        });

        assert!(once.is_ok_and(|evaluation| evaluation.failed_count() == 0));
        assert_eq!(runs.load(Ordering::SeqCst), 2);
        assert!(always.is_err());
        assert!(after.is_ok());
    }

    #[test]
    fn requested_meshes_are_served_newest_first() {
        let (old, new) = (empty_body(1), empty_body(2));
        let mut meshes = vec![
            PendingMesh {
                result: Arc::clone(&old),
                name: "Old".to_owned(),
            },
            PendingMesh {
                result: Arc::clone(&new),
                name: "New".to_owned(),
            },
        ];
        let seen = Mutex::new(Vec::new());
        let wake = || seen.lock().push((is_meshed(&old), is_meshed(&new)));

        mesh_pending(&mut meshes, &MeshQuality::default(), &idle_shared(), &wake);

        assert_eq!(*seen.lock(), [(false, true), (true, true)]);
        assert!(meshes.is_empty());
    }

    #[test]
    fn a_cancel_while_idle_does_not_trip_the_next_job() {
        let mut worker = Recomputer::spawn(ModelEvaluator, || {}).unwrap();
        let (document, _) = sample();

        worker.cancel();
        worker.submit(document.clone(), 1).unwrap();
        let first = worker.wait();
        worker.cancel();
        worker.submit(document, 2).unwrap();
        let second = worker.wait();

        assert_eq!(first.outcome, Outcome::Finished);
        assert_eq!(second.outcome, Outcome::Finished);
    }

    struct Counting(Arc<AtomicUsize>);

    impl Evaluator for Counting {
        fn evaluate(
            &self,
            _feature: &Feature,
            _inputs: &Inputs<'_>,
            _cancel: &CancelToken,
        ) -> Result<FeatureResult, Failure> {
            self.0.fetch_add(1, Ordering::SeqCst);
            panic!("a bug that a second attempt may avoid")
        }
    }

    #[test]
    fn retrying_failures_reruns_a_cached_failure_but_a_plain_run_reuses_it() {
        let runs = Arc::new(AtomicUsize::new(0));
        let mut worker = Recomputer::spawn(Counting(Arc::clone(&runs)), || {}).unwrap();
        let (document, _) = sample();

        worker.submit(document.clone(), 1).unwrap();
        worker.wait();
        let after_first = runs.load(Ordering::SeqCst);
        worker.submit(document.clone(), 2).unwrap();
        worker.wait();
        let after_plain = runs.load(Ordering::SeqCst);
        worker.submit_retrying_failures(document, 3).unwrap();
        worker.wait();
        let after_retry = runs.load(Ordering::SeqCst);

        assert_eq!(after_first, 2);
        assert_eq!(after_plain, after_first);
        assert_eq!(after_retry, after_plain + 2);
    }
}
