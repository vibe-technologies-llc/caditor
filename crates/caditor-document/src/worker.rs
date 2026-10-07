use std::{
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, TryRecvError},
    },
    thread,
    time::{Duration, Instant},
};

use caditor_kernel::{MeshQuality, interruptible};
use parking_lot::Mutex;

use crate::{
    document::Document,
    recompute::{CancelToken, Evaluation, Evaluator, FeatureResult, Recompute},
};

const NO_JOB: u64 = u64::MAX;
const STOP_GRACE: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
    pub running: Option<String>,
    pub since: Instant,
}

impl Progress {
    pub fn running_for(&self) -> Duration {
        self.since.elapsed()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    FeaturesDone,
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
    draft: bool,
    document: Document,
}

struct Report {
    draft: bool,
    update: Update,
}

enum Message {
    Recompute(Job),
    Mesh {
        result: Arc<FeatureResult>,
        name: String,
    },
    MeshQuality(MeshQuality),
    Forget,
}

struct Shared {
    latest: AtomicU64,
    cancels: AtomicU64,
    progress: Mutex<Option<Progress>>,
    busy: AtomicBool,
    stop_asked: Mutex<Option<Instant>>,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            latest: AtomicU64::new(NO_JOB),
            cancels: AtomicU64::new(0),
            progress: Mutex::new(None),
            busy: AtomicBool::new(false),
            stop_asked: Mutex::new(None),
        }
    }
}

impl Shared {
    fn is_cancelled(&self, sequence: u64, cancels: u64) -> bool {
        self.latest.load(Ordering::SeqCst) != sequence
            || self.cancels.load(Ordering::SeqCst) != cancels
    }

    fn ask_to_stop(&self) {
        if self.busy.load(Ordering::SeqCst) {
            self.stop_asked.lock().get_or_insert_with(Instant::now);
        }
    }

    fn asking_for(&self) -> Option<Duration> {
        if !self.busy.load(Ordering::SeqCst) {
            return None;
        }
        self.stop_asked.lock().map(|since| since.elapsed())
    }
}

type SharedWake = Arc<Mutex<Box<dyn Fn() + Send>>>;

struct Handle {
    jobs: mpsc::Sender<Message>,
    updates: mpsc::Receiver<Report>,
    shared: Arc<Shared>,
}

struct Submitted {
    document: Document,
    revision: u64,
    retry_failures: bool,
    cancelled: bool,
}

pub struct Recomputer {
    evaluator: Arc<dyn Evaluator>,
    wake: SharedWake,
    handle: Handle,
    next_sequence: u64,
    quality: Option<MeshQuality>,
    newest: Option<Submitted>,
    waiting_draft: Option<(Document, u64)>,
    draft: Option<Update>,
    reported: Evaluation,
    grace: Duration,
}

impl Recomputer {
    pub fn spawn(
        evaluator: impl Evaluator,
        wake: impl Fn() + Send + 'static,
    ) -> std::io::Result<Self> {
        let evaluator: Arc<dyn Evaluator> = Arc::new(evaluator);
        let wake: SharedWake = Arc::new(Mutex::new(Box::new(wake)));
        let handle = start_worker(&evaluator, &wake)?;
        Ok(Self {
            evaluator,
            wake,
            handle,
            next_sequence: 0,
            quality: None,
            newest: None,
            waiting_draft: None,
            draft: None,
            reported: Evaluation::default(),
            grace: STOP_GRACE,
        })
    }

    #[cfg(test)]
    pub(crate) fn with_stop_grace(mut self, grace: Duration) -> Self {
        self.grace = grace;
        self
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
        if self.is_wedged() {
            self.replace_worker()?;
        }
        self.handle.shared.ask_to_stop();
        self.waiting_draft = None;
        self.newest = Some(Submitted {
            document: document.clone(),
            revision,
            retry_failures,
            cancelled: false,
        });
        self.send_job(document, revision, retry_failures, false)
    }

    pub fn submit_draft(&mut self, document: Document, serial: u64) -> Result<(), WorkerStopped> {
        if self.newest.is_some() {
            self.waiting_draft = Some((document, serial));
            return Ok(());
        }
        self.handle.shared.ask_to_stop();
        self.send_job(document, serial, false, true)
    }

    pub fn take_draft(&mut self) -> Option<Update> {
        self.draft.take()
    }

