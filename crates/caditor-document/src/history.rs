use std::{collections::BTreeMap, sync::Arc};

use crate::{
    document::FeatureId,
    recompute::{CacheEntry, FeatureResult},
};

pub(crate) const RESULTS_KEPT_PER_FEATURE: usize = 4;
pub(crate) const EARLIER_RESULTS_BUDGET: usize = 256 * 1024 * 1024;
const MESH_ALLOWANCE: usize = 2;
const SKETCH_ENTITY_BYTES: usize = 160;

#[derive(Debug, Clone)]
struct Kept {
    entry: CacheEntry,
    bytes: usize,
    used: u64,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ResultHistory {
    features: BTreeMap<FeatureId, Vec<Kept>>,
    clock: u64,
    budget: Option<usize>,
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
        kept.get_mut(..=at)?.rotate_right(1);
        let found = kept.first_mut()?;
        found.used = clock;
        Some(&found.entry)
    }

    pub(crate) fn insert(&mut self, feature: FeatureId, entry: CacheEntry) {
        self.clock += 1;
        let bytes = entry.result.as_deref().map_or(0, result_bytes);
        let kept = self.features.entry(feature).or_default();
        kept.insert(
            0,
            Kept {
                entry,
                bytes,
                used: self.clock,
            },
        );
        kept.truncate(RESULTS_KEPT_PER_FEATURE);
        self.trim();
    }

    fn trim(&mut self) {
        let budget = self.budget.unwrap_or(EARLIER_RESULTS_BUDGET);
        let mut earlier: Vec<(u64, FeatureId, usize)> = self
            .features
            .iter()
            .flat_map(|(feature, kept)| {
                kept.iter()
                    .skip(1)
                    .map(move |kept| (kept.used, *feature, kept.bytes))
            })
            .collect();
        let mut held: usize = earlier.iter().map(|(_, _, bytes)| bytes).sum();
        if held <= budget {
            return;
        }
        earlier.sort_unstable();
        for (used, feature, bytes) in earlier {
            if held <= budget {
                break;
            }
            if let Some(kept) = self.features.get_mut(&feature) {
                let mut first = true;
                kept.retain(|kept| std::mem::take(&mut first) || kept.used != used);
                held = held.saturating_sub(bytes);
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
    }

    pub(crate) fn retain(&mut self, alive: impl Fn(&FeatureId) -> bool) {
        self.features.retain(|feature, _| alive(feature));
    }

    #[cfg(test)]
    pub(crate) fn kept(&self, feature: FeatureId) -> usize {
        self.features.get(&feature).map_or(0, Vec::len)
    }
}

fn result_bytes(result: &FeatureResult) -> usize {
    match result {
        FeatureResult::Solid(solid) => {
            solid.solid.approximate_size() * MESH_ALLOWANCE
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
