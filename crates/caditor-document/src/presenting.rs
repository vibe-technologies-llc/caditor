use std::{
    collections::VecDeque,
    sync::{
        Arc,
        mpsc::{self, Receiver, RecvTimeoutError, TryRecvError},
    },
    thread,
    time::Instant,
};

use caditor_kernel::{MeshQuality, interruptible};

use crate::recompute::{CancelToken, Evaluation, FeatureResult};

pub(crate) struct Glimpse {
    pub(crate) evaluation: Evaluation,
    pub(crate) settled: Vec<SettledBody>,
}

pub(crate) struct SettledBody {
    pub(crate) name: String,
    pub(crate) result: Arc<FeatureResult>,
}

pub(crate) struct Presentation<'a> {
    pub(crate) quality: MeshQuality,
    pub(crate) cancel: &'a CancelToken,
    pub(crate) report: &'a (dyn Fn(Evaluation) + Sync),
    pub(crate) from: Instant,
}

impl Presentation<'_> {
    pub(crate) fn during<T>(&self, run: impl FnOnce(&dyn Fn(Glimpse)) -> T) -> T {
        thread::scope(|scope| {
            let (glimpses, received) = mpsc::channel();
            let spawned = thread::Builder::new()
                .name("recompute display".to_owned())
                .spawn_scoped(scope, move || self.present(&received));
            if let Err(error) = spawned {
                log::warn!(
                    "could not start a thread to show a recompute as it goes, so it shows when \
                     done: {error}"
                );
                return run(&|_| {});
            }
            let ran = run(&|glimpse| {
                let _ = glimpses.send(glimpse);
            });
            drop(glimpses);
            ran
        })
    }

    fn present(&self, glimpses: &Receiver<Glimpse>) {
        let mut latest: Option<Evaluation> = None;
        let mut unreported = false;
        let mut settled: VecDeque<SettledBody> = VecDeque::new();
        loop {
            if self.cancel.is_cancelled() {
                return;
            }
            let now = Instant::now();
            if unreported && now >= self.from {
                if let Some(evaluation) = &latest {
                    (self.report)(evaluation.clone());
                }
                unreported = false;
            }
            let received = if !settled.is_empty() {
                glimpses.try_recv().map_err(|error| match error {
                    TryRecvError::Empty => RecvTimeoutError::Timeout,
                    TryRecvError::Disconnected => RecvTimeoutError::Disconnected,
                })
            } else if unreported {
                glimpses.recv_timeout(self.from.saturating_duration_since(now))
            } else {
                glimpses
                    .recv()
                    .map_err(|mpsc::RecvError| RecvTimeoutError::Disconnected)
            };
            match received {
                Ok(glimpse) => {
                    latest = Some(glimpse.evaluation);
                    settled.extend(glimpse.settled);
                    unreported = true;
                }
                Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => {
                    if let Some(body) = settled.pop_front() {
                        unreported |= self.mesh(&body);
                    }
                }
            }
        }
    }

    fn mesh(&self, body: &SettledBody) -> bool {
        let Some(solid) = body.result.solid() else {
            return false;
        };
        if solid.is_meshed() {
            return false;
        }
        interruptible(self.cancel.interrupt(), || {
            solid.tessellate(&body.name, &self.quality);
        });
        solid.is_meshed()
    }
}