    fn send_job(
        &mut self,
        document: Document,
        revision: u64,
        retry_failures: bool,
        draft: bool,
    ) -> Result<(), WorkerStopped> {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        self.handle.shared.latest.store(sequence, Ordering::SeqCst);
        self.handle
            .jobs
            .send(Message::Recompute(Job {
                sequence,
                cancels: self.handle.shared.cancels.load(Ordering::SeqCst),
                revision,
                retry_failures,
                draft,
                document,
            }))
            .map_err(|_| WorkerStopped)
    }

    pub fn forget(&mut self) -> Result<(), WorkerStopped> {
        self.reported = Evaluation::default();
        self.newest = None;
        self.handle
            .jobs
            .send(Message::Forget)
            .map_err(|_| WorkerStopped)
    }

    pub fn mesh(&self, result: Arc<FeatureResult>, name: String) -> Result<(), WorkerStopped> {
        self.handle
            .jobs
            .send(Message::Mesh { result, name })
            .map_err(|_| WorkerStopped)
    }

    pub fn set_mesh_quality(&mut self, quality: MeshQuality) -> Result<(), WorkerStopped> {
        self.quality = Some(quality);
        self.handle
            .jobs
            .send(Message::MeshQuality(quality))
            .map_err(|_| WorkerStopped)
    }

    pub fn cancel(&mut self) {
        self.handle.shared.cancels.fetch_add(1, Ordering::SeqCst);
        self.handle.shared.ask_to_stop();
        if let Some(newest) = &mut self.newest {
            newest.cancelled = true;
        }
    }

    pub fn progress(&self) -> Option<Progress> {
        self.handle.shared.progress.lock().clone()
    }

    fn is_wedged(&self) -> bool {
        self.handle
            .shared
            .asking_for()
            .is_some_and(|waited| waited > self.grace)
    }

    fn replace_worker(&mut self) -> Result<Option<Update>, WorkerStopped> {
        let running = self.progress().and_then(|progress| {
            let name = progress.running?;
            Some(format!(
                " (running “{name}” for {:.1?})",
                progress.since.elapsed()
            ))
        });
        log::error!(
            "the recompute worker did not stop within {:?}{}, so it is left behind and a new one \
             starts",
            self.grace,
            running.unwrap_or_default()
        );
        let fresh = start_worker(&self.evaluator, &self.wake).map_err(|error| {
            log::error!("could not start a new recompute worker: {error}");
            WorkerStopped
        })?;
        let stuck = std::mem::replace(&mut self.handle, fresh);
        stuck.shared.cancels.fetch_add(1, Ordering::SeqCst);
        stuck.shared.latest.store(NO_JOB, Ordering::SeqCst);
        if let Some(quality) = self.quality {
            self.handle
                .jobs
                .send(Message::MeshQuality(quality))
                .map_err(|_| WorkerStopped)?;
        }
        let Some(newest) = self.newest.take() else {
            return Ok(None);
        };
        if newest.cancelled {
            return Ok(Some(Update {
                revision: newest.revision,
                outcome: Outcome::Cancelled,
                evaluation: self.reported.clone().outdating_pending(),
            }));
        }
        self.send_job(
            newest.document,
            newest.revision,
            newest.retry_failures,
            false,
        )?;
        Ok(None)
    }

    pub fn poll(&mut self) -> Result<Option<Update>, WorkerStopped> {
        if self.is_wedged()
            && let Some(update) = self.replace_worker()?
        {
            return Ok(Some(update));
        }
        let mut latest = None;
        loop {
            match self.handle.updates.try_recv() {
                Ok(Report {
                    draft: true,
                    update,
                }) => self.draft = Some(update),
                Ok(Report {
                    draft: false,
                    update,
                }) => latest = Some(update),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    return latest.map(Some).ok_or(WorkerStopped);
                }
            }
        }
        if let Some(update) = &latest {
            self.reported = update.evaluation.clone();
            if update.outcome != Outcome::FeaturesDone
                && self
                    .newest
                    .as_ref()
                    .is_some_and(|newest| newest.revision == update.revision)
            {
                self.newest = None;
            }
        }
        if self.newest.is_none()
            && let Some((document, serial)) = self.waiting_draft.take()
        {
            self.handle.shared.ask_to_stop();
            self.send_job(document, serial, false, true)?;
        }
        Ok(latest)
    }

    #[cfg(test)]
    pub(crate) fn wait(&self) -> Update {
        loop {
            let Report { draft, update } = self
                .handle
                .updates
                .recv()
                .expect("the worker should report before it stops");
            if !draft && update.outcome != Outcome::FeaturesDone {
                return update;
            }
        }
    }
}

