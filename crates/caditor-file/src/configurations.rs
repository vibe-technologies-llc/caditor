use std::collections::BTreeMap;

use caditor_document::{
    Configuration, ConfigurationId, Configurations, ConfiguredValue, Document, FIRST_UNSTORABLE_ID,
    FeatureId, MAX_CONFIGURATION_NAME_CHARS, MAX_CONFIGURATIONS, MAX_CONFIGURED_VALUES, Rgb,
    Setting, configuration_name,
};
use caditor_expression::{Expression, ParameterId};
use serde::{Deserialize, Serialize};

use crate::format::Lenient;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct ConfigurationsRecord {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<Lenient<ConfiguredValueRecord>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<Lenient<ConfigurationRecord>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<u64>,
    #[serde(default)]
    pub next_id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConfiguredValueRecord {
    Parameter(u64),
    Suppressed(u64),
    Colour(u64),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ConfigurationRecord {
    pub id: u64,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub settings: Vec<Lenient<SettingRecord>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct SettingRecord {
    pub value: ConfiguredValueRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suppressed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colour: Option<String>,
}

fn value_record(value: ConfiguredValue) -> ConfiguredValueRecord {
    match value {
        ConfiguredValue::Parameter(id) => ConfiguredValueRecord::Parameter(id.raw()),
        ConfiguredValue::Suppressed(id) => ConfiguredValueRecord::Suppressed(id.raw()),
        ConfiguredValue::Colour(id) => ConfiguredValueRecord::Colour(id.raw()),
    }
}

fn setting_record(value: ConfiguredValue, setting: &Setting) -> SettingRecord {
    let mut record = SettingRecord {
        value: value_record(value),
        expression: None,
        suppressed: None,
        colour: None,
    };
    match setting {
        Setting::Expression(expression) => record.expression = Some(expression.to_stored_text()),
        Setting::Suppressed(suppressed) => record.suppressed = Some(*suppressed),
        Setting::Colour(colour) => record.colour = colour.map(Rgb::hex),
    }
    record
}

pub(crate) fn configurations_record_of(configurations: &Configurations) -> ConfigurationsRecord {
    ConfigurationsRecord {
        values: configurations
            .values
            .iter()
            .map(|value| Lenient::Read(value_record(*value)))
            .collect(),
        rows: configurations
            .rows
            .iter()
            .map(|row| {
                Lenient::Read(ConfigurationRecord {
                    id: row.id.raw(),
                    name: row.name.clone(),
                    settings: row
                        .settings
                        .iter()
                        .map(|(value, setting)| Lenient::Read(setting_record(*value, setting)))
                        .collect(),
                })
            })
            .collect(),
        active: configurations.active.map(ConfigurationId::raw),
        next_id: configurations.next_id,
    }
}

pub(crate) fn configurations_record(document: &Document) -> Option<ConfigurationsRecord> {
    let configurations = document.configurations();
    (!configurations.is_empty()).then(|| configurations_record_of(configurations))
}

fn storable(raw: u64) -> Option<u64> {
    (raw < FIRST_UNSTORABLE_ID).then_some(raw)
}

fn restore_value(record: ConfiguredValueRecord) -> Option<ConfiguredValue> {
    Some(match record {
        ConfiguredValueRecord::Parameter(raw) => {
            ConfiguredValue::Parameter(ParameterId::from_raw(storable(raw)?))
        }
        ConfiguredValueRecord::Suppressed(raw) => {
            ConfiguredValue::Suppressed(FeatureId::from_raw(storable(raw)?))
        }
        ConfiguredValueRecord::Colour(raw) => {
            ConfiguredValue::Colour(FeatureId::from_raw(storable(raw)?))
        }
    })
}

fn restore_setting(record: &SettingRecord) -> Option<(ConfiguredValue, Setting)> {
    let value = restore_value(record.value)?;
    let setting = match value {
        ConfiguredValue::Parameter(_) => {
            Setting::Expression(Expression::parse_stored(record.expression.as_deref()?).ok()?)
        }
        ConfiguredValue::Suppressed(_) => Setting::Suppressed(record.suppressed?),
        ConfiguredValue::Colour(_) => Setting::Colour(match &record.colour {
            Some(hex) => Some(Rgb::from_hex(hex)?),
            None => None,
        }),
    };
    Some((value, setting))
}

fn fitting_name(name: &str, taken: &Configurations, issues: &mut Vec<String>) -> String {
    let mut name = configuration_name(name);
    if name.is_empty() {
        let numbered = taken.unused_name();
        issues.push(format!(
            "A configuration had no name, so it is called “{numbered}”."
        ));
        return numbered;
    }
    if name.chars().count() > MAX_CONFIGURATION_NAME_CHARS {
        name = name
            .chars()
            .take(MAX_CONFIGURATION_NAME_CHARS)
            .collect::<String>()
            .trim_end()
            .to_owned();
        issues.push(format!(
            "The name of the configuration “{name}” was longer than \
             {MAX_CONFIGURATION_NAME_CHARS} characters, so its end was cut off."
        ));
    }
    if !taken.is_taken(&name) {
        return name;
    }
    let room = MAX_CONFIGURATION_NAME_CHARS - 6;
    let base: String = name.chars().take(room).collect();
    let numbered = (2..)
        .map(|number| format!("{base} {number}"))
        .find(|candidate| !taken.is_taken(candidate))
        .unwrap_or_else(|| name.clone());
    issues.push(format!(
        "Two configurations were called “{name}”, so one is called “{numbered}”."
    ));
    numbered
}

fn restore_values(
    records: Vec<Lenient<ConfiguredValueRecord>>,
    issues: &mut Vec<String>,
) -> Vec<ConfiguredValue> {
    let mut values = Vec::new();
    let mut unreadable = 0_usize;
    for record in records {
        match record {
            Lenient::Read(record) => match restore_value(record) {
                Some(value) if !values.contains(&value) => values.push(value),
                Some(_) => {}
                None => unreadable += 1,
            },
            Lenient::Unreadable(_) => unreadable += 1,
        }
    }
    match unreadable {
        0 => {}
        1 => issues.push(
            "One value set by the configurations could not be read, so it is no longer \
             configured."
                .to_owned(),
        ),
        count => issues.push(format!(
            "{count} values set by the configurations could not be read, so they are no longer \
             configured."
        )),
    }
    if values.len() > MAX_CONFIGURED_VALUES {
        values.truncate(MAX_CONFIGURED_VALUES);
        issues.push(format!(
            "Configurations set at most {MAX_CONFIGURED_VALUES} values, so the rest are no \
             longer configured."
        ));
    }
    values
}

fn restore_settings(
    record: &ConfigurationRecord,
    values: &[ConfiguredValue],
    issues: &mut Vec<String>,
) -> BTreeMap<ConfiguredValue, Setting> {
    let mut settings = BTreeMap::new();
    let mut unreadable = 0_usize;
    for setting in &record.settings {
        let restored = match setting {
            Lenient::Read(setting) => restore_setting(setting),
            Lenient::Unreadable(_) => None,
        };
        match restored {
            Some((value, setting)) if values.contains(&value) => {
                settings.insert(value, setting);
            }
            Some(_) => {}
            None => unreadable += 1,
        }
    }
    let name = &record.name;
    match unreadable {
        0 => {}
        1 => issues.push(format!(
            "One value of the configuration “{name}” could not be read, so switching to it \
             leaves that value as it is."
        )),
        count => issues.push(format!(
            "{count} values of the configuration “{name}” could not be read, so switching to it \
             leaves them as they are."
        )),
    }
    settings
}

pub(crate) fn restore_configurations(
    record: ConfigurationsRecord,
    issues: &mut Vec<String>,
) -> Configurations {
    let mut configurations = Configurations {
        values: restore_values(record.values, issues),
        next_id: storable(record.next_id).unwrap_or(0),
        ..Configurations::default()
    };
    for row in record.rows {
        let Lenient::Read(row) = row else {
            issues.push("A configuration could not be read, so it was left out.".to_owned());
            continue;
        };
        if configurations.rows.len() >= MAX_CONFIGURATIONS {
            issues.push(format!(
                "A model keeps at most {MAX_CONFIGURATIONS} configurations, so the rest were \
                 left out."
            ));
            break;
        }
        let id = ConfigurationId::from_raw(row.id);
        if storable(row.id).is_none() || configurations.row(id).is_some() {
            issues.push(format!(
                "The configuration “{}” could not be told apart from another, so it was left out.",
                row.name
            ));
            continue;
        }
        let settings = restore_settings(&row, &configurations.values, issues);
        let name = fitting_name(&row.name, &configurations, issues);
        configurations.next_id = configurations.next_id.max(row.id.saturating_add(1));
        configurations
            .rows
            .push(Configuration { id, name, settings });
    }
    configurations.active = record
        .active
        .map(ConfigurationId::from_raw)
        .filter(|active| configurations.row(*active).is_some());
    if record.active.is_some() && configurations.active.is_none() {
        issues.push(
            "Which configuration was active could not be read, so none is active; the model keeps \
             its values."
                .to_owned(),
        );
    }
    configurations
}
