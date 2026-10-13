use std::{
    cmp::Reverse,
    collections::{BTreeMap, BinaryHeap},
    sync::Arc,
};

use crate::{
    document::{Document, Feature, FeatureId},
    measurement::Measured,
};

#[derive(Debug, Clone, PartialEq)]
pub struct UseOrder {
    pub features: Vec<Arc<Feature>>,
    pub lowered: Vec<Lowered>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lowered {
    pub name: String,
    pub below: String,
}

impl Document {
    pub fn in_use_order(&self, features: Vec<Arc<Feature>>) -> UseOrder {
        let uses = used_positions(self, &features);
        let stored_in_order = uses
            .iter()
            .enumerate()
            .all(|(user, used)| used.iter().all(|position| *position < user));
        if stored_in_order {
            return UseOrder {
                features,
                lowered: Vec::new(),
            };
        }

        let order = stable_order(&uses);
        let mut placed = vec![0; features.len()];
        for (place, position) in order.iter().enumerate() {
            if let Some(slot) = placed.get_mut(*position) {
                *slot = place;
            }
        }

        let lowered = uses
            .iter()
            .enumerate()
            .filter_map(|(user, used)| lowered_below(&features, &placed, user, used))
            .collect();
        let features = order
            .iter()
            .filter_map(|position| features.get(*position).cloned())
            .collect();
        UseOrder { features, lowered }
    }
}

fn used_positions(document: &Document, features: &[Arc<Feature>]) -> Vec<Vec<usize>> {
    let measured = Measured::among(document, features.iter().map(Arc::as_ref));
    let positions: BTreeMap<FeatureId, usize> = features
        .iter()
        .enumerate()
        .map(|(position, feature)| (feature.id(), position))
        .collect();
    features
        .iter()
        .enumerate()
        .map(|(user, feature)| {
            measured
                .features_read_by(&feature.kind)
                .into_iter()
                .filter_map(|used| positions.get(&used).copied())
                .filter(|used| *used != user)
                .collect()
        })
        .collect()
}

fn stable_order(uses: &[Vec<usize>]) -> Vec<usize> {
    let count = uses.len();
    let mut waiting_on: Vec<usize> = uses.iter().map(Vec::len).collect();
    let mut users: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (user, used) in uses.iter().enumerate() {
        for position in used {
            if let Some(slot) = users.get_mut(*position) {
                slot.push(user);
            }
        }
    }

    let mut ready: BinaryHeap<Reverse<usize>> = waiting_on
        .iter()
        .enumerate()
        .filter(|(_, waiting)| **waiting == 0)
        .map(|(position, _)| Reverse(position))
        .collect();
    let mut placed = vec![false; count];
    let mut order = Vec::with_capacity(count);
    let mut first_unplaced = 0;
    while order.len() < count {
        let next = match ready.pop() {
            Some(Reverse(position)) => position,
            None => {
                while placed.get(first_unplaced).copied().unwrap_or(false) {
                    first_unplaced += 1;
                }
                first_unplaced
            }
        };
        let Some(slot) = placed.get_mut(next) else {
            break;
        };
        if *slot {
            continue;
        }
        *slot = true;
        order.push(next);
        for user in users.get(next).into_iter().flatten() {
            if let Some(waiting) = waiting_on.get_mut(*user) {
                *waiting = waiting.saturating_sub(1);
                if *waiting == 0 && !placed.get(*user).copied().unwrap_or(true) {
                    ready.push(Reverse(*user));
                }
            }
        }
    }
    order
}

fn lowered_below(
    features: &[Arc<Feature>],
    placed: &[usize],
    user: usize,
    used: &[usize],
) -> Option<Lowered> {
    let place_of = |position: usize| placed.get(position).copied();
    let user_place = place_of(user)?;
    if used
        .iter()
        .any(|position| place_of(*position).is_none_or(|place| place > user_place))
    {
        return None;
    }
    let below = used
        .iter()
        .filter(|position| **position > user)
        .max_by_key(|position| place_of(**position))?;
    Some(Lowered {
        name: features.get(user)?.name.clone(),
        below: features.get(*below)?.name.clone(),
    })
}
