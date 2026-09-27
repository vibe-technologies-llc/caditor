use std::time::Instant;

use caditor_document::{
    Document, Editor, Evaluation, ModelEvaluator, Outcome, ParameterValues, Progress, Recomputer,
    Transaction,
};

pub type Waker = Box<dyn Fn() + Send>;
pub type WakerFactory = Box<dyn Fn() -> Waker>;

#[derive(Debug, Clone)]
pub enum Action {
    Apply(Transaction),
    Undo,
    Redo,
    Recompute,
    CancelRecompute,
    DismissNotice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecomputeStatus {
    UpToDate,
    Running { since: Instant },
    Cancelled,
    Stopped,
}

pub struct Model {
    editor: Editor,
    parameters: ParameterValues,
    evaluation: Evaluation,
    recomputer: Option<Recomputer>,
    make_waker: WakerFactory,
    status: RecomputeStatus,
    notice: Option<String>,
}

impl Model {
    pub fn new(document: Document, make_waker: WakerFactory) -> Self {
        let mut model = Self {
            parameters: ParameterValues::evaluate(&document),
            editor: Editor::new(document),
            evaluation: Evaluation::default(),
            recomputer: None,
            make_waker,
            status: RecomputeStatus::UpToDate,
            notice: None,
        };
        model.recompute();
        model
    }

    pub fn document(&self) -> &Document {
        self.editor.document()
    }

    pub fn parameters(&self) -> &ParameterValues {
        &self.parameters
    }

    pub fn evaluation(&self) -> &Evaluation {
        &self.evaluation
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.editor.undo_label()
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.editor.redo_label()
    }

    pub fn status(&self) -> RecomputeStatus {
        self.status
    }

    pub fn progress(&self) -> Option<Progress> {
        self.recomputer.as_ref().and_then(Recomputer::progress)
    }

    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    pub fn perform(&mut self, action: Action) {
        let changed = match action {
            Action::Apply(transaction) => {
                let label = transaction.label().to_owned();
                self.editor
                    .apply(transaction)
                    .map(|()| true)
                    .map_err(|error| format!("{label}: {error}"))
            }
            Action::Undo => self
                .editor
                .undo()
                .map(|undone| undone.is_some())
                .map_err(|error| format!("Undo failed: {error}")),
            Action::Redo => self
                .editor
                .redo()
                .map(|redone| redone.is_some())
                .map_err(|error| format!("Redo failed: {error}")),
            Action::Recompute => {
                self.recompute();
                Ok(false)
            }
            Action::CancelRecompute => {
                if let Some(recomputer) = &self.recomputer {
                    recomputer.cancel();
                }
                Ok(false)
            }
            Action::DismissNotice => {
                self.notice = None;
                Ok(false)
            }
        };
        match changed {
            Ok(true) => {
                self.notice = None;
                self.parameters = ParameterValues::evaluate(self.editor.document());
                self.recompute();
            }
            Ok(false) => {}
            Err(message) => {
                log::warn!("{message}");
                self.notice = Some(message);
            }
        }
    }

    pub fn poll(&mut self) -> bool {
        let Some(recomputer) = &self.recomputer else {
            return false;
        };
        match recomputer.poll() {
            Ok(Some(update)) => {
                if update.revision == self.editor.revision() {
                    self.status = match update.outcome {
                        Outcome::Finished => RecomputeStatus::UpToDate,
                        Outcome::Cancelled => RecomputeStatus::Cancelled,
                    };
                }
                self.evaluation = update.evaluation;
                true
            }
            Ok(None) => false,
            Err(error) => {
                log::error!("{error}");
                self.recomputer = None;
                self.status = RecomputeStatus::Stopped;
                true
            }
        }
    }

    fn recompute(&mut self) {
        if self.recomputer.is_none() {
            match Recomputer::spawn(ModelEvaluator, (self.make_waker)()) {
                Ok(recomputer) => self.recomputer = Some(recomputer),
                Err(error) => {
                    log::error!("could not start the recompute worker: {error}");
                    self.status = RecomputeStatus::Stopped;
                    return;
                }
            }
        }
        let document = self.editor.document().clone();
        let revision = self.editor.revision();
        let submitted = self
            .recomputer
            .as_mut()
            .map(|recomputer| recomputer.submit(document, revision));
        self.status = match submitted {
            Some(Ok(())) => RecomputeStatus::Running {
                since: Instant::now(),
            },
            Some(Err(error)) => {
                log::error!("{error}");
                self.recomputer = None;
                RecomputeStatus::Stopped
            }
            None => RecomputeStatus::Stopped,
        };
    }
}
