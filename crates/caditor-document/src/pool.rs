use std::{
    collections::{BTreeMap, BTreeSet},
    num::NonZeroUsize,
    panic::{self, AssertUnwindSafe},
    sync::{Arc, OnceLock},
    thread,
};

use caditor_kernel::{MeshQuality, interruptible};
use parking_lot::{Condvar, Mutex, MutexGuard};
use rayon_core::{Scope, ThreadPool, ThreadPoolBuilder};

use crate::{
    document::{Feature, FeatureId},
    presenting::SettledBody,
    recompute::{CancelToken, Computed, Context, Failure, FeatureResult, View, compute},
};

pub(crate) fn available_workers() -> usize {
    thread::available_parallelism().map_or(1, NonZeroUsize::get)
}

#[derive(Debug)]
pub(crate) struct Threads {
    workers: usize,
    pool: OnceLock<Option<ThreadPool>>,
}

impl Threads {
    pub(crate) fn new(workers: usize) -> Self {
        Self {
            workers,
            pool: OnceLock::new(),
        }
    }

    fn pool(&self) -> Option<&ThreadPool> {
        if self.workers == 0 {
            return None;
        }
        self.pool
            .get_or_init(|| {
                ThreadPoolBuilder::new()
                    .num_threads(self.workers)
                    .thread_name(|_| "recompute worker".to_owned())
                    .build()
                    .inspect_err(|error| {
                        log::warn!(
                            "could not start threads to recompute on, so features compute one at \
                             a time: {error}"
                        );
                    })
                    .ok()
            })
            .as_ref()
    }

    pub(crate) fn run<'scope, R>(
        &self,
        work: &'scope Work<'scope>,
        body: impl FnOnce(&Pool<'_, 'scope>) -> R,
    ) -> R {
        let Some(threads) = self.pool() else {
            return body(&Pool { scope: None, work });
        };
        work.queue.lock().limit = threads.current_num_threads();
        threads.in_place_scope(|scope| {
            body(&Pool {
                scope: Some(scope),
                work,
            })
        })
    }
}

#[derive(Debug, Default)]
pub(crate) struct LastMeshes(Mutex<BTreeMap<FeatureId, Arc<FeatureResult>>>);

impl LastMeshes {
    fn of(&self, body: FeatureId) -> Option<Arc<FeatureResult>> {
        self.0.lock().get(&body).cloned()
    }

    fn meshed(&self, body: FeatureId, result: &Arc<FeatureResult>) {
        self.0.lock().insert(body, Arc::clone(result));
    }

    pub(crate) fn retain(&self, alive: impl Fn(FeatureId) -> bool) {
        self.0.lock().retain(|body, _| alive(*body));
    }

    pub(crate) fn clear(&self) {
        self.0.lock().clear();
    }
}

pub(crate) struct Job {
    pub(crate) feature: Arc<Feature>,
    pub(crate) view: View,
    pub(crate) previous: Option<Arc<FeatureResult>>,
}

pub(crate) type Meshed = Box<dyn FnOnce(bool) + Send>;

struct Meshing {
    body: SettledBody,
    running: bool,
    meshed: Vec<Meshed>,
}

pub(crate) enum Landed {
    Stands(Arc<FeatureResult>),
    Falls,
    Stopped,
    Lost,
}

pub(crate) enum Claim {
    Finished(View, Computed),
    Pending,
    Absent,
}

enum Task {
    Evaluate(usize, Job),
    Mesh(SettledBody),
}

#[derive(Default)]
struct Queue {
    queued: BTreeMap<usize, Job>,
    running: BTreeSet<usize>,
    finished: BTreeMap<usize, (View, Computed)>,
    landed: Vec<(usize, Landed)>,
    meshes: Vec<Meshing>,
    idle: usize,
    started: usize,
    limit: usize,
    closed: bool,
}

impl Queue {
    fn waiting(&self) -> usize {
        self.queued.len()
            + self
                .meshes
                .iter()
                .filter(|meshing| !meshing.running)
                .count()
    }

    fn take(&mut self) -> Option<Task> {
        if let Some((index, job)) = self.queued.pop_first() {
            self.running.insert(index);
            return Some(Task::Evaluate(index, job));
        }
        let meshing = self.meshes.iter_mut().find(|meshing| !meshing.running)?;
        meshing.running = true;
        Some(Task::Mesh(meshing.body.clone()))
    }
}

pub(crate) struct Work<'a> {
    context: Context<'a>,
    quality: MeshQuality,
    last_meshes: Arc<LastMeshes>,
    queue: Mutex<Queue>,
    ready: Condvar,
    finished: Condvar,
}

impl<'a> Work<'a> {
    pub(crate) fn new(
        context: Context<'a>,
        quality: MeshQuality,
        last_meshes: Arc<LastMeshes>,
    ) -> Self {
        Self {
            context,
            quality,
            last_meshes,
            queue: Mutex::new(Queue::default()),
            ready: Condvar::new(),
            finished: Condvar::new(),
        }
    }

    fn serve(&self) {
        let mut queue = self.queue.lock();
        loop {
            if queue.closed {
                return;
            }
            if let Some(task) = queue.take() {
                MutexGuard::unlocked(&mut queue, || self.run(task));
                continue;
            }
            queue.idle += 1;
            self.ready.wait(&mut queue);
            queue.idle -= 1;
        }
    }

    fn run(&self, task: Task) {
        match task {
            Task::Evaluate(index, job) => self.evaluate(index, job),
            Task::Mesh(body) => self.mesh(&body),
        }
    }

