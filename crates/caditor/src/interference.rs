use std::{
    collections::{BTreeMap, BTreeSet},
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

use caditor_document::{FeatureId, FeatureResult};
use caditor_geometry::{Aabb, Point3};
use caditor_kernel::{
    Interference as Contact, LINEAR_RESOLUTION, MeshQuality, interference, interruptible,
};

use crate::{
    bodies::BodyMass,
    model::{Model, Waker},
    selection::{Pickable, Selection},
    visibility,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pair {
    pub first: FeatureId,
    pub second: FeatureId,
}

impl Pair {
    fn of(one: FeatureId, other: FeatureId) -> Self {
        Self {
            first: one.min(other),
            second: one.max(other),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Overlap {
    pub mass: Option<BodyMass>,
    pub place: Point3,
    pub outline: Arc<Vec<Vec<Point3>>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Finding {
    Apart,
    Touching(Point3),
    Overlapping(Overlap),
    Unchecked(Option<Point3>),
}

impl Finding {
    pub fn place(&self) -> Option<Point3> {
        match self {
            Self::Apart => None,
            Self::Touching(place) => Some(*place),
            Self::Overlapping(overlap) => Some(overlap.place),
            Self::Unchecked(place) => *place,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Everything,
    Chosen,
    AgainstTheRest(FeatureId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bodies {
    pub scope: Scope,
    pub pairs: Vec<Pair>,
    pub count: usize,
}

impl Bodies {
    pub fn of(model: &Model, selection: &Selection, tree_selected: Option<FeatureId>) -> Self {
        let evaluation = model.evaluation();
        let document = model.document();
        let shown: Vec<FeatureId> = evaluation
            .bodies()
            .map(|(body, _)| body)
            .filter(|body| visibility::is_shown(document, *body))
            .filter(|body| bodies_solid(model, *body).is_some())
            .collect();
        let chosen: BTreeSet<FeatureId> = selection
            .iter()
            .filter_map(|pickable| match pickable {
                Pickable::Face { body, .. }
                | Pickable::Edge { body, .. }
                | Pickable::Vertex { body, .. } => Some(body),
                _ => None,
            })
            .chain(tree_selected)
            .filter(|body| shown.contains(body))
            .collect();
        Self::among(&shown, &chosen)
    }

    fn among(shown: &[FeatureId], chosen: &BTreeSet<FeatureId>) -> Self {
        let every_pair = |bodies: &[FeatureId]| -> Vec<Pair> {
            bodies
                .iter()
                .enumerate()
                .flat_map(|(index, one)| {
                    bodies
                        .iter()
                        .skip(index + 1)
                        .map(move |other| Pair::of(*one, *other))
                })
                .collect()
        };
        match chosen.iter().copied().collect::<Vec<_>>().as_slice() {
            [] => Self {
                scope: Scope::Everything,
                pairs: every_pair(shown),
                count: shown.len(),
            },
            [one] => Self {
                scope: Scope::AgainstTheRest(*one),
                pairs: shown
                    .iter()
                    .filter(|other| *other != one)
                    .map(|other| Pair::of(*one, *other))
                    .collect(),
                count: shown.len(),
            },
            several => Self {
                scope: Scope::Chosen,
                pairs: every_pair(several),
                count: several.len(),
            },
        }
    }
}

fn bodies_solid(model: &Model, body: FeatureId) -> Option<Arc<FeatureResult>> {
    let result = model.evaluation().body_result(body)?;
    result.solid()?;
    Some(Arc::clone(result))
}

fn check(first: &FeatureResult, second: &FeatureResult, quality: &MeshQuality) -> Finding {
    let (Some(first), Some(second)) = (first.solid(), second.solid()) else {
        return Finding::Apart;
    };
    if let (Some(one), Some(other)) = (first.bounding_box(), second.bounding_box())
        && apart(&one, &other)
    {
        return Finding::Apart;
    }
    match interference(&first.solid, &second.solid) {
        Ok(Contact::Apart) => Finding::Apart,
        Ok(Contact::Touching(place)) => Finding::Touching(place),
        Ok(Contact::Overlapping(solid)) => {
            let mesh = solid.display_mesh(quality).ok();
            let mass = mesh.as_ref().map(|mesh| BodyMass::of(&solid, mesh));
            let place = mass
                .as_ref()
                .map(|mass| mass.properties.centroid)
                .or_else(|| solid.bounding_box().map(|bounds| bounds.center()))
                .unwrap_or(Point3::ZERO);
            let outline = mesh
                .map(|mesh| {
                    mesh.edges()
                        .iter()
                        .map(|edge| {
                            edge.positions
                                .iter()
                                .filter_map(|position| mesh.position(*position))
                                .collect()
                        })
                        .collect()
                })
                .unwrap_or_default();
            Finding::Overlapping(Overlap {
                mass,
                place,
                outline: Arc::new(outline),
            })
        }
        Err(error) => {
            log::warn!("checking interference failed: {error}");
            Finding::Unchecked(error.site().and_then(|site| site.point))
        }
    }
}

fn apart(one: &Aabb, other: &Aabb) -> bool {
    let low = one.min().max(other.min());
    let high = one.max().min(other.max());
    (high - low).min_element() < -10.0 * LINEAR_RESOLUTION
}

fn contained_check(
    first: &FeatureResult,
    second: &FeatureResult,
    quality: &MeshQuality,
) -> Finding {
    panic::catch_unwind(AssertUnwindSafe(|| check(first, second, quality))).unwrap_or_else(|_| {
        log::error!("checking interference panicked");
        Finding::Unchecked(None)
    })
}

struct Task {
    pair: Pair,
    first: Arc<FeatureResult>,
    second: Arc<FeatureResult>,
}

struct Job {
    ticket: u64,
    tasks: Vec<Task>,
    quality: MeshQuality,
}

enum Message {
    Found {
        ticket: u64,
        pair: Pair,
        finding: Finding,
    },
    Stopped {
        ticket: u64,
        pair: Pair,
    },
}

struct Worker {
    jobs: Sender<Job>,
    done: Receiver<Message>,
}

fn run_job(job: &Job, current: &AtomicU64, send: &mut dyn FnMut(Message) -> bool) {
    let ticket = job.ticket;
    for task in &job.tasks {
        if current.load(Ordering::SeqCst) != ticket {
            return;
        }
        let finding = contained_check(&task.first, &task.second, &job.quality);
        let message = if current.load(Ordering::SeqCst) == ticket {
            Message::Found {
                ticket,
                pair: task.pair,
                finding,
            }
        } else {
            Message::Stopped {
                ticket,
                pair: task.pair,
            }
        };
        if !send(message) {
            return;
        }
    }
}

impl Worker {
    fn spawn(wake: Waker, current: Arc<AtomicU64>) -> Option<Self> {
        let (jobs, queue) = mpsc::channel::<Job>();
        let (sender, done) = mpsc::channel();
        let spawned = thread::Builder::new()
            .name("interference".to_owned())
            .spawn(move || {
                while let Ok(mut job) = queue.recv() {
                    while let Ok(newer) = queue.try_recv() {
                        job = newer;
                    }
                    let ticket = job.ticket;
                    let watched = Arc::clone(&current);
                    let interrupt = Arc::new(move || watched.load(Ordering::SeqCst) != ticket);
                    let mut alive = true;
                    interruptible(interrupt, || {
                        run_job(&job, &current, &mut |message| {
                            alive = sender.send(message).is_ok();
                            wake();
                            alive
                        });
                    });
                    if !alive {
                        break;
                    }
                }
            });
        match spawned {
            Ok(_) => Some(Self { jobs, done }),
            Err(error) => {
                log::error!("could not start the interference worker: {error}");
                None
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Basis {
    bodies: Bodies,
    revision: u64,
    evaluation: u64,
    quality: MeshQuality,
}

struct Checked {
    results: [Arc<FeatureResult>; 2],
    finding: Finding,
}

impl Checked {
    fn holds(&self, first: &Arc<FeatureResult>, second: &Arc<FeatureResult>) -> bool {
        let [one, other] = &self.results;
        Arc::ptr_eq(one, first) && Arc::ptr_eq(other, second)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub bodies: Bodies,
    pub found: Vec<(Pair, Finding)>,
    pub pending: usize,
}

impl Report {
    pub fn contacts(&self) -> impl Iterator<Item = &(Pair, Finding)> {
        self.found
            .iter()
            .filter(|(_, finding)| *finding != Finding::Apart)
    }

    pub fn is_checking(&self) -> bool {
        self.pending > 0
    }

    pub fn checked(&self) -> usize {
        self.found.len()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Inputs {
    selection: u64,
    tree_selected: Option<FeatureId>,
    revision: u64,
    evaluation: u64,
}

#[derive(Default)]
pub struct Interference {
    worker: Option<Worker>,
    current: Arc<AtomicU64>,
    inputs: Option<Inputs>,
    basis: Option<Basis>,
    ticket: u64,
    submitted: BTreeMap<Pair, [Arc<FeatureResult>; 2]>,
    checked: BTreeMap<Pair, Checked>,
    changed: bool,
}

impl Interference {
    pub fn refresh(
        &mut self,
        model: &Model,
        selection: &Selection,
        tree_selected: Option<FeatureId>,
    ) -> Option<Report> {
        let inputs = Inputs {
            selection: selection.generation(),
            tree_selected,
            revision: model.revision(),
            evaluation: model.evaluation_generation(),
        };
        let quality = model.mesh_quality();
        let same_quality = self
            .basis
            .as_ref()
            .is_some_and(|basis| basis.quality == quality);
        if self.inputs != Some(inputs) || !same_quality {
            self.inputs = Some(inputs);
            let basis = Basis {
                bodies: Bodies::of(model, selection, tree_selected),
                revision: inputs.revision,
                evaluation: inputs.evaluation,
                quality,
            };
            if self.basis.as_ref() != Some(&basis) {
                if !same_quality {
                    self.checked.clear();
                }
                self.start(model, &basis);
                self.basis = Some(basis);
                self.changed = true;
            }
        }
        self.poll();
        std::mem::take(&mut self.changed).then(|| self.report(model))
    }

    fn start(&mut self, model: &Model, basis: &Basis) {
        self.ticket = self.ticket.wrapping_add(1);
        self.current.store(self.ticket, Ordering::SeqCst);
        self.submitted.clear();
        let results: BTreeMap<FeatureId, Arc<FeatureResult>> = model
            .evaluation()
            .bodies()
            .filter_map(|(body, _)| Some((body, bodies_solid(model, body)?)))
            .collect();
        self.checked.retain(|pair, checked| {
            match (results.get(&pair.first), results.get(&pair.second)) {
                (Some(first), Some(second)) => checked.holds(first, second),
                _ => false,
            }
        });
        let tasks: Vec<Task> = basis
            .bodies
            .pairs
            .iter()
            .filter(|pair| !self.checked.contains_key(pair))
            .filter_map(|pair| {
                Some(Task {
                    pair: *pair,
                    first: Arc::clone(results.get(&pair.first)?),
                    second: Arc::clone(results.get(&pair.second)?),
                })
            })
            .collect();
        for task in &tasks {
            self.submitted.insert(
                task.pair,
                [Arc::clone(&task.first), Arc::clone(&task.second)],
            );
        }
        if !tasks.is_empty() {
            self.submit(
                model,
                Job {
                    ticket: self.ticket,
                    tasks,
                    quality: basis.quality,
                },
            );
        }
    }

    fn submit(&mut self, model: &Model, job: Job) {
        if self.worker.is_none() {
            self.worker = Worker::spawn(model.waker(), Arc::clone(&self.current));
        }
        let refused = match &self.worker {
            Some(worker) => worker.jobs.send(job).err().map(|refused| refused.0),
            None => Some(job),
        };
        if let Some(job) = refused {
            log::error!("no interference worker, so bodies are checked on the UI thread");
            self.worker = None;
            let mut arrived = Vec::new();
            run_job(&job, &self.current, &mut |message| {
                arrived.push(message);
                true
            });
            for message in arrived {
                self.arrive(message);
            }
        }
    }

    fn arrive(&mut self, message: Message) {
        match message {
            Message::Found {
                ticket,
                pair,
                finding,
            } if ticket == self.ticket => {
                if let Some(results) = self.submitted.remove(&pair) {
                    self.checked.insert(pair, Checked { results, finding });
                    self.changed = true;
                }
            }
            Message::Stopped { ticket, pair } if ticket == self.ticket => {
                self.changed |= self.submitted.remove(&pair).is_some();
            }
            Message::Found { .. } | Message::Stopped { .. } => {}
        }
    }

    fn poll(&mut self) {
        let Some(worker) = &self.worker else {
            return;
        };
        let arrived: Vec<Message> = worker.done.try_iter().collect();
        for message in arrived {
            self.arrive(message);
        }
    }

    fn report(&self, model: &Model) -> Report {
        let Some(basis) = &self.basis else {
            return Report {
                bodies: Bodies::among(&[], &BTreeSet::new()),
                found: Vec::new(),
                pending: 0,
            };
        };
        let evaluation = model.evaluation();
        let found = basis
            .bodies
            .pairs
            .iter()
            .filter_map(|pair| {
                let checked = self.checked.get(pair)?;
                let first = evaluation.body_result(pair.first)?;
                let second = evaluation.body_result(pair.second)?;
                checked
                    .holds(first, second)
                    .then(|| (*pair, checked.finding.clone()))
            })
            .collect();
        Report {
            bodies: basis.bodies.clone(),
            found,
            pending: self.submitted.len(),
        }
    }

    pub fn forget(&mut self) {
        self.ticket = self.ticket.wrapping_add(1);
        self.current.store(self.ticket, Ordering::SeqCst);
        self.inputs = None;
        self.basis = None;
        self.submitted.clear();
        self.checked.clear();
        self.changed = false;
    }
}

#[derive(Default)]
pub struct InterferenceTool {
    pub open: bool,
    pub interference: Interference,
    pub report: Option<Report>,
}

impl InterferenceTool {
    pub fn toggle(&mut self) {
        self.open = !self.open;
        if !self.open {
            self.interference.forget();
            self.report = None;
        }
    }
}
