use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Deref,
    sync::{Arc, OnceLock},
};

use caditor_expression::ParameterId;

use crate::{
    body_appearance::BodyAppearance,
    body_pieces::{PieceLooks, PieceStandIns, Pieces},
    document::{Feature, FeatureId},
    tally,
};

#[derive(Clone, Debug, Default)]
pub struct FeatureList {
    items: Vec<Arc<Feature>>,
    index: Box<Index>,
}

pub struct Unindexed<'a> {
    pub hidden: &'a mut bool,
    pub suppressed: &'a mut bool,
    pub appearance: &'a mut BodyAppearance,
    pub group: &'a mut Option<String>,
    pub pieces: &'a mut Pieces,
    pub piece_looks: &'a mut PieceLooks,
    pub piece_stand_ins: &'a mut PieceStandIns,
}

#[derive(Clone, Debug, Default)]
struct Index {
    positions: BTreeMap<FeatureId, usize>,
    by_body: OnceLock<Arc<BTreeMap<FeatureId, Vec<FeatureId>>>>,
    by_name: OnceLock<Arc<BTreeMap<String, Vec<FeatureId>>>>,
    by_use: Grouped<FeatureId>,
    by_tie: Grouped<FeatureId>,
    by_consumed: Grouped<FeatureId>,
    by_measured: Grouped<ParameterId>,
    by_parameter: Grouped<ParameterId>,
}

type Grouped<K> = OnceLock<Arc<BTreeMap<K, Vec<FeatureId>>>>;

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
        lookup(&self.index.by_use, &self.items, features_used, &used)
    }

    pub fn tied_to(&self, maker: FeatureId) -> &[FeatureId] {
        lookup(&self.index.by_tie, &self.items, makers_tied, &maker)
    }

    pub fn consuming(&self, maker: FeatureId) -> &[FeatureId] {
        lookup(
            &self.index.by_consumed,
            &self.items,
            makers_consumed,
            &maker,
        )
    }

    pub fn index_ties(&self) {
        built(&self.index.by_use, &self.items, features_used);
        built(&self.index.by_tie, &self.items, makers_tied);
        built(&self.index.by_consumed, &self.items, makers_consumed);
        built(&self.index.by_measured, &self.items, parameter_measured);
    }

    pub fn measurements_of(&self, parameter: ParameterId) -> &[FeatureId] {
        lookup(
            &self.index.by_measured,
            &self.items,
            parameter_measured,
            &parameter,
        )
    }

    pub fn measures_nothing(&self) -> bool {
        built(&self.index.by_measured, &self.items, parameter_measured).is_empty()
    }

    pub fn reading(&self, parameter: ParameterId) -> &[FeatureId] {
        lookup(
            &self.index.by_parameter,
            &self.items,
            parameters_used,
            &parameter,
        )
    }

    pub fn get_mut_by_id(&mut self, id: FeatureId) -> Option<&mut Arc<Feature>> {
        let index = self.position(id)?;

        self.index.by_body = OnceLock::new();
        self.index.by_name = OnceLock::new();
        self.index.by_use = OnceLock::new();
        self.index.by_tie = OnceLock::new();
        self.index.by_consumed = OnceLock::new();
        self.index.by_measured = OnceLock::new();
        self.index.by_parameter = OnceLock::new();
        self.items.get_mut(index)
    }

    pub fn unindexed_by_id(&mut self, id: FeatureId) -> Option<Unindexed<'_>> {
        let index = self.position(id)?;
        let Feature {
            hidden,
            suppressed,
            appearance,
            group,
            pieces,
            piece_looks,
            piece_stand_ins,
            ..
        } = Arc::make_mut(self.items.get_mut(index)?);
        Some(Unindexed {
            hidden,
            suppressed,
            appearance,
            group,
            pieces,
            piece_looks,
            piece_stand_ins,
        })
    }

    pub fn insert(&mut self, index: usize, feature: Arc<Feature>) {
        let index = index.min(self.items.len());

        if let Some(names) = self.index.by_name.get_mut() {
            Arc::make_mut(names)
                .entry(feature.name.clone())
                .or_default()
                .push(feature.id());
        }
        enter(&mut self.index.by_use, &feature, features_used);
        enter(&mut self.index.by_tie, &feature, makers_tied);
        enter(&mut self.index.by_consumed, &feature, makers_consumed);
        enter(&mut self.index.by_measured, &feature, parameter_measured);
        enter(&mut self.index.by_parameter, &feature, parameters_used);
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
        leave(&mut self.index.by_use, &feature, features_used);
        leave(&mut self.index.by_tie, &feature, makers_tied);
        leave(&mut self.index.by_consumed, &feature, makers_consumed);
        leave(&mut self.index.by_measured, &feature, parameter_measured);
        leave(&mut self.index.by_parameter, &feature, parameters_used);
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

fn features_used(feature: &Feature) -> impl Iterator<Item = FeatureId> + use<> {
    let id = feature.id();
    feature
        .kind
        .features()
        .into_iter()
        .filter(move |used| *used != id)
}

fn makers_tied(feature: &Feature) -> impl Iterator<Item = FeatureId> + use<> {
    let kind = &feature.kind;
    let mut tied = kind.dependencies();

    tied.extend(kind.bodies_used());
    tied.extend(kind.consumed_bodies());
    tied.into_iter()
        .map(FeatureId::maker)
        .collect::<BTreeSet<_>>()
        .into_iter()
}

fn makers_consumed(feature: &Feature) -> impl Iterator<Item = FeatureId> + use<> {
    feature
        .kind
        .consumed_bodies()
        .into_iter()
        .map(FeatureId::maker)
        .collect::<BTreeSet<_>>()
        .into_iter()
}

fn parameter_measured(feature: &Feature) -> impl Iterator<Item = ParameterId> + use<> {
    feature
        .kind
        .measurement()
        .and_then(|measurement| measurement.parameter)
        .into_iter()
}

fn parameters_used(feature: &Feature) -> impl Iterator<Item = ParameterId> + use<> {
    feature.kind.parameters().into_iter()
}

fn lookup<'a, K: Ord, I: Iterator<Item = K>>(
    index: &'a Grouped<K>,
    items: &[Arc<Feature>],
    keys: fn(&Feature) -> I,
    key: &K,
) -> &'a [FeatureId] {
    built(index, items, keys)
        .get(key)
        .map_or(&[], Vec::as_slice)
}

