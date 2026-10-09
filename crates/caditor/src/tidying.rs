use std::{
    collections::BTreeSet,
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
};

use caditor_document::{FeatureId, ParameterValues, Transaction};
use caditor_sketch::{
    ANGLE_DEGREES, Constraint, EntityId, Fix, Flaw, InferenceError, Kept, RELATIVE_DISTANCE,
    RelationKind, Sketch, Tolerance,
};

use crate::{
    model::{Model, Waker},
    sketch_tools,
    units::Units,
};

pub const NOT_SETTLED: &str = "Waiting for the sketch to solve…";
pub const UNSOLVED: &str = "The sketch has to solve before anything can be found in it. Fix the \
                            problem the sketch bar names first.";
pub const FAILED: &str = "Looking through the sketch failed. Try again after changing it.";
const TIGHT_SHARE: f64 = 2e-4;
const TIGHT_DEGREES: f64 = 0.25;
const LOOSE_SHARE: f64 = 5e-3;
const LOOSE_DEGREES: f64 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Task {
    #[default]
    Relations,
    Dimensions,
    Check,
}

impl Task {
    pub const ALL: [Self; 3] = [Self::Relations, Self::Dimensions, Self::Check];

    pub fn label(self) -> &'static str {
        match self {
            Self::Relations => "Relations",
            Self::Dimensions => "Dimensions",
            Self::Check => "Check",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Self::Relations => {
                "Finds the relations the drawing already shows, such as ends that meet, lines \
                 that are level or parallel and curves that touch, and adds the ones the sketch \
                 does not hold yet as one change you can undo."
            }
            Self::Dimensions => {
                "Adds dimensions from a datum point to everything still free, at the sizes as \
                 drawn, so the sketch becomes fully constrained in one change you can undo."
            }
            Self::Check => {
                "Names what is hard to see: ends a hair apart that are not joined, curves lying \
                 on others and curves of no length, each with a fix."
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Looseness {
    Tight,
    #[default]
    Normal,
    Loose,
}

impl Looseness {
    pub const ALL: [Self; 3] = [Self::Tight, Self::Normal, Self::Loose];

    pub fn label(self) -> &'static str {
        match self {
            Self::Tight => "Tight",
            Self::Normal => "Normal",
            Self::Loose => "Loose",
        }
    }

    pub fn tolerance(self, sketch: &Sketch) -> Tolerance {
        let (share, degrees) = match self {
            Self::Tight => (TIGHT_SHARE, TIGHT_DEGREES),
            Self::Normal => (RELATIVE_DISTANCE, ANGLE_DEGREES),
            Self::Loose => (LOOSE_SHARE, LOOSE_DEGREES),
        };
        Tolerance::relative(sketch, share, degrees)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Proposal {
    Relations(Kept),
    Dimensions(Kept),
    Flaws(Vec<(Flaw, Fix)>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub proposal: Proposal,
    pub tolerance: Tolerance,
    pub sketch: Sketch,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Progress<'a> {
    Closed,
    NotSettled,
    Working,
    Found(&'a Found),
    Failed(&'a str),
}

#[derive(Debug, Clone, PartialEq)]
struct Basis {
    feature: FeatureId,
    revision: u64,
    evaluation: u64,
    task: Task,
    looseness: Looseness,
    kinds: BTreeSet<RelationKind>,
    datum: EntityId,
}

type Answer = Result<Found, String>;

struct Running {
    basis: Basis,
    cancelled: Arc<AtomicBool>,
    answer: Receiver<Answer>,
}

pub struct Tidying {
    pub open: bool,
    pub task: Task,
    pub looseness: Looseness,
    pub kinds: BTreeSet<RelationKind>,
    pub datum: EntityId,
    feature: Option<FeatureId>,
    running: Option<Running>,
    answered: Option<(Basis, Answer)>,
}

impl Default for Tidying {
    fn default() -> Self {
        Self {
            open: false,
            task: Task::default(),
            looseness: Looseness::default(),
            kinds: RelationKind::ALL.into_iter().collect(),
            datum: EntityId::ORIGIN,
            feature: None,
            running: None,
            answered: None,
        }
    }
}

impl Tidying {
    pub fn open_at(&mut self, task: Task, feature: FeatureId) {
        if self.feature != Some(feature) {
            self.datum = EntityId::ORIGIN;
        }
        self.open = true;
        self.task = task;
        self.feature = Some(feature);
    }

    pub fn close(&mut self) {
        self.open = false;
        self.feature = None;
        self.answered = None;
        self.cancel();
    }

    fn cancel(&mut self) {
        if let Some(running) = self.running.take() {
            running.cancelled.store(true, Ordering::Relaxed);
        }
    }

    pub fn feature(&self) -> Option<FeatureId> {
        self.feature.filter(|_| self.open)
    }

    pub fn refresh(&mut self, model: &Model, edited: Option<FeatureId>) {
        if !self.open {
            return;
        }
        if edited.is_none() || edited != self.feature {
            self.close();
            return;
        }
        let Some(feature) = self.feature else {
            return;
        };
        let Some(sketch) = model.settled_sketch(feature) else {
            self.answered = None;
            self.cancel();
            return;
        };
        if sketch.point(self.datum).is_none() {
            self.datum = EntityId::ORIGIN;
        }
        let basis = Basis {
            feature,
            revision: model.revision(),
            evaluation: model.evaluation_generation(),
            task: self.task,
            looseness: self.looseness,
            kinds: self.kinds.clone(),
            datum: self.datum,
        };
        if self
            .answered
            .as_ref()
            .is_some_and(|(answered, _)| *answered == basis)
        {
            return;
        }
        if let Some(running) = &self.running
            && running.basis == basis
        {
            self.poll();
            return;
        }
        self.cancel();
        self.answered = None;
        self.start(
            basis,
            sketch.clone(),
            model.parameters().clone(),
            model.waker(),
        );
    }

    pub fn progress(&self) -> Progress<'_> {
        if self.feature().is_none() {
            return Progress::Closed;
        }
        match (&self.answered, &self.running) {
            (Some((_, Ok(found))), _) => Progress::Found(found),
            (Some((_, Err(reason))), _) => Progress::Failed(reason),
            (None, Some(_)) => Progress::Working,
            (None, None) => Progress::NotSettled,
        }
    }

    #[cfg(test)]
    pub fn wait(&mut self) {
        if let Some(running) = self.running.take() {
            let answer = running
                .answer
                .recv()
                .unwrap_or_else(|_| Err(FAILED.to_owned()));
            self.answered = Some((running.basis, answer));
        }
    }

    fn poll(&mut self) {
        let Some(running) = &self.running else {
            return;
        };
        let answer = match running.answer.try_recv() {
            Ok(answer) => answer,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(FAILED.to_owned()),
        };
        if let Some(running) = self.running.take() {
            self.answered = Some((running.basis, answer));
        }
    }

    fn start(&mut self, basis: Basis, sketch: Sketch, values: ParameterValues, wake: Waker) {
        let cancelled = Arc::new(AtomicBool::new(false));
        let (sender, answer) = mpsc::channel();
        let job = Job {
            task: basis.task,
            tolerance: basis.looseness.tolerance(&sketch),
            kinds: basis.kinds.clone(),
            datum: basis.datum,
            sketch,
            values,
            cancelled: Arc::clone(&cancelled),
        };
        let spawned = thread::Builder::new()
            .name("sketch tidying".to_owned())
            .spawn(move || {
                let answer =
                    panic::catch_unwind(AssertUnwindSafe(|| job.run())).unwrap_or_else(|_| {
                        log::error!("looking through a sketch panicked");
                        Some(Err(FAILED.to_owned()))
                    });
                if let Some(answer) = answer
                    && sender.send(answer).is_ok()
                {
                    wake();
                }
            });
        match spawned {
            Ok(_) => {
                self.running = Some(Running {
                    basis,
                    cancelled,
                    answer,
                });
            }
            Err(error) => {
                log::warn!("could not start looking through the sketch: {error}");
                self.answered = Some((basis, Err(FAILED.to_owned())));
            }
        }
    }
}

struct Job {
    task: Task,
    tolerance: Tolerance,
    kinds: BTreeSet<RelationKind>,
    datum: EntityId,
    sketch: Sketch,
    values: ParameterValues,
    cancelled: Arc<AtomicBool>,
}

impl Job {
    fn run(self) -> Option<Answer> {
        let value_of = |id| self.values.value(id);
        let cancelled = || self.cancelled.load(Ordering::Relaxed);
        let proposal = match self.task {
            Task::Relations => self
                .sketch
                .inferred_relations(self.tolerance, &self.kinds, &value_of, &cancelled)
                .map(Proposal::Relations),
            Task::Dimensions => self
                .sketch
                .datum_dimensions(self.datum, self.tolerance, &value_of, &cancelled)
                .map(Proposal::Dimensions),
            Task::Check => Ok(Proposal::Flaws(
                self.sketch
                    .flaws(self.tolerance)
                    .into_iter()
                    .map(|flaw| {
                        let fix = self.sketch.fix(&flaw);
                        (flaw, fix)
                    })
                    .collect(),
            )),
        };
        match proposal {
            Ok(proposal) => Some(Ok(Found {
                proposal,
                tolerance: self.tolerance,
                sketch: self.sketch,
            })),
            Err(InferenceError::Cancelled) => None,
            Err(InferenceError::Unsolved(_)) => Some(Err(UNSOLVED.to_owned())),
            Err(error @ InferenceError::NotAPoint { .. }) => {
                Some(Err(sentence(&format!("the datum is not usable: {error}"))))
            }
        }
    }
}

pub fn sentence(text: &str) -> String {
    let mut characters = text.chars();
    match characters.next() {
        Some(first) => format!("{}{}.", first.to_uppercase(), characters.as_str()),
        None => String::new(),
    }
}

pub fn count(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}

pub fn relations_transaction(
    model: &Model,
    feature: FeatureId,
    constraints: &[Constraint],
) -> Transaction {
    let label = format!(
        "Add {} the drawing shows",
        count(constraints.len(), "relation", "relations")
    );
    let mut transaction = sketch_tools::settled_transaction(model, feature, label);
    for constraint in constraints {
        transaction.add_sketch_constraint(feature, constraint.clone());
    }
    transaction.finish()
}

pub fn dimensions_transaction(
    model: &Model,
    feature: FeatureId,
    datum: &str,
    constraints: &[Constraint],
    units: Units,
) -> Transaction {
    let label = format!(
        "Add {} from {datum}",
        count(constraints.len(), "dimension", "dimensions")
    );
    let mut transaction = sketch_tools::settled_transaction(model, feature, label);
    for constraint in sketch_tools::in_unit(constraints.to_vec(), units) {
        transaction.add_sketch_constraint(feature, constraint);
    }
    transaction.finish()
}

pub fn fixes_transaction(
    model: &Model,
    feature: FeatureId,
    label: String,
    fixes: &[&Fix],
) -> Transaction {
    let removed: BTreeSet<EntityId> = fixes
        .iter()
        .flat_map(|fix| fix.remove.iter().copied())
        .collect();
    let mut added: Vec<Constraint> = Vec::new();
    for constraint in fixes.iter().flat_map(|fix| fix.add.iter()) {
        let touches_removed = constraint
            .entities()
            .iter()
            .any(|entity| removed.contains(entity));
        if !touches_removed && !added.contains(constraint) {
            added.push(constraint.clone());
        }
    }
    let mut transaction = sketch_tools::settled_transaction(model, feature, label);
    transaction.remove_sketch_items(feature, removed, []);
    for constraint in added {
        transaction.add_sketch_constraint(feature, constraint);
    }
    transaction.finish()
}
