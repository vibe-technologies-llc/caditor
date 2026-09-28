use std::collections::{BTreeMap, BTreeSet, VecDeque};

use caditor_expression::{EvalError, Expression, ParameterId, Quantity};

use crate::document::{Document, path_to};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ParameterError {
    #[error(transparent)]
    Evaluation(#[from] EvalError),
    #[error("it depends on itself ({path})")]
    Cycle { path: String },
}

#[derive(Debug, Clone, PartialEq)]
struct Entry {
    name: String,
    value: Result<Quantity, ParameterError>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParameterValues {
    entries: BTreeMap<ParameterId, Entry>,
}

impl ParameterValues {
    pub fn evaluate(document: &Document) -> Self {
        let order = EvaluationOrder::of(document);
        let mut values = Self::default();
        for (id, cycle) in order.cycles {
            let Some(parameter) = document.parameter(id) else {
                continue;
            };
            let path = cycle
                .iter()
                .map(|step| document.parameter_name(*step).unwrap_or("?"))
                .collect::<Vec<_>>()
                .join(" → ");
            values.entries.insert(
                id,
                Entry {
                    name: parameter.name.clone(),
                    value: Err(ParameterError::Cycle { path }),
                },
            );
        }
        for id in order.sequence {
            let Some(parameter) = document.parameter(id) else {
                continue;
            };
            let value = values
                .evaluate_expression(&parameter.expression)
                .map_err(ParameterError::Evaluation);
            values.entries.insert(
                id,
                Entry {
                    name: parameter.name.clone(),
                    value,
                },
            );
        }
        values
    }

    pub fn get(&self, id: ParameterId) -> Option<&Result<Quantity, ParameterError>> {
        self.entries.get(&id).map(|entry| &entry.value)
    }

    pub fn value(&self, id: ParameterId) -> Result<Quantity, EvalError> {
        let entry = self.entries.get(&id).ok_or(EvalError::ParameterMissing)?;
        entry.value.clone().map_err(|_| EvalError::ParameterFailed {
            id,
            name: entry.name.clone(),
        })
    }

    pub fn evaluate_expression(&self, expression: &Expression) -> Result<Quantity, EvalError> {
        expression.evaluate(&|id| self.value(id))
    }

    pub(crate) fn fingerprint(
        &self,
        used: &BTreeSet<ParameterId>,
    ) -> Vec<(ParameterId, Option<Quantity>)> {
        used.iter()
            .map(|id| {
                let value = self.entries.get(id);
                (
                    *id,
                    value.and_then(|entry| entry.value.as_ref().ok().copied()),
                )
            })
            .collect()
    }

    pub(crate) fn names(&self, used: &BTreeSet<ParameterId>) -> Vec<String> {
        used.iter()
            .map(|id| {
                self.entries
                    .get(id)
                    .map(|entry| entry.name.clone())
                    .unwrap_or_default()
            })
            .collect()
    }
}

struct EvaluationOrder {
    sequence: Vec<ParameterId>,
    cycles: BTreeMap<ParameterId, Vec<ParameterId>>,
}

impl EvaluationOrder {
    fn of(document: &Document) -> Self {
        let dependencies = document.parameter_dependencies();
        let cycles: BTreeMap<ParameterId, Vec<ParameterId>> = dependencies
            .iter()
            .filter_map(|(id, used)| {
                path_to(*id, used.iter().copied(), &dependencies).map(|cycle| (*id, cycle))
            })
            .collect();

        let mut waiting: BTreeMap<ParameterId, usize> = BTreeMap::new();
        let mut dependents: BTreeMap<ParameterId, Vec<ParameterId>> = BTreeMap::new();
        for (id, used) in &dependencies {
            if cycles.contains_key(id) {
                continue;
            }
            let pending: Vec<ParameterId> = used
                .iter()
                .filter(|dependency| !cycles.contains_key(dependency))
                .copied()
                .collect();
            waiting.insert(*id, pending.len());
            for dependency in pending {
                dependents.entry(dependency).or_default().push(*id);
            }
        }

        let mut ready: VecDeque<ParameterId> = waiting
            .iter()
            .filter(|(_, count)| **count == 0)
            .map(|(id, _)| *id)
            .collect();
        let mut sequence = Vec::with_capacity(waiting.len());
        while let Some(id) = ready.pop_front() {
            sequence.push(id);
            for dependent in dependents.get(&id).into_iter().flatten() {
                if let Some(count) = waiting.get_mut(dependent) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        ready.push_back(*dependent);
                    }
                }
            }
        }
        let placed: BTreeSet<ParameterId> = sequence.iter().copied().collect();
        sequence.extend(waiting.keys().filter(|id| !placed.contains(id)));
        Self { sequence, cycles }
    }
}
