use std::{collections::BTreeMap, sync::Arc};

use ahash::AHashMap;
use caditor_kernel::{DisplayMesh, SharedBuffer};

use crate::{
    document::FeatureId,
    recompute::{CacheEntry, FeatureResult},
};

pub(crate) const RESULTS_KEPT_PER_FEATURE: usize = 4;
pub(crate) const EARLIER_RESULTS_BUDGET: usize = 256 * 1024 * 1024;
const SKETCH_ENTITY_BYTES: usize = 160;

#[derive(Debug, Clone)]
struct Kept {
    entry: Arc<CacheEntry>,
    held: Arc<[usize]>,
    used: u64,
}

#[derive(Debug, Clone, Copy)]
struct Holding {
    holders: usize,
    bytes: usize,
}

#[derive(Debug, Clone, Default)]
struct Holdings {
    buffers: AHashMap<usize, Holding>,
    bytes: usize,
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

#[derive(Debug, Clone, Default)]
pub(crate) struct ResultHistory {
    features: BTreeMap<FeatureId, Vec<Kept>>,
    clock: u64,
    budget: Option<usize>,
    earlier: Holdings,
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
        let clock = self.clock;
        let kept = self.features.get_mut(&feature)?;
        let at = kept.iter().position(|kept| matches(kept.entry.as_ref()))?;
        if at > 0 {
            if let Some(found) = kept.get_mut(at) {
                self.earlier.release(found);
            }
            if let Some(latest) = kept.first_mut() {
                self.earlier.hold(latest);
            }
        }
        kept.get_mut(..=at)?.rotate_right(1);
        let found = kept.first_mut()?;
        found.used = clock;
        Some(found.entry.as_ref())
    }

    pub(crate) fn insert(&mut self, feature: FeatureId, entry: CacheEntry) {
        self.clock += 1;
        let kept = self.features.entry(feature).or_default();
        if let Some(latest) = kept.first_mut() {
            self.earlier.hold(latest);
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
            self.earlier.release(&mut dropped);
        }
        self.trim();
    }

    fn trim(&mut self) {
        let budget = self.budget.unwrap_or(EARLIER_RESULTS_BUDGET);
        if self.earlier.bytes <= budget {
            return;
        }
        let mut earlier: Vec<(u64, FeatureId)> = self
            .features
            .iter()
            .flat_map(|(feature, kept)| kept.iter().skip(1).map(move |kept| (kept.used, *feature)))
            .collect();
        earlier.sort_unstable();
        for (used, feature) in earlier {
            if self.earlier.bytes <= budget {
                break;
            }
            let Some(kept) = self.features.get_mut(&feature) else {
                continue;
            };
            let Some(at) = kept.iter().skip(1).position(|kept| kept.used == used) else {
                continue;
            };
            let mut dropped = kept.remove(at + 1);
            self.earlier.release(&mut dropped);
        }
    }

    pub(crate) fn entries_mut(&mut self) -> impl Iterator<Item = &mut Arc<CacheEntry>> {
        self.features
            .values_mut()
            .flat_map(|kept| kept.iter_mut().map(|kept| &mut kept.entry))
    }

    pub(crate) fn clear(&mut self) {
        self.features.clear();
        self.earlier = Holdings::default();
    }

    pub(crate) fn retain(&mut self, alive: impl Fn(&FeatureId) -> bool) {
        let earlier = &mut self.earlier;
        self.features.retain(|feature, kept| {
            let keep = alive(feature);
            if !keep {
                for dropped in kept.iter_mut().skip(1) {
                    earlier.release(dropped);
                }
            }
            keep
        });
    }

    #[cfg(test)]
    pub(crate) fn kept(&self, feature: FeatureId) -> usize {
        self.features.get(&feature).map_or(0, Vec::len)
    }

    #[cfg(test)]
    pub(crate) fn shared_with(&self, other: &Self) -> (usize, usize) {
        let total = self.features.values().map(Vec::len).sum();
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
    pub(crate) fn earlier_bytes(&self) -> usize {
        self.earlier.bytes
    }

    #[cfg(test)]
    pub(crate) fn measured_earlier_bytes(&self) -> usize {
        let mut counted = ahash::AHashSet::new();
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

fn result_buffers(result: &Arc<FeatureResult>, found: &mut dyn FnMut(SharedBuffer)) {
    found(SharedBuffer::of(result, owned_bytes(result)));
    if let FeatureResult::Solid(solid) = result.as_ref() {
        solid.solid.shared_buffers(found);
        for part in solid
            .others()
            .iter()
            .chain(solid.cuts())
            .chain(solid.joins())
        {
            result_buffers(part, found);
        }
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
    }
}