    fn evaluate(&self, index: usize, job: Job) {
        let computed = if self.context.cancel.is_cancelled() {
            Some(Err(Failure::Cancelled))
        } else {
            panic::catch_unwind(AssertUnwindSafe(|| compute(&self.context, &job)))
                .inspect_err(|_| {
                    log::error!(
                        "recomputing {} on a worker thread panicked outside its evaluator",
                        job.feature.name
                    );
                })
                .ok()
        };
        let landed = match &computed {
            Some(Ok((result, _))) => Landed::Stands(Arc::clone(result)),
            Some(Err(Failure::Error(_))) => Landed::Falls,
            Some(Err(Failure::Cancelled)) => Landed::Stopped,
            None => Landed::Lost,
        };
        let mut queue = self.queue.lock();
        queue.running.remove(&index);
        queue.landed.push((index, landed));
        if let Some(computed) = computed {
            queue.finished.insert(index, (job.view, computed));
        }
        drop(queue);
        self.finished.notify_all();
    }

    fn mesh(&self, body: &SettledBody) {
        let meshed = !self.context.cancel.is_cancelled() && self.tessellate(body);
        let callbacks = {
            let mut queue = self.queue.lock();
            let at = queue
                .meshes
                .iter()
                .position(|meshing| Arc::ptr_eq(&meshing.body.result, &body.result));
            at.map(|at| queue.meshes.remove(at).meshed)
                .unwrap_or_default()
        };
        for callback in callbacks {
            callback(meshed);
        }
    }

    fn tessellate(&self, body: &SettledBody) -> bool {
        let Some(solid) = body.result.solid() else {
            return false;
        };
        if solid.is_meshed() {
            return false;
        }
        let earlier = self.last_meshes.of(solid.body);
        interruptible(self.context.cancel.interrupt(), || {
            solid.tessellate(
                &body.name,
                &self.quality,
                earlier.as_deref().and_then(FeatureResult::solid),
            );
        });
        if solid.mesh().is_some() {
            self.last_meshes.meshed(solid.body, &body.result);
        }
        solid.is_meshed()
    }
}

pub(crate) struct Pool<'a, 'scope> {
    scope: Option<&'a Scope<'scope>>,
    work: &'scope Work<'scope>,
}

impl<'scope> Pool<'_, 'scope> {
    pub(crate) fn cancel(&self) -> &CancelToken {
        self.work.context.cancel
    }

    pub(crate) fn closing(&self) -> Closing<'_> {
        Closing(self.work)
    }

    pub(crate) fn queue(&self, index: usize, job: Job) {
        let mut queue = self.work.queue.lock();
        if queue.limit == 0 || queue.closed {
            return;
        }
        queue.queued.insert(index, job);
        self.wake(queue);
    }

    pub(crate) fn mesh(&self, body: SettledBody, meshed: Meshed) {
        let mut queue = self.work.queue.lock();
        if queue.closed {
            return;
        }
        if let Some(meshing) = queue
            .meshes
            .iter_mut()
            .find(|meshing| Arc::ptr_eq(&meshing.body.result, &body.result))
        {
            meshing.meshed.push(meshed);
            return;
        }
        queue.meshes.push(Meshing {
            body,
            running: false,
            meshed: vec![meshed],
        });
        self.wake(queue);
    }

    pub(crate) fn parallel(&self) -> bool {
        self.work.queue.lock().limit > 0
    }

    pub(crate) fn landed(&self) -> Vec<(usize, Landed)> {
        std::mem::take(&mut self.work.queue.lock().landed)
    }

    pub(crate) fn claim(&self, index: usize) -> Claim {
        let mut queue = self.work.queue.lock();
        if queue.queued.contains_key(&index) {
            if queue.started > 0 {
                return Claim::Pending;
            }
            queue.queued.remove(&index);
            return Claim::Absent;
        }
        if queue.running.contains(&index) {
            return Claim::Pending;
        }
        match queue.finished.remove(&index) {
            Some((view, computed)) => Claim::Finished(view, computed),
            None => Claim::Absent,
        }
    }

    pub(crate) fn wait(&self) {
        let mut queue = self.work.queue.lock();
        while queue.landed.is_empty() {
            self.work.finished.wait(&mut queue);
        }
    }

    pub(crate) fn help(&self) -> bool {
        let task = self.work.queue.lock().take();
        let Some(task) = task else {
            return false;
        };
        self.work.run(task);
        true
    }

    pub(crate) fn forget_features(&self) {
        let mut queue = self.work.queue.lock();
        queue.queued.clear();
        queue.finished.clear();
        queue.landed.clear();
    }

    fn wake(&self, mut queue: MutexGuard<'_, Queue>) {
        let spawn = queue.waiting() > queue.idle && queue.started < queue.limit;
        if spawn {
            queue.started += 1;
        }
        drop(queue);
        self.work.ready.notify_one();
        if !spawn {
            return;
        }
        if let Some(scope) = self.scope.filter(|_| spawn) {
            let work = self.work;
            scope.spawn(move |_| work.serve());
        }
    }
}

pub(crate) struct Closing<'a>(&'a Work<'a>);

impl Drop for Closing<'_> {
    fn drop(&mut self) {
        let mut queue = self.0.queue.lock();
        queue.closed = true;
        queue.queued.clear();
        queue.finished.clear();
        queue.meshes.retain(|meshing| meshing.running);
        drop(queue);
        self.0.ready.notify_all();
    }
}