fn built<'a, K: Ord, I: Iterator<Item = K>>(
    index: &'a Grouped<K>,
    items: &[Arc<Feature>],
    keys: fn(&Feature) -> I,
) -> &'a BTreeMap<K, Vec<FeatureId>> {
    index.get_or_init(|| Arc::new(grouped(items, keys)))
}

fn grouped<K: Ord, I: Iterator<Item = K>>(
    items: &[Arc<Feature>],
    keys: fn(&Feature) -> I,
) -> BTreeMap<K, Vec<FeatureId>> {
    let mut grouped: BTreeMap<K, Vec<FeatureId>> = BTreeMap::new();

    for feature in items {
        tally::add(1);

        for key in keys(feature) {
            grouped.entry(key).or_default().push(feature.id());
        }
    }
    grouped
}

fn enter<K: Ord + Clone, I: Iterator<Item = K>>(
    index: &mut Grouped<K>,
    feature: &Feature,
    keys: fn(&Feature) -> I,
) {
    if let Some(grouped) = index.get_mut() {
        let grouped = Arc::make_mut(grouped);
        for key in keys(feature) {
            grouped.entry(key).or_default().push(feature.id());
        }
    }
}

fn leave<K: Ord + Clone, I: Iterator<Item = K>>(
    index: &mut Grouped<K>,
    feature: &Feature,
    keys: fn(&Feature) -> I,
) {
    if let Some(grouped) = index.get_mut() {
        let grouped = Arc::make_mut(grouped);
        for key in keys(feature) {
            if let Some(holders) = grouped.get_mut(&key) {
                holders.retain(|holder| *holder != feature.id());
                if holders.is_empty() {
                    grouped.remove(&key);
                }
            }
        }
    }
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
    use caditor_expression::{Expression, Unit};

    use super::*;
    use crate::{
        Between, Datum, DatumPoint, MeasuredItem, Measurement, PieceKey, PlaneReference,
        PointReference, PrincipalPlane, Reading, document::FeatureKind, removal::Remove,
    };

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

    fn tied_to(list: &FeatureList, maker: u64) -> Vec<u64> {
        list.tied_to(FeatureId::from_raw(maker))
            .iter()
            .map(|id| id.raw())
            .collect()
    }

    fn consuming(list: &FeatureList, maker: u64) -> Vec<u64> {
        list.consuming(FeatureId::from_raw(maker))
            .iter()
            .map(|id| id.raw())
            .collect()
    }

    #[test]
    fn the_features_tied_to_a_maker_follow_inserts_removals_and_changed_kinds() {
        let mut list = FeatureList::default();
        let piece = FeatureId::from_raw(7)
            .piece(PieceKey::from_raw(3))
            .unwrap()
            .raw();

        list.insert(0, removal(1, 7));
        list.insert(1, removal(2, piece));

        let inserted = (tied_to(&list, 7), consuming(&list, 7));

        list.insert(2, removal(3, 8));
        list.move_to(2, 0);

        let moved = (tied_to(&list, 8), consuming(&list, 8));

        list.remove(1);

        let removed = (tied_to(&list, 7), consuming(&list, 7));

        if let Some(feature) = list.get_mut_by_id(FeatureId::from_raw(3)) {
            Arc::make_mut(feature).kind = FeatureKind::Remove(Remove {
                body: FeatureId::from_raw(7),
            });
        }

        let changed = (tied_to(&list, 7), consuming(&list, 8));

        assert_eq!(inserted, (vec![1, 2], vec![1, 2]));
        assert_eq!(moved, (vec![3], vec![3]));
        assert_eq!(removed, (vec![2], vec![2]));
        assert_eq!(changed, (vec![3, 2], vec![]));
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

    #[test]
    fn a_change_to_what_no_index_reads_keeps_every_index() {
        let mut list = FeatureList::default();
        let one = FeatureId::from_raw(1);

        list.insert(0, removal(1, 7));
        list.insert(1, removal(2, 7));

        let taken = list.name_taken_besides("Remove 2", one);
        let users = users_of(&list, 7);
        let on = on_body(&list, 7);

        if let Some(feature) = list.unindexed_by_id(one) {
            *feature.hidden = true;
            *feature.group = Some("Folder".to_owned());
        }

        assert!(taken);
        assert_eq!(users, [1, 2]);
        assert_eq!(on, [1, 2]);
        assert!(list.index.by_name.get().is_some());
        assert!(list.index.by_use.get().is_some());
        assert!(list.index.by_body.get().is_some());
        assert!(list.get_by_id(one).is_some_and(|feature| feature.hidden));
    }

    fn measuring(raw: u64, parameter: u64) -> Arc<Feature> {
        Arc::new(Feature::new(
            FeatureId::from_raw(raw),
            format!("Measurement {raw}"),
            FeatureKind::from(Measurement {
                reading: Reading::Between {
                    quantity: Between::Distance,
                    first: MeasuredItem::Point(PointReference::Origin),
                    second: MeasuredItem::Plane(PlaneReference::Principal(PrincipalPlane::Xy)),
                },
                parameter: Some(ParameterId::from_raw(parameter)),
            }),
        ))
    }

    fn offset_by(raw: u64, parameter: u64) -> Arc<Feature> {
        Arc::new(Feature::new(
            FeatureId::from_raw(raw),
            format!("Point {raw}"),
            FeatureKind::Datum(Datum::Point(DatumPoint {
                base: PointReference::Origin,
                offset: [
                    Expression::Parameter(ParameterId::from_raw(parameter)),
                    Expression::Measure(0.0, Unit::Millimetre),
                    Expression::Measure(0.0, Unit::Millimetre),
                ],
            })),
        ))
    }

    fn measurements_of(list: &FeatureList, parameter: u64) -> Vec<u64> {
        list.measurements_of(ParameterId::from_raw(parameter))
            .iter()
            .map(|id| id.raw())
            .collect()
    }

    fn reading(list: &FeatureList, parameter: u64) -> Vec<u64> {
        list.reading(ParameterId::from_raw(parameter))
            .iter()
            .map(|id| id.raw())
            .collect()
    }

    #[test]
    fn measurements_and_readers_of_a_parameter_follow_inserts_removals_and_changed_kinds() {
        let mut list = FeatureList::default();

        let nothing_at_first = list.measures_nothing();

        list.insert(0, measuring(1, 7));
        list.insert(1, offset_by(2, 7));
        list.insert(2, offset_by(3, 8));

        let inserted = (
            list.measures_nothing(),
            measurements_of(&list, 7),
            reading(&list, 7),
            reading(&list, 8),
        );

        list.insert(3, measuring(4, 8));
        list.insert(4, offset_by(5, 7));

        let kept = (
            measurements_of(&list, 8),
            reading(&list, 7),
            reading(&list, 8),
        );

        list.remove(0);

        let removed = (measurements_of(&list, 7), reading(&list, 7));

        if let Some(feature) = list.get_mut_by_id(FeatureId::from_raw(4)) {
            *Arc::make_mut(feature) = Arc::unwrap_or_clone(offset_by(4, 7));
        }

        let changed = (
            list.measures_nothing(),
            measurements_of(&list, 8),
            reading(&list, 7),
        );

        assert!(nothing_at_first);
        assert_eq!(inserted, (false, vec![1], vec![2], vec![3]));
        assert_eq!(kept, (vec![4], vec![2, 5], vec![3]));
        assert_eq!(removed, (vec![], vec![2, 5]));
        assert_eq!(changed, (true, vec![], vec![2, 4, 5]));
    }
}
