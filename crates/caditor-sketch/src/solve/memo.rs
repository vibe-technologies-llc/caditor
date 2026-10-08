use std::collections::{BTreeMap, BTreeSet};

use crate::{
    id::{ConstraintId, EntityId},
    sketch::DimensionValues,
    solve::{
        equation::value,
        numeric::{Component, ComponentAnalysis},
        system::System,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Variable {
    X(EntityId),
    Y(EntityId),
    Radius(EntityId),
    Parameter(ConstraintId, usize),
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
    analysed: bool,
    solved: Vec<f64>,
    rank: usize,
    fixed_columns: Vec<usize>,
    contributions: Vec<(ConstraintId, bool, Vec<ConstraintId>)>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SolveMemo {
    anchors: Vec<u64>,
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
    parts: &'a [Component],
    keys: Vec<Option<Key>>,
    part_from_first_equation: Vec<Option<usize>>,
    previous: Option<&'a SolveMemo>,
    next: SolveMemo,
}

impl<'a> Recall<'a> {
    pub fn new(
        system: &System,
        dimensions: &DimensionValues,
        parts: &'a [Component],
        stiff: &BTreeSet<usize>,
        previous: Option<&'a SolveMemo>,
    ) -> Self {
        let names = Names::of(system);
        let anchors = system.anchor_bits();
        let keys = parts
            .iter()
            .map(|component| {
                let remembered = !component.variables.is_empty()
                    && !component
                        .variables
                        .iter()
                        .any(|variable| stiff.contains(variable));
                remembered
                    .then(|| Key::of(system, dimensions, component, &names))
                    .flatten()
            })
            .collect();
        let mut part_from_first_equation = vec![None; system.equations.len()];
        for (index, component) in parts.iter().enumerate() {
            if let Some(slot) = component
                .equations
                .first()
                .and_then(|first| part_from_first_equation.get_mut(*first))
            {
                *slot = Some(index);
            }
        }
        Self {
            parts,
            keys,
            part_from_first_equation,
            previous: previous.filter(|previous| previous.anchors == anchors),
            next: SolveMemo {
                anchors,
                ..SolveMemo::default()
            },
        }
    }

    pub fn start_from(&self, values: &mut [f64]) {
        let Some(previous) = self.previous else {
            return;
        };
        for (component, key) in self.parts.iter().zip(&self.keys) {
            let Some(outcome) = key.as_ref().and_then(|key| previous.outcomes.get(key)) else {
                continue;
            };
            for (variable, solved) in component.variables.iter().zip(&outcome.solved) {
                if let Some(slot) = values.get_mut(*variable) {
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
        let Some(key) = self.key_of(component).cloned() else {
            return analyze();
        };
        let recalled = self
            .previous
            .and_then(|previous| previous.outcomes.get(&key))
            .filter(|outcome| outcome.analysed && outcome.holds(component, values))
            .cloned();
        let outcome = match recalled {
            Some(outcome) => {
                self.next.recalled += 1;
                outcome
            }
            None => Outcome::analysed(component, values, analyze()),
        };
        let analysis = outcome.restore(component);
        self.remember(key, outcome);
        analysis
    }

    pub fn remember_geometry(&mut self, component: &Component, values: &[f64]) {
        let Some(key) = self.key_of(component).cloned() else {
            return;
        };
        let outcome = Outcome {
            analysed: false,
            solved: solved_values(component, values),
            rank: 0,
            fixed_columns: Vec::new(),
            contributions: Vec::new(),
        };
        self.remember(key, outcome);
    }

    pub fn finish(self) -> SolveMemo {
        self.next
    }

    fn key_of(&self, component: &Component) -> Option<&Key> {
        let index = component
            .equations
            .first()
            .and_then(|first| self.part_from_first_equation.get(*first))
            .copied()
            .flatten()?;
        let part = self.parts.get(index)?;
        let same = part.equations == component.equations && part.variables == component.variables;
        if !same {
            return None;
        }
        self.keys.get(index)?.as_ref()
    }

    fn remember(&mut self, key: Key, outcome: Outcome) {
        let settled = key.settled(&outcome);
        if settled != key {
            self.next.outcomes.insert(settled, outcome.clone());
        }
        self.next.outcomes.insert(key, outcome);
    }
}

fn solved_values(component: &Component, values: &[f64]) -> Vec<f64> {
    component
        .variables
        .iter()
        .map(|variable| value(values, *variable))
        .collect()
}

impl Outcome {
    fn analysed(component: &Component, values: &[f64], analysis: ComponentAnalysis) -> Self {
        Self {
            analysed: true,
            solved: solved_values(component, values),
            rank: analysis.rank,
            fixed_columns: analysis
                .fixed
                .iter()
                .filter_map(|variable| component.variables.binary_search(variable).ok())
                .collect(),
            contributions: analysis.contributions,
        }
    }

    fn holds(&self, component: &Component, values: &[f64]) -> bool {
        component.variables.len() == self.solved.len()
            && component
                .variables
                .iter()
                .zip(&self.solved)
                .all(|(variable, solved)| value(values, *variable).to_bits() == solved.to_bits())
    }

    fn restore(&self, component: &Component) -> ComponentAnalysis {
        ComponentAnalysis {
            rank: self.rank,
            fixed: self
                .fixed_columns
                .iter()
                .filter_map(|column| component.variables.get(*column).copied())
                .collect(),
            contributions: self.contributions.clone(),
        }
    }
}

struct Names {
    variables: Vec<Option<Variable>>,
    first_owner: Vec<usize>,
    owners: Vec<EntityId>,
}

impl Names {
    fn of(system: &System) -> Self {
        let mut variables = vec![None; system.values.len()];
        let mut name = |index: usize, variable: Variable| {
            if let Some(slot) = variables.get_mut(index) {
                *slot = Some(variable);
            }
        };
        for (entity, x) in &system.points {
            name(*x, Variable::X(*entity));
            name(x + 1, Variable::Y(*entity));
        }
        for (entity, radius) in &system.radii {
            name(*radius, Variable::Radius(*entity));
        }
        for (constraint, parameters) in &system.parameters {
            for (ordinal, parameter) in parameters.iter().enumerate() {
                name(*parameter, Variable::Parameter(*constraint, ordinal));
            }
        }
        let mut counts = vec![0; system.values.len()];
        for variable in system.entity_variables.values().flatten() {
            if let Some(count) = counts.get_mut(*variable) {
                *count += 1;
            }
        }
        let first_owner: Vec<usize> = std::iter::once(0)
            .chain(counts.iter().scan(0, |total, count| {
                *total += count;
                Some(*total)
            }))
            .collect();
        let mut owners = vec![EntityId::ORIGIN; first_owner.last().copied().unwrap_or(0)];
        let mut filled = first_owner.clone();
        for (entity, variables) in &system.entity_variables {
            for variable in variables {
                let Some(next) = filled.get_mut(*variable) else {
                    continue;
                };
                if let Some(slot) = owners.get_mut(*next) {
                    *slot = *entity;
                }
                *next += 1;
            }
        }
        Self {
            variables,
            first_owner,
            owners,
        }
    }

    fn owners_of(&self, variable: usize) -> &[EntityId] {
        let first = self.first_owner.get(variable).copied().unwrap_or(0);
        let end = self.first_owner.get(variable + 1).copied().unwrap_or(first);
        self.owners.get(first..end).unwrap_or_default()
    }
}

impl Key {
    fn settled(&self, outcome: &Outcome) -> Self {
        Self {
            start: self
                .start
                .iter()
                .zip(&outcome.solved)
                .map(|((variable, _), solved)| (*variable, solved.to_bits()))
                .collect(),
            ..self.clone()
        }
    }

    fn of(
        system: &System,
        dimensions: &DimensionValues,
        component: &Component,
        names: &Names,
    ) -> Option<Self> {
        let mut entities: Vec<EntityId> = component
            .variables
            .iter()
            .flat_map(|variable| names.owners_of(*variable))
            .copied()
            .collect();
        entities.sort_unstable();
        entities.dedup();
        let mut constraints: Vec<ConstraintId> = component
            .equations
            .iter()
            .filter_map(|index| system.equations.get(*index)?.owner)
            .collect();
        constraints.sort_unstable();
        constraints.dedup();
        let start = component
            .variables
            .iter()
            .map(|index| {
                let name = names.variables.get(*index).copied().flatten()?;
                Some((name, value(&system.values, *index).to_bits()))
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            scale: system.context_of(component).scale.to_bits(),
            entities,
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
