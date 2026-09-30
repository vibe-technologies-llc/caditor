use std::{
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

use caditor_document::{FeatureId, ParameterValues};
use caditor_sketch::{Drag, Sketch, SolveMemo};
use parking_lot::{Condvar, Mutex};

use crate::model::Waker;

#[derive(Debug, Clone, PartialEq)]
pub enum DragCommand {
    Move {
        feature: FeatureId,
        label: String,
        drags: Vec<Drag>,
    },
    Finish,
    Cancel,
}

#[derive(Debug, Clone)]
pub struct Finished {
    pub feature: FeatureId,
    pub label: String,
    pub sketch: Option<Arc<Sketch>>,
}

#[derive(Debug, Default)]
pub struct Polled {
    pub shown: Option<(FeatureId, Arc<Sketch>)>,
    pub finished: Option<Finished>,
    pub abandoned: bool,
}

impl Polled {
    fn abandoned() -> Self {
        Self {
            abandoned: true,
            ..Self::default()
        }
    }
}

struct Job {
    drag: u64,
    sequence: u64,
    start: Arc<Sketch>,
    parameters: Arc<ParameterValues>,
    drags: Vec<Drag>,
}

struct Solution {
    drag: u64,
    sequence: u64,
    solved: Option<Arc<Sketch>>,
}

#[derive(Default)]
struct Slot {
    job: Option<Job>,
    closed: bool,
}

#[derive(Default)]
struct Shared {
    slot: Mutex<Slot>,
    ready: Condvar,
    wanted_from: AtomicU64,
}

struct Previous {
    drag: u64,
    sketch: Arc<Sketch>,
    memo: SolveMemo,
}

struct Worker {
    shared: Arc<Shared>,
    done: Receiver<Solution>,
}

impl Worker {
    fn spawn(wake: Waker) -> Option<Self> {
        let shared = Arc::new(Shared::default());
        let (sender, done) = mpsc::channel();
        let working = Arc::clone(&shared);
        let spawned = thread::Builder::new()
            .name("sketch drags".to_owned())
            .spawn(move || serve(&working, &sender, &wake));
        match spawned {
            Ok(_) => Some(Self { shared, done }),
            Err(error) => {
                log::error!("could not start the sketch drag worker: {error}");
                None
            }
        }
    }

    fn submit(&self, job: Job) {
        self.shared.slot.lock().job = Some(job);
        self.shared.ready.notify_one();
    }

    fn abandon_before(&self, drag: u64) {
        self.shared.wanted_from.store(drag, Ordering::Relaxed);
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.shared.slot.lock().closed = true;
        self.shared.ready.notify_one();
    }
}

fn serve(shared: &Shared, sender: &Sender<Solution>, wake: &Waker) {
    let mut previous: Option<Previous> = None;
    loop {
        let job = {
            let mut slot = shared.slot.lock();
            loop {
                if slot.closed {
                    return;
                }
                if let Some(job) = slot.job.take() {
                    break job;
                }
                shared.ready.wait(&mut slot);
            }
        };
        let continued = previous.take().filter(|previous| previous.drag == job.drag);
        let wanted = || job.drag >= shared.wanted_from.load(Ordering::Relaxed);
        let solution = solve(&job, continued.as_ref(), &|| !wanted());
        previous = match &solution {
            Some((sketch, memo)) => Some(Previous {
                drag: job.drag,
                sketch: Arc::clone(sketch),
                memo: memo.clone(),
            }),
            None => continued,
        };
        if !wanted() {
            continue;
        }
        let sent = sender.send(Solution {
            drag: job.drag,
            sequence: job.sequence,
            solved: solution.map(|(sketch, _)| sketch),
        });
        if sent.is_err() {
            return;
        }
        wake();
    }
}

fn solve(
    job: &Job,
    previous: Option<&Previous>,
    cancelled: &dyn Fn() -> bool,
) -> Option<(Arc<Sketch>, SolveMemo)> {
    let (from, memo) = match previous {
        Some(previous) => (&previous.sketch, Some(&previous.memo)),
        None => (&job.start, None),
    };
    let solved = panic::catch_unwind(AssertUnwindSafe(|| {
        from.solve_from(&|id| job.parameters.value(id), cancelled, &job.drags, memo)
    }));
    match solved {
        Ok(Ok(solved)) => Some((Arc::new(solved.geometry), solved.memo)),
        Ok(Err(error)) => {
            log::debug!("a dragged sketch did not solve: {error}");
            None
        }
        Err(_) => {
            log::error!("solving a dragged sketch panicked");
            None
        }
    }
}

struct Active {
    drag: u64,
    feature: FeatureId,
    label: String,
    revision: u64,
    start: Arc<Sketch>,
    parameters: Arc<ParameterValues>,
    sent: u64,
    received: u64,
    finishing: bool,
    latest: Option<Arc<Sketch>>,
    previous: Option<Previous>,
}

#[derive(Default)]
pub struct SketchDragging {
    worker: Option<Worker>,
    next_drag: u64,
    active: Option<Active>,
}

pub struct Start<'a> {
    pub revision: u64,
    pub parameters: &'a ParameterValues,
    pub shown: &'a dyn Fn(FeatureId) -> Option<Arc<Sketch>>,
    pub wake: &'a dyn Fn() -> Waker,
}

