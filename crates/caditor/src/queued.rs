use std::borrow::Cow;

use caditor_document::FeatureId;
use egui::Ui;

use crate::{
    commands::{Command, Reason},
    editing::EditingCommand,
    model::{Action, Model, Notice, RecomputeStatus},
    selection::Pickable,
    widgets,
};

pub const MODEL_UP_TO_THERE: &str = "The model is not recomputed up to there yet";
pub const QUEUE_ON_CLICK: &str = "Click to run it once the model has recomputed.";
pub const CANCEL_HOVER: &str = "Do not run it when the model is ready (Esc)";
const RECOMPUTE_FIRST: &str = "recompute the model first";
const RESTART_FIRST: &str = "restart the recompute first";
const MEND_FIRST: &str = "mend the failed feature first";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Awaited {
    Recompute,
    ConstraintCheck,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Pending {
    #[error("The body has no shape yet")]
    BodyShape,
    #[error("A selected body has no shape yet")]
    BodiesShape,
    #[error("{MODEL_UP_TO_THERE}")]
    ModelUpToThere,
    #[error("The sketch is being recalculated")]
    SketchShown,
    #[error("The last constraint is still being checked against the sketch")]
    ConstraintCheck,
}

impl Pending {
    pub fn awaited(self) -> Awaited {
        match self {
            Self::BodyShape | Self::BodiesShape | Self::ModelUpToThere | Self::SketchShown => {
                Awaited::Recompute
            }
            Self::ConstraintCheck => Awaited::ConstraintCheck,
        }
    }
}

impl Reason for Pending {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }

    fn pending(&self) -> Option<Pending> {
        Some(*self)
    }
}

impl Awaited {
    fn what(self) -> &'static str {
        match self {
            Self::Recompute => "the model to recompute",
            Self::ConstraintCheck => "the constraint check",
        }
    }
}

#[derive(Debug, Clone)]
pub enum Rerun {
    Command(Command),
    Editing(EditingCommand),
    Typed(String),
    OwnThickness {
        feature: FeatureId,
        pickable: Pickable,
    },
}

#[derive(Debug, Clone)]
pub struct Queued {
    rerun: Rerun,
    title: String,
    pending: Pending,
}

impl Queued {
    pub fn new(rerun: Rerun, title: impl Into<String>, pending: Pending) -> Self {
        Self {
            rerun,
            title: title.into(),
            pending,
        }
    }

    pub fn rerun(&self) -> &Rerun {
        &self.rerun
    }

    pub fn awaited(&self) -> Awaited {
        self.pending.awaited()
    }

    pub fn words(&self) -> String {
        format!("{}: waiting for {}…", self.title, self.awaited().what())
    }

    pub fn refusal(&self, status: RecomputeStatus) -> String {
        let first = match (self.awaited(), status) {
            (Awaited::Recompute, RecomputeStatus::Cancelled) => Some(RECOMPUTE_FIRST),
            (Awaited::Recompute, RecomputeStatus::Stopped) => Some(RESTART_FIRST),
            (Awaited::Recompute, RecomputeStatus::UpToDate | RecomputeStatus::Running { .. }) => {
                Some(MEND_FIRST)
            }
            (Awaited::ConstraintCheck, _) => None,
        };
        match first {
            Some(first) => format!("{}: {}; {first}.", self.title, self.pending),
            None => format!("{}: {}.", self.title, self.pending),
        }
    }
}

pub fn refused(command: Command, title: &str, reason: &impl Reason) -> Action {
    refused_rerun(Rerun::Command(command), title, reason)
}

pub fn refused_rerun(rerun: Rerun, title: &str, reason: &impl Reason) -> Action {
    match reason.pending() {
        Some(pending) => Action::Queue(Box::new(Queued::new(rerun, title, pending))),
        None => Action::Inform(Notice::warning(format!("{title}: {}", reason.reason()))),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Basis {
    session: u64,
    revision: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Queue {
    held: Option<(Queued, Basis)>,
}

impl Queue {
    pub fn hold(&mut self, queued: Queued, session: u64, revision: u64) {
        self.held = Some((queued, Basis { session, revision }));
    }

    pub fn let_go(&mut self) -> bool {
        self.held.take().is_some()
    }

    pub fn current(&self, session: u64, revision: u64) -> Option<&Queued> {
        let (queued, basis) = self.held.as_ref()?;
        (*basis == Basis { session, revision }).then_some(queued)
    }

    pub fn follow(&mut self, from: u64, to: u64) {
        if let Some((queued, basis)) = &mut self.held
            && queued.awaited() == Awaited::ConstraintCheck
            && basis.revision == from
        {
            basis.revision = to;
        }
    }

    pub fn take_due(
        &mut self,
        session: u64,
        revision: u64,
        busy: impl Fn(Awaited) -> bool,
    ) -> Option<Rerun> {
        let awaited = self.current(session, revision).map(Queued::awaited);
        match awaited {
            None => {
                self.held = None;
                None
            }
            Some(awaited) if busy(awaited) => None,
            Some(_) => self.held.take().map(|(queued, _)| queued.rerun),
        }
    }
}

pub fn note(ui: &mut Ui, model: &Model, actions: &mut Vec<Action>) {
    if let Some(queued) = model.queued()
        && widgets::progress(queued.words())
            .cancel(CANCEL_HOVER)
            .show(ui)
    {
        actions.push(Action::DropQueued);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moving() -> Queued {
        Queued::new(Rerun::Command(Command::Move), "Move", Pending::BodyShape)
    }

    #[test]
    fn only_a_reason_that_something_running_can_answer_is_queued() {
        let unanswerable = refused(Command::Move, "Move", &"Select a body");
        let answerable = refused(Command::Move, "Move", &Pending::BodyShape);

        assert!(matches!(unanswerable, Action::Inform(_)));
        assert!(matches!(answerable, Action::Queue(_)));
        assert_eq!(moving().awaited(), Awaited::Recompute);
        assert_eq!(Pending::ConstraintCheck.awaited(), Awaited::ConstraintCheck);
    }

    #[test]
    fn a_queued_action_runs_once_nothing_it_waits_for_is_running_and_never_on_another_model() {
        let mut queue = Queue::default();
        queue.hold(moving(), 1, 4);

        let while_busy = queue.take_due(1, 4, |_| true);
        let due = queue.take_due(1, 4, |_| false);
        let again = queue.take_due(1, 4, |_| false);

        assert!(while_busy.is_none());
        assert!(matches!(due, Some(Rerun::Command(Command::Move))));
        assert!(again.is_none());

        queue.hold(moving(), 1, 4);
        assert!(queue.current(1, 5).is_none());
        assert!(queue.take_due(1, 5, |_| false).is_none());
        assert!(queue.current(1, 4).is_none());

        queue.hold(moving(), 1, 4);
        assert!(queue.take_due(2, 4, |_| false).is_none());
    }

    #[test]
    fn a_constraint_waiting_for_a_check_follows_the_change_the_check_applied() {
        let mut queue = Queue::default();
        let parallel = Queued::new(
            Rerun::Command(Command::Move),
            "Parallel",
            Pending::ConstraintCheck,
        );
        queue.hold(parallel, 1, 4);
        queue.follow(4, 5);
        assert!(queue.current(1, 5).is_some());

        queue.hold(moving(), 1, 5);
        queue.follow(5, 6);
        assert!(queue.current(1, 6).is_none());
    }
}
