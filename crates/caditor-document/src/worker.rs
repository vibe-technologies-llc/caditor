use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, TryRecvError},
    },
    thread,
};

use parking_lot::Mutex;

use crate::{
    document::Document,
    recompute::{CancelToken, Evaluation, Evaluator, Recompute},
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
    revision: u64,
    document: Document,
}

struct Shared {
    latest: AtomicU64,
    cancelled: AtomicU64,
    progress: Mutex<Option<Progress>>,
}

impl Shared {
    fn is_cancelled(&self, sequence: u64) -> bool {
        self.latest.load(Ordering::SeqCst) != sequence
            || self.cancelled.load(Ordering::SeqCst) == sequence
    }
}

pub struct Recomputer {
    jobs: mpsc::Sender<Job>,
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
            cancelled: AtomicU64::new(NO_JOB),
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
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        self.shared.latest.store(sequence, Ordering::SeqCst);
        self.jobs
            .send(Job {
                sequence,
                revision,
                document,
            })
            .map_err(|_| WorkerStopped)
    }

    pub fn cancel(&self) {
        self.shared
            .cancelled
            .store(self.shared.latest.load(Ordering::SeqCst), Ordering::SeqCst);
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

fn work(
    evaluator: &dyn Evaluator,
    queue: &mpsc::Receiver<Job>,
    updates: &mpsc::Sender<Update>,
    shared: &Arc<Shared>,
    wake: &dyn Fn(),
) {
    let mut recompute = Recompute::default();
    while let Ok(mut job) = queue.recv() {
        while let Ok(newer) = queue.try_recv() {
            job = newer;
        }
        let sequence = job.sequence;
        let watched = Arc::clone(shared);
        let cancel = CancelToken::new(move || watched.is_cancelled(sequence));
        let evaluation = recompute.run(&job.document, evaluator, &cancel, &|done, total| {
            *shared.progress.lock() = Some(Progress { done, total });
        });
        *shared.progress.lock() = None;

        if shared.latest.load(Ordering::SeqCst) != sequence {
            continue;
        }
        let outcome = if evaluation.is_complete() {
            Outcome::Finished
        } else {
            Outcome::Cancelled
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
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize},
    };

    use super::*;
    use crate::{
        document::Feature,
        recompute::{Failure, FeatureResult, FeatureState, Inputs, ModelEvaluator},
        tests::sample,
    };

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
}
