use std::{
    sync::{
        Arc,
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread,
    time::{Duration, Instant},
};

use crate::{
    pool::Meshed,
    recompute::{CancelToken, Evaluation, FeatureResult},
};

pub(crate) const MESHES_REPORTED_EVERY: Duration = Duration::from_millis(750);

pub(crate) struct Glimpse {
    pub(crate) evaluation: Evaluation,
    pub(crate) settled: Vec<SettledBody>,
}

#[derive(Clone)]
pub(crate) struct SettledBody {
    pub(crate) name: String,
    pub(crate) result: Arc<FeatureResult>,
}

enum Shown {
    Glimpse(Box<Glimpse>),
    Meshed,
    Ended,
}

struct Ending(Sender<Shown>);

impl Drop for Ending {
    fn drop(&mut self) {
        let _ = self.0.send(Shown::Ended);
    }
}

pub(crate) struct Presentation<'a> {
    pub(crate) cancel: &'a CancelToken,
    pub(crate) report: &'a (dyn Fn(Evaluation) + Sync),
    pub(crate) mesh: &'a (dyn Fn(SettledBody, Meshed) + Sync),
    pub(crate) from: Instant,
}

impl Presentation<'_> {
    pub(crate) fn during<T>(&self, run: impl FnOnce(&dyn Fn(Glimpse)) -> T) -> T {
        thread::scope(|scope| {
            let (shown, received) = mpsc::channel();
            let meshes = shown.clone();
            let spawned = thread::Builder::new()
                .name("recompute display".to_owned())
                .spawn_scoped(scope, move || self.present(&received, &meshes));
            if let Err(error) = spawned {
                log::warn!(
                    "could not start a thread to show a recompute as it goes, so it shows when \
                     done: {error}"
                );
                return run(&|_| {});
            }
            let ending = Ending(shown);
            run(&|glimpse| {
                let _ = ending.0.send(Shown::Glimpse(Box::new(glimpse)));
            })
        })
    }

    fn present(&self, shown: &Receiver<Shown>, meshes: &Sender<Shown>) {
        let mut latest: Option<Evaluation> = None;
        let mut unreported = false;
        let mut only_meshes = false;
        let mut reported: Option<Instant> = None;
        loop {
            if self.cancel.is_cancelled() {
                return;
            }
            let now = Instant::now();
            let due = match reported {
                Some(at) if only_meshes => self.from.max(at + MESHES_REPORTED_EVERY),
                Some(_) | None => self.from,
            };
            if unreported && now >= due {
                if let Some(evaluation) = &latest {
                    (self.report)(evaluation.clone());
                    reported = Some(now);
                }
                unreported = false;
            }
            let received = if unreported {
                shown.recv_timeout(due.saturating_duration_since(now))
            } else {
                shown
                    .recv()
                    .map_err(|mpsc::RecvError| RecvTimeoutError::Disconnected)
            };
            match received {
                Ok(Shown::Glimpse(glimpse)) => {
                    let Glimpse {
                        evaluation,
                        settled,
                    } = *glimpse;
                    latest = Some(evaluation);
                    only_meshes = false;
                    for body in settled {
                        let meshes = meshes.clone();
                        (self.mesh)(
                            body,
                            Box::new(move |meshed| {
                                if meshed {
                                    let _ = meshes.send(Shown::Meshed);
                                }
                            }),
                        );
                    }
                    unreported = true;
                }
                Ok(Shown::Meshed) => {
                    only_meshes = !unreported || only_meshes;
                    unreported = true;
                }
                Ok(Shown::Ended) | Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }
}
