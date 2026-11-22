use std::{
    fmt,
    sync::{Arc, Weak},
};

use ahash::AHashMap;
use glam::DVec2;
use parking_lot::Mutex;

use crate::{
    camera::View,
    picking::PICK_RADIUS_POINTS,
    scene::Batch,
    viewport::{BatchBounds, batch_lines},
};

const REACH_MARGIN_PIXELS: f64 = 1.0;

impl BatchBounds {
    pub(crate) fn is_out_of_reach(
        &self,
        view: &View,
        cursor: DVec2,
        pixels_per_point: f64,
    ) -> bool {
        let mut low = DVec2::splat(f64::INFINITY);
        let mut high = DVec2::splat(f64::NEG_INFINITY);

        for corner in self.corners {
            let Some(at) = view.project(corner).filter(|at| at.is_finite()) else {
                return false;
            };

            low = low.min(at);
            high = high.max(at);
        }

        let reach =
            (PICK_RADIUS_POINTS + self.reach_points) * pixels_per_point + REACH_MARGIN_PIXELS;

        cursor.x < low.x - reach
            || cursor.x > high.x + reach
            || cursor.y < low.y - reach
            || cursor.y > high.y + reach
    }
}

struct Kept {
    batch: Weak<Batch>,
    bounds: Option<BatchBounds>,
}

#[derive(Default)]
pub struct KeptBatchBounds(Mutex<AHashMap<usize, Kept>>);

impl KeptBatchBounds {
    pub(crate) fn of(&self, batches: &[Arc<Batch>]) -> Vec<Option<BatchBounds>> {
        let mut kept = self.0.lock();

        let mut before = std::mem::take(&mut *kept);

        batches
            .iter()
            .map(|batch| {
                let key = Arc::as_ptr(batch).addr();
                let held = before
                    .remove(&key)
                    .or_else(|| kept.remove(&key))
                    .filter(|held| held.batch.ptr_eq(&Arc::downgrade(batch)))
                    .unwrap_or_else(|| Kept {
                        batch: Arc::downgrade(batch),
                        bounds: BatchBounds::of(batch, &batch_lines(batch)),
                    });
                let bounds = held.bounds;

                kept.insert(key, held);
                bounds
            })
            .collect()
    }
}

impl Clone for KeptBatchBounds {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl fmt::Debug for KeptBatchBounds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeptBatchBounds")
            .field("batches", &self.0.lock().len())
            .finish()
    }
}

impl PartialEq for KeptBatchBounds {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