impl SketchDragging {
    #[cfg(test)]
    pub fn is_dragging(&self) -> bool {
        self.active.is_some()
    }

    pub fn perform(&mut self, command: DragCommand, start: &Start<'_>) -> Polled {
        match command {
            DragCommand::Move {
                feature,
                label,
                drags,
            } => self.drag(feature, label, drags, start),
            DragCommand::Finish => {
                let Some(active) = &mut self.active else {
                    return Polled::default();
                };
                if active.revision != start.revision {
                    self.cancel();
                    return Polled::abandoned();
                }
                if active.received < active.sent {
                    active.finishing = true;
                    return Polled::default();
                }
                Polled {
                    finished: self.active.take().map(Active::finished),
                    ..Polled::default()
                }
            }
            DragCommand::Cancel => {
                self.cancel();
                Polled::abandoned()
            }
        }
    }

    pub fn cancel(&mut self) {
        if let (Some(active), Some(worker)) = (self.active.take(), &self.worker) {
            worker.abandon_before(active.drag.saturating_add(1));
        }
    }

    pub fn poll(&mut self) -> Polled {
        let mut polled = Polled::default();
        let Some(worker) = &self.worker else {
            return polled;
        };
        while let Ok(solution) = worker.done.try_recv() {
            let Some(active) = self
                .active
                .as_mut()
                .filter(|active| active.drag == solution.drag)
            else {
                continue;
            };
            active.received = active.received.max(solution.sequence);
            if let Some(solved) = solution.solved {
                active.latest = Some(Arc::clone(&solved));
                polled.shown = Some((active.feature, solved));
            }
        }
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.finishing && active.received >= active.sent)
        {
            polled.finished = self.active.take().map(Active::finished);
        }
        polled
    }

    fn drag(
        &mut self,
        feature: FeatureId,
        label: String,
        drags: Vec<Drag>,
        start: &Start<'_>,
    ) -> Polled {
        let continues = self.active.as_ref().is_some_and(|active| {
            active.feature == feature && active.revision == start.revision && !active.finishing
        });
        if !continues {
            self.cancel();
            let Some(sketch) = (start.shown)(feature) else {
                return Polled::default();
            };
            self.next_drag = self.next_drag.saturating_add(1);
            self.active = Some(Active {
                drag: self.next_drag,
                feature,
                label,
                revision: start.revision,
                start: sketch,
                parameters: Arc::new(start.parameters.clone()),
                sent: 0,
                received: 0,
                finishing: false,
                latest: None,
                previous: None,
            });
        }
        if self.worker.is_none() {
            self.worker = Worker::spawn((start.wake)());
        }
        let Some(active) = self.active.as_mut() else {
            return Polled::default();
        };
        active.sent = active.sent.saturating_add(1);
        let job = Job {
            drag: active.drag,
            sequence: active.sent,
            start: Arc::clone(&active.start),
            parameters: Arc::clone(&active.parameters),
            drags,
        };
        match &self.worker {
            Some(worker) => {
                worker.submit(job);
                Polled::default()
            }
            None => {
                let continued = active.previous.take();
                let solution = solve(&job, continued.as_ref(), &|| false);
                active.received = job.sequence;
                active.previous = match solution {
                    Some((sketch, memo)) => {
                        active.latest = Some(Arc::clone(&sketch));
                        Some(Previous {
                            drag: job.drag,
                            sketch,
                            memo,
                        })
                    }
                    None => continued,
                };
                Polled {
                    shown: active
                        .latest
                        .as_ref()
                        .map(|latest| (feature, Arc::clone(latest))),
                    ..Polled::default()
                }
            }
        }
    }
}

