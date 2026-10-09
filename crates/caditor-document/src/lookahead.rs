use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use crate::{
    document::{Feature, FeatureId},
    recompute::{BodyState, FeatureResult, View},
    solid::body_part,
};

struct Node {
    feature: Arc<Feature>,
    used: Vec<(FeatureId, usize)>,
    bodies_used: BTreeSet<FeatureId>,
    touched: Vec<FeatureId>,
    touched_before: BTreeMap<FeatureId, usize>,
    waiting: usize,
    dependents: Vec<usize>,
    settled: Option<Settled>,
}

struct Settled {
    stood: Option<Arc<FeatureResult>>,
    after: BTreeMap<FeatureId, Option<BodyState>>,
}

pub(crate) struct Lookahead {
    nodes: BTreeMap<usize, Node>,
    consumers: BTreeMap<FeatureId, Vec<usize>>,
    ready: BTreeSet<usize>,
    halted: bool,
}

impl Lookahead {
    pub(crate) fn new(features: &[Arc<Feature>], bar: usize) -> Self {
        let position: BTreeMap<FeatureId, usize> = features
            .iter()
            .enumerate()
            .map(|(index, feature)| (feature.id(), index))
            .collect();
        let mut lookahead = Self {
            nodes: BTreeMap::new(),
            consumers: BTreeMap::new(),
            ready: BTreeSet::new(),
            halted: false,
        };
        let mut last_touch: BTreeMap<FeatureId, usize> = BTreeMap::new();
        for (index, feature) in features.iter().enumerate().take(bar) {
            if feature.suppressed {
                continue;
            }
            let consumed = feature.kind.consumed_bodies();
            let touched: Vec<FeatureId> = feature
                .bodies()
                .into_iter()
                .chain(consumed.iter().copied())
                .collect();
            let bodies_used = feature.kind.bodies_used();
            let touched_before: BTreeMap<FeatureId, usize> = bodies_used
                .iter()
                .chain(&touched)
                .filter_map(|body| Some((*body, *last_touch.get(body)?)))
                .collect();
            let used: Vec<(FeatureId, usize)> = feature
                .kind
                .features()
                .into_iter()
                .filter_map(|used| Some((used, *position.get(&used)?)))
                .filter(|(_, at)| *at < index)
                .collect();
            let needed: BTreeSet<usize> = used
                .iter()
                .map(|(_, at)| *at)
                .chain(touched_before.values().copied())
                .filter(|at| lookahead.nodes.contains_key(at))
                .collect();
            for at in &needed {
                if let Some(needed) = lookahead.nodes.get_mut(at) {
                    needed.dependents.push(index);
                }
            }
            if needed.is_empty() {
                lookahead.ready.insert(index);
            }
            for body in consumed {
                lookahead.consumers.entry(body).or_default().push(index);
            }
            for body in &touched {
                last_touch.insert(*body, index);
            }
            lookahead.nodes.insert(
                index,
                Node {
                    feature: Arc::clone(feature),
                    used,
                    bodies_used,
                    touched,
                    touched_before,
                    waiting: needed.len(),
                    dependents: Vec::new(),
                    settled: None,
                },
            );
        }
        lookahead
    }

    pub(crate) fn halt(&mut self) {
        self.halted = true;
    }

    pub(crate) fn next_ready(&mut self) -> Option<usize> {
        while !self.halted {
            let index = self.ready.pop_first()?;
            if self
                .nodes
                .get(&index)
                .is_some_and(|node| node.settled.is_none())
            {
                return Some(index);
            }
        }
        None
    }

    pub(crate) fn view(&self, index: usize) -> View {
        let mut view = View::default();
        let Some(node) = self.nodes.get(&index) else {
            return view;
        };
        for (used, at) in &node.used {
            if let Some(stood) = self.stood(*at) {
                view.features.insert(*used, Arc::clone(stood));
            }
        }
        for body in &node.bodies_used {
            if let Some(state) = self.before(node, *body) {
                view.bodies.insert(*body, state);
            }
            let consumers = self.consumers.get(body).into_iter().flatten();
            for consumer in consumers.filter(|consumer| **consumer < index) {
                if let (Some(stood), Some(node)) = (self.stood(*consumer), self.nodes.get(consumer))
                {
                    view.features.insert(node.feature.id(), Arc::clone(stood));
                }
            }
        }
        view
    }

    pub(crate) fn settle(&mut self, index: usize, stood: Option<Arc<FeatureResult>>) {
        let Some(node) = self.nodes.get(&index) else {
            return;
        };
        if node.settled.is_some() {
            return;
        }
        let consumed = node.feature.kind.consumed_bodies();
        let after = node
            .touched
            .iter()
            .map(|body| {
                let before = self.before(node, *body);
                let state = match &stood {
                    Some(_) if consumed.contains(body) => None,
                    Some(result) => body_part(result, *body)
                        .map(|part| (node.feature.id(), Arc::clone(part)))
                        .or(before),
                    None => before,
                };
                (*body, state)
            })
            .collect();
        let dependents = node.dependents.clone();
        if let Some(node) = self.nodes.get_mut(&index) {
            node.settled = Some(Settled { stood, after });
        }
        for dependent in dependents {
            if let Some(node) = self.nodes.get_mut(&dependent) {
                node.waiting = node.waiting.saturating_sub(1);
                if node.waiting == 0 && node.settled.is_none() {
                    self.ready.insert(dependent);
                }
            }
        }
    }

    fn stood(&self, index: usize) -> Option<&Arc<FeatureResult>> {
        self.nodes.get(&index)?.settled.as_ref()?.stood.as_ref()
    }

    fn before(&self, node: &Node, body: FeatureId) -> Option<BodyState> {
        let toucher = node.touched_before.get(&body)?;
        self.nodes
            .get(toucher)?
            .settled
            .as_ref()?
            .after
            .get(&body)?
            .clone()
    }
}
