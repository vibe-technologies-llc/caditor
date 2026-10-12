#[cfg(test)]
use std::time::Duration;
use std::{
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
};

use caditor_document::{
    CancelToken, CarriedParameters, Document, Feature, FeatureId, FeaturePaste, LeftOut,
    ModelEvaluator, PasteError, PasteOrigin, PickSearch, Transaction,
};

use crate::{
    feature_tree::count,
    model::{Notice, Waker},
};

pub const STILL_PASTING: &str = "Still finding the faces and edges the last paste picks; wait for \
                                 it or cancel it first";
pub const NOT_PASTING: &str = "Nothing is being pasted";
pub const STOPPED: &str = "Stopped pasting. Nothing was added.";
const CRASHED: &str = "Nothing was pasted: finding the faces and edges the copies pick failed \
                       unexpectedly. Your model is safe.";
#[cfg(test)]
const PATIENCE: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq)]
pub struct PasteRequest {
    pub features: Vec<Feature>,
    pub parameters: CarriedParameters,
    pub origin: PasteOrigin,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Planned {
    Paste {
        transaction: Transaction,
        pasted: Vec<FeatureId>,
        notice: Option<Notice>,
    },
    Refused(Notice),
}

pub enum PasteFinished {
    Planned(Planned),
    Outdated(Box<PasteRequest>),
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PastedFeatures {
    pub serial: u64,
    pub features: Vec<FeatureId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PasteProgress {
    pub features: usize,
    pub found: usize,
    pub total: usize,
}

enum Outcome {
    Planned(Result<FeaturePaste, PasteError>),
    Crashed,
}

struct Running {
    request: Arc<PasteRequest>,
    revision: u64,
    cancelled: Arc<AtomicBool>,
    found: Arc<AtomicUsize>,
    total: Arc<AtomicUsize>,
    outcome: Receiver<Outcome>,
}

#[derive(Default)]
pub struct Pastes {
    running: Option<Running>,
    #[cfg(test)]
    held: bool,
}

impl Pastes {
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    pub fn progress(&self) -> Option<PasteProgress> {
        let running = self.running.as_ref()?;
        Some(PasteProgress {
            features: running.request.features.len(),
            found: running.found.load(Ordering::Relaxed),
            total: running.total.load(Ordering::Relaxed),
        })
    }

    pub fn start(
        &mut self,
        request: PasteRequest,
        document: &Document,
        revision: u64,
        wake: Waker,
    ) -> Result<(), &'static str> {
        if self.running.is_some() {
            return Err(STILL_PASTING);
        }
        let request = Arc::new(request);
        let cancelled = Arc::new(AtomicBool::new(false));
        let found = Arc::new(AtomicUsize::new(0));
        let total = Arc::new(AtomicUsize::new(0));
        let (sender, outcome) = mpsc::channel();
        let job = Job {
            document: document.clone(),
            request: Arc::clone(&request),
            cancelled: Arc::clone(&cancelled),
            found: Arc::clone(&found),
            total: Arc::clone(&total),
        };
        let spawned = thread::Builder::new()
            .name("paste".to_owned())
            .spawn(move || {
                let outcome = panic::catch_unwind(AssertUnwindSafe(|| job.run())).map_or_else(
                    |_| {
                        log::error!("finding the picks of pasted features panicked");
                        Outcome::Crashed
                    },
                    Outcome::Planned,
                );
                if sender.send(outcome).is_ok() {
                    wake();
                }
            });
        if let Err(error) = spawned {
            log::warn!("could not start pasting: {error}");
            return Err(CRASHED);
        }
        self.running = Some(Running {
            request,
            revision,
            cancelled,
            found,
            total,
            outcome,
        });
        Ok(())
    }

    pub fn poll(&mut self, revision: u64) -> Option<PasteFinished> {
        #[cfg(test)]
        if self.held {
            return None;
        }
        let outcome = match self.running.as_ref()?.outcome.try_recv() {
            Ok(outcome) => outcome,
            Err(TryRecvError::Disconnected) => Outcome::Crashed,
            Err(TryRecvError::Empty) => return None,
        };
        self.finish(outcome, revision)
    }

    #[cfg(test)]
    pub fn hold(&mut self, held: bool) {
        self.held = held;
    }

    #[cfg(test)]
    pub fn wait(&mut self, revision: u64) -> Option<PasteFinished> {
        let outcome = self
            .running
            .as_ref()?
            .outcome
            .recv_timeout(PATIENCE)
            .unwrap_or(Outcome::Crashed);
        self.finish(outcome, revision)
    }

    fn finish(&mut self, outcome: Outcome, revision: u64) -> Option<PasteFinished> {
        let running = self.running.take()?;
        if running.revision != revision {
            let request = Arc::unwrap_or_clone(running.request);
            return Some(PasteFinished::Outdated(Box::new(request)));
        }
        Some(match outcome {
            Outcome::Planned(Err(PasteError::Cancelled)) => PasteFinished::Cancelled,
            Outcome::Planned(planned) => {
                PasteFinished::Planned(self::planned(planned, &running.request.notes))
            }
            Outcome::Crashed => PasteFinished::Planned(Planned::Refused(Notice::warning(CRASHED))),
        })
    }

    pub fn cancel(&mut self) -> bool {
        let Some(running) = self.running.take() else {
            return false;
        };
        running.cancelled.store(true, Ordering::Relaxed);
        true
    }
}

struct Job {
    document: Document,
    request: Arc<PasteRequest>,
    cancelled: Arc<AtomicBool>,
    found: Arc<AtomicUsize>,
    total: Arc<AtomicUsize>,
}

impl Job {
    fn run(self) -> Result<FeaturePaste, PasteError> {
        let cancelled = Arc::clone(&self.cancelled);
        let cancel = CancelToken::new(move || cancelled.load(Ordering::Relaxed));
        let progress = |found: usize, total: usize| {
            self.found.store(found, Ordering::Relaxed);
            self.total.store(total, Ordering::Relaxed);
        };
        self.document.paste_features_finding_picks(
            &self.request.features,
            &self.request.parameters,
            self.request.origin,
            &PickSearch {
                evaluator: &ModelEvaluator,
                cancel: &cancel,
                progress: &progress,
            },
        )
    }
}

pub fn planned(result: Result<FeaturePaste, PasteError>, notes: &[String]) -> Planned {
    match result {
        Ok(paste) => {
            let notice = paste_note(&paste, notes).map(Notice::warning);
            Planned::Paste {
                transaction: paste.transaction,
                pasted: paste.pasted,
                notice,
            }
        }
        Err(PasteError::NothingPasted { left_out }) => {
            let mut parts = vec!["Nothing was pasted.".to_owned()];
            parts.extend(left_out.iter().map(left_out_sentence));
            parts.extend(notes.iter().cloned());
            Planned::Refused(Notice::warning(parts.join(" ")))
        }
        Err(error) => Planned::Refused(Notice::warning(format!("Nothing was pasted: {error}."))),
    }
}

pub fn progress_text(progress: PasteProgress) -> String {
    let pasting = count(progress.features, "feature", "features");
    if progress.total == 0 {
        return format!("Pasting {pasting}…");
    }
    format!(
        "Pasting {pasting}: finding picked faces and edges, {} of {}…",
        (progress.found + 1).min(progress.total),
        progress.total
    )
}

fn left_out_sentence(left_out: &LeftOut) -> String {
    format!(
        "“{}” was left out because {}.",
        left_out.name, left_out.reason
    )
}

fn paste_note(paste: &FeaturePaste, notes: &[String]) -> Option<String> {
    let mut parts: Vec<String> = paste.left_out.iter().map(left_out_sentence).collect();
    if paste.inlined > 0 {
        let one = paste.inlined == 1;
        parts.push(format!(
            "{} written as {}, since this model has no parameter of {} name.",
            count(paste.inlined, "value", "values"),
            if one { "a number" } else { "numbers" },
            if one { "its" } else { "their" },
        ));
    }
    parts.extend(notes.iter().cloned());
    if parts.is_empty() {
        return None;
    }
    let pasted = count(paste.pasted.len(), "feature", "features");
    Some(format!("Pasted {pasted}. {}", parts.join(" ")))
}
