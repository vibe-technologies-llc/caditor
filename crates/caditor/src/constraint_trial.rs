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
    pub touched: Vec<ConstraintId>,
    pub intent: Intent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Added,
    Enabled,
    Edited,
}

impl Intent {
    fn past(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Enabled => "turned on",
            Self::Edited => "changed",
        }
    }
}

pub fn editing_dimension(
    feature: FeatureId,
    constraint: ConstraintId,
    transaction: Transaction,
) -> ConstraintTrial {
    ConstraintTrial {
        feature,
        transaction,
        touched: vec![constraint],
        intent: Intent::Edited,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    #[error(
        "“{label}” was not applied, because “{running}” was still being checked against the sketch."
    )]
    Busy { label: String, running: String },
    #[error("{}", conflict_sentence(.subject, (*.intent, *.touched), .named, *.more))]
    Conflict {
        subject: String,
        intent: Intent,
        touched: usize,
        named: Vec<String>,
        more: usize,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Apply(Transaction),
    Refuse(Refusal),
    Outdated { label: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    Holds,
    Conflicts(Refusal),
    Unknown,
}

struct Running {
    transaction: Transaction,
    label: String,
    revision: u64,
    started: Instant,
    cancelled: Arc<AtomicBool>,
    outcome: Receiver<Outcome>,
}

#[derive(Default)]
pub struct Trials {
    running: Option<Running>,
    #[cfg(test)]
    held: bool,
}

impl Trials {
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    pub fn start(
        &mut self,
        trial: ConstraintTrial,
        document: &Document,
        revision: u64,
        wake: Waker,
    ) -> Option<Verdict> {
        if let Some(running) = &self.running {
            return Some(Verdict::Refuse(Refusal::Busy {
                label: trial.transaction.label().to_owned(),
                running: running.label.clone(),
            }));
        }
        let ConstraintTrial {
            feature,
            transaction,
            touched,
            intent,
        } = trial;
        let cancelled = Arc::new(AtomicBool::new(false));
        let (sender, outcome) = mpsc::channel();
        let started = Instant::now();
        let job = Job {
            document: document.clone(),
            transaction: transaction.clone(),
            feature,
            touched,
            intent,
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
            revision,
            transaction,
            started,
            cancelled,
            outcome,
        });
        None
    }

    pub fn poll(&mut self, revision: u64) -> Option<Verdict> {
        #[cfg(test)]
        if self.held {
            return None;
        }
        let running = self.running.as_ref()?;
        let outcome = match running.outcome.try_recv() {
            Ok(outcome) => outcome,
            Err(TryRecvError::Disconnected) => Outcome::Unknown,
            Err(TryRecvError::Empty) if running.started.elapsed() >= PATIENCE => Outcome::Unknown,
            Err(TryRecvError::Empty) => return None,
        };
        self.finish(outcome, revision)
    }

    #[cfg(test)]
    pub fn hold(&mut self, held: bool) {
        self.held = held;
    }

    #[cfg(test)]
    pub fn wait(&mut self, revision: u64) -> Option<Verdict> {
        if self.held {
            return None;
        }
        let running = self.running.as_ref()?;
        let outcome = running
            .outcome
            .recv_timeout(PATIENCE)
            .unwrap_or(Outcome::Unknown);
        self.finish(outcome, revision)
    }

    fn finish(&mut self, outcome: Outcome, revision: u64) -> Option<Verdict> {
        let running = self.running.take()?;
        running.cancelled.store(true, Ordering::Relaxed);
        if running.revision != revision {
            return Some(Verdict::Outdated {
                label: running.label,
            });
        }
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
    touched: Vec<ConstraintId>,
    intent: Intent,
    cancelled: Arc<AtomicBool>,
    started: Instant,
}

impl Job {
    fn run(self) -> Outcome {
        let cancelled =
            || self.cancelled.load(Ordering::Relaxed) || self.started.elapsed() >= PATIENCE;
        let trial = ConstraintTrial {
            feature: self.feature,
            transaction: self.transaction,
            touched: self.touched,
            intent: self.intent,
        };
        outcome(self.document, &trial, &cancelled)
    }
}

pub fn conflict(
    document: &Document,
    trial: &ConstraintTrial,
    cancelled: &dyn Fn() -> bool,
) -> Option<Refusal> {
    match outcome(document.clone(), trial, cancelled) {
        Outcome::Conflicts(reason) => Some(reason),
        Outcome::Holds | Outcome::Unknown => None,
    }
}

fn outcome(
    mut document: Document,
    trial: &ConstraintTrial,
    cancelled: &dyn Fn() -> bool,
) -> Outcome {
    if document.apply(trial.transaction.clone()).is_err() {
        return Outcome::Unknown;
    }
    let Some(sketch) = document
        .feature(trial.feature)
        .and_then(|feature| feature.kind.sketch())
    else {
        return Outcome::Unknown;
    };
    let values = ParameterValues::evaluate(&document);
    match sketch.solve(&|id| values.value(id), cancelled) {
        Ok(_) => Outcome::Holds,
        Err(SketchError::Conflict { constraints }) => Outcome::Conflicts(refusal(
            sketch,
            (trial.intent, &trial.touched),
            &constraints,
        )),
        Err(_) => Outcome::Unknown,
    }
}

fn refusal(
    sketch: &Sketch,
    (intent, touched): (Intent, &[ConstraintId]),
    conflicting: &[ConstraintId],
) -> Refusal {
    let subject = match touched {
        [only] => sketch.describe_constraint(*only),
        touched => format!("The {} constraints", touched.len()),
    };
    let others: Vec<ConstraintId> = conflicting
        .iter()
        .copied()
        .filter(|constraint| !touched.contains(constraint))
        .collect();
    let named = others
        .iter()
        .take(NAMED_CONFLICTS)
        .map(|constraint| sketch.describe_constraint(*constraint))
        .collect();
    let more = others.len().saturating_sub(NAMED_CONFLICTS);
    Refusal::Conflict {
        subject,
        intent,
        touched: touched.len(),
        named,
        more,
    }
}

fn conflict_sentence(
    subject: &str,
    (intent, touched): (Intent, usize),
    named: &[String],
    more: usize,
) -> String {
    let (verb, pronoun, conflicts) = match touched {
        1 => ("was", "it", "conflicts"),
        _ => ("were", "they", "conflict"),
    };
    let past = intent.past();
    let with = match (named, more) {
        ([], _) => {
            return format!(
                "{subject} {verb} not {past}: the sketch could not hold {} with its other \
                 constraints.",
                match (intent, touched) {
                    (Intent::Edited, _) => "the new value",
                    (_, 1) => "it",
                    (_, _) => "them",
                }
            );
        }
        ([only], _) => only.clone(),
        ([first, second], 0) => format!("{first} and {second}"),
        (named, more) => format!("{} and {more} more", named.join(", ")),
    };
    format!(
        "{subject} {verb} not {past}: {pronoun} {conflicts} with {with}. Remove or turn one of \
         them off first."
    )
}
