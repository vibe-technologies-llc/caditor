use std::{collections::BTreeMap, sync::Arc};

use caditor_kernel::DisplayMesh;

use crate::{
    document::FeatureId,
    recompute::{CacheEntry, FeatureResult},
};

pub(crate) const RESULTS_KEPT_PER_FEATURE: usize = 4;
pub(crate) const EARLIER_RESULTS_BUDGET: usize = 256 * 1024 * 1024;
const SKETCH_ENTITY_BYTES: usize = 160;

#[derive(Debug, Clone)]
struct Kept {
    entry: CacheEntry,
    bytes: usize,
    used: u64,
}

impl Kept {
    fn demote(&mut self) -> usize {
        self.bytes = self.entry.result.as_deref().map_or(0, result_bytes);
        self.bytes
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ResultHistory {
    features: BTreeMap<FeatureId, Vec<Kept>>,
    clock: u64,
    budget: Option<usize>,
    earlier_bytes: usize,
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
            .map(|kept| &kept.entry)
    }

    pub(crate) fn peek(
        &self,
        feature: FeatureId,
        matches: impl Fn(&CacheEntry) -> bool,
    ) -> Option<&CacheEntry> {
        self.features
            .get(&feature)?
            .iter()
            .map(|kept| &kept.entry)
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
        let at = kept.iter().position(|kept| matches(&kept.entry))?;
        if at > 0 {
            if let Some(found) = kept.get(at) {
                self.earlier_bytes = self.earlier_bytes.saturating_sub(found.bytes);
            }
            if let Some(latest) = kept.first_mut() {
                self.earlier_bytes += latest.demote();
            }
        }
        kept.get_mut(..=at)?.rotate_right(1);
        let found = kept.first_mut()?;
        found.used = clock;
        Some(&found.entry)
    }

    pub(crate) fn insert(&mut self, feature: FeatureId, entry: CacheEntry) {
        self.clock += 1;
        let kept = self.features.entry(feature).or_default();
        if let Some(latest) = kept.first_mut() {
            self.earlier_bytes += latest.demote();
        }
        kept.insert(
            0,
            Kept {
                entry,
                bytes: 0,
                used: self.clock,
            },
        );
        let dropped: usize = kept
            .drain(RESULTS_KEPT_PER_FEATURE.min(kept.len())..)
            .map(|kept| kept.bytes)
            .sum();
        self.earlier_bytes = self.earlier_bytes.saturating_sub(dropped);
        self.trim();
    }

    fn trim(&mut self) {
        let budget = self.budget.unwrap_or(EARLIER_RESULTS_BUDGET);
        if self.earlier_bytes <= budget {
            return;
        }
        let mut earlier: Vec<(u64, FeatureId, usize)> = self
            .features
            .iter()
            .flat_map(|(feature, kept)| {
                kept.iter()
                    .skip(1)
                    .map(move |kept| (kept.used, *feature, kept.bytes))
            })
            .collect();
        earlier.sort_unstable();
        for (used, feature, bytes) in earlier {
            if self.earlier_bytes <= budget {
                break;
            }
            if let Some(kept) = self.features.get_mut(&feature) {
                let mut first = true;
                kept.retain(|kept| std::mem::take(&mut first) || kept.used != used);
                self.earlier_bytes = self.earlier_bytes.saturating_sub(bytes);
            }
        }
    }

    pub(crate) fn entries_mut(&mut self) -> impl Iterator<Item = &mut CacheEntry> {
        self.features
            .values_mut()
            .flat_map(|kept| kept.iter_mut().map(|kept| &mut kept.entry))
    }

    pub(crate) fn clear(&mut self) {
        self.features.clear();
        self.earlier_bytes = 0;
    }

    pub(crate) fn retain(&mut self, alive: impl Fn(&FeatureId) -> bool) {
        let mut dropped = 0;
        self.features.retain(|feature, kept| {
            let keep = alive(feature);
            if !keep {
                dropped += kept.iter().skip(1).map(|kept| kept.bytes).sum::<usize>();
            }
            keep
        });
        self.earlier_bytes = self.earlier_bytes.saturating_sub(dropped);
    }

    #[cfg(test)]
    pub(crate) fn kept(&self, feature: FeatureId) -> usize {
        self.features.get(&feature).map_or(0, Vec::len)
    }

    #[cfg(test)]
    pub(crate) fn earlier_bytes(&self) -> usize {
        self.earlier_bytes
    }

    #[cfg(test)]
    pub(crate) fn measured_earlier_bytes(&self) -> usize {
        self.features
            .values()
            .flat_map(|kept| kept.iter().skip(1))
            .map(|kept| kept.entry.result.as_deref().map_or(0, result_bytes))
            .sum()
    }
}

fn result_bytes(result: &FeatureResult) -> usize {
    match result {
        FeatureResult::Solid(solid) => {
            solid.solid.approximate_size()
                + solid
                    .display_mesh()
                    .map_or(0, DisplayMesh::approximate_size)
                + solid
                    .others()
                    .iter()
                    .chain(solid.cuts())
                    .chain(solid.joins())
                    .map(|part| result_bytes(Arc::as_ref(part)))
                    .sum::<usize>()
        }
        FeatureResult::Sketch(sketch) => {
            size_of_val(sketch) + sketch.geometry.entities().len() * SKETCH_ENTITY_BYTES
        }
        FeatureResult::Datum(datum) => size_of_val(datum),
        FeatureResult::Thread(thread) => size_of_val(thread) + thread.designation.len(),
    }
}
