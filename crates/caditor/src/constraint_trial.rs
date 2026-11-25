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

use crate::{constraint_remedies::ConstraintRemedies, model::Waker, sketch_tools::Measuring};

pub const PATIENCE: Duration = Duration::from_millis(400);
const NAMED_CONFLICTS: usize = 2;

#[derive(Clone, Debug, PartialEq)]
pub struct ConstraintTrial {
    pub feature: FeatureId,
    pub transaction: Transaction,
    pub touched: Vec<ConstraintId>,
    pub intent: Intent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    #[error(
        "“{label}” was not applied, because “{running}” was still being checked against the sketch."
    )]
    Busy { label: String, running: String },
    #[error("{}", conflict_sentence(.subject, (*.intent, *.touched), .named, .conflicting))]
    Conflict {
        subject: String,
        intent: Intent,
        touched: usize,
        named: Vec<String>,
        feature: FeatureId,
        conflicting: Vec<ConstraintId>,
    },
}

impl Refusal {
    pub fn remedies(&self) -> Option<ConstraintRemedies> {
        match self {
            Self::Busy { .. } => None,
            Self::Conflict {
                feature,
                conflicting,
                ..
            } => ConstraintRemedies::of(*feature, conflicting.clone()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    Apply(Transaction),
    Measured {
        measuring: Box<Measuring>,
        determined: Vec<bool>,
    },
    Refuse(Refusal),
    Outdated {
        label: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Outcome {
    Holds,
    Conflicts(Refusal),
    Measured(Vec<bool>),
    Unknown,
}

enum Checked {
    Trial(Transaction),
    Dimension(Box<Measuring>),
}

impl Checked {
    fn label(&self) -> String {
        match self {
            Self::Trial(transaction) => transaction.label().to_owned(),
            Self::Dimension(measuring) => measuring.label(),
        }
    }

    fn verdict(self, outcome: Outcome) -> Verdict {
        match (self, outcome) {
            (Self::Trial(_), Outcome::Conflicts(reason)) => Verdict::Refuse(reason),
            (
                Self::Trial(transaction),
                Outcome::Holds | Outcome::Measured(_) | Outcome::Unknown,
            ) => Verdict::Apply(transaction),
            (Self::Dimension(measuring), Outcome::Measured(determined)) => Verdict::Measured {
                measuring,
                determined,
            },
            (
                Self::Dimension(measuring),
                Outcome::Holds | Outcome::Conflicts(_) | Outcome::Unknown,
            ) => Verdict::Measured {
                determined: measuring.unmeasured(),
                measuring,
            },
        }
    }

    fn unchecked(self) -> Verdict {
        match self {
            Self::Trial(transaction) => Verdict::Apply(transaction),
            Self::Dimension(measuring) => Verdict::Measured {
                determined: measuring.determined(&|| false),
                measuring,
            },
        }
    }
}

struct Running {
    checked: Checked,
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
        let job = Job {
            document: document.clone(),
            transaction: trial.transaction.clone(),
            feature: trial.feature,
            touched: trial.touched,
            intent: trial.intent,
        };

        self.check(
            Checked::Trial(trial.transaction),
            revision,
            wake,
            move |cancelled| job.run(cancelled),
        )
    }

    pub fn measure(
        &mut self,
        measuring: Box<Measuring>,
        revision: u64,
        wake: Waker,
    ) -> Option<Verdict> {
        let probe = measuring.clone();

        self.check(
            Checked::Dimension(measuring),
            revision,
            wake,
            move |cancelled| Outcome::Measured(probe.determined(cancelled)),
        )
    }

    fn check(
        &mut self,
        checked: Checked,
        revision: u64,
        wake: Waker,
        work: impl FnOnce(&dyn Fn() -> bool) -> Outcome + Send + 'static,
    ) -> Option<Verdict> {
        let label = checked.label();

        if let Some(running) = &self.running {
            return Some(Verdict::Refuse(Refusal::Busy {
                label,
                running: running.label.clone(),
            }));
        }

        let cancelled = Arc::new(AtomicBool::new(false));
        let (sender, outcome) = mpsc::channel();
        let started = Instant::now();

        let stopped = Arc::clone(&cancelled);
        let spawned = thread::Builder::new()
            .name("constraint trial".to_owned())
            .spawn(move || {
                let cancelled = || stopped.load(Ordering::Relaxed) || started.elapsed() >= PATIENCE;
                let outcome = panic::catch_unwind(AssertUnwindSafe(|| work(&cancelled)))
                    .unwrap_or_else(|_| {
                        log::error!("checking a new constraint panicked");
                        Outcome::Unknown
                    });
                if sender.send(outcome).is_ok() {
                    wake();
                }
            });

        if let Err(error) = spawned {
            log::warn!("could not start checking a new constraint: {error}");
            return Some(checked.unchecked());
        }
        self.running = Some(Running {
            checked,
            label,
            revision,
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
        Some(running.checked.verdict(outcome))
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
}

impl Job {
    fn run(self, cancelled: &dyn Fn() -> bool) -> Outcome {
        let trial = ConstraintTrial {
            feature: self.feature,
            transaction: self.transaction,
            touched: self.touched,
            intent: self.intent,
        };
        outcome(self.document, &trial, cancelled)
    }
}

pub fn conflict(
    document: &Document,
    trial: &ConstraintTrial,
    cancelled: &dyn Fn() -> bool,
) -> Option<Refusal> {
    match outcome(document.clone(), trial, cancelled) {
        Outcome::Conflicts(reason) => Some(reason),
        Outcome::Holds | Outcome::Measured(_) | Outcome::Unknown => None,
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
            (trial.feature, sketch),
            (trial.intent, &trial.touched),
            &constraints,
        )),
        Err(_) => Outcome::Unknown,
    }
}

fn refusal(
    (feature, sketch): (FeatureId, &Sketch),
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
    Refusal::Conflict {
        subject,
        intent,
        touched: touched.len(),
        named,
        feature,
        conflicting: others,
    }
}

fn conflict_sentence(
    subject: &str,
    (intent, touched): (Intent, usize),
    named: &[String],
    conflicting: &[ConstraintId],
) -> String {
    let more = conflicting.len().saturating_sub(named.len());
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
