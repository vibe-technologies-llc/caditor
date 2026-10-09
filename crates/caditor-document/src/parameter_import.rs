use std::collections::{BTreeMap, BTreeSet};

use caditor_expression::{Expression, NameError, ParameterId, ParseError, check_name};

use crate::{
    dependencies::DependencyGraph,
    document::{Document, Parameter},
    edit::{Edit, EditError, MAX_PARAMETER_NOTE_CHARS, Transaction},
    values::{ParameterError, ParameterValues},
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportedParameter {
    pub name: String,
    pub expression: String,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ImportRefusal {
    #[error("{0}")]
    Name(NameError),
    #[error("the name appears on an earlier row")]
    Repeated,
    #[error("its note is longer than {MAX_PARAMETER_NOTE_CHARS} characters")]
    NoteTooLong,
    #[error("its expression cannot be read: {0}")]
    Unreadable(ParseError),
    #[error("it depends on itself ({path})")]
    Cycle { path: String },
    #[error("it uses {name}, which is not imported")]
    UsesRefused { name: String },
    #[error("its value cannot be worked out: {0}")]
    Value(ParameterError),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImportOutcome {
    Added,
    Changed { before: String },
    Kept { theirs: String },
    Unchanged,
    Refused(ImportRefusal),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportRow {
    pub name: String,
    pub expression: String,
    pub outcome: ImportOutcome,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParameterImport {
    pub rows: Vec<ImportRow>,
    pub transaction: Option<Transaction>,
}

impl ParameterImport {
    pub fn count(&self, matches: impl Fn(&ImportOutcome) -> bool) -> usize {
        self.rows.iter().filter(|row| matches(&row.outcome)).count()
    }

    pub fn conflicts(&self) -> usize {
        self.count(|outcome| {
            matches!(
                outcome,
                ImportOutcome::Changed { .. } | ImportOutcome::Kept { .. }
            )
        })
    }
}

struct Candidate {
    row: usize,
    id: ParameterId,
    existing: Option<Expression>,
    expression: Expression,
}

impl Document {
    pub fn plan_parameter_import(
        &self,
        label: impl Into<String>,
        rows: &[ImportedParameter],
        replace_existing: bool,
    ) -> Result<ParameterImport, EditError> {
        let label = label.into();
        let mut outcomes: Vec<Option<ImportOutcome>> = vec![None; rows.len()];
        let mut ids: BTreeMap<String, ParameterId> = BTreeMap::new();
        let mut next_new = self.next_parameter_id();
        let mut new_ids = BTreeSet::new();
        for (index, row) in rows.iter().enumerate() {
            let name = row.name.trim();
            let refusal = if let Err(error) = check_name(name) {
                Some(ImportRefusal::Name(error))
            } else if ids.contains_key(name) {
                Some(ImportRefusal::Repeated)
            } else if row.note.chars().count() > MAX_PARAMETER_NOTE_CHARS {
                Some(ImportRefusal::NoteTooLong)
            } else {
                None
            };
            if let Some(refusal) = refusal {
                set_refused(&mut outcomes, index, refusal);
                continue;
            }
            let id = match self.parameter_named(name) {
                Some(existing) => existing.id(),
                None => {
                    let id = ParameterId::from_raw(next_new);
                    next_new = next_new.saturating_add(1);
                    new_ids.insert(id);
                    id
                }
            };
            ids.insert(name.to_owned(), id);
        }
        let resolve = |name: &str| {
            ids.get(name)
                .copied()
                .or_else(|| self.parameter_named(name).map(Parameter::id))
        };
        let mut candidates = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            if outcomes.get(index).is_some_and(Option::is_some) {
                continue;
            }
            let Some(id) = ids.get(row.name.trim()).copied() else {
                continue;
            };
            match Expression::parse(row.expression.trim(), &resolve) {
                Ok(expression) => {
                    let existing = self
                        .parameter(id)
                        .map(|parameter| parameter.expression.clone());
                    if let Some(existing) = &existing
                        && *existing != expression
                        && !replace_existing
                    {
                        set_outcome(
                            &mut outcomes,
                            index,
                            ImportOutcome::Kept {
                                theirs: row.expression.trim().to_owned(),
                            },
                        );
                        continue;
                    }
                    candidates.push(Candidate {
                        row: index,
                        id,
                        existing,
                        expression,
                    });
                }
                Err(error) => set_refused(&mut outcomes, index, ImportRefusal::Unreadable(error)),
            }
        }
        let name_of = |id: ParameterId| {
            ids.iter()
                .find(|(_, candidate)| **candidate == id)
                .map(|(name, _)| name.clone())
                .or_else(|| self.parameter_name(id).map(str::to_owned))
                .unwrap_or_else(|| "?".to_owned())
        };
        loop {
            let refused_ids: BTreeSet<ParameterId> = candidates
                .iter()
                .filter(|candidate| is_refused(&outcomes, candidate.row))
                .map(|candidate| candidate.id)
                .chain(
                    new_ids
                        .iter()
                        .copied()
                        .filter(|id| !candidates.iter().any(|candidate| candidate.id == *id)),
                )
                .collect();
            let mut refused_now = false;
            for candidate in &candidates {
                if is_refused(&outcomes, candidate.row) {
                    continue;
                }
                let missing = candidate
                    .expression
                    .parameters()
                    .into_iter()
                    .find(|used| refused_ids.contains(used) && new_ids.contains(used));
                if let Some(missing) = missing {
                    set_refused(
                        &mut outcomes,
                        candidate.row,
                        ImportRefusal::UsesRefused {
                            name: name_of(missing),
                        },
                    );
                    refused_now = true;
                }
            }
            let mut graph = DependencyGraph::of(self);
            for candidate in candidates
                .iter()
                .filter(|candidate| !is_refused(&outcomes, candidate.row))
            {
                graph.set(candidate.id, &candidate.expression);
            }
            for candidate in &candidates {
                if is_refused(&outcomes, candidate.row) {
                    continue;
                }
                if let Some(path) = graph.cycle(candidate.id, &candidate.expression) {
                    let path = path
                        .into_iter()
                        .map(name_of)
                        .collect::<Vec<_>>()
                        .join(" → ");
                    set_refused(&mut outcomes, candidate.row, ImportRefusal::Cycle { path });
                    refused_now = true;
                }
            }
            if refused_now {
                continue;
            }
            let transaction = self.import_transaction(label.clone(), rows, &candidates, &outcomes);
            let mut trial = self.clone();
            trial.apply(transaction.clone())?;
            let values = ParameterValues::evaluate(&trial);
            for candidate in &candidates {
                if is_refused(&outcomes, candidate.row) {
                    continue;
                }
                if let Some(Err(error)) = values.get(candidate.id) {
                    set_refused(
                        &mut outcomes,
                        candidate.row,
                        ImportRefusal::Value(error.clone()),
                    );
                    refused_now = true;
                }
            }
            if refused_now {
                continue;
            }
            for candidate in &candidates {
                if is_refused(&outcomes, candidate.row) {
                    continue;
                }
                let note_changes = rows
                    .get(candidate.row)
                    .is_some_and(|row| self.note_changes(candidate.id, &row.note));
                let outcome = match &candidate.existing {
                    None => ImportOutcome::Added,
                    Some(existing) if *existing == candidate.expression && !note_changes => {
                        ImportOutcome::Unchanged
                    }
                    Some(existing) => ImportOutcome::Changed {
                        before: existing.to_text(&|id| self.parameter_name(id)),
                    },
                };
                set_outcome(&mut outcomes, candidate.row, outcome);
            }
            let rows = rows
                .iter()
                .zip(outcomes)
                .map(|(row, outcome)| ImportRow {
                    name: row.name.trim().to_owned(),
                    expression: row.expression.trim().to_owned(),
                    outcome: outcome.unwrap_or(ImportOutcome::Unchanged),
                })
                .collect();
            let transaction = (!transaction.is_empty()).then_some(transaction);
            return Ok(ParameterImport { rows, transaction });
        }
    }

    fn note_changes(&self, id: ParameterId, note: &str) -> bool {
        let note = note.trim();
        !note.is_empty()
            && self
                .parameter(id)
                .is_some_and(|parameter| parameter.note != note)
    }

    fn import_transaction(
        &self,
        label: String,
        rows: &[ImportedParameter],
        candidates: &[Candidate],
        outcomes: &[Option<ImportOutcome>],
    ) -> Transaction {
        let accepted: Vec<&Candidate> = candidates
            .iter()
            .filter(|candidate| !is_refused(outcomes, candidate.row))
            .collect();
        let mut edits = Vec::new();
        let mut index = self.parameters().len();
        for candidate in accepted
            .iter()
            .filter(|candidate| candidate.existing.is_none())
        {
            let Some(row) = rows.get(candidate.row) else {
                continue;
            };
            let parameter = Parameter::new(
                candidate.id,
                row.name.trim().to_owned(),
                Expression::number(0.0),
            )
            .with_note(row.note.trim().to_owned());
            edits.push(Edit::InsertParameter { index, parameter });
            index += 1;
        }
        for candidate in accepted {
            if candidate.existing.as_ref() != Some(&candidate.expression) {
                edits.push(Edit::SetParameterExpression {
                    id: candidate.id,
                    expression: candidate.expression.clone(),
                });
            }
            if let Some(row) = rows.get(candidate.row)
                && candidate.existing.is_some()
                && self.note_changes(candidate.id, &row.note)
            {
                edits.push(Edit::SetParameterNote {
                    id: candidate.id,
                    note: row.note.trim().to_owned(),
                });
            }
        }
        Transaction::new(label, edits)
    }
}

fn is_refused(outcomes: &[Option<ImportOutcome>], row: usize) -> bool {
    matches!(outcomes.get(row), Some(Some(ImportOutcome::Refused(_))))
}

fn set_refused(outcomes: &mut [Option<ImportOutcome>], row: usize, refusal: ImportRefusal) {
    set_outcome(outcomes, row, ImportOutcome::Refused(refusal));
}

fn set_outcome(outcomes: &mut [Option<ImportOutcome>], row: usize, outcome: ImportOutcome) {
    if let Some(slot) = outcomes.get_mut(row) {
        *slot = Some(outcome);
    }
}
