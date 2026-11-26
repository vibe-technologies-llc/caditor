use std::{
    collections::VecDeque,
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

use caditor_document::{FeatureId, Outline, ParameterValues, ProjectionSource};
use caditor_geometry::Point2;
use caditor_sketch::{Constraint, ConstraintId, Drag, EntityId, Sketch, SketchError, SolveMemo};
use parking_lot::{Condvar, Mutex};

use crate::{body_snap::BodyPart, model::Waker};

#[derive(Clone, Debug, PartialEq)]
pub enum DragCommand {
    Move {
        feature: FeatureId,
        label: String,
        drags: Vec<Drag>,
    },
    Finish {
        join: Option<Join>,
    },
    Cancel,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Join {
    pub point: EntityId,
    pub at: Point2,
    pub constraints: Vec<Constraint>,
    pub body: Option<Box<BodyJoin>>,
    pub rim_of: Option<EntityId>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BodyJoin {
    pub projection: ProjectionSource,
    pub outline: Outline,
    pub part: BodyPart,
}

#[derive(Clone, Debug)]
pub struct Finished {
    pub feature: FeatureId,
    pub label: String,
    pub revision: u64,
    pub sketch: Option<Arc<Sketch>>,
    pub join: Option<Join>,
}

#[derive(Debug, Default)]
pub struct Polled {
    pub shown: Option<(FeatureId, Arc<Sketch>)>,
    pub finished: Option<Finished>,
    pub abandoned: bool,
    pub blocked: Option<Blocking>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Blocking {
    #[default]
    Clear,
    By(Vec<ConstraintId>),
}

impl Blocking {
    pub fn is_blocked(&self) -> bool {
        matches!(self, Self::By(_))
    }

    pub fn limits(&self) -> &[ConstraintId] {
        match self {
            Self::Clear => &[],
            Self::By(limits) => limits,
        }
    }
}

impl Polled {
    fn abandoned() -> Self {
        Self {
            abandoned: true,
            ..Self::default()
        }
    }

    fn then(self, later: Self) -> Self {
        Self {
            shown: later.shown.or(self.shown),
            finished: later.finished.or(self.finished),
            abandoned: self.abandoned || later.abandoned,
            blocked: later.blocked.or(self.blocked),
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
    solved: Result<Arc<Sketch>, Vec<ConstraintId>>,
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
            Ok((sketch, memo)) => Some(Previous {
                drag: job.drag,
                sketch: Arc::clone(sketch),
                memo: memo.clone(),
            }),
            Err(_) => continued,
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
) -> Result<(Arc<Sketch>, SolveMemo), Vec<ConstraintId>> {
    let (from, memo) = match previous {
        Some(previous) => (&previous.sketch, Some(&previous.memo)),
        None => (&job.start, None),
    };

    let solved = panic::catch_unwind(AssertUnwindSafe(|| {
        from.solve_geometry_from(&|id| job.parameters.value(id), cancelled, &job.drags, memo)
    }));
    match solved {
        Ok(Ok((geometry, memo))) => Ok((Arc::new(geometry), memo)),
        Ok(Err(error)) => {
            log::debug!("a dragged sketch did not solve: {error}");
            Err(limiting(&error))
        }
        Err(_) => {
            log::error!("solving a dragged sketch panicked");
            Err(Vec::new())
        }
    }
}

fn limiting(error: &SketchError) -> Vec<ConstraintId> {
    match error {
        SketchError::Conflict { constraints } => constraints.clone(),
        SketchError::Unsolvable {
            newest: Some(newest),
            ..
        } => vec![*newest],
        SketchError::Several(errors) => errors.iter().flat_map(limiting).collect(),
        _ => Vec::new(),
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
    join: Option<Join>,
    latest: Option<Arc<Sketch>>,
    previous: Option<Previous>,
}

struct Queued {
    feature: FeatureId,
    label: String,
    drags: Vec<Drag>,
    finish: Option<Option<Join>>,
}

impl Queued {
    fn is_held(&self) -> bool {
        self.finish.is_none()
    }
}

#[derive(Default)]
pub struct SketchDragging {
    worker: Option<Worker>,
    next_drag: u64,
    active: Option<Active>,
    queued: VecDeque<Queued>,
    #[cfg(test)]
    results_held: bool,
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

    #[cfg(test)]
    pub fn hold_results(&mut self, held: bool) {
        self.results_held = held;
    }

    pub fn perform(&mut self, command: DragCommand, start: &Start<'_>) -> Polled {
        match command {
            DragCommand::Move {
                feature,
                label,
                drags,
            } => self.drag(feature, label, drags, start),
            DragCommand::Finish { join } if !self.queued.is_empty() => {
                if let Some(held) = self.queued.back_mut().filter(|queued| queued.is_held()) {
                    held.finish = Some(join);
                }
                Polled::default()
            }
            DragCommand::Finish { join } => self.finish(join, start),
            DragCommand::Cancel if self.waits_for_finishing() => {
                self.drop_queued_held();
                Polled::default()
            }
            DragCommand::Cancel => {
                self.cancel();
                Polled::abandoned()
            }
        }
    }

    pub fn resume(&mut self, start: &Start<'_>) -> Polled {
        let mut polled = Polled::default();

        while self.active.is_none() && polled.finished.is_none() {
            let Some(queued) = self.queued.pop_front() else {
                break;
            };

            let moved = self.move_active(queued.feature, queued.label, queued.drags, start);

            polled = polled.then(moved);
            if let Some(join) = queued.finish {
                let finished = self.finish(join, start);

                polled = polled.then(finished);
            }
        }
        polled
    }

    pub fn cancel_held(&mut self) -> bool {
        if self.waits_for_finishing() {
            self.drop_queued_held();
            return false;
        }

        let held = self.active.is_some();

        if held {
            self.cancel();
        }
        held
    }

    pub fn cancel(&mut self) {
        self.queued.clear();
        self.drop_active();
    }

    fn drop_active(&mut self) {
        if let (Some(active), Some(worker)) = (self.active.take(), &self.worker) {
            worker.abandon_before(active.drag.saturating_add(1));
        }
    }

    fn waits_for_finishing(&self) -> bool {
        !self.queued.is_empty() || self.active.as_ref().is_some_and(|active| active.finishing)
    }

    fn drop_queued_held(&mut self) {
        if self.queued.back().is_some_and(Queued::is_held) {
            self.queued.pop_back();
        }
    }

    fn queue(&mut self, feature: FeatureId, label: String, drags: Vec<Drag>) {
        match self.queued.back_mut().filter(|queued| queued.is_held()) {
            Some(held) if held.feature == feature => held.drags = drags,
            Some(held) => {
                *held = Queued {
                    feature,
                    label,
                    drags,
                    finish: None,
                };
            }
            None => self.queued.push_back(Queued {
                feature,
                label,
                drags,
                finish: None,
            }),
        }
    }

    pub fn poll(&mut self) -> Polled {
        let mut polled = Polled::default();

        #[cfg(test)]
        if self.results_held {
            return polled;
        }

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
            match solution.solved {
                Ok(solved) => {
                    polled.blocked = Some(Blocking::Clear);
                    active.latest = Some(Arc::clone(&solved));
                    polled.shown = Some((active.feature, solved));
                }
                Err(limits) => polled.blocked = Some(Blocking::By(limits)),
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
        if self.waits_for_finishing() {
            self.queue(feature, label, drags);
            return Polled::default();
        }
        self.move_active(feature, label, drags, start)
    }

    fn finish(&mut self, join: Option<Join>, start: &Start<'_>) -> Polled {
        let Some(active) = &mut self.active else {
            return Polled::default();
        };

        active.join = join;
        if active.revision != start.revision {
            self.drop_active();
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

    fn move_active(
        &mut self,
        feature: FeatureId,
        label: String,
        drags: Vec<Drag>,
        start: &Start<'_>,
    ) -> Polled {
        let continues = self
            .active
            .as_ref()
            .is_some_and(|active| active.feature == feature && active.revision == start.revision);

        if !continues {
            self.drop_active();

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
                join: None,
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
                    Ok((sketch, memo)) => {
                        active.latest = Some(Arc::clone(&sketch));
                        Some(Previous {
                            drag: job.drag,
                            sketch,
                            memo,
                        })
                    }
                    Err(_) => continued,
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
            revision: self.revision,
            sketch: self.latest,
            join: self.join,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        time::{Duration, Instant},
    };

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

        let immediately = dragging
            .perform(DragCommand::Finish { join: None }, &start)
            .finished;
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

        let stale = dragging.perform(
            DragCommand::Finish { join: None },
            &start(2, &parameters, &shown),
        );

        assert!(stale.finished.is_none());
        assert!(stale.abandoned);
        assert!(!dragging.is_dragging());

        dragging.perform(moved, &start(2, &parameters, &shown));
        dragging.perform(DragCommand::Cancel, &start(2, &parameters, &shown));

        assert!(!dragging.is_dragging());

        thread::sleep(Duration::from_millis(20));

        assert!(dragging.poll().finished.is_none());
    }

    fn moved(feature: FeatureId, label: &str, point: EntityId, to: Point2) -> DragCommand {
        DragCommand::Move {
            feature,
            label: label.to_owned(),
            drags: vec![Drag::Point { point, to }],
        }
    }

    #[test]
    fn a_drag_begun_while_the_last_is_finishing_commits_after_it_from_its_result() {
        let (sketch, _, end) = line_from_origin();
        let mut sketch = Sketch::clone(&sketch);

        let other = sketch.add_point(Point2::new(0.0, 20.0));
        let feature = feature();
        let parameters = ParameterValues::default();
        let committed = RefCell::new(Arc::new(sketch));
        let shown = |_| Some(Arc::clone(&committed.borrow()));
        let mut dragging = SketchDragging::default();

        dragging.hold_results(true);

        let before = start(1, &parameters, &shown);
        let first_to = Point2::new(12.0, 5.0);
        let second_to = Point2::new(-4.0, 22.0);

        dragging.perform(moved(feature, "Drag Line", end, first_to), &before);

        let first_released = dragging.perform(DragCommand::Finish { join: None }, &before);

        dragging.perform(moved(feature, "Drag Point", other, second_to), &before);

        let second_released = dragging.perform(DragCommand::Finish { join: None }, &before);

        assert!(first_released.finished.is_none());
        assert!(second_released.finished.is_none());
        assert!(!second_released.abandoned);
        assert!(dragging.poll().finished.is_none());

        dragging.hold_results(false);

        let first = until_finished(&mut dragging);
        let first_sketch = first.sketch.unwrap();

        assert_eq!(first.label, "Drag Line");
        assert_eq!(first.revision, 1);
        assert!(first_sketch.point(end).unwrap().distance(first_to) < 1e-9);
        assert!(dragging.poll().finished.is_none());

        *committed.borrow_mut() = first_sketch;

        let after = start(2, &parameters, &shown);
        let resumed = dragging.resume(&after);
        let second = resumed
            .finished
            .unwrap_or_else(|| until_finished(&mut dragging));
        let second_sketch = second.sketch.unwrap();

        assert_eq!(second.label, "Drag Point");
        assert_eq!(second.revision, 2);
        assert!(second_sketch.point(end).unwrap().distance(first_to) < 1e-9);
        assert!(second_sketch.point(other).unwrap().distance(second_to) < 1e-9);
        assert!(!dragging.is_dragging());
        assert!(dragging.resume(&after).finished.is_none());
    }

    #[test]
    fn cancelling_while_the_last_drag_finishes_drops_only_the_held_one() {
        let (sketch, _, end) = line_from_origin();
        let feature = feature();
        let parameters = ParameterValues::default();
        let shown = move |_| Some(Arc::clone(&sketch));
        let mut dragging = SketchDragging::default();

        dragging.hold_results(true);

        let start = start(1, &parameters, &shown);
        let first_to = Point2::new(12.0, 5.0);

        dragging.perform(moved(feature, "Drag Line", end, first_to), &start);
        dragging.perform(DragCommand::Finish { join: None }, &start);
        dragging.perform(
            moved(feature, "Drag Again", end, Point2::new(3.0, 3.0)),
            &start,
        );

        let cancelled = dragging.perform(DragCommand::Cancel, &start);

        assert!(!cancelled.abandoned);
        assert!(!dragging.cancel_held());

        dragging.hold_results(false);

        let first = until_finished(&mut dragging);

        assert_eq!(first.label, "Drag Line");
        assert!(first.sketch.unwrap().point(end).unwrap().distance(first_to) < 1e-9);
        assert!(dragging.resume(&start).finished.is_none());
        assert!(!dragging.is_dragging());
    }

    #[test]
    fn a_failed_solve_names_the_constraints_that_limit_it() {
        let limit = ConstraintId::from_raw(3);
        let other = ConstraintId::from_raw(5);

        assert_eq!(
            limiting(&SketchError::Conflict {
                constraints: vec![limit, other]
            }),
            vec![limit, other]
        );
        assert_eq!(
            limiting(&SketchError::Unsolvable {
                entities: Vec::new(),
                newest: Some(other)
            }),
            vec![other]
        );
        assert_eq!(
            limiting(&SketchError::Several(vec![
                SketchError::Conflict {
                    constraints: vec![limit]
                },
                SketchError::Cancelled,
            ])),
            vec![limit]
        );
        assert!(limiting(&SketchError::Cancelled).is_empty());
    }
}
