use std::{
    collections::BTreeMap,
    ops::Deref,
    sync::{Arc, OnceLock},
};

use crate::document::{Feature, FeatureId};

#[derive(Debug, Clone, Default)]
pub struct FeatureList {
    items: Vec<Arc<Feature>>,
    index: Box<Index>,
}

#[derive(Debug, Clone, Default)]
struct Index {
    positions: BTreeMap<FeatureId, usize>,
    by_body: OnceLock<Arc<BTreeMap<FeatureId, Vec<FeatureId>>>>,
}

impl PartialEq for FeatureList {
    fn eq(&self, other: &Self) -> bool {
        self.items == other.items
    }
}

impl Deref for FeatureList {
    type Target = [Arc<Feature>];

    fn deref(&self) -> &[Arc<Feature>] {
        &self.items
    }
}

impl FeatureList {
    pub fn position(&self, id: FeatureId) -> Option<usize> {
        self.index.positions.get(&id).copied()
    }

    pub fn get_by_id(&self, id: FeatureId) -> Option<&Arc<Feature>> {
        self.items.get(self.position(id)?)
    }

    pub fn on_body(&self, body: FeatureId) -> &[FeatureId] {
        self.index
            .by_body
            .get_or_init(|| Arc::new(grouped_by_body(&self.items)))
            .get(&body)
            .map_or(&[], Vec::as_slice)
    }

    pub fn get_mut_by_id(&mut self, id: FeatureId) -> Option<&mut Arc<Feature>> {
        let index = self.position(id)?;
        self.index.by_body = OnceLock::new();
        self.items.get_mut(index)
    }

    pub fn insert(&mut self, index: usize, feature: Arc<Feature>) {
        let index = index.min(self.items.len());
        self.items.insert(index, feature);
        self.reindex(index, self.items.len());
    }

    pub fn remove(&mut self, index: usize) -> Option<Arc<Feature>> {
        if index >= self.items.len() {
            return None;
        }
        let feature = self.items.remove(index);
        self.index.positions.remove(&feature.id());
        self.reindex(index, self.items.len());
        Some(feature)
    }

    pub fn move_to(&mut self, from: usize, to: usize) {
        if from >= self.items.len() || to >= self.items.len() || from == to {
            return;
        }
        let feature = self.items.remove(from);
        self.items.insert(to, feature);
        self.reindex(from.min(to), from.max(to) + 1);
    }

    fn reindex(&mut self, start: usize, end: usize) {
        self.index.by_body = OnceLock::new();
        for (index, feature) in self.items.iter().enumerate().take(end).skip(start) {
            self.index.positions.insert(feature.id(), index);
        }
    }
}

fn grouped_by_body(items: &[Arc<Feature>]) -> BTreeMap<FeatureId, Vec<FeatureId>> {
    let mut grouped: BTreeMap<FeatureId, Vec<FeatureId>> = BTreeMap::new();
    for feature in items {
        if let Some(body) = feature.body() {
            grouped.entry(body).or_default().push(feature.id());
        }
    }
    grouped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::FeatureKind, removal::Remove};

    fn feature(raw: u64) -> Arc<Feature> {
        Arc::new(Feature::new(
            FeatureId::from_raw(raw),
            format!("Feature {raw}"),
            FeatureKind::from(caditor_sketch::Sketch::new(caditor_geometry::Plane::XY)),
        ))
    }

    fn removal(raw: u64, body: u64) -> Arc<Feature> {
        Arc::new(Feature::new(
            FeatureId::from_raw(raw),
            format!("Remove {raw}"),
            FeatureKind::Remove(Remove {
                body: FeatureId::from_raw(body),
            }),
        ))
    }

    fn on_body(list: &FeatureList, body: u64) -> Vec<u64> {
        list.on_body(FeatureId::from_raw(body))
            .iter()
            .map(|id| id.raw())
            .collect()
    }

    fn order(list: &FeatureList) -> Vec<u64> {
        list.iter().map(|feature| feature.id().raw()).collect()
    }

    fn positions_agree(list: &FeatureList) -> bool {
        list.iter()
            .enumerate()
            .all(|(index, feature)| list.position(feature.id()) == Some(index))
            && list.index.positions.len() == list.len()
    }

    #[test]
    fn lookups_follow_inserts_in_the_middle_moves_and_removals() {
        let mut list = FeatureList::default();

        list.insert(0, feature(1));
        list.insert(1, feature(2));
        list.insert(1, feature(3));
        list.insert(0, feature(4));
        let inserted = order(&list);
        let inserted_agree = positions_agree(&list);

        list.move_to(0, 3);
        let moved_down = order(&list);
        let moved_down_agree = positions_agree(&list);

        list.move_to(2, 0);
        let moved_up = order(&list);
        let moved_up_agree = positions_agree(&list);

        let removed = list.remove(1).map(|feature| feature.id().raw());
        let past_the_end = list.remove(9);

        assert_eq!(inserted, [4, 1, 3, 2]);
        assert!(inserted_agree);
        assert_eq!(moved_down, [1, 3, 2, 4]);
        assert!(moved_down_agree);
        assert_eq!(moved_up, [2, 1, 3, 4]);
        assert!(moved_up_agree);
        assert_eq!(removed, Some(1));
        assert!(past_the_end.is_none());
        assert_eq!(order(&list), [2, 3, 4]);
        assert!(positions_agree(&list));
        assert_eq!(list.position(FeatureId::from_raw(1)), None);
        assert_eq!(
            list.get_by_id(FeatureId::from_raw(3))
                .map(|feature| feature.id().raw()),
            Some(3)
        );
    }

    #[test]
    fn the_features_on_a_body_follow_inserts_moves_and_changed_kinds() {
        let mut list = FeatureList::default();

        list.insert(0, removal(1, 7));
        list.insert(1, removal(2, 8));
        list.insert(2, removal(3, 7));
        let inserted = on_body(&list, 7);

        list.move_to(2, 0);
        let moved = on_body(&list, 7);

        if let Some(feature) = list.get_mut_by_id(FeatureId::from_raw(1)) {
            Arc::make_mut(feature).kind = FeatureKind::Remove(Remove {
                body: FeatureId::from_raw(8),
            });
        }
        let changed = (on_body(&list, 7), on_body(&list, 8));

        list.remove(0);
        let removed = on_body(&list, 7);

        assert_eq!(inserted, [1, 3]);
        assert_eq!(moved, [3, 1]);
        assert_eq!(changed, (vec![3], vec![1, 2]));
        assert!(removed.is_empty());
        assert!(on_body(&list, 9).is_empty());
    }
}
