use std::collections::BTreeMap;

use caditor_expression::{Expression, ParameterId};

use crate::{
    body_appearance::Rgb,
    document::{Document, FeatureId},
    edit::{Edit, EditError, Transaction},
    views::view_name,
};

pub const MAX_CONFIGURATIONS: usize = 256;
pub const MAX_CONFIGURATION_NAME_CHARS: usize = 60;
pub const MAX_CONFIGURED_VALUES: usize = 256;
const NUMBERED_NAME: &str = "Configuration";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConfigurationId(u64);

impl ConfigurationId {
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ConfiguredValue {
    Parameter(ParameterId),
    Suppressed(FeatureId),
    Colour(FeatureId),
}

impl ConfiguredValue {
    pub fn parameter(self) -> Option<ParameterId> {
        match self {
            Self::Parameter(id) => Some(id),
            Self::Suppressed(_) | Self::Colour(_) => None,
        }
    }

    pub fn feature(self) -> Option<FeatureId> {
        match self {
            Self::Suppressed(id) | Self::Colour(id) => Some(id),
            Self::Parameter(_) => None,
        }
    }

    fn fits(self, setting: &Setting) -> bool {
        matches!(
            (self, setting),
            (Self::Parameter(_), Setting::Expression(_))
                | (Self::Suppressed(_), Setting::Suppressed(_))
                | (Self::Colour(_), Setting::Colour(_))
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Setting {
    Expression(Expression),
    Suppressed(bool),
    Colour(Option<Rgb>),
}

impl Setting {
    pub fn expression(&self) -> Option<&Expression> {
        match self {
            Self::Expression(expression) => Some(expression),
            Self::Suppressed(_) | Self::Colour(_) => None,
        }
    }

    fn heap_size(&self) -> usize {
        match self {
            Self::Expression(expression) => expression.heap_size(),
            Self::Suppressed(_) | Self::Colour(_) => 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Configuration {
    pub id: ConfigurationId,
    pub name: String,
    pub settings: BTreeMap<ConfiguredValue, Setting>,
}

impl Configuration {
    pub fn setting(&self, value: ConfiguredValue) -> Option<&Setting> {
        self.settings.get(&value)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Configurations {
    pub values: Vec<ConfiguredValue>,
    pub rows: Vec<Configuration>,
    pub active: Option<ConfigurationId>,
    pub next_id: u64,
}

pub fn configuration_name(text: &str) -> String {
    view_name(text)
}

fn same_name(left: &str, right: &str) -> bool {
    left.to_lowercase() == right.to_lowercase()
}

impl Configurations {
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty() && self.values.is_empty()
    }

    pub fn row(&self, id: ConfigurationId) -> Option<&Configuration> {
        self.rows.iter().find(|row| row.id == id)
    }

    pub fn active_row(&self) -> Option<&Configuration> {
        self.row(self.active?)
    }

    pub fn is_active(&self, id: ConfigurationId) -> bool {
        self.active == Some(id)
    }

    pub fn position(&self, name: &str) -> Option<usize> {
        self.rows.iter().position(|row| same_name(&row.name, name))
    }

    pub fn is_taken(&self, name: &str) -> bool {
        self.position(name).is_some()
    }

    pub fn unused_name(&self) -> String {
        (self.rows.len() + 1..)
            .map(|number| format!("{NUMBERED_NAME} {number}"))
            .find(|name| !self.is_taken(name))
            .unwrap_or_else(|| NUMBERED_NAME.to_owned())
    }

    pub fn copy_name(&self, name: &str) -> String {
        let first = format!("{name} copy");
        if !self.is_taken(&first) {
            return first;
        }
        (2..)
            .map(|number| format!("{name} copy {number}"))
            .find(|candidate| !self.is_taken(candidate))
            .unwrap_or(first)
    }

    pub fn configures(&self, value: ConfiguredValue) -> bool {
        self.values.contains(&value)
    }

    pub fn same_content(&self, other: &Self) -> bool {
        self.values == other.values && self.rows == other.rows && self.active == other.active
    }

    pub fn heap_size(&self) -> usize {
        size_of_val(self.values.as_slice())
            + self
                .rows
                .iter()
                .map(|row| {
                    size_of::<Configuration>()
                        + row.name.len()
                        + row
                            .settings
                            .values()
                            .map(|setting| {
                                size_of::<(ConfiguredValue, Setting)>() + setting.heap_size()
                            })
                            .sum::<usize>()
                })
                .sum::<usize>()
    }

    fn fresh_id(&mut self) -> ConfigurationId {
        let id = ConfigurationId(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    pub(crate) fn parameters_named(&self) -> impl Iterator<Item = ParameterId> + '_ {
        let columns = self.values.iter().filter_map(|value| value.parameter());
        let used = self.rows.iter().flat_map(|row| {
            row.settings
                .values()
                .filter_map(Setting::expression)
                .flat_map(Expression::parameters)
        });
        columns.chain(used)
    }

    pub(crate) fn features_named(&self) -> impl Iterator<Item = FeatureId> + '_ {
        self.values.iter().filter_map(|value| value.feature())
    }

    pub(crate) fn checked(mut self) -> Result<Self, EditError> {
        if self.rows.len() > MAX_CONFIGURATIONS {
            return Err(EditError::TooManyConfigurations);
        }
        if self.values.len() > MAX_CONFIGURED_VALUES {
            return Err(EditError::TooManyConfiguredValues);
        }
        let mut columns = Vec::with_capacity(self.values.len());
        for value in self.values {
            if !columns.contains(&value) {
                columns.push(value);
            }
        }
        self.values = columns;
        let mut seen = Vec::with_capacity(self.rows.len());
        for row in &mut self.rows {
            row.name = configuration_name(&row.name);
            let length = row.name.chars().count();
            if length == 0 {
                return Err(EditError::ConfigurationNameEmpty);
            }
            if length > MAX_CONFIGURATION_NAME_CHARS {
                return Err(EditError::ConfigurationNameTooLong { length });
            }
            if seen
                .iter()
                .any(|other: &String| same_name(other, &row.name))
            {
                return Err(EditError::ConfigurationNameTaken(row.name.clone()));
            }
            seen.push(row.name.clone());
            let values = &self.values;
            row.settings
                .retain(|value, setting| values.contains(value) && value.fits(setting));
        }
        let ids: Vec<ConfigurationId> = self.rows.iter().map(|row| row.id).collect();
        for (index, id) in ids.iter().enumerate() {
            if ids.iter().take(index).any(|other| other == id) {
                return Err(EditError::DuplicateId);
            }
        }
        if let Some(active) = self.active
            && !ids.contains(&active)
        {
            return Err(EditError::MissingConfiguration);
        }
        let beyond = ids
            .iter()
            .map(|id| id.raw().saturating_add(1))
            .max()
            .unwrap_or(0);
        self.next_id = self.next_id.max(beyond);
        Ok(self)
    }
}

impl Document {
    pub fn configurations(&self) -> &Configurations {
        &self.configurations
    }

    pub fn active_configuration(&self) -> Option<&Configuration> {
        self.configurations.active_row()
    }

    pub fn live_setting(&self, value: ConfiguredValue) -> Option<Setting> {
        match value {
            ConfiguredValue::Parameter(id) => self
                .parameter(id)
                .map(|parameter| Setting::Expression(parameter.expression.clone())),
            ConfiguredValue::Suppressed(id) => self
                .feature(id)
                .map(|feature| Setting::Suppressed(feature.suppressed)),
            ConfiguredValue::Colour(id) => self
                .feature(id)
                .filter(|feature| feature.makes_body())
                .map(|feature| Setting::Colour(feature.appearance.colour)),
        }
    }

    pub fn configuration_setting(
        &self,
        id: ConfigurationId,
        value: ConfiguredValue,
    ) -> Option<Setting> {
        if self.configurations.is_active(id) {
            return self.live_setting(value);
        }
        self.configurations.row(id)?.setting(value).cloned()
    }

    pub(crate) fn live_settings(&self) -> BTreeMap<ConfiguredValue, Setting> {
        self.live_settings_of(&self.configurations.values)
    }

    pub(crate) fn live_settings_of(
        &self,
        values: &[ConfiguredValue],
    ) -> BTreeMap<ConfiguredValue, Setting> {
        values
            .iter()
            .filter_map(|value| Some((*value, self.live_setting(*value)?)))
            .collect()
    }

    pub(crate) fn out_of_step(&self, row: &Configuration) -> Option<ConfiguredValue> {
        self.configurations.values.iter().copied().find(|value| {
            self.live_setting(*value)
                .is_some_and(|live| row.setting(*value) != Some(&live))
        })
    }

    pub fn activating(&self, id: ConfigurationId) -> Result<Transaction, EditError> {
        let row = self
            .configurations
            .row(id)
            .ok_or(EditError::MissingConfiguration)?;
        let label = format!("Switch to {}", row.name);
        if self.configurations.is_active(id) {
            return Ok(Transaction::new(label, Vec::new()));
        }
        let mut edits = vec![Edit::SetActiveConfiguration { active: None }];
        edits.extend(self.applying(row)?);
        edits.push(Edit::SetActiveConfiguration { active: Some(id) });
        Ok(Transaction::new(label, edits))
    }

    fn applying(&self, row: &Configuration) -> Result<Vec<Edit>, EditError> {
        let changed: Vec<(ConfiguredValue, &Setting)> = self
            .configurations
            .values
            .iter()
            .filter_map(|value| {
                let live = self.live_setting(*value)?;
                let wanted = row.setting(*value)?;
                (wanted != &live).then_some((*value, wanted))
            })
            .collect();
        let cleared = changed.iter().filter_map(|(value, setting)| {
            let id = value.parameter()?;
            let expression = setting.expression()?;
            (!expression.parameters().is_empty()).then_some(Edit::SetParameterExpression {
                id,
                expression: Expression::Number(0.0),
            })
        });
        let mut edits: Vec<Edit> = cleared.collect();
        let (plain, using): (Vec<_>, Vec<_>) = changed.into_iter().partition(|(_, setting)| {
            setting
                .expression()
                .is_none_or(|expression| expression.parameters().is_empty())
        });
        for (value, setting) in plain.into_iter().chain(using) {
            edits.push(self.setting_edit(value, setting.clone())?);
        }
        Ok(edits)
    }

    pub(crate) fn setting_edit(
        &self,
        value: ConfiguredValue,
        setting: Setting,
    ) -> Result<Edit, EditError> {
        match (value, setting) {
            (ConfiguredValue::Parameter(id), Setting::Expression(expression)) => {
                Ok(Edit::SetParameterExpression { id, expression })
            }
            (ConfiguredValue::Suppressed(id), Setting::Suppressed(suppressed)) => {
                Ok(Edit::SetFeatureSuppressed { id, suppressed })
            }
            (ConfiguredValue::Colour(id), Setting::Colour(colour)) => {
                let feature = self.feature(id).ok_or(EditError::MissingFeature)?;
                let mut appearance = feature.appearance.clone();
                appearance.colour = colour;
                Ok(Edit::SetBodyAppearance { id, appearance })
            }
            _ => Err(EditError::SettingMismatch),
        }
    }

    fn with_table(&self, label: String, table: Configurations) -> Transaction {
        Transaction::single(
            label,
            Edit::SetConfigurations {
                configurations: Box::new(table),
            },
        )
    }

    pub fn adding_configuration(&self, name: Option<&str>) -> (Transaction, ConfigurationId) {
        let mut table = Configurations::clone(&self.configurations);
        let id = table.fresh_id();
        let name = name
            .map(configuration_name)
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| table.unused_name());
        let label = format!("Add configuration {name}");
        table.rows.push(Configuration {
            id,
            name,
            settings: self.live_settings(),
        });
        if table.active.is_none() {
            table.active = Some(id);
        }
        (self.with_table(label, table), id)
    }

    pub fn duplicating_configuration(
        &self,
        source: ConfigurationId,
    ) -> Result<(Transaction, ConfigurationId), EditError> {
        let mut table = Configurations::clone(&self.configurations);
        let original = table.row(source).ok_or(EditError::MissingConfiguration)?;
        let name = table.copy_name(&original.name);
        let settings = if table.is_active(source) {
            self.live_settings()
        } else {
            original.settings.clone()
        };
        let label = format!("Duplicate {}", original.name);
        let index = table
            .rows
            .iter()
            .position(|row| row.id == source)
            .map_or(table.rows.len(), |index| index + 1);
        let id = table.fresh_id();
        table
            .rows
            .insert(index, Configuration { id, name, settings });
        Ok((self.with_table(label, table), id))
    }

    pub fn renaming_configuration(
        &self,
        id: ConfigurationId,
        name: &str,
    ) -> Result<Transaction, EditError> {
        let mut table = Configurations::clone(&self.configurations);
        let row = table
            .rows
            .iter_mut()
            .find(|row| row.id == id)
            .ok_or(EditError::MissingConfiguration)?;
        let label = format!("Rename {}", row.name);
        row.name = configuration_name(name);
        Ok(self.with_table(label, table))
    }

    pub fn moving_configuration(
        &self,
        id: ConfigurationId,
        index: usize,
    ) -> Result<Transaction, EditError> {
        let mut table = Configurations::clone(&self.configurations);
        let from = table
            .rows
            .iter()
            .position(|row| row.id == id)
            .ok_or(EditError::MissingConfiguration)?;
        let row = table.rows.remove(from);
        let label = format!("Move {}", row.name);
        table.rows.insert(index.min(table.rows.len()), row);
        Ok(self.with_table(label, table))
    }

    pub fn deleting_configuration(&self, id: ConfigurationId) -> Result<Transaction, EditError> {
        let row = self
            .configurations
            .row(id)
            .ok_or(EditError::MissingConfiguration)?;
        let label = format!("Delete configuration {}", row.name);
        let mut edits = Vec::new();
        let mut table = Configurations::clone(&self.configurations);
        if table.is_active(id) {
            let next = table.rows.iter().find(|other| other.id != id);
            edits.push(Edit::SetActiveConfiguration { active: None });
            if let Some(next) = next {
                edits.extend(self.applying(next)?);
                edits.push(Edit::SetActiveConfiguration {
                    active: Some(next.id),
                });
            }
            table.active = next.map(|next| next.id);
        }
        table.rows.retain(|row| row.id != id);
        edits.push(Edit::SetConfigurations {
            configurations: Box::new(table),
        });
        Ok(Transaction::new(label, edits))
    }

    pub fn configuring(&self, value: ConfiguredValue) -> Result<Transaction, EditError> {
        let live = self.live_setting(value).ok_or(match value {
            ConfiguredValue::Parameter(_) => EditError::MissingParameter,
            ConfiguredValue::Suppressed(_) | ConfiguredValue::Colour(_) => {
                EditError::MissingFeature
            }
        })?;
        let mut table = Configurations::clone(&self.configurations);
        if table.configures(value) {
            return Ok(Transaction::new("Configure", Vec::new()));
        }
        table.values.push(value);
        for row in &mut table.rows {
            row.settings.insert(value, live.clone());
        }
        Ok(self.with_table("Configure a value".to_owned(), table))
    }

    pub fn unconfiguring(&self, value: ConfiguredValue) -> Transaction {
        let mut table = Configurations::clone(&self.configurations);
        table.values.retain(|other| *other != value);
        for row in &mut table.rows {
            row.settings.remove(&value);
        }
        self.with_table("Stop configuring a value".to_owned(), table)
    }

    pub fn setting_configuration(
        &self,
        id: ConfigurationId,
        value: ConfiguredValue,
        setting: Setting,
    ) -> Result<Transaction, EditError> {
        let row = self
            .configurations
            .row(id)
            .ok_or(EditError::MissingConfiguration)?;
        let label = format!("Change {}", row.name);
        if !value.fits(&setting) || !self.configurations.configures(value) {
            return Err(EditError::SettingMismatch);
        }
        if self.configurations.is_active(id) {
            return Ok(Transaction::single(
                label,
                self.setting_edit(value, setting)?,
            ));
        }
        let mut table = Configurations::clone(&self.configurations);
        if let Some(row) = table.rows.iter_mut().find(|row| row.id == id) {
            row.settings.insert(value, setting);
        }
        Ok(self.with_table(label, table))
    }
}
