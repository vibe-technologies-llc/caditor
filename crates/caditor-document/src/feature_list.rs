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
    by_name: OnceLock<Arc<BTreeMap<String, Vec<FeatureId>>>>,
    by_use: OnceLock<Arc<BTreeMap<FeatureId, Vec<FeatureId>>>>,
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

    pub fn name_taken_besides(&self, name: &str, id: FeatureId) -> bool {
        self.index
            .by_name
            .get_or_init(|| Arc::new(grouped_by_name(&self.items)))
            .get(name)
            .is_some_and(|holders| holders.iter().any(|holder| *holder != id))
    }

    pub fn users_of(&self, used: FeatureId) -> &[FeatureId] {
        self.index
            .by_use
            .get_or_init(|| Arc::new(grouped_by_use(&self.items)))
            .get(&used)
            .map_or(&[], Vec::as_slice)
    }

    pub fn get_mut_by_id(&mut self, id: FeatureId) -> Option<&mut Arc<Feature>> {
        let index = self.position(id)?;
        self.index.by_body = OnceLock::new();
        self.index.by_name = OnceLock::new();
        self.index.by_use = OnceLock::new();
        self.items.get_mut(index)
    }

    pub fn insert(&mut self, index: usize, feature: Arc<Feature>) {
        let index = index.min(self.items.len());
        if let Some(names) = self.index.by_name.get_mut() {
            Arc::make_mut(names)
                .entry(feature.name.clone())
                .or_default()
                .push(feature.id());
        }
        if let Some(uses) = self.index.by_use.get_mut() {
            let uses = Arc::make_mut(uses);
            for used in features_used(&feature) {
                uses.entry(used).or_default().push(feature.id());
            }
        }
        self.items.insert(index, feature);
        self.reindex(index, self.items.len());
    }

    pub fn remove(&mut self, index: usize) -> Option<Arc<Feature>> {
        if index >= self.items.len() {
            return None;
        }
        let feature = self.items.remove(index);
        self.index.positions.remove(&feature.id());
        if let Some(names) = self.index.by_name.get_mut() {
            let names = Arc::make_mut(names);
            if let Some(holders) = names.get_mut(&feature.name) {
                holders.retain(|holder| *holder != feature.id());
                if holders.is_empty() {
                    names.remove(&feature.name);
                }
            }
        }
        if let Some(uses) = self.index.by_use.get_mut() {
            let uses = Arc::make_mut(uses);
            for used in features_used(&feature) {
                if let Some(users) = uses.get_mut(&used) {
                    users.retain(|user| *user != feature.id());
                    if users.is_empty() {
                        uses.remove(&used);
                    }
                }
            }
        }
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

fn features_used(feature: &Feature) -> impl Iterator<Item = FeatureId> {
    let id = feature.id();
    feature
        .kind
        .features()
        .into_iter()
        .filter(move |used| *used != id)
}

fn grouped_by_use(items: &[Arc<Feature>]) -> BTreeMap<FeatureId, Vec<FeatureId>> {
    let mut grouped: BTreeMap<FeatureId, Vec<FeatureId>> = BTreeMap::new();
    for feature in items {
        for used in features_used(feature) {
            grouped.entry(used).or_default().push(feature.id());
        }
    }
    grouped
}

fn grouped_by_name(items: &[Arc<Feature>]) -> BTreeMap<String, Vec<FeatureId>> {
    let mut grouped: BTreeMap<String, Vec<FeatureId>> = BTreeMap::new();
    for feature in items {
        grouped
            .entry(feature.name.clone())
            .or_default()
            .push(feature.id());
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

    fn users_of(list: &FeatureList, used: u64) -> Vec<u64> {
        list.users_of(FeatureId::from_raw(used))
            .iter()
            .map(|id| id.raw())
            .collect()
    }

    #[test]
    fn the_users_of_a_feature_follow_inserts_removals_and_changed_kinds() {
        let mut list = FeatureList::default();

        list.insert(0, removal(1, 7));
        let asked_first = users_of(&list, 7);
        list.insert(1, removal(2, 7));
        list.insert(2, removal(3, 8));
        let inserted = (users_of(&list, 7), users_of(&list, 8));

        list.remove(0);
        let removed = users_of(&list, 7);

        if let Some(feature) = list.get_mut_by_id(FeatureId::from_raw(3)) {
            Arc::make_mut(feature).kind = FeatureKind::Remove(Remove {
                body: FeatureId::from_raw(7),
            });
        }
        let changed = (users_of(&list, 7), users_of(&list, 8));

        assert_eq!(asked_first, [1]);
        assert_eq!(inserted, (vec![1, 2], vec![3]));
        assert_eq!(removed, [2]);
        assert_eq!(changed, (vec![2, 3], vec![]));
    }

    #[test]
    fn taken_names_follow_inserts_removals_and_renames() {
        let mut list = FeatureList::default();
        let one = FeatureId::from_raw(1);
        let two = FeatureId::from_raw(2);

        list.insert(0, feature(1));
        let asked_before = list.name_taken_besides("Feature 2", two);
        list.insert(1, feature(2));
        let after_insert = (
            list.name_taken_besides("Feature 2", one),
            list.name_taken_besides("Feature 2", two),
        );

        if let Some(renamed) = list.get_mut_by_id(two) {
            Arc::make_mut(renamed).name = "Boss".to_owned();
        }
        let after_rename = (
            list.name_taken_besides("Feature 2", one),
            list.name_taken_besides("Boss", one),
            list.name_taken_besides("Boss", two),
        );

        list.remove(1);
        let after_removal = list.name_taken_besides("Boss", one);

        assert!(!asked_before);
        assert_eq!(after_insert, (true, false));
        assert_eq!(after_rename, (false, true, false));
        assert!(!after_removal);
    }
}
