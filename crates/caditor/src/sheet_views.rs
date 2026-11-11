use std::{
    collections::BTreeMap,
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

use caditor_document::{
    DetailView, Drawing, DrawingId, DrawingViewError, DrawnBodies, DrawnView, FeatureId,
    FeatureResult, ParameterValues, ResolvedPlace, SectionView, StandardView, ViewLines,
    detail_view, drawing_view, drawn_bodies, section_view,
};
use caditor_geometry::Plane;
use caditor_kernel::interruptible;

use crate::model::{Model, Waker};

pub type ViewOutcome = Arc<Result<ViewLines, DrawingViewError>>;

#[derive(Debug, Clone)]
enum ParentLines {
    Done(ViewOutcome),
    NotDrawn,
}

impl PartialEq for ParentLines {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Done(one), Self::Done(other)) => Arc::ptr_eq(one, other),
            (Self::NotDrawn, Self::NotDrawn) => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum ViewJob {
    Standard(StandardView),
    Section {
        section: SectionView,
        place: Result<ResolvedPlace, DrawingViewError>,
    },
    Detail {
        detail: DetailView,
        parent: ParentLines,
    },
}

impl ViewJob {
    fn view(&self) -> DrawnView {
        match self {
            Self::Standard(view) => DrawnView::Standard(*view),
            Self::Section { section, .. } => DrawnView::Section(section.letter),
            Self::Detail { detail, .. } => DrawnView::Detail(detail.letter),
        }
    }

    fn is_detail(&self) -> bool {
        matches!(self, Self::Detail { .. })
    }
}

fn enlarged(detail: &DetailView, parent: &ParentLines) -> Result<ViewLines, DrawingViewError> {
    match parent {
        ParentLines::Done(outcome) => match outcome.as_ref() {
            Ok(lines) => detail_view(detail, lines),
            Err(_) => Err(DrawingViewError::ParentFailed),
        },
        ParentLines::NotDrawn => Err(DrawingViewError::ParentNotDrawn),
    }
}

#[derive(Debug, Clone)]
struct ViewKey {
    job: ViewJob,
    bodies: Vec<(FeatureId, Arc<FeatureResult>)>,
}

impl PartialEq for ViewKey {
    fn eq(&self, other: &Self) -> bool {
        self.job == other.job
            && self.bodies.len() == other.bodies.len()
            && self
                .bodies
                .iter()
                .zip(&other.bodies)
                .all(|((one, first), (other, second))| one == other && Arc::ptr_eq(first, second))
    }
}

struct Job {
    ticket: u64,
    key: ViewKey,
}

struct Finished {
    key: ViewKey,
    outcome: Result<ViewLines, DrawingViewError>,
}

struct Worker {
    jobs: Sender<Job>,
    done: Receiver<Finished>,
}

fn work_out(key: &ViewKey) -> Result<ViewLines, DrawingViewError> {
    let solids: Vec<_> = key
        .bodies
        .iter()
        .filter_map(|(_, result)| result.solid())
        .map(|result| &result.solid)
        .collect();
    panic::catch_unwind(AssertUnwindSafe(|| match &key.job {
        ViewJob::Standard(view) => drawing_view(*view, &solids),
        ViewJob::Section { section, place } => match place {
            Ok(place) => section_view(section, *place, &solids),
            Err(error) => Err(error.clone()),
        },
        ViewJob::Detail { detail, parent } => enlarged(detail, parent),
    }))
    .unwrap_or_else(|_| {
        log::error!("working out a drawing view panicked");
        Err(DrawingViewError::Panicked)
    })
}

impl Worker {
    fn spawn(wake: Waker, current: Arc<AtomicU64>) -> Option<Self> {
        let (jobs, queue) = mpsc::channel::<Job>();
        let (sender, done) = mpsc::channel();
        let spawned = thread::Builder::new()
            .name("drawing views".to_owned())
            .spawn(move || {
                while let Ok(job) = queue.recv() {
                    if current.load(Ordering::SeqCst) != job.ticket {
                        continue;
                    }
                    let watched = Arc::clone(&current);
                    let ticket = job.ticket;
                    let interrupt = Arc::new(move || watched.load(Ordering::SeqCst) != ticket);
                    let outcome = interruptible(interrupt, || work_out(&job.key));
                    if sender
                        .send(Finished {
                            key: job.key,
                            outcome,
                        })
                        .is_err()
                    {
                        break;
                    }
                    wake();
                }
            });
        match spawned {
            Ok(_) => Some(Self { jobs, done }),
            Err(error) => {
                log::error!("could not start the drawing views worker: {error}");
                None
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum ViewState {
    Working,
    Done(ViewOutcome),
}

#[derive(Debug, Clone, Default)]
pub struct SheetViews {
    pub drawing: Option<DrawingId>,
    pub views: BTreeMap<DrawnView, ViewState>,
    pub missing: Vec<FeatureId>,
}

impl SheetViews {
    pub fn working(&self) -> usize {
        self.views
            .values()
            .filter(|state| matches!(state, ViewState::Working))
            .count()
    }

    pub fn finished(&self) -> impl Iterator<Item = (DrawnView, &ViewLines)> {
        self.views.iter().filter_map(|(view, state)| match state {
            ViewState::Done(outcome) => outcome.as_ref().as_ref().ok().map(|lines| (*view, lines)),
            ViewState::Working => None,
        })
    }

    pub fn failed(&self) -> impl Iterator<Item = (DrawnView, &DrawingViewError)> {
        self.views.iter().filter_map(|(view, state)| match state {
            ViewState::Done(outcome) => outcome.as_ref().as_ref().err().map(|error| (*view, error)),
            ViewState::Working => None,
        })
    }

    pub fn cuts(&self) -> BTreeMap<char, Plane> {
        self.finished()
            .filter_map(|(view, lines)| match (view, &lines.section) {
                (DrawnView::Section(letter), Some(section)) => Some((letter, section.cutting)),
                _ => None,
            })
            .collect()
    }

    pub fn outcome(&self, view: DrawnView) -> Option<ViewOutcome> {
        match self.views.get(&view)? {
            ViewState::Done(outcome) => Some(Arc::clone(outcome)),
            ViewState::Working => None,
        }
    }

    pub fn is_ready(&self) -> bool {
        self.working() == 0
    }
}

#[derive(Default)]
pub struct DrawingViews {
    worker: Option<Worker>,
    current: Arc<AtomicU64>,
    ticket: u64,
    wanted: Vec<ViewKey>,
    in_flight: Vec<ViewKey>,
    done: Vec<(ViewKey, ViewOutcome)>,
}

fn keys_of(drawing: &Drawing, bodies: &DrawnBodies, values: &ParameterValues) -> Vec<ViewKey> {
    drawing
        .views
        .iter()
        .map(|view| ViewJob::Standard(*view))
        .chain(drawing.sections.iter().map(|section| ViewJob::Section {
            section: section.clone(),
            place: section.place.resolve(values),
        }))
        .map(|job| ViewKey {
            job,
            bodies: bodies.results.clone(),
        })
        .collect()
}

impl DrawingViews {
    pub fn refresh(&mut self, model: &Model, drawing: &Drawing) -> SheetViews {
        self.poll();
        let bodies = drawn_bodies(model.document(), model.evaluation(), drawing);
        let mut wanted = keys_of(drawing, &bodies, model.parameters());
        let waiting = self.add_details(drawing, &bodies, &mut wanted);
        if wanted != self.wanted {
            if self.in_flight.iter().any(|key| !wanted.contains(key)) {
                self.ticket = self.ticket.wrapping_add(1);
                self.current.store(self.ticket, Ordering::SeqCst);
                self.in_flight.clear();
            }
            let same_bodies = |key: &ViewKey| {
                wanted.iter().any(|kept| {
                    ViewKey {
                        job: key.job.clone(),
                        bodies: kept.bodies.clone(),
                    } == *key
                })
            };
            self.done.retain(|(key, _)| {
                same_bodies(key) && (!key.job.is_detail() || wanted.contains(key))
            });
            self.wanted = wanted;
        }
        let missing: Vec<ViewKey> = self
            .wanted
            .iter()
            .filter(|key| !self.done.iter().any(|(done, _)| done == *key))
            .filter(|key| !self.in_flight.contains(key))
            .cloned()
            .collect();
        for key in missing {
            self.submit(model, key);
        }
        let views = self
            .wanted
            .iter()
            .map(|key| {
                let state = self
                    .done
                    .iter()
                    .find(|(done, _)| done == key)
                    .map_or(ViewState::Working, |(_, outcome)| {
                        ViewState::Done(Arc::clone(outcome))
                    });
                (key.job.view(), state)
            })
            .chain(waiting.into_iter().map(|view| (view, ViewState::Working)))
            .collect();
        SheetViews {
            drawing: Some(drawing.id),
            views,
            missing: bodies.missing,
        }
    }

    fn add_details(
        &self,
        drawing: &Drawing,
        bodies: &DrawnBodies,
        wanted: &mut Vec<ViewKey>,
    ) -> Vec<DrawnView> {
        let mut waiting = Vec::new();
        let mut details = Vec::new();
        for detail in &drawing.details {
            let parent = detail.parent.drawn();
            let Some(parent_key) = wanted.iter().find(|key| key.job.view() == parent) else {
                details.push((detail, ParentLines::NotDrawn));
                continue;
            };
            match self.done.iter().find(|(done, _)| done == parent_key) {
                Some((_, outcome)) => {
                    details.push((detail, ParentLines::Done(Arc::clone(outcome))));
                }
                None => waiting.push(DrawnView::Detail(detail.letter)),
            }
        }
        wanted.extend(details.into_iter().map(|(detail, parent)| ViewKey {
            job: ViewJob::Detail {
                detail: *detail,
                parent,
            },
            bodies: bodies.results.clone(),
        }));
        waiting
    }

    fn submit(&mut self, model: &Model, key: ViewKey) {
        if self.worker.is_none() {
            self.worker = Worker::spawn(model.waker(), Arc::clone(&self.current));
        }
        let job = Job {
            ticket: self.ticket,
            key: key.clone(),
        };
        let refused = match &self.worker {
            Some(worker) => worker.jobs.send(job).err().map(|refused| refused.0),
            None => Some(job),
        };
        match refused {
            None => self.in_flight.push(key),
            Some(job) => {
                log::error!("no drawing views worker, so a view is worked out on the UI thread");
                self.worker = None;
                let outcome = work_out(&job.key);
                self.arrive(Finished {
                    key: job.key,
                    outcome,
                });
            }
        }
    }

    fn arrive(&mut self, finished: Finished) {
        self.in_flight.retain(|key| *key != finished.key);
        if matches!(finished.outcome, Err(DrawingViewError::Cancelled)) {
            return;
        }
        if self.wanted.contains(&finished.key) {
            self.done.push((finished.key, Arc::new(finished.outcome)));
        }
    }

    fn poll(&mut self) {
        let Some(worker) = &self.worker else {
            return;
        };
        let arrived: Vec<Finished> = worker.done.try_iter().collect();
        for finished in arrived {
            self.arrive(finished);
        }
    }

    pub fn forget(&mut self) {
        self.ticket = self.ticket.wrapping_add(1);
        self.current.store(self.ticket, Ordering::SeqCst);
        self.wanted.clear();
        self.in_flight.clear();
        self.done.clear();
    }
}
