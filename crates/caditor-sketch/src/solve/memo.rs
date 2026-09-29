use std::collections::{BTreeMap, BTreeSet};

use crate::{
    id::{ConstraintId, EntityId},
    sketch::DimensionValues,
    solve::{
        equation::value,
        numeric::{Component, ComponentAnalysis, components},
        system::System,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Variable {
    X(EntityId),
    Y(EntityId),
    Radius(EntityId),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    scale: u64,
    entities: Vec<EntityId>,
    constraints: Vec<(ConstraintId, Option<u64>)>,
    start: Vec<(Variable, u64)>,
}

#[derive(Debug, Clone, PartialEq)]
struct Outcome {
    solved: Vec<(Variable, f64)>,
    rank: usize,
    fixed: Vec<Variable>,
    contributions: Vec<(ConstraintId, bool, Vec<ConstraintId>)>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SolveMemo {
    outcomes: BTreeMap<Key, Outcome>,
    recalled: usize,
}

impl SolveMemo {
    pub fn remembered(&self) -> usize {
        self.outcomes.len()
    }

    pub fn recalled(&self) -> usize {
        self.recalled
    }
}

pub(crate) struct Recall<'a> {
    names: BTreeMap<usize, Variable>,
    indices: BTreeMap<Variable, usize>,
    keys: BTreeMap<Vec<usize>, Key>,
    previous: Option<&'a SolveMemo>,
    next: SolveMemo,
}

impl<'a> Recall<'a> {
    pub fn new(
        system: &System,
        dimensions: &DimensionValues,
        active: &[usize],
        stiff: &BTreeSet<usize>,
        previous: Option<&'a SolveMemo>,
    ) -> Self {
        let mut names = BTreeMap::new();
        for (entity, x) in &system.points {
            names.insert(*x, Variable::X(*entity));
            names.insert(x + 1, Variable::Y(*entity));
        }
        for (entity, radius) in &system.radii {
            names.insert(*radius, Variable::Radius(*entity));
        }
        let indices = names.iter().map(|(index, name)| (*name, *index)).collect();
        let mut owners: BTreeMap<usize, Vec<EntityId>> = BTreeMap::new();
        for (entity, variables) in &system.entity_variables {
            for variable in variables {
                owners.entry(*variable).or_default().push(*entity);
            }
        }
        let keys = components(system, active, &system.values)
            .into_iter()
            .filter(|component| {
                !component.variables.is_empty()
                    && !component
                        .variables
                        .iter()
                        .any(|variable| stiff.contains(variable))
            })
            .filter_map(|component| {
                let key = Key::of(system, dimensions, &component, &names, &owners)?;
                Some((component.equations, key))
            })
            .collect();
        Self {
            names,
            indices,
            keys,
            previous,
            next: SolveMemo::default(),
        }
    }

    pub fn start_from(&self, values: &mut [f64]) {
        let Some(previous) = self.previous else {
            return;
        };
        for key in self.keys.values() {
            let Some(outcome) = previous.outcomes.get(key) else {
                continue;
            };
            for (variable, solved) in &outcome.solved {
                if let Some(slot) = self
                    .indices
                    .get(variable)
                    .and_then(|index| values.get_mut(*index))
                {
                    *slot = *solved;
                }
            }
        }
    }

    pub fn analysis(
        &mut self,
        component: &Component,
        values: &[f64],
        analyze: impl FnOnce() -> ComponentAnalysis,
    ) -> ComponentAnalysis {
        let Some(key) = self.keys.get(&component.equations) else {
            return analyze();
        };
        let recalled = self
            .previous
            .and_then(|previous| previous.outcomes.get(key))
            .filter(|outcome| self.holds(outcome, values))
            .cloned();
        let outcome = match recalled {
            Some(outcome) => {
                self.next.recalled += 1;
                outcome
            }
            None => self.outcome(component, values, &analyze()),
        };
        let analysis = self.restore(&outcome);
        let settled = key.settled(&outcome);
        if settled != *key {
            self.next.outcomes.insert(settled, outcome.clone());
        }
        self.next.outcomes.insert(key.clone(), outcome);
        analysis
    }

    pub fn finish(self) -> SolveMemo {
        self.next
    }

    fn holds(&self, outcome: &Outcome, values: &[f64]) -> bool {
        outcome.solved.iter().all(|(variable, solved)| {
            self.indices
                .get(variable)
                .is_some_and(|index| value(values, *index).to_bits() == solved.to_bits())
        })
    }

    fn outcome(
        &self,
        component: &Component,
        values: &[f64],
        analysis: &ComponentAnalysis,
    ) -> Outcome {
        let named = |index: &usize| self.names.get(index).copied();
        Outcome {
            solved: component
                .variables
                .iter()
                .filter_map(|index| Some((named(index)?, value(values, *index))))
                .collect(),
            rank: analysis.rank,
            fixed: analysis.fixed.iter().filter_map(named).collect(),
            contributions: analysis.contributions.clone(),
        }
    }

    fn restore(&self, outcome: &Outcome) -> ComponentAnalysis {
        ComponentAnalysis {
            rank: outcome.rank,
            fixed: outcome
                .fixed
                .iter()
                .filter_map(|variable| self.indices.get(variable).copied())
                .collect(),
            contributions: outcome.contributions.clone(),
        }
    }
}

impl Key {
    fn settled(&self, outcome: &Outcome) -> Self {
        Self {
            start: outcome
                .solved
                .iter()
                .map(|(variable, solved)| (*variable, solved.to_bits()))
                .collect(),
            ..self.clone()
        }
    }

    fn of(
        system: &System,
        dimensions: &DimensionValues,
        component: &Component,
        names: &BTreeMap<usize, Variable>,
        owners: &BTreeMap<usize, Vec<EntityId>>,
    ) -> Option<Self> {
        let entities: BTreeSet<EntityId> = component
            .variables
            .iter()
            .filter_map(|variable| owners.get(variable))
            .flatten()
            .copied()
            .collect();
        let constraints: BTreeSet<ConstraintId> = component
            .equations
            .iter()
            .filter_map(|index| system.equations.get(*index)?.owner)
            .collect();
        let start = component
            .variables
            .iter()
            .map(|index| {
                let name = names.get(index)?;
                Some((*name, value(&system.values, *index).to_bits()))
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            scale: system.context_of(component).scale.to_bits(),
            entities: entities.into_iter().collect(),
            constraints: constraints
                .into_iter()
                .map(|constraint| {
                    let dimension = dimensions.dimension(constraint).map(f64::to_bits);
                    (constraint, dimension)
                })
                .collect(),
            start,
        })
    }
}
