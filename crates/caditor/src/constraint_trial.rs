use std::{
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
    time::{Duration, Instant},
};

use caditor_document::{Document, FeatureId, ParameterValues, Transaction};
use caditor_sketch::{ConstraintId, Sketch, SketchError};

use crate::model::Waker;

pub const PATIENCE: Duration = Duration::from_millis(400);
const NAMED_CONFLICTS: usize = 2;

#[derive(Debug, Clone, PartialEq)]
pub struct ConstraintTrial {
    pub feature: FeatureId,
    pub transaction: Transaction,
    pub added: Vec<ConstraintId>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Apply(Transaction),
    Refuse(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    Holds,
    Conflicts(String),
    Unknown,
}

struct Running {
    transaction: Transaction,
    label: String,
    started: Instant,
    cancelled: Arc<AtomicBool>,
    outcome: Receiver<Outcome>,
}

#[derive(Default)]
pub struct Trials {
    running: Option<Running>,
}

impl Trials {
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    pub fn start(
        &mut self,
        trial: ConstraintTrial,
        document: &Document,
        wake: Waker,
    ) -> Option<Verdict> {
        if let Some(running) = &self.running {
            return Some(Verdict::Refuse(format!(
                "Still checking “{}” against the sketch; try again in a moment.",
                running.label
            )));
        }
        let ConstraintTrial {
            feature,
            transaction,
            added,
        } = trial;
        let cancelled = Arc::new(AtomicBool::new(false));
        let (sender, outcome) = mpsc::channel();
        let started = Instant::now();
        let job = Job {
            document: document.clone(),
            transaction: transaction.clone(),
            feature,
            added,
            cancelled: Arc::clone(&cancelled),
            started,
        };
        let spawned = thread::Builder::new()
            .name("constraint trial".to_owned())
            .spawn(move || {
                let outcome =
                    panic::catch_unwind(AssertUnwindSafe(|| job.run())).unwrap_or_else(|_| {
                        log::error!("checking a new constraint panicked");
                        Outcome::Unknown
                    });
                if sender.send(outcome).is_ok() {
                    wake();
                }
            });
        if let Err(error) = spawned {
            log::warn!("could not start checking a new constraint: {error}");
            return Some(Verdict::Apply(transaction));
        }
        self.running = Some(Running {
            label: transaction.label().to_owned(),
            transaction,
            started,
            cancelled,
            outcome,
        });
        None
    }

    pub fn poll(&mut self) -> Option<Verdict> {
        let running = self.running.as_ref()?;
        let outcome = match running.outcome.try_recv() {
            Ok(outcome) => outcome,
            Err(TryRecvError::Disconnected) => Outcome::Unknown,
            Err(TryRecvError::Empty) if running.started.elapsed() >= PATIENCE => Outcome::Unknown,
            Err(TryRecvError::Empty) => return None,
        };
        self.finish(outcome)
    }

    #[cfg(test)]
    pub fn wait(&mut self) -> Option<Verdict> {
        let running = self.running.as_ref()?;
        let outcome = running
            .outcome
            .recv_timeout(PATIENCE)
            .unwrap_or(Outcome::Unknown);
        self.finish(outcome)
    }

    fn finish(&mut self, outcome: Outcome) -> Option<Verdict> {
        let running = self.running.take()?;
        running.cancelled.store(true, Ordering::Relaxed);
        Some(match outcome {
            Outcome::Holds | Outcome::Unknown => Verdict::Apply(running.transaction),
            Outcome::Conflicts(reason) => Verdict::Refuse(reason),
        })
    }

    pub fn cancel(&mut self) {
        if let Some(running) = self.running.take() {
            running.cancelled.store(true, Ordering::Relaxed);
        }
    }
}

struct Job {
    document: Document,
    transaction: Transaction,
    feature: FeatureId,
    added: Vec<ConstraintId>,
    cancelled: Arc<AtomicBool>,
    started: Instant,
}

impl Job {
    fn run(mut self) -> Outcome {
        if self.document.apply(self.transaction).is_err() {
            return Outcome::Unknown;
        }
        let Some(sketch) = self
            .document
            .feature(self.feature)
            .and_then(|feature| feature.kind.sketch())
        else {
            return Outcome::Unknown;
        };
        let values = ParameterValues::evaluate(&self.document);
        let cancelled =
            || self.cancelled.load(Ordering::Relaxed) || self.started.elapsed() >= PATIENCE;
        match sketch.solve(&|id| values.value(id), &cancelled) {
            Ok(_) => Outcome::Holds,
            Err(SketchError::Conflict { constraints }) => {
                Outcome::Conflicts(refusal(sketch, &self.added, &constraints))
            }
            Err(_) => Outcome::Unknown,
        }
    }
}

fn refusal(sketch: &Sketch, added: &[ConstraintId], conflicting: &[ConstraintId]) -> String {
    let subject = match added {
        [only] => sketch.describe_constraint(*only),
        added => format!("The {} new constraints", added.len()),
    };
    let others: Vec<ConstraintId> = conflicting
        .iter()
        .copied()
        .filter(|constraint| !added.contains(constraint))
        .collect();
    let named: Vec<String> = others
        .iter()
        .take(NAMED_CONFLICTS)
        .map(|constraint| sketch.describe_constraint(*constraint))
        .collect();
    let more = others.len().saturating_sub(NAMED_CONFLICTS);
    let (verb, pronoun, conflicts) = match added {
        [_] => ("was", "it", "conflicts"),
        _ => ("were", "they", "conflict"),
    };
    let with = match (named.as_slice(), more) {
        ([], _) => {
            return format!(
                "{subject} {verb} not added: the sketch could not hold {} with its other \
                 constraints.",
                if added.len() == 1 { "it" } else { "them" }
            );
        }
        ([only], _) => only.clone(),
        ([first, second], 0) => format!("{first} and {second}"),
        (named, more) => format!("{} and {more} more", named.join(", ")),
    };
    format!(
        "{subject} {verb} not added: {pronoun} {conflicts} with {with}. Remove or turn one of \
         them off first."
    )
}