impl Active {
    fn finished(self) -> Finished {
        Finished {
            feature: self.feature,
            label: self.label,
            sketch: self.latest,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use caditor_document::Document;
    use caditor_geometry::{Plane, Point2};
    use caditor_sketch::{Constraint, EntityId};

    use super::*;

    const TIMEOUT: Duration = Duration::from_secs(10);

    fn line_from_origin() -> (Arc<Sketch>, EntityId, EntityId) {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
        let Some((start, end)) = sketch.entity(line).map(|line| {
            let points = line.points();
            (points[0], points[1])
        }) else {
            panic!("the line has its points");
        };
        sketch
            .add_constraint(Constraint::Coincident(start, EntityId::ORIGIN))
            .unwrap();
        (Arc::new(sketch), line, end)
    }

    fn until_finished(dragging: &mut SketchDragging) -> Finished {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(finished) = dragging.poll().finished {
                return finished;
            }
            assert!(Instant::now() < deadline, "the drag never finished");
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn start<'a>(
        revision: u64,
        parameters: &'a ParameterValues,
        shown: &'a dyn Fn(FeatureId) -> Option<Arc<Sketch>>,
    ) -> Start<'a> {
        Start {
            revision,
            parameters,
            shown,
            wake: &|| Box::new(|| {}),
        }
    }

    fn feature() -> FeatureId {
        let mut document = Document::default();
        let mut transaction = document.transaction("Add sketch");
        let feature = transaction.add_feature(
            "Sketch",
            caditor_document::FeatureKind::from(Sketch::new(Plane::XY)),
        );
        document.apply(transaction.finish()).unwrap();
        feature
    }

    #[test]
    fn a_finished_drag_hands_back_the_solution_for_the_last_position() {
        let (sketch, _, end) = line_from_origin();
        let feature = feature();
        let parameters = ParameterValues::default();
        let shown = move |_| Some(Arc::clone(&sketch));
        let start = start(1, &parameters, &shown);
        let mut dragging = SketchDragging::default();

        for x in [12.0, 14.0, 16.0] {
            dragging.perform(
                DragCommand::Move {
                    feature,
                    label: "Drag Line".to_owned(),
                    drags: vec![Drag::Point {
                        point: end,
                        to: Point2::new(x, 3.0),
                    }],
                },
                &start,
            );
        }
        let immediately = dragging.perform(DragCommand::Finish, &start).finished;
        let finished = immediately.unwrap_or_else(|| until_finished(&mut dragging));

        let solved = finished.sketch.unwrap();
        assert_eq!(finished.label, "Drag Line");
        assert_eq!(finished.feature, feature);
        assert!(solved.point(end).unwrap().distance(Point2::new(16.0, 3.0)) < 1e-9);
        assert!(!dragging.is_dragging());
    }

    #[test]
    fn a_drag_made_on_an_older_revision_or_cancelled_commits_nothing() {
        let (sketch, _, end) = line_from_origin();
        let feature = feature();
        let parameters = ParameterValues::default();
        let shown = move |_| Some(Arc::clone(&sketch));
        let mut dragging = SketchDragging::default();
        let moved = DragCommand::Move {
            feature,
            label: "Drag Line".to_owned(),
            drags: vec![Drag::Point {
                point: end,
                to: Point2::new(3.0, 3.0),
            }],
        };

        dragging.perform(moved.clone(), &start(1, &parameters, &shown));
        let stale = dragging.perform(DragCommand::Finish, &start(2, &parameters, &shown));
        assert!(stale.finished.is_none());
        assert!(stale.abandoned);
        assert!(!dragging.is_dragging());

        dragging.perform(moved, &start(2, &parameters, &shown));
        dragging.perform(DragCommand::Cancel, &start(2, &parameters, &shown));
        assert!(!dragging.is_dragging());
        thread::sleep(Duration::from_millis(20));
        assert!(dragging.poll().finished.is_none());
    }
}
