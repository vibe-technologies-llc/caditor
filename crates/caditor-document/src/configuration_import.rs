use std::{borrow::Cow, collections::BTreeMap};

use caditor_expression::{Expression, ParseError};

use crate::{
    body_appearance::Rgb,
    configurations::{
        Configuration, ConfigurationId, ConfiguredValue, MAX_CONFIGURATION_NAME_CHARS,
        MAX_CONFIGURATIONS, MAX_CONFIGURED_VALUES, Setting, configuration_name,
    },
    dependencies::DependencyGraph,
    document::{Document, Parameter},
    edit::{Edit, EditError, Transaction},
    values::{ParameterError, ParameterValues},
};

const HEADER_SEPARATOR: char = ':';
const DEFAULT_COLOUR: &str = "default";
const YES: &str = "yes";
const NO: &str = "no";
const YES_SPELLINGS: [&str; 5] = [YES, "y", "true", "1", "on"];
const NO_SPELLINGS: [&str; 5] = [NO, "n", "false", "0", "off"];
const DEFAULT_COLOUR_SPELLINGS: [&str; 2] = [DEFAULT_COLOUR, "none"];
const UNKNOWN_PATH_NAME: &str = "?";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnKind {
    Parameter,
    Suppressed,
    Colour,
}

impl ColumnKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Parameter => "Parameter",
            Self::Suppressed => "Suppressed",
            Self::Colour => "Colour",
        }
    }

    fn of(text: &str) -> Option<Self> {
        match text.trim().to_lowercase().as_str() {
            "parameter" => Some(Self::Parameter),
            "suppressed" => Some(Self::Suppressed),
            "colour" | "color" => Some(Self::Colour),
            _ => None,
        }
    }

    fn of_value(value: ConfiguredValue) -> Self {
        match value {
            ConfiguredValue::Parameter(_) => Self::Parameter,
            ConfiguredValue::Suppressed(_) => Self::Suppressed,
            ConfiguredValue::Colour(_) => Self::Colour,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedColumn {
    pub header: String,
    pub kind: Option<ColumnKind>,
    pub name: String,
}

impl ImportedColumn {
    pub fn from_header(header: &str) -> Self {
        let header = header.trim();
        let parsed = header
            .split_once(HEADER_SEPARATOR)
            .and_then(|(kind, name)| Some((ColumnKind::of(kind)?, name.trim())));
        match parsed {
            Some((kind, name)) => Self {
                header: header.to_owned(),
                kind: Some(kind),
                name: name.to_owned(),
            },
            None => Self {
                header: header.to_owned(),
                kind: None,
                name: header.to_owned(),
            },
        }
    }

    pub fn header_of(kind: ColumnKind, name: &str) -> String {
        format!("{}{HEADER_SEPARATOR} {name}", kind.label())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportedConfiguration {
    pub name: String,
    pub cells: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportedConfigurations {
    pub columns: Vec<ImportedColumn>,
    pub rows: Vec<ImportedConfiguration>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ColumnRefusal {
    #[error("its heading does not start with “Parameter:”, “Suppressed:” or “Colour:”")]
    UnknownKind,
    #[error("the model has no parameter named “{name}”")]
    MissingParameter { name: String },
    #[error("the model has no feature named “{name}”")]
    MissingFeature { name: String },
    #[error("the model has no body named “{name}”")]
    MissingBody { name: String },
    #[error("“{name}” is not a feature that makes a body, so it has no colour to configure")]
    NotABody { name: String },
    #[error("more than one body is named “{name}”")]
    AmbiguousBody { name: String },
    #[error("an earlier column already sets the same value")]
    Repeated,
    #[error("a model configures at most {MAX_CONFIGURED_VALUES} values")]
    TooManyValues,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ColumnOutcome {
    Known,
    Added,
    Refused(ColumnRefusal),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportColumn {
    pub header: String,
    pub outcome: ColumnOutcome,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CellProblem {
    #[error("“{text}” cannot be read as an expression: {error}")]
    Expression { text: String, error: ParseError },
    #[error("“{text}” is not yes or no")]
    NotYesNo { text: String },
    #[error("“{text}” is not a colour; write it as #rrggbb, or default")]
    NotColour { text: String },
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RowRefusal {
    #[error("it has no configuration name")]
    NameMissing,
    #[error("its name is longer than {MAX_CONFIGURATION_NAME_CHARS} characters")]
    NameTooLong,
    #[error("the name appears on an earlier row")]
    Repeated,
    #[error("a model keeps at most {MAX_CONFIGURATIONS} configurations")]
    TooMany,
    #[error("{column}: {problem}")]
    Cell {
        column: String,
        problem: CellProblem,
    },
    #[error("its parameters depend on each other in a circle ({path})")]
    Cycle { path: String },
    #[error("{column} cannot be worked out: {error}")]
    Value {
        column: String,
        error: ParameterError,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellChange {
    pub column: String,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RowOutcome {
    Added { values: usize },
    Changed { changes: Vec<CellChange> },
    Kept { changes: Vec<CellChange> },
    Unchanged,
    Refused(RowRefusal),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportRowPlan {
    pub name: String,
    pub outcome: RowOutcome,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConfigurationImport {
    pub columns: Vec<ImportColumn>,
    pub rows: Vec<ImportRowPlan>,
    pub transaction: Option<Transaction>,
}

impl ConfigurationImport {
    pub fn count_rows(&self, matches: impl Fn(&RowOutcome) -> bool) -> usize {
        self.rows.iter().filter(|row| matches(&row.outcome)).count()
    }

    pub fn count_columns(&self, matches: impl Fn(&ColumnOutcome) -> bool) -> usize {
        self.columns
            .iter()
            .filter(|column| matches(&column.outcome))
            .count()
    }

    pub fn conflicts(&self) -> usize {
        self.count_rows(|outcome| {
            matches!(
                outcome,
                RowOutcome::Changed { .. } | RowOutcome::Kept { .. }
            )
        })
    }
}

struct Cell {
    value: ConfiguredValue,
    setting: Setting,
    column: String,
}

struct Prepared {
    index: usize,
    name: String,
    existing: Option<ConfigurationId>,
    cells: Vec<Cell>,
    changes: Vec<CellChange>,
}

impl Prepared {
    fn writes(&self, replace_existing: bool) -> bool {
        match self.existing {
            None => true,
            Some(_) => replace_existing && !self.changes.is_empty(),
        }
    }
}

impl Document {
    pub fn configuration_column_header(&self, value: ConfiguredValue) -> Option<String> {
        let name = match value {
            ConfiguredValue::Parameter(id) => Cow::Borrowed(self.parameter_name(id)?),
            ConfiguredValue::Suppressed(id) => Cow::Borrowed(self.feature(id)?.name.as_str()),
            ConfiguredValue::Colour(id) => self.body_name(id)?,
        };
        Some(ImportedColumn::header_of(
            ColumnKind::of_value(value),
            &name,
        ))
    }

    pub fn configuration_cell_text(
        &self,
        id: ConfigurationId,
        value: ConfiguredValue,
    ) -> Option<String> {
        Some(self.setting_text(&self.configuration_setting(id, value)?))
    }

    fn setting_text(&self, setting: &Setting) -> String {
        match setting {
            Setting::Expression(expression) => self.expression_text(expression),
            Setting::Suppressed(true) => YES.to_owned(),
            Setting::Suppressed(false) => NO.to_owned(),
            Setting::Colour(Some(colour)) => colour.hex(),
            Setting::Colour(None) => DEFAULT_COLOUR.to_owned(),
        }
    }

    fn parse_cell(&self, value: ConfiguredValue, text: &str) -> Result<Setting, CellProblem> {
        let text = text.trim();
        let lowered = text.to_lowercase();
        match value {
            ConfiguredValue::Parameter(_) => {
                let resolve = |name: &str| self.parameter_named(name).map(Parameter::id);
                Expression::parse(text, &resolve)
                    .map(Setting::Expression)
                    .map_err(|error| CellProblem::Expression {
                        text: text.to_owned(),
                        error,
                    })
            }
            ConfiguredValue::Suppressed(_) => {
                if YES_SPELLINGS.contains(&lowered.as_str()) {
                    Ok(Setting::Suppressed(true))
                } else if NO_SPELLINGS.contains(&lowered.as_str()) {
                    Ok(Setting::Suppressed(false))
                } else {
                    Err(CellProblem::NotYesNo {
                        text: text.to_owned(),
                    })
                }
            }
            ConfiguredValue::Colour(_) => {
                if DEFAULT_COLOUR_SPELLINGS.contains(&lowered.as_str()) {
                    return Ok(Setting::Colour(None));
                }
                Rgb::from_hex(text)
                    .map(|colour| Setting::Colour(Some(colour)))
                    .ok_or_else(|| CellProblem::NotColour {
                        text: text.to_owned(),
                    })
            }
        }
    }

    fn resolve_import_columns(
        &self,
        columns: &[ImportedColumn],
    ) -> Vec<Result<ConfiguredValue, ColumnRefusal>> {
        let table = self.configurations();
        let mut resolved: Vec<ConfiguredValue> = Vec::new();
        let mut added = 0;
        columns
            .iter()
            .map(|column| {
                let value = self.resolve_import_column(column)?;
                if resolved.contains(&value) {
                    return Err(ColumnRefusal::Repeated);
                }
                if !table.configures(value) {
                    if table.values.len() + added >= MAX_CONFIGURED_VALUES {
                        return Err(ColumnRefusal::TooManyValues);
                    }
                    added += 1;
                }
                resolved.push(value);
                Ok(value)
            })
            .collect()
    }

    fn resolve_import_column(
        &self,
        column: &ImportedColumn,
    ) -> Result<ConfiguredValue, ColumnRefusal> {
        let name = column.name.as_str();
        let owned = || name.to_owned();
        match column.kind.ok_or(ColumnRefusal::UnknownKind)? {
            ColumnKind::Parameter => self
                .parameter_named(name)
                .map(|parameter| ConfiguredValue::Parameter(parameter.id()))
                .ok_or_else(|| ColumnRefusal::MissingParameter { name: owned() }),
            ColumnKind::Suppressed => self
                .features()
                .find(|feature| feature.name == name)
                .map(|feature| ConfiguredValue::Suppressed(feature.id()))
                .ok_or_else(|| ColumnRefusal::MissingFeature { name: owned() }),
            ColumnKind::Colour => {
                let bodies: Vec<_> = self
                    .features()
                    .filter(|feature| feature.makes_body())
                    .filter(|feature| self.body_name(feature.id()).as_deref() == Some(name))
                    .collect();
                match bodies.as_slice() {
                    [body] => Ok(ConfiguredValue::Colour(body.id())),
                    [] if self.features().any(|feature| feature.name == name) => {
                        Err(ColumnRefusal::NotABody { name: owned() })
                    }
                    [] => Err(ColumnRefusal::MissingBody { name: owned() }),
                    _ => Err(ColumnRefusal::AmbiguousBody { name: owned() }),
                }
            }
        }
    }

    fn prepare_import_row(
        &self,
        index: usize,
        name: String,
        row: &ImportedConfiguration,
        columns: &[ImportedColumn],
        resolved: &[Result<ConfiguredValue, ColumnRefusal>],
    ) -> Result<Prepared, RowRefusal> {
        let table = self.configurations();
        let existing = table.position(&name).and_then(|at| table.rows.get(at));
        let mut cells = Vec::new();
        let mut changes = Vec::new();
        for (position, resolved) in resolved.iter().enumerate() {
            let Ok(value) = resolved else {
                continue;
            };
            let Some(text) = row.cells.get(position).map(|text| text.trim()) else {
                continue;
            };
            if text.is_empty() {
                continue;
            }
            let header = columns
                .get(position)
                .map(|column| column.header.clone())
                .unwrap_or_default();
            let setting = self
                .parse_cell(*value, text)
                .map_err(|problem| RowRefusal::Cell {
                    column: header.clone(),
                    problem,
                })?;
            let before = existing
                .and_then(|row| self.configuration_setting(row.id, *value))
                .or_else(|| self.live_setting(*value));
            if existing.is_some() && before.as_ref() != Some(&setting) {
                changes.push(CellChange {
                    column: header.clone(),
                    before: before
                        .as_ref()
                        .map(|before| self.setting_text(before))
                        .unwrap_or_default(),
                    after: self.setting_text(&setting),
                });
            }
            cells.push(Cell {
                value: *value,
                setting,
                column: header,
            });
        }
        Ok(Prepared {
            index,
            name,
            existing: existing.map(|row| row.id),
            cells,
            changes,
        })
    }

    fn final_import_settings(
        &self,
        prepared: &Prepared,
        values: &[ConfiguredValue],
    ) -> BTreeMap<ConfiguredValue, Setting> {
        values
            .iter()
            .filter_map(|value| {
                let imported = prepared.cells.iter().find(|cell| cell.value == *value);
                let setting = match (imported, prepared.existing) {
                    (Some(cell), _) => cell.setting.clone(),
                    (None, Some(id)) => self
                        .configuration_setting(id, *value)
                        .or_else(|| self.live_setting(*value))?,
                    (None, None) => self.live_setting(*value)?,
                };
                Some((*value, setting))
            })
            .collect()
    }

    fn import_cycle(&self, settings: &BTreeMap<ConfiguredValue, Setting>) -> Option<String> {
        let cells: Vec<_> = settings
            .iter()
            .filter_map(|(value, setting)| Some((value.parameter()?, setting.expression()?)))
            .collect();
        let mut graph = DependencyGraph::of(self);
        for (id, expression) in &cells {
            graph.set(*id, expression);
        }
        cells.iter().find_map(|(id, expression)| {
            let cycle = graph.cycle(*id, expression)?;
            let names: Vec<&str> = cycle
                .iter()
                .map(|step| self.parameter_name(*step).unwrap_or(UNKNOWN_PATH_NAME))
                .collect();
            Some(names.join(" → "))
        })
    }

    fn configuration_import_transaction(
        &self,
        label: &str,
        new_values: &[ConfiguredValue],
        written: &[&Prepared],
    ) -> Result<Transaction, EditError> {
        let mut table = crate::configurations::Configurations::clone(self.configurations());
        for value in new_values {
            let Some(live) = self.live_setting(*value) else {
                continue;
            };
            table.values.push(*value);
            for row in &mut table.rows {
                row.settings.insert(*value, live.clone());
            }
        }
        let mut active_wanted: Option<Configuration> = None;
        for prepared in written {
            let id = match prepared.existing {
                Some(id) => id,
                None => {
                    let id = table.fresh_id();
                    table.rows.push(Configuration {
                        id,
                        name: prepared.name.clone(),
                        settings: self.live_settings_of(&table.values),
                    });
                    id
                }
            };
            if let Some(row) = table.rows.iter_mut().find(|row| row.id == id) {
                for cell in &prepared.cells {
                    row.settings.insert(cell.value, cell.setting.clone());
                }
                if table.active == Some(id) {
                    active_wanted = Some(Configuration {
                        id,
                        name: prepared.name.clone(),
                        settings: self.final_import_settings(prepared, &table.values),
                    });
                }
            }
        }
        let table_edit = Edit::SetConfigurations {
            configurations: Box::new(table),
        };
        let mut edits = vec![table_edit.clone()];
        if let Some(wanted) = active_wanted {
            let mut trial = self.clone();
            trial.apply(Transaction::single(label, table_edit))?;
            edits.extend(trial.applying(&wanted)?);
        }
        Ok(Transaction::new(label, edits))
    }

    fn refused_by_value(
        &self,
        trial: &Document,
        prepared: &[&Prepared],
    ) -> Vec<(usize, RowRefusal)> {
        let mut refused = Vec::new();
        for row in prepared {
            let parameters: Vec<_> = row
                .cells
                .iter()
                .filter_map(|cell| Some((cell.value.parameter()?, &cell.column)))
                .collect();
            if parameters.is_empty() {
                continue;
            }
            let Some(id) = trial
                .configurations()
                .position(&row.name)
                .and_then(|at| trial.configurations().rows.get(at))
                .map(|found| found.id)
            else {
                continue;
            };
            let mut probe = trial.clone();
            if !trial.configurations().is_active(id) {
                let switched = trial
                    .activating(id)
                    .and_then(|switch| probe.apply(switch).map(|_| ()));
                if switched.is_err() {
                    continue;
                }
            }
            let values = ParameterValues::evaluate(&probe);
            let failed =
                parameters
                    .iter()
                    .find_map(|(parameter, column)| match values.get(*parameter)? {
                        Err(error) => Some(RowRefusal::Value {
                            column: (*column).clone(),
                            error: error.clone(),
                        }),
                        Ok(_) => None,
                    });
            if let Some(refusal) = failed {
                refused.push((row.index, refusal));
            }
        }
        refused
    }

    pub fn plan_configuration_import(
        &self,
        label: impl Into<String>,
        file: &ImportedConfigurations,
        replace_existing: bool,
    ) -> Result<ConfigurationImport, EditError> {
        let label = label.into();
        let table = self.configurations();
        let resolved = self.resolve_import_columns(&file.columns);
        let mut outcomes: Vec<Option<RowOutcome>> = vec![None; file.rows.len()];
        let mut prepared: Vec<Prepared> = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        let mut new_rows = 0;
        for (index, row) in file.rows.iter().enumerate() {
            let name = configuration_name(&row.name);
            let length = name.chars().count();
            let refusal = if length == 0 {
                Some(RowRefusal::NameMissing)
            } else if length > MAX_CONFIGURATION_NAME_CHARS {
                Some(RowRefusal::NameTooLong)
            } else if seen
                .iter()
                .any(|other| other.to_lowercase() == name.to_lowercase())
            {
                Some(RowRefusal::Repeated)
            } else if !table.is_taken(&name) && table.rows.len() + new_rows >= MAX_CONFIGURATIONS {
                Some(RowRefusal::TooMany)
            } else {
                None
            };
            if let Some(refusal) = refusal {
                set_outcome(&mut outcomes, index, RowOutcome::Refused(refusal));
                continue;
            }
            seen.push(name.clone());
            match self.prepare_import_row(index, name, row, &file.columns, &resolved) {
                Ok(ready) => {
                    new_rows += usize::from(ready.existing.is_none());
                    prepared.push(ready);
                }
                Err(refusal) => set_outcome(&mut outcomes, index, RowOutcome::Refused(refusal)),
            }
        }
        let new_values: Vec<ConfiguredValue> = resolved
            .iter()
            .flatten()
            .copied()
            .filter(|value| !table.configures(*value))
            .collect();
        let all_values: Vec<ConfiguredValue> = table
            .values
            .iter()
            .copied()
            .chain(new_values.iter().copied())
            .collect();
        for ready in &prepared {
            let settings = self.final_import_settings(ready, &all_values);
            if let Some(path) = self.import_cycle(&settings) {
                set_outcome(
                    &mut outcomes,
                    ready.index,
                    RowOutcome::Refused(RowRefusal::Cycle { path }),
                );
            }
        }
        prepared.retain(|ready| !is_refused(&outcomes, ready.index));
        let (transaction, changes_the_model) = loop {
            let written: Vec<&Prepared> = prepared
                .iter()
                .filter(|ready| ready.writes(replace_existing))
                .collect();
            let effective: &[ConfiguredValue] = if prepared.is_empty() {
                &[]
            } else {
                &new_values
            };
            let transaction = self.configuration_import_transaction(&label, effective, &written)?;
            let mut trial = self.clone();
            trial.apply(transaction.clone())?;
            let refused = self.refused_by_value(&trial, &written);
            if refused.is_empty() {
                let changes = !self.same_content(&trial);
                break (transaction, changes);
            }
            for (index, refusal) in refused {
                set_outcome(&mut outcomes, index, RowOutcome::Refused(refusal));
            }
            prepared.retain(|ready| !is_refused(&outcomes, ready.index));
        };
        for ready in &prepared {
            let outcome = match ready.existing {
                None => RowOutcome::Added {
                    values: ready.cells.len(),
                },
                Some(_) if ready.changes.is_empty() => RowOutcome::Unchanged,
                Some(_) if replace_existing => RowOutcome::Changed {
                    changes: ready.changes.clone(),
                },
                Some(_) => RowOutcome::Kept {
                    changes: ready.changes.clone(),
                },
            };
            set_outcome(&mut outcomes, ready.index, outcome);
        }
        let columns = file
            .columns
            .iter()
            .zip(&resolved)
            .map(|(column, resolved)| ImportColumn {
                header: column.header.clone(),
                outcome: match resolved {
                    Err(refusal) => ColumnOutcome::Refused(refusal.clone()),
                    Ok(value) if table.configures(*value) => ColumnOutcome::Known,
                    Ok(_) => ColumnOutcome::Added,
                },
            })
            .collect();
        let rows = file
            .rows
            .iter()
            .zip(outcomes)
            .map(|(row, outcome)| ImportRowPlan {
                name: configuration_name(&row.name),
                outcome: outcome.unwrap_or(RowOutcome::Unchanged),
            })
            .collect();
        Ok(ConfigurationImport {
            columns,
            rows,
            transaction: changes_the_model.then_some(transaction),
        })
    }
}

fn is_refused(outcomes: &[Option<RowOutcome>], index: usize) -> bool {
    matches!(outcomes.get(index), Some(Some(RowOutcome::Refused(_))))
}

fn set_outcome(outcomes: &mut [Option<RowOutcome>], index: usize, outcome: RowOutcome) {
    if let Some(slot) = outcomes.get_mut(index) {
        *slot = Some(outcome);
    }
}