fn start_worker(evaluator: &Arc<dyn Evaluator>, wake: &SharedWake) -> std::io::Result<Handle> {
    let (jobs, queue) = mpsc::channel();
    let (sender, updates) = mpsc::channel();
    let shared = Arc::new(Shared::default());
    let worker = Arc::clone(&shared);
    let evaluator = Arc::clone(evaluator);
    let wake = Arc::clone(wake);
    thread::Builder::new()
        .name("recompute".to_owned())
        .spawn(move || {
            let wake = move || (wake.lock())();
            work(evaluator.as_ref(), &queue, &sender, &worker, &wake);
        })?;
    Ok(Handle {
        jobs,
        updates,
        shared,
    })
}

struct PendingMesh {
    result: Arc<FeatureResult>,
    name: String,
}

fn work(
    evaluator: &dyn Evaluator,
    queue: &mpsc::Receiver<Message>,
    updates: &mpsc::Sender<Report>,
    shared: &Arc<Shared>,
    wake: &(dyn Fn() + Sync),
) {
    let mut recompute = Recompute::default();
    let mut reported = Evaluation::default();
    let mut meshes: Vec<PendingMesh> = Vec::new();
    loop {
        shared.busy.store(false, Ordering::SeqCst);
        let Ok(first) = queue.recv() else {
            break;
        };
        shared.busy.store(true, Ordering::SeqCst);
        *shared.stop_asked.lock() = None;
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
                Message::Forget => {
                    recompute.clear_cache();
                    reported = Evaluation::default();
                    meshes.clear();
                }
            }
        }
        if let Some(job) = latest.take_if(|job| job.draft) {
            let mut draft = recompute.draft_copy();
            let ran = run_contained(&mut draft, &job, evaluator, shared, &|_| {});
            if let Some(Ok(evaluation)) =
                ran.filter(|_| !shared.is_cancelled(job.sequence, job.cancels))
                && evaluation.is_complete()
            {
                let report = Report {
                    draft: true,
                    update: Update {
                        revision: job.revision,
                        outcome: Outcome::Finished,
                        evaluation,
                    },
                };
                if updates.send(report).is_err() {
                    break;
                }
                wake();
            }
        }
        if let Some(job) = latest {
            if job.retry_failures {
                recompute.retry_failures();
            }
            let report_features = |evaluation| {
                let update = Update {
                    revision: job.revision,
                    outcome: Outcome::FeaturesDone,
                    evaluation,
                };
                if updates
                    .send(Report {
                        draft: false,
                        update,
                    })
                    .is_ok()
                {
                    wake();
                }
            };
            let Some(ran) =
                run_contained(&mut recompute, &job, evaluator, shared, &report_features)
            else {
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
            if updates
                .send(Report {
                    draft: false,
                    update,
                })
                .is_err()
            {
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
    features_done: &(dyn Fn(Evaluation) + Sync),
) -> Option<Result<Evaluation, Panicked>> {
    let (sequence, cancels) = (job.sequence, job.cancels);
    let watched = Arc::clone(shared);
    let cancel = CancelToken::new(move || watched.is_cancelled(sequence, cancels));
    let report = |done: usize, total: usize| {
        let running = job
            .document
            .feature_handles()
            .get(done)
            .map(|feature| feature.name.clone());
        *shared.progress.lock() = Some(Progress {
            done,
            total,
            running,
            since: Instant::now(),
        });
    };
    let evaluation = contained(recompute, |recompute| {
        recompute.run_reporting(&job.document, evaluator, &cancel, &report, features_done)
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
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize},
        },
        time::Duration,
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
        Arc::new(Shared::default())
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

    struct Stuck {
        started: Arc<AtomicUsize>,
        release: Arc<AtomicBool>,
    }

    impl Evaluator for Stuck {
        fn evaluate(
            &self,
            feature: &Feature,
            inputs: &Inputs<'_>,
            cancel: &CancelToken,
        ) -> Result<FeatureResult, Failure> {
            self.started.fetch_add(1, Ordering::SeqCst);
            while !self.release.load(Ordering::SeqCst) {
                thread::yield_now();
            }
            ModelEvaluator.evaluate(feature, inputs, cancel)
        }
    }

    fn stuck() -> (Stuck, Arc<AtomicUsize>, Arc<AtomicBool>) {
        let started = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(AtomicBool::new(false));
        let evaluator = Stuck {
            started: Arc::clone(&started),
            release: Arc::clone(&release),
        };
        (evaluator, started, release)
    }

    fn poll_until_update(worker: &mut Recomputer) -> Update {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(update) = worker.poll().unwrap() {
                return update;
            }
            assert!(Instant::now() < deadline, "no update arrived");
            thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn a_worker_that_ignores_a_cancel_is_left_behind_and_the_cancel_is_reported() {
        let (evaluator, started, release) = stuck();
        let mut worker = Recomputer::spawn(evaluator, || {})
            .unwrap()
            .with_stop_grace(Duration::from_millis(50));
        let (document, _) = sample();

        worker.submit(document.clone(), 3).unwrap();
        while started.load(Ordering::SeqCst) == 0 {
            thread::yield_now();
        }
        worker.cancel();
        let update = poll_until_update(&mut worker);

        assert_eq!((update.revision, update.outcome), (3, Outcome::Cancelled));
        release.store(true, Ordering::SeqCst);
        worker.submit(document, 4).unwrap();
        let next = poll_until_update(&mut worker);
        assert_eq!((next.revision, next.outcome), (4, Outcome::Finished));
    }

    #[test]
    fn a_worker_that_ignores_a_newer_submission_is_replaced_and_the_newer_one_runs() {
        let (evaluator, started, release) = stuck();
        let mut worker = Recomputer::spawn(evaluator, || {})
            .unwrap()
            .with_stop_grace(Duration::from_millis(50));
        let (document, _) = sample();

        worker.submit(document.clone(), 1).unwrap();
        while started.load(Ordering::SeqCst) == 0 {
            thread::yield_now();
        }
        worker.submit(document, 2).unwrap();
        thread::sleep(Duration::from_millis(120));
        assert_eq!(worker.poll().unwrap().map(|update| update.revision), None);
        release.store(true, Ordering::SeqCst);

        let update = poll_until_update(&mut worker);

        assert_eq!((update.revision, update.outcome), (2, Outcome::Finished));
    }

    struct StuckOn {
        name: &'static str,
        release: Arc<AtomicBool>,
    }

    impl Evaluator for StuckOn {
        fn evaluate(
            &self,
            feature: &Feature,
            inputs: &Inputs<'_>,
            cancel: &CancelToken,
        ) -> Result<FeatureResult, Failure> {
            while feature.name == self.name && !self.release.load(Ordering::SeqCst) {
                thread::yield_now();
            }
            ModelEvaluator.evaluate(feature, inputs, cancel)
        }
    }

    #[test]
    fn a_forgotten_evaluation_is_never_reported_for_the_next_document() {
        let (evaluator, started, release) = stuck();
        release.store(true, Ordering::SeqCst);
        let mut worker = Recomputer::spawn(evaluator, || {})
            .unwrap()
            .with_stop_grace(Duration::from_millis(50));
        let (document, ids) = sample();
        worker.submit(document.clone(), 1).unwrap();
        let first = poll_until_finished(&mut worker);
        release.store(false, Ordering::SeqCst);
        let evaluated = started.load(Ordering::SeqCst);

        worker.forget().unwrap();
        worker.submit(document, 2).unwrap();
        while started.load(Ordering::SeqCst) == evaluated {
            thread::yield_now();
        }
        worker.cancel();
        let cancelled = poll_until_update(&mut worker);
        release.store(true, Ordering::SeqCst);

        assert!(first.evaluation.feature(ids.base).is_some());
        assert_eq!(
            (cancelled.revision, cancelled.outcome),
            (2, Outcome::Cancelled)
        );
        assert!(
            cancelled
                .evaluation
                .feature(ids.base)
                .is_none_or(|status| status.state != FeatureState::UpToDate)
        );
    }

    fn poll_until_finished(worker: &mut Recomputer) -> Update {
        loop {
            let update = poll_until_update(worker);
            if update.outcome != Outcome::FeaturesDone {
                return update;
            }
        }
    }

    #[test]
    fn a_worker_stuck_after_showing_features_reports_the_cancel_with_the_rest_outdated() {
        let release = Arc::new(AtomicBool::new(false));
        let evaluator = StuckOn {
            name: "Side sketch",
            release: Arc::clone(&release),
        };
        let mut worker = Recomputer::spawn(evaluator, || {})
            .unwrap()
            .with_stop_grace(Duration::from_millis(50));
        let (document, ids) = sample();

        worker.submit(document, 5).unwrap();
        let early = poll_until_update(&mut worker);
        worker.cancel();
        let cancelled = poll_until_update(&mut worker);
        release.store(true, Ordering::SeqCst);

        assert_eq!(early.outcome, Outcome::FeaturesDone);
        assert!(early.evaluation.is_pending(ids.side));
        assert_eq!(
            (cancelled.revision, cancelled.outcome),
            (5, Outcome::Cancelled)
        );
        let state = |id| cancelled.evaluation.feature(id).unwrap().state.clone();
        assert_eq!(state(ids.base), FeatureState::UpToDate);
        assert_eq!(state(ids.side), FeatureState::Outdated);
        assert!(!cancelled.evaluation.is_pending(ids.side));
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
        let progress = worker.progress().unwrap();
        assert_eq!(progress.running.as_deref(), Some("Base sketch"));
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
    fn a_slow_recompute_reports_each_feature_as_it_goes_and_all_of_them_before_meshing() {
        let (document, ids) = sample();
        let reported = Mutex::new(Vec::new());
        let mut recompute = Recompute::default();
        recompute.report_features_done_after(Duration::ZERO);

        let evaluation = recompute.run_reporting(
            &document,
            &ModelEvaluator,
            &CancelToken::never(),
            &|_, _| {},
            &|early| reported.lock().push(early),
        );

        let reported = reported.into_inner();
        let [first, last] = reported.as_slice() else {
            panic!("the features should be reported as they go and once all are done");
        };
        assert_eq!(first.feature(ids.base), evaluation.feature(ids.base));
        assert!(first.is_pending(ids.side));
        assert_eq!(first.feature(ids.side), None);
        assert!(!last.is_pending(ids.side));
        assert!(!last.is_complete());
        assert!(evaluation.is_complete());
        assert_eq!(last.feature(ids.side), evaluation.feature(ids.side));
    }

    #[test]
    fn a_quick_recompute_reports_only_once() {
        let (document, _) = sample();
        let reported = Mutex::new(0);

        Recompute::default().run_reporting(
            &document,
            &ModelEvaluator,
            &CancelToken::never(),
            &|_, _| {},
            &|_| *reported.lock() += 1,
        );

        assert_eq!(reported.into_inner(), 0);
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

    fn poll_until_draft(worker: &mut Recomputer) -> Update {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            worker.poll().unwrap();
            if let Some(update) = worker.take_draft() {
                return update;
            }
            assert!(Instant::now() < deadline, "no draft arrived");
            thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn a_draft_waits_for_the_model_and_reports_apart_from_it() {
        let mut worker = Recomputer::spawn(ModelEvaluator, || {}).unwrap();
        let (document, _) = sample();

        worker.submit(document.clone(), 1).unwrap();
        worker.submit_draft(document.clone(), 40).unwrap();
        let model = poll_until_update(&mut worker);
        assert_eq!((model.revision, model.outcome), (1, Outcome::Finished));

        let draft = poll_until_draft(&mut worker);
        assert_eq!((draft.revision, draft.outcome), (40, Outcome::Finished));
        assert!(draft.evaluation.is_complete());
        thread::sleep(Duration::from_millis(20));
        assert_eq!(worker.poll().unwrap().map(|update| update.revision), None);
    }

    #[test]
    fn a_newer_model_submission_drops_a_waiting_draft() {
        let mut worker = Recomputer::spawn(ModelEvaluator, || {}).unwrap();
        let (document, _) = sample();

        worker.submit(document.clone(), 1).unwrap();
        worker.submit_draft(document.clone(), 40).unwrap();
        worker.submit(document, 2).unwrap();
        let mut finished = poll_until_update(&mut worker);
        while finished.revision != 2 || finished.outcome != Outcome::Finished {
            finished = poll_until_update(&mut worker);
        }
        thread::sleep(Duration::from_millis(50));
        worker.poll().unwrap();

        assert_eq!(worker.take_draft().map(|update| update.revision), None);
    }
}
