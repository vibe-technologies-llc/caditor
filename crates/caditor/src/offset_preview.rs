#[cfg(test)]
use std::time::Duration;
use std::{
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
};

use caditor_geometry::Point2;
use caditor_sketch::{Caps, Chain, Faceting, OffsetError};

#[cfg(test)]
use crate::bodies::Hold;
use crate::{model::Waker, offsetting::Placement};

#[cfg(test)]
const PATIENCE: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq)]
pub struct OffsetRequest {
    pub chains: Vec<Chain>,
    pub caps: Option<Caps>,
    pub pointer: Option<Point2>,
    pub typed: Option<f64>,
    pub faceting: Faceting,
}

impl OffsetRequest {
    fn offsets_alike(&self, other: &Self) -> bool {
        self.caps == other.caps && self.chains == other.chains
    }

    fn previewed(&self, cancelled: &dyn Fn() -> bool) -> Option<Previewed> {
        let placement = Placement::of(&self.chains, self.pointer, self.typed)?;
        let mut curves = Vec::new();

        for chain in &self.chains {
            if cancelled() {
                return None;
            }

            let made = match self.caps {
                Some(caps) => chain.both_sides(placement.distance, caps),
                None => chain
                    .outline(placement.side_of(chain), placement.distance)
                    .map(|outline| vec![outline]),
            };
            match made {
                Ok(made) => curves.extend(
                    made.iter()
                        .flat_map(|outline| outline.faceted(self.faceting)),
                ),
                Err(error) => return Some(Err(error)),
            }
        }
        Some(Ok(Offsets {
            distance: placement.distance,
            curves,
        }))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Offsets {
    pub distance: f64,
    pub curves: Vec<Vec<Point2>>,
}

pub type Previewed = Result<Offsets, OffsetError>;

#[derive(Debug)]
struct Running {
    request: OffsetRequest,
    cancelled: Arc<AtomicBool>,
    answer: Receiver<Previewed>,
}

#[derive(Debug, Default)]
pub struct OffsetPreview {
    running: Option<Running>,
    shown: Option<(OffsetRequest, Option<Previewed>)>,
    #[cfg(test)]
    hold: Option<Hold>,
}

impl OffsetPreview {
    pub fn refresh(&mut self, request: Option<&OffsetRequest>, wake: impl FnOnce() -> Waker) {
        self.poll();

        let Some(request) = request else {
            self.cancel();
            return;
        };

        if self
            .shown
            .as_ref()
            .is_some_and(|(shown, _)| shown == request)
        {
            return;
        }
        match &self.running {
            Some(running) if running.request.offsets_alike(request) => {}
            Some(_) | None => {
                self.cancel();
                self.start(request.clone(), wake());
            }
        }
    }

    pub fn shown_for(&self, request: Option<&OffsetRequest>) -> Option<&Previewed> {
        let request = request?;
        self.shown
            .as_ref()
            .filter(|(shown, _)| shown.offsets_alike(request))
            .and_then(|(_, previewed)| previewed.as_ref())
    }

    pub fn is_working(&self) -> bool {
        self.running.is_some()
    }

    fn poll(&mut self) {
        let Some(running) = &self.running else {
            return;
        };
        let previewed = match running.answer.try_recv() {
            Ok(previewed) => Some(previewed),
            Err(TryRecvError::Disconnected) => None,
            Err(TryRecvError::Empty) => return,
        };
        if let Some(running) = self.running.take() {
            self.shown = Some((running.request, previewed));
        }
    }

    fn cancel(&mut self) {
        if let Some(running) = self.running.take() {
            running.cancelled.store(true, Ordering::Relaxed);
        }
    }

    fn start(&mut self, request: OffsetRequest, wake: Waker) {
        let cancelled = Arc::new(AtomicBool::new(false));
        let (sender, answer) = mpsc::channel();
        let job = Job {
            request: request.clone(),
            cancelled: Arc::clone(&cancelled),
            #[cfg(test)]
            hold: self.hold.clone(),
        };

        let spawned = thread::Builder::new()
            .name("offset preview".to_owned())
            .spawn(move || {
                let previewed =
                    panic::catch_unwind(AssertUnwindSafe(|| job.run())).unwrap_or_else(|_| {
                        log::error!("previewing an offset panicked");
                        None
                    });
                if let Some(previewed) = previewed
                    && sender.send(previewed).is_ok()
                {
                    wake();
                }
            });

        match spawned {
            Ok(_) => {
                self.running = Some(Running {
                    request,
                    cancelled,
                    answer,
                });
            }
            Err(error) => {
                log::warn!("could not start previewing an offset: {error}");
                self.shown = Some((request, None));
            }
        }
    }

    #[cfg(test)]
    pub fn hold(&mut self, hold: Hold) {
        self.hold = Some(hold);
    }

    #[cfg(test)]
    pub fn release(&mut self) {
        if let Some(hold) = self.hold.take() {
            hold.release();
        }
    }

    #[cfg(test)]
    pub fn awaits(&self) -> bool {
        self.running.is_some() && self.hold.is_none()
    }

    #[cfg(test)]
    pub fn wait(&mut self) {
        let Some(running) = self.running.take() else {
            return;
        };
        let previewed = running.answer.recv_timeout(PATIENCE).ok();

        self.shown = Some((running.request, previewed));
    }
}

impl Drop for OffsetPreview {
    fn drop(&mut self) {
        self.cancel();
    }
}

struct Job {
    request: OffsetRequest,
    cancelled: Arc<AtomicBool>,
    #[cfg(test)]
    hold: Option<Hold>,
}

impl Job {
    fn run(self) -> Option<Previewed> {
        #[cfg(test)]
        if let Some(hold) = &self.hold {
            hold.wait().ok()?;
        }

        let cancelled = || self.cancelled.load(Ordering::Relaxed);
        self.request.previewed(&cancelled)
    }
}
