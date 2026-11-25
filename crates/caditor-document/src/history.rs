use std::{collections::BTreeMap, sync::Arc};

use ahash::{AHashMap, AHashSet};
use caditor_kernel::{DisplayMesh, SharedBuffer};

use crate::{
    document::FeatureId,
    landing::LandingInputs,
    recompute::{CacheEntry, FeatureResult},
};

pub(crate) const RESULTS_KEPT_PER_FEATURE: usize = 4;
pub(crate) const EARLIER_RESULTS_BUDGET: usize = 256 * 1024 * 1024;
const SKETCH_ENTITY_BYTES: usize = 160;
const HELIX_TURN_BYTES: usize = 1_600;

#[derive(Clone, Debug)]
struct Kept {
    entry: Arc<CacheEntry>,
    held: Arc<[usize]>,
    used: u64,
}

impl Kept {
    fn demoted(&mut self, last_used: Option<&u64>) -> &mut Self {
        if let Some(used) = last_used {
            self.used = *used;
        }
        self
    }
}

#[derive(Clone, Copy, Debug)]
struct Holding {
    holders: usize,
    bytes: usize,
}

#[derive(Clone, Debug, Default)]
struct Holdings {
    buffers: AHashMap<usize, Holding>,
    bytes: usize,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SharedStorage {
    pub(crate) results: bool,
    pub(crate) recency: bool,
    pub(crate) holdings: bool,
}

impl Holdings {
    fn hold(&mut self, kept: &mut Kept) {
        let mut found = Vec::new();

        if let Some(result) = &kept.entry.result {
            result_buffers(result, &mut |buffer| found.push(buffer));
        }
        found.sort_unstable_by_key(|buffer| buffer.address());
        found.dedup_by_key(|buffer| buffer.address());
        for buffer in &found {
            let holding = self
                .buffers
                .entry(buffer.address())
                .or_insert_with(|| Holding {
                    holders: 0,
                    bytes: buffer.bytes(),
                });

            if holding.holders == 0 {
                self.bytes += holding.bytes;
            }
            holding.holders += 1;
        }
        kept.held = found.iter().map(|buffer| buffer.address()).collect();
    }

