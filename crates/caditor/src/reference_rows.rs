use std::{
    collections::{BTreeMap, btree_map},
    sync::{Arc, Weak},
};

use caditor_document::{FeatureId, FeatureResult};

use crate::selection::Pickable;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReferenceRows {
    pub summary: String,
    pub rows: Vec<String>,
}

#[derive(Debug, Clone)]
struct Entry {
    source: Option<Weak<FeatureResult>>,
    revision: u64,
    rows: ReferenceRows,
    used: bool,
}

impl Entry {
    fn made_from(&self, source: Option<&Arc<FeatureResult>>, revision: u64) -> bool {
        let same_source = match (&self.source, source) {
            (Some(cached), Some(source)) => std::ptr::eq(cached.as_ptr(), Arc::as_ptr(source)),
            (None, None) => true,
            (Some(_), None) | (None, Some(_)) => false,
        };
        same_source && self.revision == revision
    }
}

#[derive(Debug, Clone, Default)]
pub struct RowCache {
    entries: BTreeMap<FeatureId, Entry>,
    previewed: Vec<Pickable>,
}

impl RowCache {
    pub fn preview(&mut self, pickables: Vec<Pickable>) {
        self.previewed = pickables;
    }

    pub fn take_previewed(&mut self) -> Vec<Pickable> {
        std::mem::take(&mut self.previewed)
    }

    pub fn begin_frame(&mut self) {
        self.entries
            .retain(|_, entry| std::mem::take(&mut entry.used));
    }

    pub fn rows(
        &mut self,
        feature: FeatureId,
        source: Option<&Arc<FeatureResult>>,
        revision: u64,
        build: impl FnOnce() -> ReferenceRows,
    ) -> &ReferenceRows {
        let fresh = || Entry {
            source: source.map(Arc::downgrade),
            revision,
            rows: build(),
            used: true,
        };
        let entry = match self.entries.entry(feature) {
            btree_map::Entry::Occupied(occupied) => {
                let entry = occupied.into_mut();
                if !entry.made_from(source, revision) {
                    *entry = fresh();
                }
                entry
            }
            btree_map::Entry::Vacant(vacant) => vacant.insert(fresh()),
        };
        entry.used = true;
        &entry.rows
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use caditor_document::DatumResult;
    use caditor_geometry::Plane;

    use super::*;

    fn rows(text: &str) -> ReferenceRows {
        ReferenceRows {
            summary: text.to_owned(),
            rows: vec![text.to_owned()],
        }
    }

    #[test]
    fn rows_are_rebuilt_only_when_the_input_or_the_revision_changes() {
        let mut cache = RowCache::default();
        let feature = FeatureId::from_raw(3);
        let first = Arc::new(FeatureResult::Datum(DatumResult::Plane(Plane::XY)));
        let second = Arc::new(FeatureResult::Datum(DatumResult::Plane(Plane::XY)));
        let builds = Cell::new(0);
        let list = |cache: &mut RowCache, source: Option<&Arc<FeatureResult>>, revision| {
            cache
                .rows(feature, source, revision, || {
                    builds.set(builds.get() + 1);
                    rows(&format!("build {}", builds.get()))
                })
                .summary
                .clone()
        };

        assert_eq!(list(&mut cache, Some(&first), 1), "build 1");
        assert_eq!(list(&mut cache, Some(&first), 1), "build 1");
        assert_eq!(list(&mut cache, Some(&first), 2), "build 2");
        assert_eq!(list(&mut cache, Some(&second), 2), "build 3");
        assert_eq!(list(&mut cache, None, 2), "build 4");
        assert_eq!(list(&mut cache, None, 2), "build 4");
        assert_eq!(builds.get(), 4);
    }

    #[test]
    fn rows_not_shown_for_a_frame_are_dropped() {
        let mut cache = RowCache::default();
        let shown = FeatureId::from_raw(1);
        let hidden = FeatureId::from_raw(2);

        cache.rows(shown, None, 1, || rows("shown"));
        cache.rows(hidden, None, 1, || rows("hidden"));
        cache.begin_frame();
        cache.rows(shown, None, 1, || rows("rebuilt"));
        cache.begin_frame();

        assert_eq!(cache.entries.len(), 1);
        assert_eq!(
            cache.rows(shown, None, 1, || rows("rebuilt")).summary,
            "shown"
        );
        assert_eq!(
            cache.rows(hidden, None, 1, || rows("again")).summary,
            "again"
        );
    }
}
