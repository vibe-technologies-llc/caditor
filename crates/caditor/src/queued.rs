use egui::Ui;

use crate::{
    commands::Command,
    editing::EditingCommand,
    model::{Action, Model, Notice, RecomputeStatus},
    widgets,
};

pub const BODY_SHAPE: &str = "The body has no shape yet";
pub const BODIES_SHAPE: &str = "A selected body has no shape yet";
pub const MODEL_UP_TO_THERE: &str = "The model is not recomputed up to there yet";
pub const SKETCH_SHOWN: &str = "The sketch is being recalculated";
pub const CONSTRAINT_CHECK: &str = "The last constraint is still being checked against the sketch";
pub const CANCEL_HOVER: &str = "Do not run it when the model is ready (Esc)";
const RECOMPUTE_FIRST: &str = "recompute the model first";
const RESTART_FIRST: &str = "restart the recompute first";
const MEND_FIRST: &str = "mend the failed feature first";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Awaited {
    Recompute,
    ConstraintCheck,
}

impl Awaited {
    pub fn of(reason: &str) -> Option<Self> {
        match reason {
            BODY_SHAPE | BODIES_SHAPE | MODEL_UP_TO_THERE | SKETCH_SHOWN => Some(Self::Recompute),
            CONSTRAINT_CHECK => Some(Self::ConstraintCheck),
            _ => None,
        }
    }

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
}

#[derive(Debug, Clone)]
pub struct Queued {
    rerun: Rerun,
    title: String,
    reason: String,
    awaited: Awaited,
}

impl Queued {
    pub fn new(rerun: Rerun, title: impl Into<String>, reason: &str) -> Option<Self> {
        Some(Self {
            awaited: Awaited::of(reason)?,
            rerun,
            title: title.into(),
            reason: reason.to_owned(),
        })
    }

    pub fn rerun(&self) -> &Rerun {
        &self.rerun
    }

    pub fn awaited(&self) -> Awaited {
        self.awaited
    }

    pub fn words(&self) -> String {
        format!("{}: waiting for {}…", self.title, self.awaited.what())
    }

    pub fn refusal(&self, status: RecomputeStatus) -> String {
        let first = match (self.awaited, status) {
            (Awaited::Recompute, RecomputeStatus::Cancelled) => Some(RECOMPUTE_FIRST),
            (Awaited::Recompute, RecomputeStatus::Stopped) => Some(RESTART_FIRST),
            (Awaited::Recompute, RecomputeStatus::UpToDate | RecomputeStatus::Running { .. }) => {
                Some(MEND_FIRST)
            }
            (Awaited::ConstraintCheck, _) => None,
        };
        match first {
            Some(first) => format!("{}: {}; {first}.", self.title, self.reason),
            None => format!("{}: {}.", self.title, self.reason),
        }
    }
}

pub fn refused(command: Command, title: &str, reason: &str) -> Action {
    match Queued::new(Rerun::Command(command), title, reason) {
        Some(queued) => Action::Queue(Box::new(queued)),
        None => Action::Inform(Notice::warning(format!("{title}: {reason}."))),
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
            && queued.awaited == Awaited::ConstraintCheck
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
        Queued::new(Rerun::Command(Command::Move), "Move", BODY_SHAPE).unwrap()
    }

    #[test]
    fn only_a_reason_that_something_running_can_answer_is_queued() {
        assert!(Queued::new(Rerun::Command(Command::Move), "Move", "Select a body").is_none());
        assert_eq!(moving().awaited(), Awaited::Recompute);
        assert_eq!(
            Awaited::of(CONSTRAINT_CHECK),
            Some(Awaited::ConstraintCheck)
        );
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
        let parallel =
            Queued::new(Rerun::Command(Command::Move), "Parallel", CONSTRAINT_CHECK).unwrap();
        queue.hold(parallel, 1, 4);
        queue.follow(4, 5);
        assert!(queue.current(1, 5).is_some());

        queue.hold(moving(), 1, 5);
        queue.follow(5, 6);
        assert!(queue.current(1, 6).is_none());
    }
}