    fn release(&mut self, kept: &mut Kept) {
        for address in std::mem::take(&mut kept.held).iter() {
            let Some(holding) = self.buffers.get_mut(address) else {
                continue;
            };

            holding.holders = holding.holders.saturating_sub(1);
            if holding.holders == 0 {
                self.bytes = self.bytes.saturating_sub(holding.bytes);
                self.buffers.remove(address);
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ResultHistory {
    features: Arc<BTreeMap<FeatureId, Arc<Vec<Kept>>>>,
    latest_used: Arc<AHashMap<FeatureId, u64>>,
    clock: u64,
    budget: Option<usize>,
    earlier: Arc<Holdings>,
}

impl ResultHistory {
    #[cfg(test)]
    pub(crate) fn with_budget(budget: usize) -> Self {
        Self {
            budget: Some(budget),
            ..Self::default()
        }
    }

    pub(crate) fn latest(&self, feature: FeatureId) -> Option<&CacheEntry> {
        self.features
            .get(&feature)
            .and_then(|kept| kept.first())
            .map(|kept| kept.entry.as_ref())
    }

    pub(crate) fn peek(
        &self,
        feature: FeatureId,
        matches: impl Fn(&CacheEntry) -> bool,
    ) -> Option<&CacheEntry> {
        self.features
            .get(&feature)?
            .iter()
            .map(|kept| kept.entry.as_ref())
            .find(|entry| matches(entry))
    }

    pub(crate) fn reuse(
        &mut self,
        feature: FeatureId,
        matches: impl Fn(&CacheEntry) -> bool,
    ) -> Option<&CacheEntry> {
        self.clock += 1;

        let at = self
            .features
            .get(&feature)?
            .iter()
            .position(|kept| matches(kept.entry.as_ref()))?;

        if at > 0 {
            let kept = Arc::make_mut(Arc::make_mut(&mut self.features).get_mut(&feature)?);
            let earlier = Arc::make_mut(&mut self.earlier);

            if let Some(found) = kept.get_mut(at) {
                earlier.release(found);
            }
            if let Some(latest) = kept.first_mut() {
                earlier.hold(latest.demoted(self.latest_used.get(&feature)));
            }
            kept.get_mut(..=at)?.rotate_right(1);
        }
        Arc::make_mut(&mut self.latest_used).insert(feature, self.clock);
        self.latest(feature)
    }

    pub(crate) fn insert(&mut self, feature: FeatureId, entry: CacheEntry) {
        self.clock += 1;

        let kept = Arc::make_mut(
            Arc::make_mut(&mut self.features)
                .entry(feature)
                .or_default(),
        );

        if let Some(latest) = kept.first_mut() {
            Arc::make_mut(&mut self.earlier).hold(latest.demoted(self.latest_used.get(&feature)));
        }
        kept.insert(
            0,
            Kept {
                entry: Arc::new(entry),
                held: Arc::default(),
                used: self.clock,
            },
        );
        for mut dropped in kept.drain(RESULTS_KEPT_PER_FEATURE.min(kept.len())..) {
            Arc::make_mut(&mut self.earlier).release(&mut dropped);
        }
        Arc::make_mut(&mut self.latest_used).insert(feature, self.clock);
        self.trim();
    }

    fn trim(&mut self) {
        let budget = self.budget.unwrap_or(EARLIER_RESULTS_BUDGET);

        if self.earlier.bytes <= budget {
            return;
        }

        let mut candidates: Vec<(u64, FeatureId)> = self
            .features
            .iter()
            .flat_map(|(feature, kept)| kept.iter().skip(1).map(move |kept| (kept.used, *feature)))
            .collect();
        let features = Arc::make_mut(&mut self.features);
        let earlier = Arc::make_mut(&mut self.earlier);

        candidates.sort_unstable();
        for (used, feature) in candidates {
            if earlier.bytes <= budget {
                break;
            }

            let Some(kept) = features.get_mut(&feature).map(Arc::make_mut) else {
                continue;
            };
            let Some(at) = kept.iter().skip(1).position(|kept| kept.used == used) else {
                continue;
            };

            let mut dropped = kept.remove(at + 1);

            earlier.release(&mut dropped);
        }
    }

    pub(crate) fn change_entries(
        &mut self,
        wanted: impl Fn(&CacheEntry) -> bool,
        change: impl Fn(&mut CacheEntry),
    ) {
        let touched: Vec<FeatureId> = self
            .features
            .iter()
            .filter(|(_, kept)| kept.iter().any(|kept| wanted(&kept.entry)))
            .map(|(feature, _)| *feature)
            .collect();

        if touched.is_empty() {
            return;
        }

        let features = Arc::make_mut(&mut self.features);

        for feature in touched {
            let Some(kept) = features.get_mut(&feature).map(Arc::make_mut) else {
                continue;
            };

            for kept in kept.iter_mut().filter(|kept| wanted(&kept.entry)) {
                change(Arc::make_mut(&mut kept.entry));
            }
        }
    }

    pub(crate) fn clear(&mut self) {
        self.features = Arc::default();
        self.latest_used = Arc::default();
        self.earlier = Arc::default();
    }

    pub(crate) fn retain(&mut self, alive: impl Fn(&FeatureId) -> bool) {
        let gone: Vec<FeatureId> = self
            .features
            .keys()
            .filter(|feature| !alive(feature))
            .copied()
            .collect();

        if gone.is_empty() {
            return;
        }

        let features = Arc::make_mut(&mut self.features);
        let latest_used = Arc::make_mut(&mut self.latest_used);
        let earlier = Arc::make_mut(&mut self.earlier);

        for feature in gone {
            latest_used.remove(&feature);

            let Some(mut kept) = features.remove(&feature) else {
                continue;
            };

            for dropped in Arc::make_mut(&mut kept).iter_mut().skip(1) {
                earlier.release(dropped);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn kept(&self, feature: FeatureId) -> usize {
        self.features.get(&feature).map_or(0, |kept| kept.len())
    }

    #[cfg(test)]
    pub(crate) fn storage_shared_with(&self, other: &Self) -> SharedStorage {
        SharedStorage {
            results: Arc::ptr_eq(&self.features, &other.features),
            recency: Arc::ptr_eq(&self.latest_used, &other.latest_used),
            holdings: Arc::ptr_eq(&self.earlier, &other.earlier),
        }
    }

    #[cfg(test)]
    pub(crate) fn shared_with(&self, other: &Self) -> (usize, usize) {
        let total = self.features.values().map(|kept| kept.len()).sum();
        let shared = self
            .features
            .iter()
            .filter_map(|(feature, kept)| Some((kept, other.features.get(feature)?)))
            .flat_map(|(kept, other)| {
                kept.iter().filter(|kept| {
                    other
                        .iter()
                        .any(|other| Arc::ptr_eq(&kept.entry, &other.entry))
                })
            })
            .count();
        (shared, total)
    }

    #[cfg(test)]
    pub(crate) fn earlier_kept(&self) -> usize {
        self.features
            .values()
            .map(|kept| kept.len().saturating_sub(1))
            .sum()
    }

    #[cfg(test)]
    pub(crate) fn earlier_buffers(&self) -> usize {
        self.earlier.buffers.len()
    }

    #[cfg(test)]
    pub(crate) fn earlier_bytes(&self) -> usize {
        self.earlier.bytes
    }

    #[cfg(test)]
    pub(crate) fn measured_earlier_bytes(&self) -> usize {
        let mut counted = AHashSet::new();
        let mut bytes = 0;

        for result in self
            .features
            .values()
            .flat_map(|kept| kept.iter().skip(1))
            .filter_map(|kept| kept.entry.result.as_ref())
        {
            result_buffers(result, &mut |buffer| {
                if counted.insert(buffer.address()) {
                    bytes += buffer.bytes();
                }
            });
        }
        bytes
    }

    #[cfg(test)]
    pub(crate) fn earlier_bytes_in_full(&self) -> usize {
        self.features
            .values()
            .flat_map(|kept| kept.iter().skip(1))
            .filter_map(|kept| kept.entry.result.as_ref())
            .map(|result| {
                let mut bytes = 0;

                result_buffers(result, &mut |buffer| bytes += buffer.bytes());
                bytes
            })
            .sum()
    }
}

pub(crate) fn result_buffers(result: &Arc<FeatureResult>, found: &mut dyn FnMut(SharedBuffer)) {
    let mut visited = AHashSet::new();
    let mut pending = vec![result];

    while let Some(result) = pending.pop() {
        if !visited.insert(Arc::as_ptr(result)) {
            continue;
        }

        found(SharedBuffer::of(result, owned_bytes(result)));

        let FeatureResult::Solid(solid) = result.as_ref() else {
            continue;
        };

        solid.solid.shared_buffers(found);
        pending.extend(
            solid
                .others()
                .iter()
                .chain(solid.cuts())
                .chain(solid.joins())
                .chain(solid.shelled().map(|recipe| &recipe.before))
                .chain(solid.landing().into_iter().flat_map(LandingInputs::results)),
        );
    }
}

fn owned_bytes(result: &FeatureResult) -> usize {
    match result {
        FeatureResult::Solid(solid) => {
            solid.solid.owned_size()
                + solid
                    .display_mesh()
                    .map_or(0, DisplayMesh::approximate_size)
        }
        FeatureResult::Sketch(sketch) => {
            size_of_val(sketch) + sketch.geometry.entities().len() * SKETCH_ENTITY_BYTES
        }
        FeatureResult::Datum(datum) => size_of_val(datum),
        FeatureResult::Thread(thread) => size_of_val(thread) + thread.designation.len(),
        FeatureResult::Measurement(measurement) => size_of_val(measurement),
        FeatureResult::Image(image) => size_of_val(image),
        FeatureResult::Helix(helix) => size_of_val(helix) + helix.turns.len() * HELIX_TURN_BYTES,
    }
}
