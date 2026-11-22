use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
};

use caditor_expression::{EvalError, Expression, ParameterId, Quantity};

use crate::{
    datum::feature_name,
    document::{Document, Parameter, path_to},
    measurement::Measured,
    tally,
};

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum ParameterError {
    #[error(transparent)]
    Evaluation(#[from] EvalError),
    #[error("it depends on itself ({path})")]
    Cycle { path: String },
    #[error("it is read from {measurement}, which has no reading now")]
    Unmeasured { measurement: String },
}

#[derive(Clone, Debug, PartialEq)]
struct Entry {
    name: String,
    value: Result<Quantity, ParameterError>,
}

#[derive(Clone, Debug, Default)]
pub struct ParameterValues {
    entries: Arc<BTreeMap<ParameterId, Entry>>,
    readings: BTreeMap<ParameterId, Entry>,
    measured: Arc<BTreeSet<ParameterId>>,
}

impl PartialEq for ParameterValues {
    fn eq(&self, other: &Self) -> bool {
        self.measured == other.measured
            && self.entries.len() == other.entries.len()
            && self
                .entries
                .keys()
                .all(|id| self.entry(*id) == other.entry(*id))
    }
}

impl ParameterValues {
    fn entry(&self, id: ParameterId) -> Option<&Entry> {
        self.readings.get(&id).or_else(|| self.entries.get(&id))
    }

    #[cfg(test)]
    pub(crate) fn shares_entries_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.entries, &other.entries)
    }

    pub fn evaluate(document: &Document) -> Self {
        let order = EvaluationOrder::of(document);
        let by_id: BTreeMap<ParameterId, &Parameter> = document
            .parameters()
            .iter()
            .map(|parameter| (parameter.id(), parameter))
            .collect();
        let mut values = Self::default();

        for (id, cycle) in order.cycles {
            let Some(parameter) = by_id.get(&id) else {
                continue;
            };
            let path = cycle
                .iter()
                .map(|step| {
                    by_id
                        .get(step)
                        .map_or("?", |parameter| parameter.name.as_str())
                })
                .collect::<Vec<_>>()
                .join(" → ");

            Arc::make_mut(&mut values.entries).insert(
                id,
                Entry {
                    name: parameter.name.clone(),
                    value: Err(ParameterError::Cycle { path }),
                },
            );
        }

        let measured = Measured::of(document);

        for id in order.sequence {
            let Some(parameter) = by_id.get(&id) else {
                continue;
            };

            if let Some(measurement) = measured.measurement(id) {
                Arc::make_mut(&mut values.measured).insert(id);
                Arc::make_mut(&mut values.entries).insert(
                    id,
                    Entry {
                        name: parameter.name.clone(),
                        value: Err(ParameterError::Unmeasured {
                            measurement: feature_name(document, measurement),
                        }),
                    },
                );
                continue;
            }

            let mut value = values
                .evaluate_expression(&parameter.expression)
                .map_err(ParameterError::Evaluation);

            if measured.reads_measurement(id) {
                Arc::make_mut(&mut values.measured).insert(id);
                if let (Err(_), Some(measurement)) =
                    (&value, measured.measurements_read(id).first())
                {
                    value = Err(ParameterError::Unmeasured {
                        measurement: feature_name(document, *measurement),
                    });
                }
            }
            Arc::make_mut(&mut values.entries).insert(
                id,
                Entry {
                    name: parameter.name.clone(),
                    value,
                },
            );
        }
        values
    }

    pub(crate) fn set_measured(
        &mut self,
        id: ParameterId,
        value: Result<Quantity, ParameterError>,
    ) {
        if let Some(name) = self.entry(id).map(|entry| entry.name.clone()) {
            self.readings.insert(id, Entry { name, value });
        }
    }

    pub fn take_readings(&mut self, from: &Self) {
        let taken: Vec<_> = self
            .measured
            .iter()
            .filter(|id| from.measured.contains(id))
            .filter_map(|id| Some((*id, from.entry(*id)?.value.clone())))
            .collect();

        for (id, reading) in taken {
            self.set_measured(id, reading);
        }
    }

    pub(crate) fn readings_among(&self, used: &BTreeSet<ParameterId>) -> Option<Self> {
        let measured: BTreeSet<ParameterId> = self.measured.intersection(used).copied().collect();
        let entries: BTreeMap<ParameterId, Entry> = measured
            .iter()
            .filter_map(|id| Some((*id, self.entry(*id)?.clone())))
            .collect();
        (!entries.is_empty()).then(|| Self {
            entries: Arc::new(entries),
            readings: BTreeMap::new(),
            measured: Arc::new(measured),
        })
    }

    pub fn get(&self, id: ParameterId) -> Option<&Result<Quantity, ParameterError>> {
        self.entry(id).map(|entry| &entry.value)
    }

    pub fn value(&self, id: ParameterId) -> Result<Quantity, EvalError> {
        let entry = self.entry(id).ok_or(EvalError::ParameterMissing)?;
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
                let value = self.entry(*id);
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
                self.entry(*id)
                    .map(|entry| entry.name.clone())
                    .unwrap_or_default()
            })
            .collect()
    }
}

pub(crate) fn evaluation_order(document: &Document) -> Vec<ParameterId> {
    EvaluationOrder::of(document).sequence
}

struct EvaluationOrder {
    sequence: Vec<ParameterId>,
    cycles: BTreeMap<ParameterId, Vec<ParameterId>>,
}

impl EvaluationOrder {
    fn of(document: &Document) -> Self {
        let dependencies = document.parameter_dependencies();

        tally::add(dependencies.len());

        let (_, unordered) = Self::topological(&dependencies, &BTreeMap::new());

        let cycles: BTreeMap<ParameterId, Vec<ParameterId>> = unordered
            .iter()
            .filter_map(|id| {
                let used = dependencies.get(id)?;
                path_to(*id, used.iter().copied(), &dependencies).map(|cycle| (*id, cycle))
            })
            .collect();
        let (mut sequence, rest) = Self::topological(&dependencies, &cycles);

        sequence.extend(rest);
        Self { sequence, cycles }
    }

    fn topological(
        dependencies: &BTreeMap<ParameterId, BTreeSet<ParameterId>>,
        cycles: &BTreeMap<ParameterId, Vec<ParameterId>>,
    ) -> (Vec<ParameterId>, Vec<ParameterId>) {
        let mut waiting: BTreeMap<ParameterId, usize> = BTreeMap::new();
        let mut dependents: BTreeMap<ParameterId, Vec<ParameterId>> = BTreeMap::new();

        for (id, used) in dependencies {
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
        let rest = waiting
            .keys()
            .filter(|id| !placed.contains(id))
            .copied()
            .collect();
        (sequence, rest)
    }
}
