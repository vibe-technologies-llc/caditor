use std::{
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
    time::Duration,
};

use caditor_document::{FeatureId, FeatureResult, body_outline};
use caditor_geometry::Plane;
use caditor_kernel::{HiddenLineError, interruptible};
use egui::Ui;

#[cfg(test)]
use crate::bodies::Hold;
use crate::{
    commands::{Command, CommandFrame},
    model::{Action, Model, Waker},
    projecting::{self, Projected, ProjectionError},
    widgets,
};

pub const STILL_OUTLINING: &str =
    "Still working out the outline of a body; wait for it or cancel it first";
pub const NOT_OUTLINING: &str = "No outline is being worked out";
pub const STOPPED: &str = "Stopped projecting the outline. Nothing was added.";
const CANCEL_HOVER: &str = "Stop working out the outline and project nothing";
const PROGRESS_REFRESH: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
pub struct OutlineRequest {
    pub sketch: FeatureId,
    pub body: FeatureId,
    pub name: String,
    pub plane: Plane,
    pub result: Arc<FeatureResult>,
}

pub enum OutlineFinished {
    Worked {
        request: OutlineRequest,
        pieces: Vec<Projected>,
    },
    Refused(ProjectionError),
    Cancelled,
}

enum Outcome {
    Worked(Vec<Projected>),
    Unclear(HiddenLineError),
    Cancelled,
    Crashed,
}

struct Running {
    request: OutlineRequest,
    revision: u64,
    cancelled: Arc<AtomicBool>,
    outcome: Receiver<Outcome>,
}

#[derive(Default)]
pub struct Outlines {
    running: Option<Running>,
    #[cfg(test)]
    hold: Hold,
}

impl Outlines {
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    pub fn name(&self) -> Option<&str> {
        self.running
            .as_ref()
            .map(|running| running.request.name.as_str())
    }

    pub fn start(
        &mut self,
        request: OutlineRequest,
        revision: u64,
        wake: Waker,
    ) -> Result<(), ProjectionError> {
        if self.running.is_some() {
            return Err(ProjectionError::StillOutlining);
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        let (sender, outcome) = mpsc::channel();
        let job = Job {
            request: request.clone(),
            cancelled: Arc::clone(&cancelled),
            #[cfg(test)]
            hold: self.hold.clone(),
        };
        let spawned = thread::Builder::new()
            .name("outline".to_owned())
            .spawn(move || {
                let outcome =
                    panic::catch_unwind(AssertUnwindSafe(|| job.run())).unwrap_or_else(|_| {
                        log::error!("working out an outline panicked");
                        Outcome::Crashed
                    });
                if sender.send(outcome).is_ok() {
                    wake();
                }
            });
        if let Err(error) = spawned {
            log::warn!("could not start working out an outline: {error}");
            return Err(ProjectionError::OutlineCrashed { name: request.name });
        }
        self.running = Some(Running {
            request,
            revision,
            cancelled,
            outcome,
        });
        Ok(())
    }

    pub fn poll(&mut self, revision: u64) -> Option<OutlineFinished> {
        let outcome = match self.running.as_ref()?.outcome.try_recv() {
            Ok(outcome) => outcome,
            Err(TryRecvError::Disconnected) => Outcome::Crashed,
            Err(TryRecvError::Empty) => return None,
        };
        self.finish(outcome, revision)
    }

    #[cfg(test)]
    pub fn hold_on_a_thread(&mut self, hold: Hold) {
        self.hold = hold;
    }

    fn finish(&mut self, outcome: Outcome, revision: u64) -> Option<OutlineFinished> {
        let running = self.running.take()?;
        let name = running.request.name.clone();
        Some(match outcome {
            Outcome::Cancelled => OutlineFinished::Cancelled,
            Outcome::Crashed => OutlineFinished::Refused(ProjectionError::OutlineCrashed { name }),
            Outcome::Unclear(error) => {
                OutlineFinished::Refused(projecting::outline_unclear(&name, &error))
            }
            Outcome::Worked(_) if running.revision != revision => {
                OutlineFinished::Refused(ProjectionError::OutlineOutdated { name })
            }
            Outcome::Worked(pieces) => OutlineFinished::Worked {
                request: running.request,
                pieces,
            },
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
    request: OutlineRequest,
    cancelled: Arc<AtomicBool>,
    #[cfg(test)]
    hold: Hold,
}

impl Job {
    fn run(self) -> Outcome {
        let Some(solid) = self.request.result.solid() else {
            return Outcome::Crashed;
        };
        let cancelled = Arc::clone(&self.cancelled);
        let interrupt = Arc::new(move || cancelled.load(Ordering::Relaxed));
        interruptible(interrupt, || {
            #[cfg(test)]
            if self.hold.wait().is_err() {
                return Outcome::Cancelled;
            }
            match body_outline(&solid.solid, self.request.body, &self.request.plane) {
                Ok(pieces) => Outcome::Worked(pieces),
                Err(HiddenLineError::Cancelled(_)) => Outcome::Cancelled,
                Err(error) => Outcome::Unclear(error),
            }
        })
    }
}

pub fn progress_text(name: &str) -> String {
    format!("Working out the outline of {name}…")
}

pub fn activity(
    ui: &mut Ui,
    model: &Model,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let running = model.outlining().ok_or(NOT_OUTLINING);
    let mut cancel = commands.invoke(Command::CancelOutline, &running);
    if let Ok(name) = running {
        let hover = commands.with_keys(Command::CancelOutline, CANCEL_HOVER);
        cancel |= widgets::progress(progress_text(name))
            .cancel(hover)
            .show(ui);
        ui.ctx().request_repaint_after(PROGRESS_REFRESH);
    }
    if cancel {
        actions.push(Action::CancelOutline);
    }
}
